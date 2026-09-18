//! The `mix plan` command, which predicts the timeline `mix add` would
//! build from a playlist, before any document is written.
//!
//! The prediction is the mix that `mix new` and then `mix add` for each
//! path, with no options, would build, laid out as `mix show` lays a
//! document out. The plan builds that mix with the code `mix add` uses and
//! lays it out with the code `mix show` uses, so a change to either command
//! changes the plan with it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dermixen_analysis::Camelot;
use dermixen_core::files::{LARGEST_SETTINGS, read_text};
use dermixen_core::{DEFAULT_BARS, Mix, Seconds};
use dermixen_library::{Index, Query, TrackRecord};
use dermixen_media::hash_file;
use serde::Serialize;

use crate::analyzers::Given;
use crate::document::{self, TrackOptions};
use crate::show::{self, TrackLine};
use crate::text::{note, say};

/// The lowest median key confidence at which the plan checks keys.
///
/// A library whose median key confidence is under this is a library whose
/// Camelot codes are noise, either because the `dermixen` that scanned the
/// library had no key analyzer built in or because the analyzer's confidence
/// in every key it found was low. A clash between two such codes means nothing, so the plan
/// says the keys are not checked rather than warning about them.
const KEY_CONFIDENCE_FLOOR: f64 = 0.1;

/// The text that stands between two credited names in an artist field,
/// matched without regard to case.
const CREDIT_SEPARATORS: [&str; 6] = [" & ", " feat. ", " feat ", " featuring ", " vs. ", " vs "];

/// The words that mark a remix credit inside brackets in a title, matched
/// without regard to case.
const REMIX_WORDS: [&str; 2] = ["remix", "rmx"];

/// The options `mix plan` takes, as the person typed them.
pub struct PlanArgs<'a> {
    /// The file of audio paths, one per line, in playlist order.
    pub playlist: &'a Path,
    /// The most the tempo may move at one transition, in beats per minute.
    pub max_step: f64,
    /// Whether an artist may appear on more than one track without a warning.
    pub allow_repeats: bool,
    /// The library file, when one was named.
    pub library: Option<&'a Path>,
    /// Whether to print one JSON document instead of text.
    pub json: bool,
}

/// One track of the plan, as the `mix_plan` JSON document holds it: where
/// `mix show` puts the track, and the three facts a person plans by.
#[derive(Debug, Serialize)]
struct PlanTrack {
    /// Everything `mix show` says about the track.
    #[serde(flatten)]
    placed: TrackLine,
    /// The artist the library records for the track.
    artist: Option<String>,
    /// The title the library records for the track.
    title: Option<String>,
    /// The track's Camelot code, when the library records a key for it.
    camelot: Option<Camelot>,
    /// How far the mix tempo moves at the transition into this track.
    tempo_step_bpm: Option<f64>,
}

/// The whole plan, as the `mix_plan` JSON document holds it.
#[derive(Debug, Serialize)]
struct Plan {
    /// The playlist file, as the person named it.
    playlist: String,
    /// The length the rendered mix would have.
    length_samples: i64,
    /// The same length in seconds.
    length_seconds: f64,
    /// Whether the plan checked the keys.
    keys_checked: bool,
    /// Every warning the command wrote for the plan, one sentence each.
    warnings: Vec<String>,
    /// One entry per track, in playlist order.
    tracks: Vec<PlanTrack>,
}

/// The limit `--max-step` gives, refusing anything that is not a finite
/// number of zero or more.
///
/// A limit of NaN would compare false against every step and switch the
/// tempo check off without a word, and a negative limit would warn about
/// every transition including one that moves the tempo not at all. The
/// message writes the value in lower case, because Rust writes a value that
/// is not a number as `NaN` and the documentation spells it `nan`.
fn checked_max_step(bpm: f64) -> Result<f64, String> {
    if !bpm.is_finite() || bpm < 0.0 {
        return Err(format!(
            "--max-step {} is not a number of beats per minute from zero up. Give a finite number, like 1.5",
            format!("{bpm}").to_lowercase()
        ));
    }
    Ok(bpm)
}

/// The files a playlist names, in playlist order.
///
/// A blank line is skipped, and a relative path is resolved against the
/// folder the command runs in, as every path a command takes is.
///
/// The playlist comes through [`read_text`], so it is read only from a
/// regular file of at most [`LARGEST_SETTINGS`] bytes, which is the limit
/// `DESIGN.md` states for a playlist as well as for a settings file.
fn read_playlist(playlist: &Path) -> Result<Vec<PathBuf>, String> {
    let text = read_text(playlist, LARGEST_SETTINGS).map_err(|problem| problem.to_string())?;
    let mut files = Vec::new();
    for line in text.lines() {
        let named = line.trim();
        if named.is_empty() {
            continue;
        }
        files.push(crate::analyze::canonical(Path::new(named))?);
    }
    if files.is_empty() {
        return Err(format!("{} names no tracks", playlist.display()));
    }
    Ok(files)
}

/// The library's record for each file, found by the file's bytes.
///
/// A file the library does not contain is refused rather than analyzed,
/// because a plan answers in seconds where an analysis takes minutes, and
/// it writes neither a document nor the library.
fn records_for(index: &Index, files: &[PathBuf]) -> Result<Vec<TrackRecord>, String> {
    let mut records = Vec::with_capacity(files.len());
    for file in files {
        let hash = hash_file(file).map_err(|problem| problem.to_string())?;
        let found = index
            .get(hash)
            .map_err(|problem| problem.to_string())?
            .ok_or_else(|| {
                let folder = file.parent().unwrap_or(Path::new("."));
                format!(
                    "the library has no record of {}, and a plan analyzes nothing. dermixen library scan {} adds it",
                    file.display(),
                    folder.display()
                )
            })?;
        records.push(found);
    }
    Ok(records)
}

/// The median key confidence over every record in the library that has a
/// key, or `None` when no record in the library has one.
///
/// Whether the analyzer found keys is a fact about the library rather than
/// about the tracks of one playlist, so the median is taken over every
/// record. A record with no key does not count, because a library with a
/// key analyzer built in still has real keys when the analyzer failed on a
/// few files.
fn median_key_confidence(index: &Index) -> Result<Option<f64>, String> {
    let records = index
        .query(&Query::default())
        .map_err(|problem| problem.to_string())?;
    let mut scores: Vec<f64> = records
        .iter()
        .filter_map(|record| record.key.as_ref().map(|key| key.confidence))
        .collect();
    if scores.is_empty() {
        return Ok(None);
    }
    scores.sort_by(f64::total_cmp);
    let middle = scores.len() / 2;
    Ok(Some(if scores.len().is_multiple_of(2) {
        (scores[middle - 1] + scores[middle]) / 2.0
    } else {
        scores[middle]
    }))
}

/// Whether the plan checks keys, and the note to write on standard error
/// when it does not.
struct KeyCheck {
    /// Whether the plan checks keys.
    checked: bool,
    /// The one sentence saying why the plan does not check them.
    note: Option<String>,
}

/// Reads the library's key confidences and decides whether to check keys.
fn key_check(index: &Index) -> Result<KeyCheck, String> {
    match median_key_confidence(index)? {
        Some(median) if median >= KEY_CONFIDENCE_FLOOR => Ok(KeyCheck {
            checked: true,
            note: None,
        }),
        Some(median) => Ok(KeyCheck {
            checked: false,
            note: Some(format!(
                "keys are not checked: the median key confidence in the library is {median:.2}"
            )),
        }),
        None => Ok(KeyCheck {
            checked: false,
            note: Some("keys are not checked: no track in the library has a key".to_owned()),
        }),
    }
}

/// The mix `mix add` would build from the playlist, one track at a time with
/// no options.
///
/// Each track is built from its record as `mix add` builds one and joined to
/// the track before it with the preset and the transition length `mix add`
/// uses when the person names neither, so a transition that would not fit is
/// refused here in the words `mix add` refuses it in.
fn build(files: &[PathBuf], records: &[TrackRecord]) -> Result<Mix, String> {
    let preset = document::preset_of(document::DEFAULT_PRESET, DEFAULT_BARS)?;
    let options = TrackOptions {
        bars: DEFAULT_BARS,
        intro: None,
        outro: None,
        given: &Given {
            bpm: None,
            first_beat: None,
        },
        keylock: true,
        gain: None,
    };
    let mut mix = Mix::new();
    for (file, record) in files.iter().zip(records) {
        let track = document::build_track(file.clone(), record, &options)?;
        let at = mix.tracks.len();
        document::join(&mut mix, at, track, preset)?;
    }
    Ok(mix)
}

/// Every name credited on a track: the names in the artist field, and the
/// remixer named in brackets in the title.
fn credits(record: &TrackRecord) -> Vec<String> {
    let mut names = match record.metadata.artist.as_deref() {
        Some(artist) => credited(artist),
        None => Vec::new(),
    };
    if let Some(title) = record.metadata.title.as_deref()
        && let Some(remixer) = remixer(title)
    {
        names.push(remixer);
    }
    names
}

/// The names an artist field credits, split on the text that stands between
/// two of them.
fn credited(artist: &str) -> Vec<String> {
    // The separators are all ASCII, so lowercasing the ASCII letters alone
    // matches them without regard to case and leaves every byte position of
    // the artist field where it was.
    let lower = artist.to_ascii_lowercase();
    let mut names = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while let Some(character) = artist[at..].chars().next() {
        match CREDIT_SEPARATORS
            .iter()
            .find(|separator| lower[at..].starts_with(**separator))
        {
            Some(separator) => {
                names.push(artist[start..at].trim().to_owned());
                at += separator.len();
                start = at;
            }
            None => at += character.len_utf8(),
        }
    }
    names.push(artist[start..].trim().to_owned());
    names.retain(|name| !name.is_empty());
    names
}

/// The remixer a title credits, which is the words before `remix` or `rmx`
/// inside a pair of brackets, whatever words follow.
fn remixer(title: &str) -> Option<String> {
    for inside in bracketed(title) {
        let lower = inside.to_ascii_lowercase();
        let Some(mark) = REMIX_WORDS.iter().filter_map(|word| lower.find(word)).min() else {
            continue;
        };
        let name = inside[..mark].trim();
        if !name.is_empty() {
            return Some(name.to_owned());
        }
    }
    None
}

/// The text inside each pair of brackets in a title, round or square, in the
/// order the pairs open. Text after a bracket that is never closed is left out.
fn bracketed(title: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut open: Option<(usize, char)> = None;
    for (at, character) in title.char_indices() {
        match (character, open) {
            ('(' | '[', None) => open = Some((at + character.len_utf8(), character)),
            (')', Some((from, '('))) | (']', Some((from, '['))) => {
                pieces.push(&title[from..at]);
                open = None;
            }
            _ => {}
        }
    }
    pieces
}

/// Where one artist appears in the playlist.
struct Appearances {
    /// The spelling of the earliest track that credits the artist.
    name: String,
    /// The tracks the artist appears on, counting from one, in order.
    tracks: Vec<usize>,
}

/// One warning per artist on more than one track, in the order the artists
/// first appear.
///
/// Two spellings that differ only in case are one artist, reported with the
/// spelling of the earliest track's record, and an artist credited twice on
/// one track appears once in that track's list.
fn repeats(records: &[TrackRecord]) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    let mut found: HashMap<String, Appearances> = HashMap::new();
    for (index, record) in records.iter().enumerate() {
        let position = index + 1;
        for name in credits(record) {
            let key = name.to_lowercase();
            let appearances = found.entry(key.clone()).or_insert_with(|| {
                order.push(key);
                Appearances {
                    name,
                    tracks: Vec::new(),
                }
            });
            if appearances.tracks.last() != Some(&position) {
                appearances.tracks.push(position);
            }
        }
    }
    order
        .iter()
        .filter_map(|key| found.get(key))
        .filter(|appearances| appearances.tracks.len() > 1)
        .map(|appearances| {
            format!(
                "{} appears on tracks {}",
                appearances.name,
                numbers(&appearances.tracks)
            )
        })
        .collect()
}

/// A list of track numbers as a person reads one: `2 and 3` for two of them,
/// and `1, 2, and 4` for more.
fn numbers(tracks: &[usize]) -> String {
    let written: Vec<String> = tracks.iter().map(|track| track.to_string()).collect();
    match written.len() {
        0 | 1 => written.concat(),
        2 => format!("{} and {}", written[0], written[1]),
        count => format!(
            "{}, and {}",
            written[..count - 1].join(", "),
            written[count - 1]
        ),
    }
}

/// Every warning the command writes for the plan, in the order written: the
/// tempo step and the key clash of each transition in turn, then the artists
/// on more than one track.
fn warnings(
    tracks: &[PlanTrack],
    records: &[TrackRecord],
    max_step: f64,
    allow_repeats: bool,
    keys_checked: bool,
) -> Vec<String> {
    let mut earned = Vec::new();
    for (index, track) in tracks.iter().enumerate().skip(1) {
        // Transitions count from one, so the transition into the track at
        // index one is transition one.
        let transition = index;
        if let Some(step) = track.tempo_step_bpm
            && step.abs() > max_step
        {
            earned.push(format!(
                "transition {transition} moves the tempo by {:.2} bpm, over the {max_step:.2} allowed",
                step.abs()
            ));
        }
        if keys_checked
            && let (Some(from), Some(to)) = (tracks[index - 1].camelot, track.camelot)
            && !from.is_compatible_with(to)
        {
            earned.push(format!(
                "transition {transition} goes from {from} to {to}, which are keys that do not fit"
            ));
        }
    }
    if !allow_repeats {
        earned.extend(repeats(records));
    }
    earned
}

/// Prints the plan as one line per track and a last line naming the number
/// of tracks, the length of the mix, and the tempo it opens and closes at.
fn print(plan: &Plan) {
    for track in &plan.tracks {
        say!(
            "{:>3}  {:<3}  {:>7.2} bpm  {:>6}  enters {:>7}  {} - {}",
            track.placed.position,
            track
                .camelot
                .map(|camelot| camelot.to_string())
                .unwrap_or_else(|| "--".to_owned()),
            track.placed.grid.bpm.0,
            track
                .tempo_step_bpm
                .map(|step| format!("{step:+.2}"))
                .unwrap_or_default(),
            show::length_text(Seconds(track.placed.start_seconds)),
            track.artist.as_deref().unwrap_or("-"),
            track.title.as_deref().unwrap_or("-"),
        );
    }
    let opening = plan.tracks.first().map(|track| track.placed.grid.bpm.0);
    let closing = plan.tracks.last().map(|track| track.placed.grid.bpm.0);
    if let (Some(opening), Some(closing)) = (opening, closing) {
        say!(
            "{} {}, {} long, opening at {opening:.2} bpm and closing at {closing:.2} bpm",
            plan.tracks.len(),
            if plan.tracks.len() == 1 {
                "track"
            } else {
                "tracks"
            },
            show::length_text(Seconds(plan.length_seconds)),
        );
    }
}

/// Carries out `mix plan`.
///
/// Everything that can fail is done before anything is printed, so a plan
/// that is refused prints one `error:` line and nothing on standard output.
/// The one exception is the warning `mix add` gives a track whose record has
/// no loudness, which the shared track building writes as it goes.
///
/// The settings file is read before the playlist, because the settings name
/// the library file when `--library` and the environment do not, and a
/// settings file that cannot be read stops every command that opens the
/// library.
///
/// The library file is opened through [`crate::index::open_existing`], so a
/// path that names no library file is refused and no file and no folder is
/// created. A plan writes nothing, and the library file is part of that.
pub fn run(args: &PlanArgs<'_>) -> Result<(), String> {
    let settings = crate::settings::read()?;
    let max_step = checked_max_step(args.max_step)?;
    let files = read_playlist(args.playlist)?;
    let location = crate::index::location(args.library, &settings)?;
    let index = crate::index::open_existing(&location)?;
    let records = records_for(&index, &files)?;
    let keys = key_check(&index)?;
    let mix = build(&files, &records)?;
    let laid = show::layout(args.playlist, &mix);

    let mut last_bpm: Option<f64> = None;
    let mut tracks = Vec::with_capacity(records.len());
    for (placed, record) in laid.tracks.into_iter().zip(&records) {
        let bpm = placed.grid.bpm.0;
        tracks.push(PlanTrack {
            placed,
            artist: record.metadata.artist.clone(),
            title: record.metadata.title.clone(),
            camelot: record.key.as_ref().map(|key| key.camelot),
            tempo_step_bpm: last_bpm.map(|before| bpm - before),
        });
        last_bpm = Some(bpm);
    }

    let plan = Plan {
        playlist: args.playlist.display().to_string(),
        length_samples: laid.length_samples,
        length_seconds: laid.length_seconds,
        keys_checked: keys.checked,
        warnings: warnings(
            &tracks,
            &records,
            max_step,
            args.allow_repeats,
            keys.checked,
        ),
        tracks,
    };

    if let Some(message) = &keys.note {
        note!("note: {message}");
    }
    for warning in &plan.warnings {
        note!("warning: {warning}");
    }
    if args.json {
        crate::analyze::print_json(&plan);
    } else {
        print(&plan);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_artist_field_splits_on_every_credit_separator_in_any_case() {
        assert_eq!(credited("Etnica"), vec!["Etnica"]);
        assert_eq!(
            credited("MFG & Astral Projection"),
            vec!["MFG", "Astral Projection"]
        );
        assert_eq!(
            credited("Astral Projection Vs. MFG"),
            vec!["Astral Projection", "MFG"]
        );
        assert_eq!(credited("Doof vs Prana"), vec!["Doof", "Prana"]);
        assert_eq!(
            credited("Cosmosis feat. Kate & Koxbox"),
            vec!["Cosmosis", "Kate", "Koxbox"]
        );
        assert_eq!(credited("X FEATURING Y"), vec!["X", "Y"]);
        // A name that contains one of the words without the spaces around it
        // stays whole.
        assert_eq!(credited("Various Artists"), vec!["Various Artists"]);
    }

    #[test]
    fn a_remixer_is_the_words_before_remix_inside_brackets() {
        assert_eq!(
            remixer("New Horizon (Total Eclipse remix)").as_deref(),
            Some("Total Eclipse")
        );
        assert_eq!(
            remixer("Mahadeva (Man With No Name Remix Edit)").as_deref(),
            Some("Man With No Name")
        );
        assert_eq!(
            remixer("Sunspot [Hallucinogen RMX]").as_deref(),
            Some("Hallucinogen")
        );
        assert_eq!(remixer("Alpha Centauri"), None);
        assert_eq!(remixer("Alpha (Original Mix)"), None);
        assert_eq!(remixer("Alpha (Remix)"), None);
    }

    #[test]
    fn a_list_of_track_numbers_reads_as_a_person_writes_one() {
        assert_eq!(numbers(&[2, 3]), "2 and 3");
        assert_eq!(numbers(&[1, 2, 4]), "1, 2, and 4");
        assert_eq!(numbers(&[7]), "7");
    }

    #[test]
    fn a_limit_that_is_not_a_number_from_zero_up_is_refused() {
        assert_eq!(checked_max_step(0.0).unwrap(), 0.0);
        assert_eq!(checked_max_step(1.5).unwrap(), 1.5);
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            let message = checked_max_step(bad).unwrap_err();
            assert!(message.contains("--max-step"), "{message}");
        }
        assert!(checked_max_step(f64::NAN).unwrap_err().contains("nan"));
        assert!(checked_max_step(-1.0).unwrap_err().contains("-1"));
    }
}
