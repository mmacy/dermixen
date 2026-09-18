//! The `library scan`, `library query`, and `library find` commands.

use std::path::{Path, PathBuf};

use dermixen_analysis::Camelot;
use dermixen_core::{Bpm, Seconds, Settings};
use dermixen_library::{
    Change, Progress, Query, ScanOptions, ScanSummary, TrackRecord, find as rank, scan_into,
};
use serde::Serialize;

use crate::analyze::print_json;
use crate::analyzers::{Chosen, Given};

/// What `library scan --json` prints.
#[derive(Debug, Serialize)]
struct ScanReport {
    /// The library file the scan wrote to.
    library: String,
    /// The folder scanned.
    root: String,
    /// How many files were analyzed and stored.
    added: usize,
    /// How many records were pointed at a new path.
    moved: usize,
    /// How many files were already in the library where they are.
    unchanged: usize,
    /// How many records that had no loudness were completed with one.
    completed: usize,
    /// Every file whose bytes are already in the library under another path.
    duplicates: Vec<Duplicate>,
    /// Every file that could not be analyzed.
    failed: Vec<Trouble>,
    /// Every folder the scan could not read.
    unreadable: Vec<Trouble>,
}

/// One file whose bytes the library already has under another path.
#[derive(Debug, Serialize)]
struct Duplicate {
    /// The file the scan passed over.
    path: String,
    /// The path the record kept.
    duplicate_of: String,
}

/// One file or folder the scan could not deal with, and why.
#[derive(Debug, Serialize)]
struct Trouble {
    /// The file or folder.
    path: String,
    /// What went wrong with it.
    reason: String,
}

/// One match of `library find --json`.
#[derive(Debug, Serialize)]
struct Ranked {
    /// How well the text matched, from zero to one.
    score: f64,
    /// The track that matched.
    record: TrackRecord,
}

/// The conditions `library query` accepts, as the person typed them.
///
/// Ranges are text such as `138-142` or `140`; keys are Camelot codes such
/// as `8A`. Each is checked when the query is built, and a condition that
/// cannot be read is refused with a message naming the option.
#[derive(Debug, Clone, Default)]
pub struct QueryArgs {
    /// Only tracks in this folder or one below it.
    pub under: Option<PathBuf>,
    /// The tempo range.
    pub bpm: Option<String>,
    /// The year range.
    pub year: Option<String>,
    /// The length range, each end as minutes and seconds or as seconds.
    pub length: Option<String>,
    /// The key.
    pub key: Option<String>,
    /// The key to be compatible with.
    pub compatible_with: Option<String>,
    /// Text the artist must contain.
    pub artist: Option<String>,
    /// Text the title must contain.
    pub title: Option<String>,
    /// Whether to leave out every track whose year is an estimate.
    pub no_approximate_years: bool,
    /// The least grid confidence a track may have, as text from 0 to 1.
    pub min_grid_confidence: Option<String>,
    /// The least anchor confidence a track may have, as text from 0 to 1.
    pub min_anchor_confidence: Option<String>,
}

/// Reads a range written as `LOW-HIGH`, or as one number meaning exactly
/// that value.
///
/// A negative low end is not accepted, because neither a tempo nor a year is
/// ever negative and a leading minus sign would be read as the separator.
fn range<T>(option: &str, text: &str) -> Result<(T, T), String>
where
    T: std::str::FromStr + Copy,
{
    let read = |part: &str| -> Result<T, String> {
        part.trim()
            .parse::<T>()
            .map_err(|_| format!("{option} {text} cannot be read: {part} is not a number"))
    };
    match text.split_once('-') {
        Some((low, high)) => Ok((read(low)?, read(high)?)),
        None => {
            let one = read(text)?;
            Ok((one, one))
        }
    }
}

/// Reads a length range written as `LOW-HIGH`, with each end as minutes and
/// seconds, as in `7:00`, or as seconds, as in `420`.
///
/// Unlike a tempo or a year range, a single value is not accepted: on its
/// own it would leave the reader guessing whether it meant seconds or
/// minutes, and would rarely be a useful length to filter by. Both ends
/// must be written the same way; a range that mixes minutes-and-seconds
/// with plain seconds, such as `7-9:00`, is refused rather than guessed at,
/// since a dropped colon would otherwise be read as a very different range.
fn length_range(option: &str, text: &str) -> Result<(Seconds, Seconds), String> {
    let (low_text, high_text) = text.split_once('-').ok_or_else(|| {
        format!("{option} {text} cannot be read: give a range as LOW-HIGH, such as 7:00-9:00")
    })?;
    if low_text.contains(':') != high_text.contains(':') {
        return Err(format!(
            "{option} {text} cannot be read: write both ends the same way, either both as \
             minutes and seconds or both as seconds"
        ));
    }
    let low = time_value(option, text, low_text)?;
    let high = time_value(option, text, high_text)?;
    if low.0 > high.0 {
        return Err(format!(
            "{option} {text} cannot be read: {low_text} is longer than {high_text}"
        ));
    }
    Ok((low, high))
}

/// Reads a time or a length written as minutes and seconds separated by a
/// colon, or as a plain number of seconds.
///
/// `option` names the flag the value came from, and `whole` is everything
/// that flag was given. For a flag holding one value `whole` is the same as
/// `part`, and for one end of a length range it is the whole range, so that
/// a message about one end still shows the range it came from. The result is
/// always finite and not negative, so a reversed-range comparison against
/// it is never fooled by infinity or `NaN`.
pub(crate) fn time_value(option: &str, whole: &str, part: &str) -> Result<Seconds, String> {
    let part = part.trim();
    let not_a_time = || {
        format!(
            "{option} {whole} cannot be read: {part} is neither minutes and seconds, such as \
             7:30, nor a number of seconds"
        )
    };
    let seconds = match part.split_once(':') {
        Some((minutes_text, seconds_text)) => {
            let minutes: f64 = minutes_text.trim().parse().map_err(|_| not_a_time())?;
            let seconds: f64 = seconds_text.trim().parse().map_err(|_| not_a_time())?;
            if !(0.0..60.0).contains(&seconds) {
                return Err(format!(
                    "{option} {whole} cannot be read: {seconds_text} is sixty seconds or more"
                ));
            }
            minutes * 60.0 + seconds
        }
        None => part.parse::<f64>().map_err(|_| not_a_time())?,
    };
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(not_a_time());
    }
    Ok(Seconds(seconds))
}

/// Reads a Camelot code, refusing anything that is not one of the
/// twenty-four codes with a message naming the option.
fn camelot(option: &str, text: &str) -> Result<Camelot, String> {
    text.parse::<Camelot>()
        .map_err(|problem| format!("{option} {text} cannot be read: {problem}"))
}

/// Reads a confidence bound from 0 to 1, both ends included.
///
/// The `f64` parser accepts text like `nan` and `inf`, which no confidence
/// is. The range check refuses `inf` as a number outside 0 to 1, and it
/// refuses `nan` because a comparison with `NaN` is never true.
fn confidence(option: &str, text: &str) -> Result<f64, String> {
    let value: f64 = text
        .trim()
        .parse()
        .map_err(|_| format!("{option} {text} cannot be read: {text} is not a number"))?;
    if !(0.0..=1.0).contains(&value) {
        return Err(format!(
            "{option} {text} cannot be read: a confidence runs from 0 to 1"
        ));
    }
    Ok(value)
}

/// A folder condition as an absolute path, so that it can be compared with
/// the absolute paths the library has.
fn folder(under: &Path) -> Result<PathBuf, String> {
    if let Ok(resolved) = under.canonicalize() {
        return Ok(resolved);
    }
    if under.is_absolute() {
        return Ok(under.to_path_buf());
    }
    let here = std::env::current_dir()
        .map_err(|problem| format!("cannot read the current folder: {problem}"))?;
    Ok(here.join(under))
}

/// Turns the conditions a person typed into the query the library takes.
fn query_of(args: &QueryArgs) -> Result<Query, String> {
    let bpm = match &args.bpm {
        Some(text) => {
            let (low, high) = range::<f64>("--bpm", text)?;
            Some((Bpm(low), Bpm(high)))
        }
        None => None,
    };
    let year = match &args.year {
        Some(text) => Some(range::<u16>("--year", text)?),
        None => None,
    };
    let key = match &args.key {
        Some(text) => Some(camelot("--key", text)?),
        None => None,
    };
    let compatible_with = match &args.compatible_with {
        Some(text) => Some(camelot("--compatible-with", text)?),
        None => None,
    };
    let under = match &args.under {
        Some(path) => Some(folder(path)?),
        None => None,
    };
    let length = match &args.length {
        Some(text) => Some(length_range("--length", text)?),
        None => None,
    };
    let min_grid_confidence = match &args.min_grid_confidence {
        Some(text) => Some(confidence("--min-grid-confidence", text)?),
        None => None,
    };
    let min_anchor_confidence = match &args.min_anchor_confidence {
        Some(text) => Some(confidence("--min-anchor-confidence", text)?),
        None => None,
    };
    Ok(Query {
        under,
        bpm,
        year,
        length,
        key,
        compatible_with,
        artist: args.artist.clone(),
        title: args.title.clone(),
        exclude_approximate_years: args.no_approximate_years,
        min_grid_confidence,
        min_anchor_confidence,
    })
}

/// What a scan did with one file, in the wording the command writes on
/// standard error as it goes.
fn change_text(change: Change) -> &'static str {
    match change {
        Change::Added => "added",
        Change::Moved => "moved",
        Change::Unchanged => "unchanged",
        Change::Duplicate => "duplicate",
        Change::Failed => "failed",
        Change::Completed => "completed",
    }
}

/// The folder a scan reads: the folder given on the command line, else the
/// music folder the settings name.
///
/// Either folder is resolved to an absolute path with the links along it
/// followed, so that the paths stored in the library name the files wherever
/// the command is run from next time, and so that a folder scanned by name
/// and the same folder scanned as the music folder store one spelling of
/// each path. A music folder that is not there stops the scan with a message
/// naming the setting, because the person either meant to name a folder on
/// the command line or has yet to set the music folder.
fn scan_root(root: Option<&Path>, settings: &Settings) -> Result<PathBuf, String> {
    let Some(root) = root else {
        let folder = settings.music_folder(&crate::settings::home_folder()?);
        if !folder.is_dir() {
            let trouble = match folder.exists() {
                true => format!("{} is not a folder", folder.display()),
                false => format!("there is no folder at {}", folder.display()),
            };
            return Err(format!(
                "{trouble}, and that is the music folder this scan reads. Give dermixen library \
                 scan a folder to scan, or change the music folder with dermixen settings set \
                 music_folder FOLDER"
            ));
        }
        return folder
            .canonicalize()
            .map_err(|problem| format!("cannot read {}: {problem}", folder.display()));
    };
    root.canonicalize()
        .map_err(|problem| format!("cannot read {}: {problem}", root.display()))
}

/// Carries out `library scan`.
///
/// The settings file is read first, since it names the folder to scan when
/// the command line does not and the library file when neither `--library`
/// nor the environment does.
pub fn scan(
    root: Option<&Path>,
    exclude: &[PathBuf],
    library: Option<&Path>,
    json: bool,
) -> Result<(), String> {
    let settings = crate::settings::read()?;
    let root = scan_root(root, &settings)?;
    let location = crate::index::location(library, &settings)?;
    let mut index = crate::index::open(&location)?;
    let chosen = Chosen::new(&Given::default())?;
    let options = ScanOptions {
        exclude: exclude.to_vec(),
    };

    let mut report = |progress: &Progress<'_>| {
        eprintln!(
            "[{}/{}] {} {}",
            progress.done,
            progress.total,
            change_text(progress.change),
            progress.path.display()
        );
        true
    };
    let summary = scan_into(
        &mut index,
        &root,
        &options,
        &chosen.as_library(),
        &mut report,
    )
    .map_err(|problem| problem.to_string())?;

    if json {
        print_json(&report_of(&location, &root, &summary));
    } else {
        print_summary(&summary);
    }
    Ok(())
}

/// The summary of a scan as the JSON document holds it.
fn report_of(library: &Path, root: &Path, summary: &ScanSummary) -> ScanReport {
    ScanReport {
        library: library.display().to_string(),
        root: root.display().to_string(),
        added: summary.added,
        moved: summary.moved,
        unchanged: summary.unchanged,
        completed: summary.completed,
        duplicates: summary
            .duplicates
            .iter()
            .map(|(path, kept)| Duplicate {
                path: path.display().to_string(),
                duplicate_of: kept.display().to_string(),
            })
            .collect(),
        failed: summary
            .failed
            .iter()
            .map(|(path, reason)| Trouble {
                path: path.display().to_string(),
                reason: reason.clone(),
            })
            .collect(),
        unreadable: summary
            .unreadable
            .iter()
            .map(|folder| Trouble {
                path: folder.path.display().to_string(),
                reason: folder.reason.clone(),
            })
            .collect(),
    }
}

/// Prints the summary of a scan as text: the counts, then a line for each
/// file the scan passed over or could not read.
fn print_summary(summary: &ScanSummary) {
    println!("added {}", summary.added);
    println!("moved {}", summary.moved);
    println!("unchanged {}", summary.unchanged);
    println!("completed {}", summary.completed);
    println!("duplicates {}", summary.duplicates.len());
    for (path, kept) in &summary.duplicates {
        println!(
            "duplicate {}, already in the library as {}",
            path.display(),
            kept.display()
        );
    }
    for (path, reason) in &summary.failed {
        println!("failed {}: {reason}", path.display());
    }
    for folder in &summary.unreadable {
        println!("unreadable {}: {}", folder.path.display(), folder.reason);
    }
}

/// A track's year as the text listing shows it: the year on its own, `~`
/// and the year for an estimate, or `-` for no year.
fn year_cell(record: &TrackRecord) -> String {
    match record.metadata.year {
        Some(year) if record.metadata.year_is_approximate => format!("~{year}"),
        Some(year) => format!("{year}"),
        None => "-".to_owned(),
    }
}

/// The line one track takes in the text listing: its Camelot code or two
/// dashes, its tempo, its year, its artist and title with a question mark
/// after each that was guessed from the file name, and its path.
fn listing_line(record: &TrackRecord) -> String {
    let code = record
        .key
        .as_ref()
        .map(|key| key.camelot.to_string())
        .unwrap_or_else(|| "--".to_owned());
    let guessed = match record.metadata.source {
        dermixen_library::MetadataSource::Filename => "?",
        dermixen_library::MetadataSource::Tags => "",
    };
    let named = |value: &Option<String>| match value {
        Some(text) => format!("{text}{guessed}"),
        None => "-".to_owned(),
    };
    format!(
        "{:<3}  {:>6.1} bpm  {:>5}  {:<24}  {:<32}  {}",
        code,
        record.grid.bpm.0,
        year_cell(record),
        named(&record.metadata.artist),
        named(&record.metadata.title),
        record.path.display()
    )
}

/// Carries out `library query`.
///
/// The settings file is read before the conditions are checked, because the
/// settings name the library file when `--library` and the environment do
/// not, and a settings file that cannot be read stops every command that
/// opens the library.
pub fn query(args: &QueryArgs, library: Option<&Path>, json: bool) -> Result<(), String> {
    let settings = crate::settings::read()?;
    let query = query_of(args)?;
    let location = crate::index::location(library, &settings)?;
    let index = crate::index::open(&location)?;
    let records = index.query(&query).map_err(|problem| problem.to_string())?;
    if json {
        print_json(&records);
    } else {
        for record in &records {
            println!("{}", listing_line(record));
        }
    }
    Ok(())
}

/// Carries out `library find`.
pub fn find(text: &str, limit: usize, library: Option<&Path>, json: bool) -> Result<(), String> {
    let settings = crate::settings::read()?;
    let location = crate::index::location(library, &settings)?;
    let index = crate::index::open(&location)?;
    let records = index
        .query(&Query::default())
        .map_err(|problem| problem.to_string())?;
    let matches = rank(&records, text, limit);
    if json {
        let ranked: Vec<Ranked> = matches
            .into_iter()
            .map(|found| Ranked {
                score: found.score,
                record: found.record,
            })
            .collect();
        print_json(&ranked);
    } else {
        for found in &matches {
            println!("{:.2}  {}", found.score, found.record.path.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_two_numbers_or_one() {
        assert_eq!(range::<f64>("--bpm", "138-142").unwrap(), (138.0, 142.0));
        assert_eq!(range::<f64>("--bpm", "140").unwrap(), (140.0, 140.0));
        assert_eq!(range::<u16>("--year", "1996").unwrap(), (1996, 1996));
    }

    #[test]
    fn a_range_that_cannot_be_read_names_the_option() {
        for (option, text) in [("--bpm", "fast"), ("--year", "199x")] {
            let message = range::<f64>(option, text).unwrap_err();
            assert!(message.contains(option), "{message}");
        }
    }

    #[test]
    fn a_code_that_is_not_on_the_wheel_names_the_option() {
        assert_eq!(camelot("--key", "8A").unwrap().to_string(), "8A");
        for (option, text) in [("--key", "13A"), ("--compatible-with", "8C")] {
            let message = camelot(option, text).unwrap_err();
            assert!(message.contains(option), "{message}");
        }
    }
}
