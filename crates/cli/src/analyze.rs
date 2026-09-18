//! The `analyze` command, which reports everything the library would store
//! about one audio file without storing any of it.

use std::path::{Path, PathBuf};

use dermixen_library::{TrackRecord, analyze_file};

use crate::text::say;

use crate::analyzers::{Chosen, Given};

/// A file's path as the mix documents and the library record it, which is
/// the same path however the command was run from.
///
/// The failure names the file as the person at the terminal typed it, since
/// that is the name they can look for.
pub fn canonical(file: &Path) -> Result<PathBuf, String> {
    file.canonicalize()
        .map_err(|problem| format!("cannot read {}: {problem}", file.display()))
}

/// Reads one audio file and builds the record the library would store for it.
pub fn record_of(file: &Path, given: &Given) -> Result<TrackRecord, String> {
    let path = canonical(file)?;
    let chosen = Chosen::new(given)?;
    analyze_file(&path, &chosen.as_library())
        .map_err(|problem| format!("cannot analyze {}: {problem}", path.display()))
}

/// One field of the text report: the name padded so the values line up, then
/// the value.
///
/// Twenty columns is one more than the longest field name this command
/// prints, so every name keeps at least one space before its value.
fn field(name: &str, value: &str) {
    say!("{name:<20}{value}");
}

/// A value that may be absent, written as a dash when it is.
fn or_dash(value: Option<String>) -> String {
    value.unwrap_or_else(|| "-".to_owned())
}

/// Prints a record as text, one field per line, in the order the JSON
/// document holds them.
pub fn print_record(record: &TrackRecord) {
    field("path", &record.path.display().to_string());
    field("hash", &record.hash.to_string());
    field("length_samples", &record.length.0.to_string());
    field("bpm", &record.grid.bpm.0.to_string());
    field("first_beat_sample", &record.grid.first_beat.0.to_string());
    field("grid_confidence", &format!("{:.2}", record.grid_confidence));
    field("grid_analyzer", &record.grid_analyzer);
    let key = record.key.as_ref();
    field("key", &or_dash(key.map(|key| key.key.to_string())));
    field("camelot", &or_dash(key.map(|key| key.camelot.to_string())));
    field(
        "key_confidence",
        &or_dash(key.map(|key| format!("{:.2}", key.confidence))),
    );
    field(
        "key_analyzer",
        &or_dash(key.map(|key| key.analyzer.clone())),
    );
    field("begins_sample", &record.extent.begins.0.to_string());
    field("ends_sample", &record.extent.ends.0.to_string());
    field("intro_beat", &record.anchors.intro.0.to_string());
    field("outro_beat", &record.anchors.outro.0.to_string());
    field(
        "anchor_confidence",
        &format!("{:.2}", record.anchor_confidence),
    );
    field("anchor_analyzer", &record.anchor_analyzer);
    field("artist", &or_dash(record.metadata.artist.clone()));
    field("title", &or_dash(record.metadata.title.clone()));
    field(
        "year",
        &or_dash(record.metadata.year.map(|year| year.to_string())),
    );
    field(
        "year_is_approximate",
        if record.metadata.year_is_approximate {
            "yes"
        } else {
            "no"
        },
    );
    field(
        "metadata_source",
        match record.metadata.source {
            dermixen_library::MetadataSource::Tags => "tags",
            dermixen_library::MetadataSource::Filename => "filename",
        },
    );
    let release = &record.release;
    field("label", &or_dash(release.label.clone()));
    field("catalog_number", &or_dash(release.catalog_number.clone()));
    field("release_title", &or_dash(release.title.clone()));
    field(
        "track_number",
        &or_dash(release.track_number.map(|number| number.to_string())),
    );
    field(
        "release_data_source",
        &or_dash(release.data_source.map(|source| {
            match source {
                dermixen_library::ReleaseDataSource::DiscogsExport => "discogs_export",
                dermixen_library::ReleaseDataSource::DiscogsApi => "discogs_api",
            }
            .to_owned()
        })),
    );
    // The phrase starts and the section changes are lists, and a track can
    // have dozens of each, so the text report gives how many of each the
    // analyzer found rather than every one of them. The JSON output holds
    // every position for a reader that wants them.
    let phrases = record.phrases.as_ref();
    field(
        "phrase_analyzer",
        &or_dash(phrases.map(|phrases| phrases.analyzer.clone())),
    );
    field(
        "phrase_confidence",
        &or_dash(phrases.map(|phrases| format!("{:.2}", phrases.confidence))),
    );
    field(
        "downbeat",
        &or_dash(phrases.map(|phrases| phrases.downbeat.to_string())),
    );
    field(
        "phrase_starts",
        &or_dash(phrases.map(|phrases| phrases.starts.len().to_string())),
    );
    field(
        "sections",
        &or_dash(phrases.map(|phrases| phrases.sections.len().to_string())),
    );
    field("loudness", &loudness_text(record));
}

/// A record's loudness as the one line the text report gives it: the
/// integrated loudness in LUFS and the true peak in dBTP, or `not
/// measurable` for a track the meter found nothing in, which is one
/// quieter than the meter's gate, shorter than its block, or silent.
fn loudness_text(record: &TrackRecord) -> String {
    match record.loudness {
        Some(loudness) => format!(
            "{:.1} LUFS, {:.1} dBTP",
            loudness.integrated.0, loudness.true_peak.0
        ),
        None => "not measurable".to_owned(),
    }
}

/// Prints one value as the single JSON document a command writes.
///
/// The text goes out as it stands, because JSON writes a control character
/// as an escape of its own, and it goes through `crate::text` like every
/// other write, so a reader that closes the pipe stops the writing rather
/// than failing the command.
pub fn print_json<T: serde::Serialize>(value: &T) {
    let text = serde_json::to_string_pretty(value)
        .expect("every field of a command's report has a JSON form, so writing one cannot fail");
    crate::text::print_json_text(&text);
}

/// Carries out the `analyze` command.
pub fn run(file: &Path, json: bool, given: &Given) -> Result<(), String> {
    let record = record_of(file, given)?;
    if json {
        print_json(&record);
    } else {
        print_record(&record);
    }
    Ok(())
}
