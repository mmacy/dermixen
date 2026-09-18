//! The library panel: the tracks the library contains as the rows of a
//! table, sorted by a column, narrowed by filters and a search, highlighted
//! by harmonic compatibility, and added to the mix.
//!
//! `DESIGN.md` describes the table under "Feature kernel", and names
//! harmonic mixing there: when a person selects a track, the window
//! highlights the tracks in the library that mix well with it, by the
//! Camelot codes analysis gave them. The panel is plain state the window
//! paints as rows. The window reads the records from the library once and
//! hands them here, and hands the edit the panel builds to the timeline.

use dermixen_analysis::{Camelot, Letter};
use dermixen_core::{
    Anchors, Bpm, ContentHash, DEFAULT_BARS, Decibels, Edit, Envelope, EqEnvelopes, Mix, Preset,
    Seconds, Track, leveling_gain, outro_for,
};
use dermixen_library::{MetadataSource, TrackRecord};
use std::cmp::Ordering;
use std::path::PathBuf;

/// A time as minutes, seconds, and tenths, as in `5:00.0`, which is how
/// every time in the window is written: the length in a row of the library
/// table, and the times the window paints around the timeline.
pub fn time_text(at: Seconds) -> String {
    if !at.0.is_finite() {
        return "--:--".to_owned();
    }
    let total = at.0.max(0.0);
    let minutes = (total / 60.0).floor();
    let seconds = total - minutes * 60.0;
    format!("{minutes}:{seconds:04.1}")
}

/// What the status line says about the rows of a library a query could not
/// read, which is nothing when it could read every row.
///
/// A row with a value of the wrong type, text that is not UTF-8, or a number
/// a mix document would refuse is left out of the panel and counted by
/// [`dermixen_library::Index::query_with_skipped`], so one damaged row costs
/// the person that row rather than the whole panel. A scan of the music
/// folder reads the file again and replaces the record, which is the remedy
/// this names.
pub fn skipped_note(skipped: usize) -> String {
    match skipped {
        0 => String::new(),
        1 => "1 row of the library could not be read, so the panel leaves it out. Library > Scan \
              music folder replaces it."
            .to_owned(),
        many => format!(
            "{many} rows of the library could not be read, so the panel leaves them out. Library > \
             Scan music folder replaces them."
        ),
    }
}

/// One column of the table, in the order the window draws them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    /// The artist.
    Artist,
    /// The title.
    Title,
    /// The year the music was first released.
    Year,
    /// The tempo.
    Bpm,
    /// The Camelot code.
    Key,
    /// The record label.
    Label,
    /// The label's catalog number.
    CatalogNumber,
    /// The title of the release.
    Release,
    /// The track's position on the release.
    TrackNumber,
    /// The length of the audio.
    Duration,
}

impl Column {
    /// Every column, left to right.
    pub const ALL: [Column; 10] = [
        Column::Artist,
        Column::Title,
        Column::Year,
        Column::Bpm,
        Column::Key,
        Column::Label,
        Column::CatalogNumber,
        Column::Release,
        Column::TrackNumber,
        Column::Duration,
    ];

    /// The heading the window draws over the column.
    pub fn heading(self) -> &'static str {
        match self {
            Column::Artist => "Artist",
            Column::Title => "Title",
            Column::Year => "Year",
            Column::Bpm => "BPM",
            Column::Key => "Key",
            Column::Label => "Label",
            Column::CatalogNumber => "Catalog no.",
            Column::Release => "Release",
            Column::TrackNumber => "Track no.",
            Column::Duration => "Duration",
        }
    }
}

/// Which column the rows are sorted by, and which way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    /// The column.
    pub column: Column,
    /// Whether the greatest value comes first. Rows with no value in the
    /// column come last whichever way the sort runs.
    pub descending: bool,
}

/// The conditions the rows are narrowed by, one per field. Every condition
/// given must hold. Text that is empty or only spaces, an empty list of
/// keys, and an open end of a range place no condition, so the default
/// narrows nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters {
    /// Only tracks whose artist contains this text, with the spaces around
    /// the text ignored and the trimmed text compared without regard to
    /// case. A track with no artist does not match.
    pub artist: String,
    /// Only tracks whose title contains this text, with the spaces around
    /// the text ignored and the trimmed text compared without regard to
    /// case. A track with no title does not match.
    pub title: String,
    /// Only tracks whose key is one of the keys in this list. A track with
    /// no key does not match.
    pub keys: Vec<Camelot>,
    /// Only tracks whose label contains this text, with the spaces around
    /// the text ignored and the trimmed text compared without regard to
    /// case. A track with no label does not match.
    pub label: String,
    /// Only tracks whose year is at least this. A track with no year does
    /// not match, and an estimated year counts as its year.
    pub year_from: Option<u16>,
    /// Only tracks whose year is at most this. A track with no year does
    /// not match.
    pub year_to: Option<u16>,
    /// Only tracks whose tempo is at least this.
    pub bpm_from: Option<Bpm>,
    /// Only tracks whose tempo is at most this.
    pub bpm_to: Option<Bpm>,
    /// Only tracks whose file path contains this text, with the spaces
    /// around the text ignored and the trimmed text compared without regard
    /// to case.
    pub path: String,
}

/// One track as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryRow {
    /// The track's content hash.
    pub hash: ContentHash,
    /// The file.
    pub path: PathBuf,
    /// The artist, if known.
    pub artist: Option<String>,
    /// The title, if known.
    pub title: Option<String>,
    /// The year, if known.
    pub year: Option<u16>,
    /// Whether the year is an estimate rather than a year some source
    /// states. The window writes `about` before such a year.
    pub year_is_approximate: bool,
    /// The title of the release the track came from, when a Discogs lookup
    /// has recorded one.
    pub release_title: Option<String>,
    /// The record label, when a Discogs lookup has recorded one.
    pub label: Option<String>,
    /// The label's catalog number, when a Discogs lookup has recorded one.
    pub catalog_number: Option<String>,
    /// The track's position on the release, counting from one, when a
    /// Discogs lookup has recorded one.
    pub track_number: Option<u16>,
    /// Whether the artist and title were guessed from the file name rather
    /// than read from its tags, which the window shows differently.
    pub guessed: bool,
    /// The track's tempo.
    pub bpm: Bpm,
    /// The track's length.
    pub length: Seconds,
    /// The track's Camelot code, if a key analyzer answered.
    pub camelot: Option<Camelot>,
    /// Whether the track mixes well with the reference key, by
    /// [`Camelot::is_compatible_with`], and never when the track has no key or
    /// there is no reference.
    pub compatible: bool,
    /// Whether a track with this hash is in the mix.
    pub in_mix: bool,
    /// Whether this is the selected row.
    pub selected: bool,
}

impl LibraryRow {
    /// The year as the window writes it: the year on its own, `about` and
    /// the year when the year is an estimate, or `no year`.
    pub fn year_text(&self) -> String {
        match self.year {
            Some(year) if self.year_is_approximate => format!("about {year}"),
            Some(year) => format!("{year}"),
            None => "no year".to_owned(),
        }
    }

    /// The text the window draws in one cell of the row. The artist is
    /// `Unknown` when the track has none, and the title is the file's name
    /// when the track has none. The year is [`year_text`](LibraryRow::year_text).
    /// The tempo has one decimal place, as in `140.0`. The key is the Camelot
    /// code, or `no key`. The length is minutes, seconds, and tenths, as in
    /// `5:00.0`, which is how every time in the window is written. The
    /// label, the catalog number, the release title, and the track number
    /// are empty text when the library has none.
    pub fn cell(&self, column: Column) -> String {
        match column {
            Column::Artist => self
                .artist
                .clone()
                .unwrap_or_else(|| UNKNOWN_ARTIST.to_owned()),
            Column::Title => self.title.clone().unwrap_or_else(|| self.file_name()),
            Column::Year => self.year_text(),
            Column::Bpm => format!("{:.1}", self.bpm.0),
            Column::Key => match self.camelot {
                Some(camelot) => camelot.to_string(),
                None => NO_KEY.to_owned(),
            },
            Column::Label => self.label.clone().unwrap_or_default(),
            Column::CatalogNumber => self.catalog_number.clone().unwrap_or_default(),
            Column::Release => self.release_title.clone().unwrap_or_default(),
            Column::TrackNumber => self
                .track_number
                .map(|number| number.to_string())
                .unwrap_or_default(),
            Column::Duration => time_text(self.length),
        }
    }

    /// The name of the track's file, which stands in the title cell for a
    /// track the library has no title for.
    fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// What the panel writes in the artist cell of a track the library has no
/// artist for.
const UNKNOWN_ARTIST: &str = "Unknown";

/// What the panel writes in the key cell of a track no key analyzer
/// answered for.
const NO_KEY: &str = "no key";

/// The library as the panel shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryPanel {
    /// Every record the window read from the library, in the order it read
    /// them.
    records: Vec<TrackRecord>,
    /// Positions into `records` in the order of the sort, with every record
    /// present.
    sorted: Vec<usize>,
    /// Positions into `records` for the rows shown now: the positions in
    /// `sorted` that the filters and the search leave in. The rows are
    /// worked out when the sort, the filters, or the search changes rather
    /// than on every call to [`LibraryPanel::rows`], because the window
    /// paints the rows many times over for every character typed.
    shown: Vec<usize>,
    /// The search text as given.
    search: String,
    /// The column the rows are sorted by, and which way.
    sort: Sort,
    /// The conditions the rows are narrowed by.
    filters: Filters,
    /// The key the rows are highlighted against.
    reference: Option<Camelot>,
    /// The selected track. The panel remembers it by hash and keeps the
    /// selection when a search leaves the track out.
    selected: Option<ContentHash>,
    /// The hashes of the tracks in the mix.
    in_mix: Vec<ContentHash>,
}

/// What a column sorts a track by: text compared without regard to case, a
/// number, or nothing at all where the track has no value in the column.
#[derive(Debug, Clone, PartialEq)]
enum SortKey {
    /// Text, already folded to lowercase.
    Text(String),
    /// A number. The key column is a number too, `2` for every step around
    /// the wheel and `1` more for `B` than for `A`, so that the codes run
    /// `1A`, `1B`, `2A`, and on to `12B`.
    Number(f64),
    /// The track has no value in the column.
    Missing,
}

impl SortKey {
    /// Whether the track has no value in the column.
    fn missing(&self) -> bool {
        matches!(self, SortKey::Missing)
    }

    /// How two keys of one column compare, ascending. Two keys of different
    /// kinds never meet, because every row takes its key for a column from
    /// the same field.
    fn compare(&self, other: &SortKey) -> Ordering {
        match (self, other) {
            (SortKey::Text(left), SortKey::Text(right)) => left.cmp(right),
            (SortKey::Number(left), SortKey::Number(right)) => left.total_cmp(right),
            _ => Ordering::Equal,
        }
    }
}

/// What `column` sorts `record` by.
fn sort_key(record: &TrackRecord, column: Column) -> SortKey {
    let text = |value: Option<&String>| match value {
        Some(value) => SortKey::Text(value.to_lowercase()),
        None => SortKey::Missing,
    };
    let number = |value: Option<u16>| match value {
        Some(value) => SortKey::Number(f64::from(value)),
        None => SortKey::Missing,
    };
    match column {
        Column::Artist => text(record.metadata.artist.as_ref()),
        Column::Title => text(record.metadata.title.as_ref()),
        Column::Year => number(record.metadata.year),
        Column::Bpm => SortKey::Number(record.grid.bpm.0),
        Column::Key => match &record.key {
            Some(key) => {
                let letter = match key.camelot.letter() {
                    Letter::A => 0.0,
                    Letter::B => 1.0,
                };
                SortKey::Number(f64::from(key.camelot.number()) * 2.0 + letter)
            }
            None => SortKey::Missing,
        },
        Column::Label => text(record.release.label.as_ref()),
        Column::CatalogNumber => text(record.release.catalog_number.as_ref()),
        Column::Release => text(record.release.title.as_ref()),
        Column::TrackNumber => number(record.release.track_number),
        Column::Duration => SortKey::Number(record.length.to_seconds().0),
    }
}

/// How two keys order the rows they belong to: a track with no value in the
/// column comes after every track with one, whichever way the sort runs, and
/// every other pair runs the way `descending` says.
fn ordered(left: &SortKey, right: &SortKey, descending: bool) -> Ordering {
    match (left.missing(), right.missing()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let order = left.compare(right);
            if descending { order.reverse() } else { order }
        }
    }
}

/// The order two records keep when they tie on the sorted column: the artist
/// ascending, then the title, then the path, whichever way the sort runs,
/// with a missing artist or title after every present one.
fn tie_break(left: &TrackRecord, right: &TrackRecord) -> Ordering {
    ordered(
        &sort_key(left, Column::Artist),
        &sort_key(right, Column::Artist),
        false,
    )
    .then_with(|| {
        ordered(
            &sort_key(left, Column::Title),
            &sort_key(right, Column::Title),
            false,
        )
    })
    .then_with(|| left.path.cmp(&right.path))
}

/// The text a search looks through, folded to lowercase: the artist, the
/// title, the year as digits, the tempo with one decimal place, the Camelot
/// code, the label, the catalog number, the release title, the track number,
/// and the file path, each on a line of its own so that no word is found
/// across two of them. A value the library has none of leaves its line out,
/// so the placeholders the table draws in an empty cell are not searched.
fn searchable(record: &TrackRecord) -> String {
    let mut text = String::new();
    let mut line = |value: &str| {
        text.push_str(&value.to_lowercase());
        text.push('\n');
    };
    if let Some(artist) = &record.metadata.artist {
        line(artist);
    }
    if let Some(title) = &record.metadata.title {
        line(title);
    }
    if let Some(year) = record.metadata.year {
        line(&year.to_string());
    }
    line(&format!("{:.1}", record.grid.bpm.0));
    if let Some(key) = &record.key {
        line(&key.camelot.to_string());
    }
    if let Some(label) = &record.release.label {
        line(label);
    }
    if let Some(catalog_number) = &record.release.catalog_number {
        line(catalog_number);
    }
    if let Some(release) = &record.release.title {
        line(release);
    }
    if let Some(number) = record.release.track_number {
        line(&number.to_string());
    }
    line(&record.path.to_string_lossy());
    text
}

/// Whether every word of the search is found among the values
/// [`searchable`] gathers. A search of no words leaves every record in.
fn matches_the_search(record: &TrackRecord, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let text = searchable(record);
    words.iter().all(|word| text.contains(word))
}

/// Whether `text` places a condition on the rows, which empty text and text
/// of only spaces do not.
fn a_condition(text: &str) -> bool {
    !text.trim().is_empty()
}

/// Whether `value` contains `text`, with the spaces around `text` ignored
/// and the trimmed text compared without regard to case. A field the library
/// has no value for never matches.
fn contains(value: Option<&String>, text: &str) -> bool {
    value.is_some_and(|value| value.to_lowercase().contains(&text.trim().to_lowercase()))
}

impl LibraryPanel {
    /// A panel over the library's records, with no search, no reference key,
    /// no selection, and nothing in the mix. A panel over no records shows
    /// no rows.
    pub fn new(records: Vec<TrackRecord>) -> LibraryPanel {
        let mut panel = LibraryPanel {
            records,
            sorted: Vec::new(),
            shown: Vec::new(),
            search: String::new(),
            sort: Sort {
                column: Column::Artist,
                descending: false,
            },
            filters: Filters::default(),
            reference: None,
            selected: None,
            in_mix: Vec::new(),
        };
        panel.sort_the_records();
        panel.narrow_the_rows();
        panel
    }

    /// Replaces the records with the library as it now stands, which the
    /// window does after a scan adds tracks, keeping the search, the
    /// filters, the sort, and the tracks marked as in the mix.
    ///
    /// The selection is the track rather than the row, so a track whose
    /// record is among the new ones stays selected wherever the sort and the
    /// search now put it, and a track the library no longer contains is
    /// selected no more. The key the rows are highlighted against goes with
    /// that track when the track goes, since the key came from its record.
    /// The window sets the key again from the track selected on the timeline
    /// after every one of these calls, so the highlighting follows the
    /// timeline rather than being lost. Call [`rows`](LibraryPanel::rows)
    /// afterwards for the rows to paint.
    pub fn set_records(&mut self, records: Vec<TrackRecord>) {
        self.records = records;
        // The selection is a track rather than a row, so it lives through
        // the rows moving under it. A track the library no longer contains
        // cannot be selected at all, and the key the rows were highlighted
        // against went with that track, so both go.
        if let Some(hash) = self.selected
            && !self.records.iter().any(|record| record.hash == hash)
        {
            self.selected = None;
            self.reference = None;
        }
        self.sort_the_records();
        self.narrow_the_rows();
    }

    /// Sets the search text. Text that is empty or only spaces places no
    /// condition. Anything else is split into words at spaces, and a track
    /// matches when every word is found, without regard to case, in at least
    /// one of these: the artist, the title, the year as digits, the tempo
    /// with one decimal place, the Camelot code, the label, the catalog
    /// number, the release title, the track number, or the file path. A word
    /// matches wherever it occurs among those values. A value the track does
    /// not have is left out of the text that is searched, so the words
    /// `Unknown`, `about`, `no year`, and `no key` that
    /// [`cell`](LibraryRow::cell) writes in place of a missing value are
    /// nowhere in that text and match only where a track's own values hold
    /// them, as the path of a track under a folder named `Unknown Artist`
    /// does. The length is not searched at all. Two words may match in two
    /// different fields. Every track that matches is shown, in the order of
    /// the sort, with no limit on the number.
    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_owned();
        self.narrow_the_rows();
    }

    /// The search text as given.
    pub fn search(&self) -> &str {
        &self.search
    }

    /// Sorts the rows by `column`, as a click on its heading does: a column
    /// other than the sorted one sorts ascending, and the sorted column
    /// reverses the order. Rows with no value in the column come after
    /// every row with one, whichever way the sort runs. Text is compared
    /// without regard to case. The key is ordered around the wheel, by
    /// number and then letter, so `12A` comes after `3B`. Rows that tie on
    /// the column keep the order of the artist ascending, then the title,
    /// then the path, whichever way the sort runs, with a missing artist or
    /// title after every present one.
    pub fn click_heading(&mut self, column: Column) {
        self.sort = Sort {
            column,
            descending: self.sort.column == column && !self.sort.descending,
        };
        self.sort_the_records();
        self.narrow_the_rows();
    }

    /// The column the rows are sorted by, and which way. The panel starts
    /// sorted by the artist, ascending.
    pub fn sort(&self) -> Sort {
        self.sort
    }

    /// Sets the conditions the rows are narrowed by. Every condition and the
    /// search must hold for a row to show. Setting filters that leave the
    /// selected track out keeps the selection, as a search does.
    pub fn set_filters(&mut self, filters: Filters) {
        self.filters = filters;
        self.narrow_the_rows();
    }

    /// The conditions the rows are narrowed by.
    pub fn filters(&self) -> &Filters {
        &self.filters
    }

    /// Sets the key the rows are highlighted against, as the window does
    /// when a track is selected on the timeline, without changing the
    /// panel's own selection.
    pub fn set_reference(&mut self, key: Option<Camelot>) {
        self.reference = key;
    }

    /// The key the rows are highlighted against.
    pub fn reference(&self) -> Option<Camelot> {
        self.reference
    }

    /// Tells the panel which tracks are in the mix, by their hashes.
    pub fn set_mix(&mut self, mix: &Mix) {
        self.in_mix = mix.tracks.iter().map(|track| track.hash).collect();
    }

    /// Selects the row at `row` among the rows shown, and makes its key the
    /// reference, or clears the reference when it has no key. A row past
    /// the last one changes nothing. The selection is the track, not the
    /// row: it survives a search that leaves the track out, and the track
    /// is selected again when a later search shows it.
    pub fn select(&mut self, row: usize) {
        let Some(record) = self.shown.get(row).map(|position| &self.records[*position]) else {
            return;
        };
        self.selected = Some(record.hash);
        self.reference = record.key.as_ref().map(|key| key.camelot);
    }

    /// The selected row among the rows shown, if the selected track is
    /// among them.
    pub fn selected(&self) -> Option<usize> {
        let hash = self.selected?;
        self.shown
            .iter()
            .position(|position| self.records[*position].hash == hash)
    }

    /// The rows to paint: every track that the filters and the search leave
    /// in, in the order of the sort.
    pub fn rows(&self) -> Vec<LibraryRow> {
        self.shown
            .iter()
            .map(|position| self.row(&self.records[*position]))
            .collect()
    }

    /// The edit that inserts the track at `row` into the mix at position
    /// `at`, joined to its neighbors with `preset`. The track has the
    /// record's path, hash, length, and grid. Its intro anchor is the
    /// record's, and its outro anchor is [`outro_for`](dermixen_core::outro_for)
    /// the record's anchors and the preset's bars, or
    /// [`DEFAULT_BARS`](dermixen_core::DEFAULT_BARS) for a preset with no
    /// length of its own, the blend and the cut, which is where `dermixen
    /// mix add` places it when no bar count is given. Keylock is on, as `mix add` leaves it unless told
    /// otherwise, and every envelope is empty. The track's gain is the
    /// [`leveling_gain`](dermixen_core::leveling_gain) of the record's
    /// loudness, and unity for a record the library has no loudness
    /// measurement for, which is what `mix add` writes.
    ///
    /// A row past the last one gives nothing. The timeline may still refuse
    /// the edit, as it refuses any insert whose transition does not fit
    /// between the anchors, and its refusal names the reason. The panel does
    /// not check that itself.
    pub fn insert_edit(&self, row: usize, at: usize, preset: Preset) -> Option<Edit> {
        let record = &self.records[*self.shown.get(row)?];
        let bars = match preset {
            Preset::Beatmix { bars } | Preset::BassSwap { bars } => bars,
            Preset::Blend | Preset::Cut => DEFAULT_BARS,
        };
        let track = Track {
            path: record.path.clone(),
            hash: record.hash,
            length: record.length,
            grid: record.grid,
            anchors: Anchors {
                intro: record.anchors.intro,
                outro: outro_for(record.anchors, bars),
            },
            keylock: true,
            gain: match record.loudness {
                Some(loudness) => leveling_gain(loudness.integrated, loudness.true_peak),
                None => Decibels::UNITY,
            },
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        };
        Some(Edit::InsertTrack {
            at,
            track: Box::new(track),
            preset,
        })
    }

    /// Puts every record in the order of the sort: the sorted column, and
    /// then the artist, the title, and the path where two records tie on it.
    fn sort_the_records(&mut self) {
        let Sort { column, descending } = self.sort;
        let mut sorted: Vec<usize> = (0..self.records.len()).collect();
        sorted.sort_by(|left, right| {
            let left = &self.records[*left];
            let right = &self.records[*right];
            ordered(
                &sort_key(left, column),
                &sort_key(right, column),
                descending,
            )
            .then_with(|| tie_break(left, right))
        });
        self.sorted = sorted;
    }

    /// Works out which of the sorted records to show: the ones the filters
    /// and the search leave in, in the order the sort put them.
    fn narrow_the_rows(&mut self) {
        let words: Vec<String> = self
            .search
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let shown: Vec<usize> = self
            .sorted
            .iter()
            .copied()
            .filter(|position| {
                let record = &self.records[*position];
                self.passes_the_filters(record) && matches_the_search(record, &words)
            })
            .collect();
        self.shown = shown;
    }

    /// Whether `record` meets every condition the filters place. A track
    /// with no value in a filtered field meets no condition on that field.
    fn passes_the_filters(&self, record: &TrackRecord) -> bool {
        let filters = &self.filters;
        if a_condition(&filters.artist)
            && !contains(record.metadata.artist.as_ref(), &filters.artist)
        {
            return false;
        }
        if a_condition(&filters.title) && !contains(record.metadata.title.as_ref(), &filters.title)
        {
            return false;
        }
        if a_condition(&filters.label) && !contains(record.release.label.as_ref(), &filters.label) {
            return false;
        }
        if !filters.keys.is_empty() {
            let chosen = record
                .key
                .as_ref()
                .is_some_and(|key| filters.keys.contains(&key.camelot));
            if !chosen {
                return false;
            }
        }
        if filters.year_from.is_some() || filters.year_to.is_some() {
            let Some(year) = record.metadata.year else {
                return false;
            };
            if filters.year_from.is_some_and(|from| year < from) {
                return false;
            }
            if filters.year_to.is_some_and(|to| year > to) {
                return false;
            }
        }
        let bpm = record.grid.bpm.0;
        if filters.bpm_from.is_some_and(|from| bpm < from.0) {
            return false;
        }
        if filters.bpm_to.is_some_and(|to| bpm > to.0) {
            return false;
        }
        if a_condition(&filters.path) {
            let path = record.path.to_string_lossy().to_lowercase();
            if !path.contains(&filters.path.trim().to_lowercase()) {
                return false;
            }
        }
        true
    }

    /// One record as a row, with the flags the window paints it by.
    fn row(&self, record: &TrackRecord) -> LibraryRow {
        let camelot = record.key.as_ref().map(|key| key.camelot);
        let compatible = match (camelot, self.reference) {
            (Some(camelot), Some(reference)) => camelot.is_compatible_with(reference),
            _ => false,
        };
        LibraryRow {
            hash: record.hash,
            path: record.path.clone(),
            artist: record.metadata.artist.clone(),
            title: record.metadata.title.clone(),
            year: record.metadata.year,
            year_is_approximate: record.metadata.year_is_approximate,
            release_title: record.release.title.clone(),
            label: record.release.label.clone(),
            catalog_number: record.release.catalog_number.clone(),
            track_number: record.release.track_number,
            guessed: record.metadata.source == MetadataSource::Filename,
            bpm: record.grid.bpm,
            length: record.length.to_seconds(),
            camelot,
            compatible,
            in_mix: self.in_mix.contains(&record.hash),
            selected: self.selected == Some(record.hash),
        }
    }
}
