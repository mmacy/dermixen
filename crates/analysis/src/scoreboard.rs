//! The scoreboard: ground truth, metrics, and the report that puts every
//! analyzer's quality in one table.
//!
//! Ground truth is a directory of audio files with annotation files beside
//! them; `docs/ground-truth.md` describes the format. The metrics are the
//! ones the music information retrieval community uses for tempo and beats,
//! so a number here can be compared with a published one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dermixen_core::{BEATS_PER_BAR, Bpm, Seconds};

use crate::analyzer::{BeatAnalyzer, Key, Mode, PitchClass};

/// What is known to be true about one track.
#[derive(Debug, Clone, PartialEq)]
pub struct GroundTruth {
    /// The track's name, which is its audio file name without the extension.
    pub name: String,
    /// The audio file.
    pub audio: PathBuf,
    /// The tempo, if annotated.
    pub bpm: Option<Bpm>,
    /// The time of every beat, if annotated, in order.
    pub beats: Option<Vec<Seconds>>,
    /// The key, if annotated.
    pub key: Option<Key>,
}

/// The reason ground truth could not be loaded or a run could not finish.
#[derive(Debug, thiserror::Error)]
pub enum ScoreboardError {
    /// A directory or file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The directory or file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// An annotation file holds something that is not in the format.
    #[error("{path}: {message}")]
    Annotation {
        /// The annotation file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
    /// An audio file named by the ground truth could not be decoded.
    #[error("cannot decode {path}: {message}")]
    Decode {
        /// The audio file.
        path: PathBuf,
        /// What went wrong while decoding it.
        message: String,
    },
    /// The directory holds no audio with any annotation.
    #[error("{0} holds no annotated audio")]
    Empty(PathBuf),
}

/// The extensions of the files the scoreboard treats as audio. Every other
/// file in a ground-truth directory, annotation or not, is ignored.
const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "mp3", "flac", "m4a", "mp4"];

/// Builds a read error that names the directory or file.
fn read_error(path: &Path, source: std::io::Error) -> ScoreboardError {
    ScoreboardError::Read {
        path: path.to_path_buf(),
        source,
    }
}

/// Builds an error that names the annotation file and what is wrong with it.
fn annotation_error(path: &Path, message: impl Into<String>) -> ScoreboardError {
    ScoreboardError::Annotation {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// Builds an error that names the audio file that could not be decoded.
pub(crate) fn decode_error(path: &Path, message: impl Into<String>) -> ScoreboardError {
    ScoreboardError::Decode {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// Every file directly inside a directory, keyed by file name so that the
/// entries come back in name order. Subdirectories are left out.
fn files_in(dir: &Path) -> Result<BTreeMap<String, PathBuf>, ScoreboardError> {
    let listing = std::fs::read_dir(dir).map_err(|source| read_error(dir, source))?;
    let mut files = BTreeMap::new();
    for entry in listing {
        let entry = entry.map_err(|source| read_error(dir, source))?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let name = name.to_owned();
        files.insert(name, path);
    }
    Ok(files)
}

/// A file's name without its extension, or `None` if the name is not text.
fn stem_of(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_owned)
}

/// Whether a file is audio, judged by its extension in either case. A file
/// with no extension is not audio.
fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// Reads a whole annotation file as text.
fn read_annotation(path: &Path) -> Result<String, ScoreboardError> {
    std::fs::read_to_string(path).map_err(|source| read_error(path, source))
}

/// Reads a tempo annotation: one number in beats per minute.
fn read_bpm(path: &Path) -> Result<Bpm, ScoreboardError> {
    let text = read_annotation(path)?;
    let field = text
        .split_whitespace()
        .next()
        .ok_or_else(|| annotation_error(path, "the file holds no tempo"))?;
    let value: f64 = field
        .parse()
        .map_err(|_| annotation_error(path, format!("{field} is not a number")))?;
    let bpm = Bpm(value);
    if !bpm.is_valid() {
        return Err(annotation_error(
            path,
            format!("{value} is not a usable tempo"),
        ));
    }
    Ok(bpm)
}

/// Reads a beat annotation: one line per beat, holding the beat's time in
/// seconds and optionally its position in the bar.
fn read_beats(path: &Path) -> Result<Vec<Seconds>, ScoreboardError> {
    let text = read_annotation(path)?;
    let mut beats: Vec<Seconds> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let number = index + 1;
        let mut fields = line.split_whitespace();
        let field = fields.next().unwrap_or_default();
        let time: f64 = field
            .parse()
            .map_err(|_| annotation_error(path, format!("line {number}: {field} is not a time")))?;
        if !time.is_finite() {
            return Err(annotation_error(
                path,
                format!("line {number}: {field} is not a time"),
            ));
        }
        if let Some(field) = fields.next() {
            let in_bar = field
                .parse::<u32>()
                .is_ok_and(|position| (1..=BEATS_PER_BAR).contains(&position));
            if !in_bar {
                return Err(annotation_error(
                    path,
                    format!(
                        "line {number}: {field} is not a beat position from 1 to {BEATS_PER_BAR}"
                    ),
                ));
            }
        }
        if let Some(previous) = beats.last()
            && time < previous.0
        {
            return Err(annotation_error(
                path,
                format!("line {number}: {field} comes before the beat above it"),
            ));
        }
        beats.push(Seconds(time));
    }
    Ok(beats)
}

/// Reads a key annotation.
fn read_key(path: &Path) -> Result<Key, ScoreboardError> {
    let text = read_annotation(path)?;
    parse_key(&text).ok_or_else(|| annotation_error(path, format!("{} is not a key", text.trim())))
}

/// Loads ground truth from a directory in Dermixen's own layout: each audio
/// file `name.ext` may have `name.bpm`, `name.beats`, and `name.key` beside it.
///
/// Audio files with no annotation at all are skipped. Files are returned in
/// name order.
pub fn load_directory(dir: &Path) -> Result<Vec<GroundTruth>, ScoreboardError> {
    let files = files_in(dir)?;
    let mut truth = Vec::new();
    for path in files.values() {
        if !is_audio(path) {
            continue;
        }
        let Some(name) = stem_of(path) else {
            continue;
        };
        let beside = |extension: &str| files.get(&format!("{name}.{extension}")).cloned();
        let bpm = beside("bpm").as_deref().map(read_bpm).transpose()?;
        let beats = beside("beats").as_deref().map(read_beats).transpose()?;
        let key = beside("key").as_deref().map(read_key).transpose()?;
        if bpm.is_none() && beats.is_none() && key.is_none() {
            continue;
        }
        truth.push(GroundTruth {
            name,
            audio: path.clone(),
            bpm,
            beats,
            key,
        });
    }
    if truth.is_empty() {
        return Err(ScoreboardError::Empty(dir.to_path_buf()));
    }
    Ok(truth)
}

/// Every annotation file in one of a GiantSteps checkout's annotation
/// directories, keyed by the stem of the audio file it belongs to. Returns an
/// empty map when the directory is absent, which is how a checkout with no
/// beat annotations is read.
fn annotations_in(dir: &Path) -> Result<BTreeMap<String, PathBuf>, ScoreboardError> {
    if !dir.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut by_stem = BTreeMap::new();
    for path in files_in(dir)?.into_values() {
        if let Some(stem) = stem_of(&path) {
            by_stem.insert(stem, path);
        }
    }
    Ok(by_stem)
}

/// Loads ground truth from a GiantSteps dataset checkout: audio under
/// `audio/`, and annotations under `annotations/tempo/`, `annotations/key/`,
/// and `annotations/beats/`, each named after the audio file's stem.
///
/// Audio files with no annotation at all are skipped. Files are returned in
/// name order.
pub fn load_giantsteps(dir: &Path) -> Result<Vec<GroundTruth>, ScoreboardError> {
    let audio = files_in(&dir.join("audio"))?;
    let annotations = dir.join("annotations");
    let tempo_annotations = annotations_in(&annotations.join("tempo"))?;
    let beat_annotations = annotations_in(&annotations.join("beats"))?;
    let key_annotations = annotations_in(&annotations.join("key"))?;

    let mut truth = Vec::new();
    for path in audio.values() {
        if !is_audio(path) {
            continue;
        }
        let Some(name) = stem_of(path) else {
            continue;
        };
        let bpm = tempo_annotations
            .get(&name)
            .map(PathBuf::as_path)
            .map(read_bpm)
            .transpose()?;
        let beats = beat_annotations
            .get(&name)
            .map(PathBuf::as_path)
            .map(read_beats)
            .transpose()?;
        let key = key_annotations
            .get(&name)
            .map(PathBuf::as_path)
            .map(read_key)
            .transpose()?;
        if bpm.is_none() && beats.is_none() && key.is_none() {
            continue;
        }
        truth.push(GroundTruth {
            name,
            audio: path.clone(),
            bpm,
            beats,
            key,
        });
    }
    if truth.is_empty() {
        return Err(ScoreboardError::Empty(dir.to_path_buf()));
    }
    Ok(truth)
}

/// The pitch class the given number of semitones above C.
fn pitch_class(semitones: u32) -> PitchClass {
    match semitones % 12 {
        0 => PitchClass::C,
        1 => PitchClass::Cs,
        2 => PitchClass::D,
        3 => PitchClass::Ds,
        4 => PitchClass::E,
        5 => PitchClass::F,
        6 => PitchClass::Fs,
        7 => PitchClass::G,
        8 => PitchClass::Gs,
        9 => PitchClass::A,
        10 => PitchClass::As,
        _ => PitchClass::B,
    }
}

/// Reads a key annotation such as `F minor`, `C# major`, `Bb minor`, `Fm`, or `A`.
///
/// A tonic is a letter A to G with an optional sharp (`#`) or flat (`b`); a
/// mode is `major`, `minor`, `maj`, `min`, or `m` for minor, and a bare tonic
/// is major. Case does not matter. Returns `None` for anything else.
pub fn parse_key(text: &str) -> Option<Key> {
    let mut letters = text.trim().chars();
    let semitones: u32 = match letters.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    // A flat is written `b`, which is also a note name, but the tonic is only
    // ever the first letter, so a `b` in second place is always the accidental.
    let mut rest = letters.as_str();
    let semitones = match rest.chars().next() {
        Some('#') => {
            rest = &rest[1..];
            semitones + 1
        }
        Some('b' | 'B') => {
            rest = &rest[1..];
            semitones + 11
        }
        _ => semitones,
    };
    let mode = match rest.trim().to_ascii_lowercase().as_str() {
        "" | "major" | "maj" => Mode::Major,
        "minor" | "min" | "m" => Mode::Minor,
        _ => return None,
    };
    Some(Key {
        tonic: pitch_class(semitones),
        mode,
    })
}

/// How far an estimated tempo may sit from the reference and still count: four percent.
const TEMPO_TOLERANCE: f64 = 0.04;

/// The multiples of the reference tempo that [`tempo_accuracy2`] forgives: the
/// tempo itself, double and half it, and triple and a third of it.
const TEMPO_FACTORS: [f64; 5] = [1.0, 2.0, 0.5, 3.0, 1.0 / 3.0];

/// Whether an estimated tempo is within four percent of the reference.
pub fn tempo_accuracy1(estimate: Bpm, reference: Bpm) -> bool {
    (estimate.0 - reference.0).abs() <= TEMPO_TOLERANCE * reference.0.abs()
}

/// Whether an estimated tempo is within four percent of the reference, or of
/// the reference doubled, halved, tripled, or divided by three.
pub fn tempo_accuracy2(estimate: Bpm, reference: Bpm) -> bool {
    TEMPO_FACTORS
        .iter()
        .any(|factor| tempo_accuracy1(estimate, Bpm(reference.0 * factor)))
}

/// The F-measure of estimated beat times against reference beat times.
///
/// An estimated beat counts as correct if it lies within `tolerance` of a
/// reference beat that no other estimate has already claimed. The result is
/// the harmonic mean of precision and recall, from zero to one; two empty
/// lists score one, and one empty list scores zero.
pub fn beat_f_measure(estimate: &[Seconds], reference: &[Seconds], tolerance: Seconds) -> f64 {
    if estimate.is_empty() && reference.is_empty() {
        return 1.0;
    }
    if estimate.is_empty() || reference.is_empty() {
        return 0.0;
    }
    let mut claimed = vec![false; reference.len()];
    let mut hits = 0usize;
    for beat in estimate {
        // This gives each estimate the nearest reference beat still free, so
        // two estimates on one reference beat are one hit and one miss.
        let mut nearest: Option<(usize, f64)> = None;
        for (index, other) in reference.iter().enumerate() {
            if claimed[index] {
                continue;
            }
            let distance = (beat.0 - other.0).abs();
            if distance > tolerance.0 {
                continue;
            }
            if nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some((index, distance));
            }
        }
        if let Some((index, _)) = nearest {
            claimed[index] = true;
            hits += 1;
        }
    }
    let precision = hits as f64 / estimate.len() as f64;
    let recall = hits as f64 / reference.len() as f64;
    if precision + recall == 0.0 {
        return 0.0;
    }
    2.0 * precision * recall / (precision + recall)
}

/// The tolerance within which a beat counts as found: seventy milliseconds.
pub const BEAT_TOLERANCE: Seconds = Seconds(0.07);

/// How one analyzer did on one track.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackScore {
    /// The track's name.
    pub name: String,
    /// The tempo the analyzer reported, or `None` if it failed.
    pub bpm: Option<Bpm>,
    /// Whether the tempo was within four percent, or `None` without a tempo annotation or a result.
    pub accuracy1: Option<bool>,
    /// Whether the tempo was within four percent allowing octave errors, or `None` as above.
    pub accuracy2: Option<bool>,
    /// The beat F-measure, or `None` without a beat annotation or a result.
    pub f_measure: Option<f64>,
    /// How far the tempo was from the annotation, as a fraction of the
    /// annotated tempo, or `None` without a tempo annotation or a result.
    pub tempo_error: Option<f64>,
    /// The confidence the analyzer reported, from zero to one, or `None` if it failed.
    pub confidence: Option<f64>,
    /// How long the analyzer took on this track.
    pub seconds: f64,
}

/// One analyzer's line on the scoreboard.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The analyzer's name.
    pub analyzer: String,
    /// How many tracks it was run on.
    pub tracks: usize,
    /// How many tracks it failed on.
    pub failures: usize,
    /// The share of tempo-annotated tracks within four percent, from zero to one, or `None` if there were none.
    pub tempo_accuracy1: Option<f64>,
    /// The share of tempo-annotated tracks within four percent allowing octave errors, or `None` if there were none.
    pub tempo_accuracy2: Option<f64>,
    /// The mean beat F-measure over beat-annotated tracks, or `None` if there were none.
    pub beat_f_measure: Option<f64>,
    /// The median tempo error over tempo-annotated tracks, as a fraction of
    /// the annotated tempo, or `None` if there were none. The four-percent
    /// accuracies say whether an analyzer is roughly right; this says how
    /// precisely, which is what decides whether a grid built from its tempo
    /// stays on the drums to the end of a track.
    pub tempo_error: Option<f64>,
    /// The mean seconds per track.
    pub seconds_per_track: f64,
    /// The score on every track, in the order the ground truth was given.
    pub per_track: Vec<TrackScore>,
}

/// The scoreboard for one run.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// One row per analyzer, in the order the analyzers were given.
    pub rows: Vec<Row>,
}

/// The mean of the values, or `None` if there are none.
pub(crate) fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0usize;
    for value in values {
        total += value;
        count += 1;
    }
    (count > 0).then(|| total / count as f64)
}

/// The share of the answers that are true, or `None` if there are none.
pub(crate) fn share(answers: impl Iterator<Item = bool>) -> Option<f64> {
    mean(answers.map(|answer| if answer { 1.0 } else { 0.0 }))
}

/// The median of the values, or `None` if there are none. With an even count
/// this is the mean of the two middle values.
fn median(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<f64> = values.collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2.0
    })
}

/// A metric as a percentage with one decimal, or a dash when no track had the
/// annotation the metric needs.
pub(crate) fn percent(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{:.1}", value * 100.0),
        None => "-".to_owned(),
    }
}

/// Renders a scoreboard table from its cells, the header included as the
/// first row: one line per row, columns separated by two spaces, the first
/// column left-aligned so every line begins with the row's name and every
/// other column right-aligned.
pub(crate) fn render_table<const N: usize>(cells: &[[String; N]]) -> String {
    let mut widths = [0usize; N];
    for line in cells {
        for (width, cell) in widths.iter_mut().zip(line) {
            *width = (*width).max(cell.chars().count());
        }
    }

    let mut table = String::new();
    for line in cells {
        let mut text = String::new();
        for (column, (cell, width)) in line.iter().zip(widths).enumerate() {
            if column > 0 {
                text.push_str("  ");
            }
            // The first column, the heading or the analyzer's name, is
            // left-aligned so every line begins with it.
            if column == 0 {
                text.push_str(&format!("{cell:<width$}"));
            } else {
                text.push_str(&format!("{cell:>width$}"));
            }
        }
        table.push_str(text.trim_end());
        table.push('\n');
    }
    table
}

/// The column headings of the table, in order. The `error %` column is the
/// median tempo error as a percentage of the annotated tempo.
const COLUMNS: [&str; 8] = [
    "analyzer", "tracks", "failures", "tempo 1", "tempo 2", "error %", "beats", "s/track",
];

impl Report {
    /// The report as a text table for a terminal: one header line, then one
    /// line per analyzer with its name, track count, failures, tempo
    /// accuracies as percentages, median tempo error as a percentage with two
    /// decimals, beat F-measure as a percentage, and seconds per track. A
    /// metric with no annotated tracks is shown as `-`.
    pub fn table(&self) -> String {
        let mut cells: Vec<[String; COLUMNS.len()]> = Vec::with_capacity(self.rows.len() + 1);
        cells.push(COLUMNS.map(str::to_owned));
        for row in &self.rows {
            cells.push([
                row.analyzer.clone(),
                row.tracks.to_string(),
                row.failures.to_string(),
                percent(row.tempo_accuracy1),
                percent(row.tempo_accuracy2),
                match row.tempo_error {
                    Some(error) => format!("{:.2}", error * 100.0),
                    None => "-".to_owned(),
                },
                percent(row.beat_f_measure),
                format!("{:.3}", row.seconds_per_track),
            ]);
        }
        render_table(&cells)
    }
}

/// Fills in one analyzer's summary from the tracks it has already been scored on.
///
/// A track the analyzer failed on has no metrics, so it counts toward the
/// failures and the time per track but not toward the accuracies or the
/// F-measure.
fn summarize(row: &mut Row) {
    row.tempo_accuracy1 = share(row.per_track.iter().filter_map(|track| track.accuracy1));
    row.tempo_accuracy2 = share(row.per_track.iter().filter_map(|track| track.accuracy2));
    row.beat_f_measure = mean(row.per_track.iter().filter_map(|track| track.f_measure));
    row.tempo_error = median(row.per_track.iter().filter_map(|track| track.tempo_error));
    let total: f64 = row.per_track.iter().map(|track| track.seconds).sum();
    row.seconds_per_track = if row.tracks == 0 {
        0.0
    } else {
        total / row.tracks as f64
    };
}

/// Runs every analyzer over every ground-truth track and scores it.
///
/// Each audio file is decoded once and handed to every analyzer. An analyzer
/// that returns an error on a track is counted as a failure on that track
/// and the run continues.
pub fn run(
    truth: &[GroundTruth],
    analyzers: &[&dyn BeatAnalyzer],
) -> Result<Report, ScoreboardError> {
    let mut rows: Vec<Row> = analyzers
        .iter()
        .map(|analyzer| Row {
            analyzer: analyzer.name().to_owned(),
            tracks: truth.len(),
            failures: 0,
            tempo_accuracy1: None,
            tempo_accuracy2: None,
            beat_f_measure: None,
            tempo_error: None,
            seconds_per_track: 0.0,
            per_track: Vec::with_capacity(truth.len()),
        })
        .collect();

    for track in truth {
        let decoded = dermixen_media::decode(&track.audio)
            .map_err(|error| decode_error(&track.audio, error.to_string()))?;
        for (row, analyzer) in rows.iter_mut().zip(analyzers) {
            let started = Instant::now();
            let result = analyzer.analyze(&decoded.audio);
            let seconds = started.elapsed().as_secs_f64();
            let score = match result {
                Ok(analysis) => {
                    let beats: Vec<Seconds> = analysis
                        .beats
                        .iter()
                        .map(|beat| beat.to_seconds())
                        .collect();
                    TrackScore {
                        name: track.name.clone(),
                        bpm: Some(analysis.bpm),
                        accuracy1: track
                            .bpm
                            .map(|reference| tempo_accuracy1(analysis.bpm, reference)),
                        accuracy2: track
                            .bpm
                            .map(|reference| tempo_accuracy2(analysis.bpm, reference)),
                        f_measure: track
                            .beats
                            .as_ref()
                            .map(|reference| beat_f_measure(&beats, reference, BEAT_TOLERANCE)),
                        tempo_error: track
                            .bpm
                            .map(|reference| (analysis.bpm.0 - reference.0).abs() / reference.0),
                        confidence: Some(analysis.confidence),
                        seconds,
                    }
                }
                Err(_) => {
                    row.failures += 1;
                    TrackScore {
                        name: track.name.clone(),
                        bpm: None,
                        accuracy1: None,
                        accuracy2: None,
                        f_measure: None,
                        tempo_error: None,
                        confidence: None,
                        seconds,
                    }
                }
            };
            row.per_track.push(score);
        }
    }

    for row in &mut rows {
        summarize(row);
    }
    Ok(Report { rows })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use dermixen_media::{Audio, WavDepth, write_wav};

    use super::*;
    use crate::analyzer::FixedTempo;

    /// Writes a second of silence as a playable audio file.
    fn write_silence(path: &Path) {
        let audio = Audio {
            frames: vec![[0.0, 0.0]; 44_100],
        };
        write_wav(path, &audio, WavDepth::Int16).unwrap();
    }

    #[test]
    fn only_files_with_an_audio_extension_become_ground_truth() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        write_silence(&dir.join("track.wav"));
        fs::write(dir.join("track.bpm"), "128\n").unwrap();
        // An audio extension in capitals is still an audio extension.
        write_silence(&dir.join("other.WAV"));
        fs::write(dir.join("other.bpm"), "130\n").unwrap();
        // None of these are audio, so none of them gain an entry of their own.
        fs::write(dir.join("notes.txt"), "a note to self\n").unwrap();
        fs::write(dir.join("README"), "no extension at all\n").unwrap();
        fs::write(dir.join("track.jpg"), "artwork\n").unwrap();

        let truth = load_directory(dir).unwrap();
        let names: Vec<&str> = truth.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["other", "track"]);
        assert_eq!(truth[0].audio, dir.join("other.WAV"));
        assert_eq!(truth[1].audio, dir.join("track.wav"));
        assert_eq!(truth[1].bpm, Some(Bpm(128.0)));
    }

    #[test]
    fn beat_times_that_go_backwards_are_reported_by_line() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        write_silence(&dir.join("track.wav"));
        let beats = dir.join("track.beats");
        fs::write(&beats, "0.000000 1\n1.000000 2\n0.500000 3\n").unwrap();
        match load_directory(dir) {
            Err(ScoreboardError::Annotation { path, message }) => {
                assert_eq!(path, beats);
                assert!(message.contains("line 3"), "{message}");
            }
            other => panic!("expected an annotation error, got {other:?}"),
        }
    }

    #[test]
    fn audio_that_cannot_be_decoded_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        let audio = dir.join("track.wav");
        fs::write(&audio, "this is not a wave file").unwrap();
        fs::write(dir.join("track.bpm"), "128\n").unwrap();
        let truth = load_directory(dir).unwrap();
        let analyzer = FixedTempo(Bpm(128.0));
        match run(&truth, &[&analyzer]) {
            Err(ScoreboardError::Decode { path, message }) => {
                assert_eq!(path, audio);
                assert!(!message.is_empty());
            }
            other => panic!("expected a decode error, got {other:?}"),
        }
    }
}
