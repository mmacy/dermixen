//! The SQLite library file, which contains everything the library knows
//! about each track.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use dermixen_analysis::{Camelot, Extent, Key, Loudness};
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Lufs, Samples, Seconds};
use rusqlite::types::{Type, Value};
use rusqlite::{Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::metadata::{Metadata, MetadataSource, Release, ReleaseDataSource};

/// The version of the library file's layout this crate reads and writes.
///
/// The layout is version 5, and it is the only layout this crate reads or
/// writes. A file that nobody has written yet, which contains no tables and
/// no version, is laid out as version 5. A library file of any other version
/// is refused with an error that names the version the file contains, and the
/// remedy is to scan again into a fresh file, since everything in the library
/// can be recomputed from the audio.
pub const SCHEMA_VERSION: u32 = 5;

/// A bar that starts a phrase, as the library stores it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhraseStartRecord {
    /// The bar's first beat, a whole beat of the track's grid.
    pub beat: Beats,
    /// The longest phrase that begins at this bar, in bars: eight, sixteen,
    /// or thirty-two.
    pub bars: u32,
}

/// What the phrase analyzer found in a track: which beat starts a bar, the
/// bars that start phrases, and the bars at which the arrangement changes.
///
/// The phrase analysis describes the grid the track's record contains and does
/// not move it: beat zero stays where the beat analyzer put it, and
/// `downbeat` says which beat of every four starts a bar. Every beat here is
/// a whole beat of that grid, and every phrase start and section change is
/// a bar start under `downbeat`. The app draws the phrase starts and the
/// section changes on the timeline and moves no anchor onto them, as
/// `DESIGN.md` states under "Analysis strategy". The confidence is the
/// analyzer's own figure. `docs/ground-truth.md` records that on the labeled
/// tracks the confidence does not separate the tracks the analyzer gets right
/// from the ones it gets wrong, and nothing acts on the confidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhraseRecord {
    /// Which analyzer answered.
    pub analyzer: String,
    /// How sure the analyzer was of the phrase structure, from zero to one.
    pub confidence: f64,
    /// Which beat of the grid starts a bar, from zero to three: bars begin
    /// at every beat that is this many beats past a whole multiple of four.
    pub downbeat: u32,
    /// Every bar that starts a phrase, in time order.
    pub starts: Vec<PhraseStartRecord>,
    /// Every bar at which the arrangement changes, in time order, as the
    /// bar's first beat.
    pub sections: Vec<Beats>,
}

/// The musical key found for a track.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeyRecord {
    /// The key.
    pub key: Key,
    /// The key's Camelot code, which follows from the key.
    pub camelot: Camelot,
    /// How sure the analyzer was, from zero to one.
    pub confidence: f64,
    /// Which analyzer answered.
    pub analyzer: String,
}

/// Everything the library knows about one track.
///
/// The record is keyed by the content hash: a file that moves keeps its
/// record with a new path, and a file whose bytes change is a new track.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackRecord {
    /// The hash of the file's bytes, which identifies the track.
    pub hash: ContentHash,
    /// Where the file was last seen.
    pub path: PathBuf,
    /// The length of the decoded audio.
    #[serde(rename = "length_samples")]
    pub length: Samples,
    /// The beat grid.
    pub grid: BeatGrid,
    /// How sure the beat analyzer was, from zero to one.
    pub grid_confidence: f64,
    /// Which beat analyzer answered.
    pub grid_analyzer: String,
    /// The key, or `None` when no key analyzer answered.
    pub key: Option<KeyRecord>,
    /// Where the music effectively begins and ends.
    pub extent: Extent,
    /// The anchors analysis placed, which are where a mix's own copy starts from.
    pub anchors: Anchors,
    /// How sure the anchor analyzer was, from zero to one.
    pub anchor_confidence: f64,
    /// Which anchor analyzer answered.
    pub anchor_analyzer: String,
    /// The artist, title, and year, and where they came from.
    pub metadata: Metadata,
    /// The bar lines, phrase starts, and section changes the phrase analyzer
    /// found, or `None` when the phrase analyzer failed on the track.
    pub phrases: Option<PhraseRecord>,
    /// The loudness analysis, or `None` for a track the meter finds nothing
    /// in, which is one quieter than the meter's gate, shorter than its
    /// block, or silent.
    pub loudness: Option<Loudness>,
    /// What Discogs says about the release this track came from. A scan never
    /// fills this, because tags name the pressing a file was ripped from
    /// rather than the release the music first appeared on. It is empty until
    /// a Discogs lookup records a match.
    #[serde(default)]
    pub release: Release,
}

/// The reason the library file could not be opened or used.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// The file could not be opened or created.
    #[error("cannot open the library file {path}: {message}")]
    Open {
        /// The library file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
    /// The file is a library file whose version is not the one this crate
    /// reads, whether lower or higher.
    #[error(
        "the library file {path} is version {found}, and this dermixen reads version {expected}, so scan again into a new library file"
    )]
    Schema {
        /// The library file.
        path: PathBuf,
        /// The version the file contains.
        found: u32,
        /// The version this crate reads.
        expected: u32,
    },
    /// A read or write failed.
    #[error("the library could not be read or written: {0}")]
    Storage(String),
}

/// The conditions a query places on tracks. Every condition given must hold;
/// a query with no conditions matches every track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    /// Only tracks whose file lies in this folder or one below it.
    pub under: Option<PathBuf>,
    /// Only tracks whose tempo lies in this range, both ends included.
    pub bpm: Option<(Bpm, Bpm)>,
    /// Only tracks whose decoded length lies in this range, both ends included.
    pub length: Option<(Seconds, Seconds)>,
    /// Only tracks whose year lies in this range, both ends included. A
    /// track with no year does not match.
    pub year: Option<(u16, u16)>,
    /// Only tracks in this key. A track with no key does not match.
    pub key: Option<Camelot>,
    /// Only tracks whose key mixes well with this one, by
    /// [`Camelot::is_compatible_with`]. A track with no key does not match.
    pub compatible_with: Option<Camelot>,
    /// Only tracks whose artist contains this text, compared without regard
    /// to case. A track with no artist does not match.
    pub artist: Option<String>,
    /// Only tracks whose title contains this text, compared without regard
    /// to case. A track with no title does not match.
    pub title: Option<String>,
    /// When set, only tracks whose year is not an estimate match: a track
    /// whose `year_is_approximate` mark is set does not, whether or not it
    /// has a year. A track with no year and no mark has no estimate, so this
    /// condition on its own leaves that track in, and a year range leaves it
    /// out as a year range does without this condition.
    pub exclude_approximate_years: bool,
    /// Only tracks whose beat grid scored at least this confidence, the
    /// bound included. A track scoring exactly the bound matches.
    pub min_grid_confidence: Option<f64>,
    /// Only tracks whose anchors scored at least this confidence, the bound
    /// included. A track scoring exactly the bound matches.
    pub min_anchor_confidence: Option<f64>,
}

/// The tables and indexes a library file contains.
///
/// A path is stored as text. A path that is not valid UTF-8 is stored
/// lossily, with a replacement character in place of every byte that is not
/// text, so the stored path no longer names the file. The record is still
/// found, because the hash and not the path identifies a track, but every
/// later scan of that file reports it as moved and writes the same lossy
/// text back.
const SCHEMA: &str = "\
CREATE TABLE tracks (
    hash              TEXT PRIMARY KEY NOT NULL,
    path              TEXT NOT NULL,
    length_samples    INTEGER NOT NULL,
    first_beat_sample INTEGER NOT NULL,
    bpm               REAL NOT NULL,
    grid_confidence   REAL NOT NULL,
    grid_analyzer     TEXT NOT NULL,
    key_name          TEXT,
    key_camelot       TEXT,
    key_confidence    REAL,
    key_analyzer      TEXT,
    begins_sample     INTEGER NOT NULL,
    ends_sample       INTEGER NOT NULL,
    intro_beat        REAL NOT NULL,
    outro_beat        REAL NOT NULL,
    anchor_confidence REAL NOT NULL,
    anchor_analyzer   TEXT NOT NULL,
    artist            TEXT,
    title             TEXT,
    year              INTEGER,
    metadata_source   TEXT NOT NULL,
    phrases           TEXT,
    loudness_lufs     REAL,
    true_peak_db      REAL,
    label             TEXT,
    catalog_number    TEXT,
    release_title     TEXT,
    track_number      INTEGER,
    release_data_source TEXT,
    year_is_approximate INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX tracks_by_path ON tracks (path);";

/// The columns of `tracks`, in the order [`read_record`] reads them.
const COLUMNS: &str = "hash, path, length_samples, first_beat_sample, bpm, \
grid_confidence, grid_analyzer, key_name, key_camelot, key_confidence, \
key_analyzer, begins_sample, ends_sample, intro_beat, outro_beat, \
anchor_confidence, anchor_analyzer, artist, title, year, metadata_source, phrases, \
loudness_lufs, true_peak_db, label, catalog_number, release_title, track_number, \
release_data_source, year_is_approximate";

/// An open library file.
pub struct Index {
    connection: rusqlite::Connection,
    path: PathBuf,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index").field("path", &self.path).finish()
    }
}

impl Index {
    /// Opens the library file at `path`, laying out version 5 in the file when
    /// nobody has written to it yet.
    ///
    /// A file whose version is anything other than 5 is refused with
    /// [`IndexError::Schema`], which names the version the file contains and
    /// the version this crate reads. The remedy is to scan again into a fresh
    /// file, since everything in the library can be recomputed from the audio.
    pub fn open(path: &Path) -> Result<Index, IndexError> {
        // Everything that can go wrong before the library file is open names
        // the file, because a person holding an error about a file they
        // named needs to read which file it was: one that is not a database
        // at all, one in a folder nothing can be written to, one another
        // program keeps locked.
        let opening = |problem: rusqlite::Error| IndexError::Open {
            path: path.to_path_buf(),
            message: problem.to_string(),
        };
        // SQLite reads nothing until it is asked to, so a file that is not a
        // database opens here and fails at the first query below.
        let mut connection = Connection::open(path).map_err(opening)?;
        let version: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(opening)?;
        let empty: i64 = connection
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
            .map_err(opening)?;
        // A file nobody has written yet contains no tables and no version, and
        // that is the one file this crate is free to lay out for itself. The
        // tables and the version are written together, so an interruption
        // part way through leaves the file as empty as it was rather than
        // leaving tables behind that no version claims.
        if version == 0 && empty == 0 {
            let laying_out = connection.transaction().map_err(opening)?;
            laying_out.execute_batch(SCHEMA).map_err(opening)?;
            laying_out
                .pragma_update(None, "user_version", SCHEMA_VERSION)
                .map_err(opening)?;
            laying_out.commit().map_err(opening)?;
        } else if version != SCHEMA_VERSION {
            return Err(IndexError::Schema {
                path: path.to_path_buf(),
                found: version,
                expected: SCHEMA_VERSION,
            });
        }
        Ok(Index {
            connection,
            path: path.to_path_buf(),
        })
    }

    /// Where this library file lives.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stores a record, replacing any record with the same hash.
    pub fn upsert(&mut self, record: &TrackRecord) -> Result<(), IndexError> {
        let key = record.key.as_ref();
        let (integrated, true_peak) = loudness_columns(record.loudness.as_ref());
        self.connection
            .execute(
                &format!(
                    "INSERT OR REPLACE INTO tracks ({COLUMNS}) VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)"
                ),
                rusqlite::params![
                    record.hash.to_string(),
                    path_text(&record.path),
                    record.length.0,
                    record.grid.first_beat.0,
                    record.grid.bpm.0,
                    record.grid_confidence,
                    record.grid_analyzer,
                    key.map(|key| key.key.to_string()),
                    key.map(|key| key.camelot.to_string()),
                    key.map(|key| key.confidence),
                    key.map(|key| key.analyzer.clone()),
                    record.extent.begins.0,
                    record.extent.ends.0,
                    record.anchors.intro.0,
                    record.anchors.outro.0,
                    record.anchor_confidence,
                    record.anchor_analyzer,
                    record.metadata.artist,
                    record.metadata.title,
                    record.metadata.year,
                    source_text(record.metadata.source),
                    phrases_text(record.phrases.as_ref()),
                    integrated,
                    true_peak,
                    record.release.label,
                    record.release.catalog_number,
                    record.release.title,
                    record.release.track_number,
                    record.release.data_source.map(release_data_source_text),
                    record.metadata.year_is_approximate,
                ],
            )
            .map_err(storage)?;
        Ok(())
    }

    /// The record with this hash, if there is one.
    pub fn get(&self, hash: ContentHash) -> Result<Option<TrackRecord>, IndexError> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM tracks WHERE hash = ?1"),
                [hash.to_string()],
                read_record,
            )
            .optional()
            .map_err(storage)
    }

    /// The record whose file was last seen at this path, if there is one.
    pub fn get_by_path(&self, path: &Path) -> Result<Option<TrackRecord>, IndexError> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM tracks WHERE path = ?1"),
                [path_text(path)],
                read_record,
            )
            .optional()
            .map_err(storage)
    }

    /// Records that the file with this hash is now at `path`. Returns whether
    /// a record with that hash existed to update.
    pub fn set_path(&mut self, hash: ContentHash, path: &Path) -> Result<bool, IndexError> {
        let changed = self
            .connection
            .execute(
                "UPDATE tracks SET path = ?1 WHERE hash = ?2",
                rusqlite::params![path_text(path), hash.to_string()],
            )
            .map_err(storage)?;
        Ok(changed > 0)
    }

    /// Every record that meets all of the query's conditions, in ascending
    /// order of path.
    pub fn query(&self, query: &Query) -> Result<Vec<TrackRecord>, IndexError> {
        // A condition that is a plain comparison of one column is asked of
        // SQLite. Three of them are not: a folder must match whole path
        // components, a key must mix well rather than match, and artist and
        // title text must be found within a value whatever its case. Those
        // three are applied here to the rows that come back. A track with no
        // year and a track with no key fall out of a year or key condition on
        // their own, because a comparison with a null column is null, which
        // is not true.
        let mut conditions: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        if let Some((low, high)) = query.bpm {
            condition(&mut conditions, &mut values, "bpm >= ?", Value::Real(low.0));
            condition(
                &mut conditions,
                &mut values,
                "bpm <= ?",
                Value::Real(high.0),
            );
        }
        if let Some((first, last)) = query.year {
            let (first, last) = (Value::Integer(first.into()), Value::Integer(last.into()));
            condition(&mut conditions, &mut values, "year >= ?", first);
            condition(&mut conditions, &mut values, "year <= ?", last);
        }
        if let Some((low, high)) = query.length {
            // The column contains whole samples, so the range's seconds are
            // converted to samples rather than the column converted to
            // seconds: the two bounds round to whole samples once, and the
            // comparison after that is an exact integer comparison, while
            // comparing in seconds would compare two floating-point values.
            condition(
                &mut conditions,
                &mut values,
                "length_samples >= ?",
                Value::Integer(low.to_samples().0),
            );
            condition(
                &mut conditions,
                &mut values,
                "length_samples <= ?",
                Value::Integer(high.to_samples().0),
            );
        }
        if let Some(camelot) = query.key {
            let code = Value::Text(camelot.to_string());
            condition(&mut conditions, &mut values, "key_camelot = ?", code);
        }
        if let Some(bound) = query.min_grid_confidence {
            condition(
                &mut conditions,
                &mut values,
                "grid_confidence >= ?",
                Value::Real(bound),
            );
        }
        if let Some(bound) = query.min_anchor_confidence {
            condition(
                &mut conditions,
                &mut values,
                "anchor_confidence >= ?",
                Value::Real(bound),
            );
        }
        let sql = if conditions.is_empty() {
            format!("SELECT {COLUMNS} FROM tracks")
        } else {
            format!(
                "SELECT {COLUMNS} FROM tracks WHERE {}",
                conditions.join(" AND ")
            )
        };

        let mut statement = self.connection.prepare(&sql).map_err(storage)?;
        let found = statement
            .query_map(rusqlite::params_from_iter(&values), read_record)
            .map_err(storage)?;
        let mut records = Vec::new();
        for record in found {
            let record = record.map_err(storage)?;
            if matches(&record, query) {
                records.push(record);
            }
        }
        records.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(records)
    }

    /// How many tracks the library contains.
    pub fn len(&self) -> Result<usize, IndexError> {
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM tracks", [], |row| row.get(0))
            .map_err(storage)?;
        Ok(count as usize)
    }

    /// Whether the library contains no tracks.
    pub fn is_empty(&self) -> Result<bool, IndexError> {
        Ok(self.len()? == 0)
    }
}

/// Whether a record meets the conditions that SQLite was not asked about.
fn matches(record: &TrackRecord, query: &Query) -> bool {
    let under = match &query.under {
        // A folder matches whole path components, so `/music/artist` contains
        // `/music/artist/Etnica` and does not contain `/music/artistry`.
        Some(folder) => record.path.starts_with(folder),
        None => true,
    };
    let compatible = match query.compatible_with {
        Some(wanted) => record
            .key
            .as_ref()
            .is_some_and(|key| key.camelot.is_compatible_with(wanted)),
        None => true,
    };
    let artist = match &query.artist {
        Some(text) => contains(record.metadata.artist.as_deref(), text),
        None => true,
    };
    let title = match &query.title {
        Some(text) => contains(record.metadata.title.as_deref(), text),
        None => true,
    };
    let not_approximate = !query.exclude_approximate_years || !record.metadata.year_is_approximate;
    under && compatible && artist && title && not_approximate
}

/// Adds one condition to a query, with its value bound to the next number.
fn condition(conditions: &mut Vec<String>, values: &mut Vec<Value>, text: &str, value: Value) {
    values.push(value);
    conditions.push(text.replace('?', &format!("?{}", values.len())));
}

/// A path as the text the library stores, which is lossy for a path that is
/// not valid UTF-8.
fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether a value contains the text, compared without regard to case. A value
/// that is absent contains nothing.
fn contains(value: Option<&str>, text: &str) -> bool {
    value.is_some_and(|value| value.to_lowercase().contains(&text.to_lowercase()))
}

/// A failure to read or write the library, as the error the caller sees.
fn storage(problem: rusqlite::Error) -> IndexError {
    IndexError::Storage(problem.to_string())
}

/// A record's phrase analysis as the text the library stores, which is the
/// JSON the analysis serializes to, or nothing when the record has none.
///
/// A confidence or a beat that is not a finite number is written as a JSON
/// null, which nothing reads back as a number, so a record containing one is
/// stored with its phrases absent. A scan that meets such a track stores it
/// without phrases rather than writing a row that every later query would
/// fail on.
fn phrases_text(phrases: Option<&PhraseRecord>) -> Option<String> {
    let phrases = phrases.filter(|phrases| {
        phrases.confidence.is_finite()
            && phrases.starts.iter().all(|start| start.beat.0.is_finite())
            && phrases.sections.iter().all(|beat| beat.0.is_finite())
    })?;
    serde_json::to_string(phrases).ok()
}

/// One phrase column read back as the analysis it was written from. A column
/// containing nothing means the record has no phrases, which is so for a
/// track on which the phrase analyzer failed.
fn phrases_of(row: &Row<'_>, column: usize) -> rusqlite::Result<Option<PhraseRecord>> {
    let Some(text) = row.get::<_, Option<String>>(column)? else {
        return Ok(None);
    };
    serde_json::from_str(&text).map(Some).map_err(|problem| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(problem))
    })
}

/// A record's loudness as the two columns store it: the integrated
/// loudness and the true peak, or nothing in both columns for a record with
/// no loudness.
///
/// The pair is dropped when either value is not a finite number, because
/// SQLite has no real value for NaN and stores one as null. Writing such a
/// measurement would leave the record with a loudness that reads back
/// differently from the one it was given, so no loudness is stored at all
/// and the record reads back as one with none. [`measure_loudness`] never
/// answers with a value of that kind, so this is for a record some other
/// code built.
///
/// [`measure_loudness`]: dermixen_analysis::measure_loudness
fn loudness_columns(loudness: Option<&Loudness>) -> (Option<f64>, Option<f64>) {
    match loudness.filter(|found| found.integrated.0.is_finite() && found.true_peak.0.is_finite()) {
        Some(found) => (Some(found.integrated.0), Some(found.true_peak.0)),
        None => (None, None),
    }
}

/// The two loudness columns read back as the measurement they were written
/// from. Either column containing nothing means the record has no loudness,
/// which is so for a track the meter finds nothing in.
fn loudness_of(
    row: &Row<'_>,
    integrated: usize,
    true_peak: usize,
) -> rusqlite::Result<Option<Loudness>> {
    match (
        row.get::<_, Option<f64>>(integrated)?,
        row.get::<_, Option<f64>>(true_peak)?,
    ) {
        (Some(integrated), Some(true_peak)) => Ok(Some(Loudness {
            integrated: Lufs(integrated),
            true_peak: Decibels(true_peak),
        })),
        _ => Ok(None),
    }
}

/// The text a metadata source is stored as.
fn source_text(source: MetadataSource) -> &'static str {
    match source {
        MetadataSource::Tags => "tags",
        MetadataSource::Filename => "filename",
    }
}

/// A release data source as the text the column stores.
fn release_data_source_text(source: ReleaseDataSource) -> &'static str {
    match source {
        ReleaseDataSource::DiscogsExport => "discogs_export",
        ReleaseDataSource::DiscogsApi => "discogs_api",
    }
}

/// The reason a stored value could not be read back.
#[derive(Debug, thiserror::Error)]
#[error("{0} is not a value Dermixen writes into a library file")]
struct Unreadable(String);

/// One row of `tracks` as a record.
fn read_record(row: &Row<'_>) -> rusqlite::Result<TrackRecord> {
    let key = match row.get::<_, Option<String>>(7)? {
        Some(_) => Some(KeyRecord {
            key: parsed(row, 7)?,
            camelot: parsed(row, 8)?,
            confidence: row.get(9)?,
            analyzer: row.get(10)?,
        }),
        None => None,
    };
    let source = match row.get::<_, String>(20)?.as_str() {
        "tags" => MetadataSource::Tags,
        "filename" => MetadataSource::Filename,
        other => return Err(unreadable(20, other)),
    };
    let data_source = match row.get::<_, Option<String>>(28)?.as_deref() {
        Some("discogs_export") => Some(ReleaseDataSource::DiscogsExport),
        Some("discogs_api") => Some(ReleaseDataSource::DiscogsApi),
        Some(other) => return Err(unreadable(28, other)),
        None => None,
    };
    Ok(TrackRecord {
        hash: parsed(row, 0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        length: Samples(row.get(2)?),
        grid: BeatGrid {
            first_beat: Samples(row.get(3)?),
            bpm: Bpm(row.get(4)?),
        },
        grid_confidence: row.get(5)?,
        grid_analyzer: row.get(6)?,
        key,
        extent: Extent {
            begins: Samples(row.get(11)?),
            ends: Samples(row.get(12)?),
        },
        anchors: Anchors {
            intro: Beats(row.get(13)?),
            outro: Beats(row.get(14)?),
        },
        anchor_confidence: row.get(15)?,
        anchor_analyzer: row.get(16)?,
        phrases: phrases_of(row, 21)?,
        loudness: loudness_of(row, 22, 23)?,
        metadata: Metadata {
            artist: row.get(17)?,
            title: row.get(18)?,
            year: row.get(19)?,
            year_is_approximate: row.get(29)?,
            source,
        },
        release: Release {
            label: row.get(24)?,
            catalog_number: row.get(25)?,
            title: row.get(26)?,
            track_number: row.get(27)?,
            data_source,
        },
    })
}

/// One text column read back as the value it was written from.
fn parsed<T>(row: &Row<'_>, column: usize) -> rusqlite::Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    let text: String = row.get(column)?;
    text.parse().map_err(|problem| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(problem))
    })
}

/// The error for a stored value that is not one this crate writes, which is
/// a library file someone else has edited.
fn unreadable(column: usize, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(Unreadable(format!("{value:?}"))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_a_database_is_an_open_error_that_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        std::fs::write(&path, b"this is not a database").unwrap();
        match Index::open(&path) {
            Err(IndexError::Open { path: named, .. }) => assert_eq!(named, path),
            other => panic!("expected an open error, got {other:?}"),
        }
        let message = Index::open(&path).unwrap_err().to_string();
        assert!(message.contains("library.sqlite"), "{message}");
    }

    #[test]
    fn a_fresh_file_is_laid_out_with_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        let index = Index::open(&path).unwrap();
        let version: u32 = index
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(index.is_empty().unwrap());
        // The tables and the version are written together, so a file that
        // contains one contains the other, and opening it again reads it back.
        drop(index);
        assert!(Index::open(&path).is_ok());
    }
}
