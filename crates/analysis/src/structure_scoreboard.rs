//! The anchor scoreboard: ground truth for where a track's music begins and
//! ends and where its anchors go, the metrics that say whether an anchor
//! landed on the right bar, and the report that puts every anchor analyzer
//! in one table.
//!
//! Ground truth is a directory of audio files with `name.anchors` annotation
//! files beside them; `docs/ground-truth.md` describes the format. Every
//! label names its source, because the labels have three sources of
//! different standing: a position confirmed by ear, a position read from a
//! MixMeister project file, and a default MixMeister's analysis wrote into
//! the track's plot file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, Bpm, Samples, Seconds};

use crate::analyzer::BeatAnalyzer;
use crate::scoreboard::{BEAT_TOLERANCE, ScoreboardError, tempo_accuracy1, tempo_accuracy2};
use crate::structure::AnchorAnalyzer;

/// The source of a label, in descending order of standing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A position confirmed by listening, which is the standard the other
    /// two sources are measured against.
    Ear,
    /// A position read from a MixMeister project file: the cue as the intro
    /// anchor and the measure marker as the outro anchor.
    MixMeister,
    /// The default MixMeister's own analysis wrote into the track's plot file.
    Plot,
}

impl Source {
    /// The word that names the source in an annotation file and on the report.
    pub fn name(self) -> &'static str {
        match self {
            Source::Ear => "ear",
            Source::MixMeister => "mixmeister",
            Source::Plot => "plot",
        }
    }

    /// Every source, in descending order of standing.
    pub const ALL: [Source; 3] = [Source::Ear, Source::MixMeister, Source::Plot];
}

/// The reason a word is not the name of a source.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a label's source is ear, mixmeister, or plot, got {0:?}")]
pub struct SourceParseError(pub String);

impl FromStr for Source {
    type Err = SourceParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "ear" => Ok(Source::Ear),
            "mixmeister" => Ok(Source::MixMeister),
            "plot" => Ok(Source::Plot),
            other => Err(SourceParseError(other.to_owned())),
        }
    }
}

/// One labeled position in a track and the label's source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Label {
    /// The position, in seconds from the first sample of the file.
    pub at: Seconds,
    /// The label's source.
    pub source: Source,
}

/// What is known to be true about one track's structure.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorTruth {
    /// The track's name, which is its annotation file name without the extension.
    pub name: String,
    /// The audio file.
    pub audio: PathBuf,
    /// The grid the labels sit on and the grid every analyzer is handed.
    pub grid: BeatGrid,
    /// Where the music effectively begins, if labeled.
    pub begins: Option<Label>,
    /// Where the music effectively ends, if labeled.
    pub ends: Option<Label>,
    /// Where the intro anchor belongs, if labeled.
    pub intro: Option<Label>,
    /// Where the outro anchor belongs, if labeled.
    pub outro: Option<Label>,
    /// The bars a transition was aligned to, in time order: each is a bar
    /// the label's source treated as a place a transition may start.
    pub phrases: Vec<Label>,
    /// The bars at which the arrangement changes, in time order.
    pub sections: Vec<Label>,
}

/// What a ground-truth directory holds: the tracks whose audio is present,
/// and the annotation files that had no audio beside them.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorTruthDirectory {
    /// The tracks with both an annotation and audio, in name order.
    pub tracks: Vec<AnchorTruth>,
    /// The annotation files with no audio beside them, in name order. The
    /// committed annotations are useful without the audio, which is not
    /// committed, so a missing file is reported rather than treated as an error.
    pub without_audio: Vec<PathBuf>,
    /// Labels that contradict one another within one annotation, such as an
    /// outro anchor that lies after the effective ending, one line each.
    /// The labels are kept as their source gave them; the contradiction is
    /// reported so a reader of the scoreboard knows about it.
    pub warnings: Vec<String>,
}

/// The extension of an annotation file.
pub const ANCHORS_EXTENSION: &str = "anchors";

/// The extensions of the files the scoreboard treats as audio.
const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "mp3", "flac", "m4a", "mp4"];

/// The keys an annotation file may hold. `file` records the audio file's
/// path below an audio root, for the tool that gathers audio
/// beside the annotations; the loader reads and ignores it. `phrase` and
/// `section` may appear any number of times; the others at most once.
const KEYS: [&str; 9] = [
    "bpm",
    "first_beat",
    "begins",
    "ends",
    "intro",
    "outro",
    "phrase",
    "section",
    "file",
];

/// The keys that may appear any number of times in one annotation.
const REPEATABLE_KEYS: [&str; 2] = ["phrase", "section"];

fn read_error(path: &Path, source: std::io::Error) -> ScoreboardError {
    ScoreboardError::Read {
        path: path.to_path_buf(),
        source,
    }
}

fn annotation_error(path: &Path, message: impl Into<String>) -> ScoreboardError {
    ScoreboardError::Annotation {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

/// Reads a number of seconds from one field of an annotation line.
fn read_seconds(path: &Path, number: usize, field: &str) -> Result<Seconds, ScoreboardError> {
    let value: f64 = field.parse().map_err(|_| {
        annotation_error(
            path,
            format!("line {number}: {field} is not a time in seconds"),
        )
    })?;
    if !value.is_finite() {
        return Err(annotation_error(
            path,
            format!("line {number}: {field} is not a time in seconds"),
        ));
    }
    Ok(Seconds(value))
}

/// Reads one annotation file. The audio path is filled in by the caller.
fn read_anchors(path: &Path) -> Result<AnchorTruth, ScoreboardError> {
    let text = std::fs::read_to_string(path).map_err(|source| read_error(path, source))?;
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| annotation_error(path, "the file name is not text"))?
        .to_owned();

    let mut bpm: Option<Bpm> = None;
    let mut first_beat: Option<Seconds> = None;
    let mut labels: BTreeMap<&str, Label> = BTreeMap::new();
    let mut phrases: Vec<Label> = Vec::new();
    let mut sections: Vec<Label> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(2, char::is_whitespace);
        let key = fields.next().unwrap_or_default();
        let rest = fields.next().unwrap_or_default().trim();
        if !KEYS.contains(&key) {
            return Err(annotation_error(
                path,
                format!("line {number}: {key} is not one of {}", KEYS.join(", ")),
            ));
        }
        if rest.is_empty() {
            return Err(annotation_error(
                path,
                format!("line {number}: {key} has no value"),
            ));
        }
        match key {
            "file" => {}
            "bpm" => {
                let value: f64 = rest.parse().map_err(|_| {
                    annotation_error(path, format!("line {number}: {rest} is not a number"))
                })?;
                let value = Bpm(value);
                if !value.is_valid() {
                    return Err(annotation_error(
                        path,
                        format!("line {number}: {rest} is not a usable tempo"),
                    ));
                }
                bpm = Some(value);
            }
            "first_beat" => first_beat = Some(read_seconds(path, number, rest)?),
            _ => {
                let mut parts = rest.split_whitespace();
                let at = read_seconds(path, number, parts.next().unwrap_or_default())?;
                let source = parts.next().ok_or_else(|| {
                    annotation_error(
                        path,
                        format!("line {number}: {key} names no source (ear, mixmeister, or plot)"),
                    )
                })?;
                let source: Source = source.parse().map_err(|error: SourceParseError| {
                    annotation_error(path, format!("line {number}: {error}"))
                })?;
                let label = Label { at, source };
                if REPEATABLE_KEYS.contains(&key) {
                    if key == "phrase" {
                        phrases.push(label);
                    } else {
                        sections.push(label);
                    }
                } else if labels.insert(key, label).is_some() {
                    return Err(annotation_error(
                        path,
                        format!("line {number}: {key} is given twice"),
                    ));
                }
            }
        }
    }

    let bpm = bpm.ok_or_else(|| annotation_error(path, "the file gives no bpm"))?;
    let first_beat =
        first_beat.ok_or_else(|| annotation_error(path, "the file gives no first_beat"))?;
    let grid = BeatGrid {
        first_beat: first_beat.to_samples(),
        bpm,
    };
    let (intro, outro) = (labels.get("intro").copied(), labels.get("outro").copied());
    if let (Some(intro), Some(outro)) = (intro, outro)
        && outro.at <= intro.at
    {
        return Err(annotation_error(
            path,
            "the outro anchor comes before the intro anchor",
        ));
    }
    phrases.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    sections.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
    Ok(AnchorTruth {
        name,
        audio: PathBuf::new(),
        grid,
        begins: labels.get("begins").copied(),
        ends: labels.get("ends").copied(),
        intro,
        outro,
        phrases,
        sections,
    })
}

/// Whether a file is audio, judged by its extension in either case.
fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            AUDIO_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// Loads anchor ground truth from a directory: every `name.anchors` file,
/// with the audio file `name.ext` beside it when there is one.
///
/// Tracks come back in name order. A directory with no annotation file at
/// all is an error; annotation files without audio are listed rather than
/// loaded.
pub fn load_anchor_truth(dir: &Path) -> Result<AnchorTruthDirectory, ScoreboardError> {
    let listing = std::fs::read_dir(dir).map_err(|source| read_error(dir, source))?;
    let mut annotations: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut audio: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in listing {
        let entry = entry.map_err(|source| read_error(dir, source))?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let is_annotation = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension == ANCHORS_EXTENSION);
        if is_annotation {
            annotations.insert(stem.to_owned(), path);
        } else if is_audio(&path) {
            audio.insert(stem.to_owned(), path);
        }
    }
    if annotations.is_empty() {
        return Err(ScoreboardError::Empty(dir.to_path_buf()));
    }

    let mut tracks = Vec::new();
    let mut without_audio = Vec::new();
    let mut warnings = Vec::new();
    for (stem, path) in &annotations {
        let mut truth = read_anchors(path)?;
        if let (Some(outro), Some(ends)) = (truth.outro, truth.ends)
            && outro.at > ends.at
        {
            warnings.push(format!(
                "{}: the outro anchor at {:.3} s lies after the effective ending at {:.3} s",
                truth.name, outro.at.0, ends.at.0
            ));
        }
        match audio.get(stem) {
            Some(audio) => {
                truth.audio = audio.clone();
                tracks.push(truth);
            }
            None => without_audio.push(path.clone()),
        }
    }
    Ok(AnchorTruthDirectory {
        tracks,
        without_audio,
        warnings,
    })
}

/// How far an estimated position sits from a labeled one, in beats at the
/// track's tempo: positive when the estimate is late.
pub fn error_in_beats(estimate: Seconds, label: Seconds, bpm: Bpm) -> f64 {
    (estimate - label).beats_at(bpm).0
}

/// An anchor within this many beats of its label counts as on the right bar:
/// one bar either side.
pub const BAR_TOLERANCE: f64 = BEATS_PER_BAR as f64;

/// An anchor within this many beats of its label counts as close enough for
/// the transition to work: four bars, half of the default eight-bar overlap.
pub const FOUR_BAR_TOLERANCE: f64 = 4.0 * BEATS_PER_BAR as f64;

/// How one analyzer did on one labeled position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionScore {
    /// The label's source.
    pub source: Source,
    /// The error in beats, positive when the estimate is late.
    pub error: f64,
}

/// How one analyzer did on one track.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorTrackScore {
    /// The track's name.
    pub name: String,
    /// The intro anchor's error, or `None` without a label or a result.
    pub intro: Option<PositionScore>,
    /// The outro anchor's error, or `None` without a label or a result.
    pub outro: Option<PositionScore>,
    /// The effective beginning's error, or `None` without a label or a result.
    pub begins: Option<PositionScore>,
    /// The effective ending's error, or `None` without a label or a result.
    pub ends: Option<PositionScore>,
    /// The confidence the analyzer reported, or `None` if it failed.
    pub confidence: Option<f64>,
    /// How long the analyzer took on this track.
    pub seconds: f64,
}

/// The summary of one kind of position over the tracks that label it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionSummary {
    /// How many tracks label this position and got a result.
    pub scored: usize,
    /// The share of those within one bar of the label, or `None` if there are none.
    pub within_bar: Option<f64>,
    /// The share of those within four bars of the label, or `None` if there are none.
    pub within_four_bars: Option<f64>,
    /// The median absolute error in beats, or `None` if there are none.
    pub median_beats: Option<f64>,
}

impl PositionSummary {
    /// Summarizes the errors of every scored position, in beats.
    fn of(errors: &[f64]) -> PositionSummary {
        let scored = errors.len();
        if scored == 0 {
            return PositionSummary {
                scored,
                within_bar: None,
                within_four_bars: None,
                median_beats: None,
            };
        }
        let share = |tolerance: f64| {
            Some(
                errors
                    .iter()
                    .filter(|error| error.abs() <= tolerance)
                    .count() as f64
                    / scored as f64,
            )
        };
        let absolute: Vec<f64> = errors.iter().map(|error| error.abs()).collect();
        PositionSummary {
            scored,
            within_bar: share(BAR_TOLERANCE),
            within_four_bars: share(FOUR_BAR_TOLERANCE),
            median_beats: median(&absolute),
        }
    }
}

/// One analyzer's line on the anchor scoreboard, for one source of labels
/// or for all of them together.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorRow {
    /// The analyzer's name.
    pub analyzer: String,
    /// The source the row is restricted to, or `None` for every source.
    pub source: Option<Source>,
    /// How many tracks the analyzer was run on.
    pub tracks: usize,
    /// How many tracks it failed on.
    pub failures: usize,
    /// The intro anchor summary.
    pub intro: PositionSummary,
    /// The outro anchor summary.
    pub outro: PositionSummary,
    /// The effective beginning summary.
    pub begins: PositionSummary,
    /// The effective ending summary.
    pub ends: PositionSummary,
    /// The mean seconds per track.
    pub seconds_per_track: f64,
}

/// The anchor scoreboard for one run.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorReport {
    /// One row per analyzer over every source, then one row per analyzer
    /// and source that has at least one label, in analyzer order.
    pub rows: Vec<AnchorRow>,
    /// Every analyzer's score on every track, keyed by analyzer name, in the
    /// order the ground truth was given.
    pub per_track: BTreeMap<String, Vec<AnchorTrackScore>>,
}

/// A share as a percentage with one decimal, or a dash when nothing was scored.
fn percent(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{:.1}", value * 100.0),
        None => "-".to_owned(),
    }
}

/// A median error in beats with one decimal, or a dash when nothing was scored.
fn beats(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.1}"),
        None => "-".to_owned(),
    }
}

/// The column headings of the table. For each of the four positions the
/// columns are how many tracks were scored, the share within one bar, the
/// share within four bars, and the median absolute error in beats.
const COLUMNS: [&str; 21] = [
    "analyzer",
    "labels",
    "tracks",
    "failures",
    "intro n",
    "bar %",
    "4 bars %",
    "med beats",
    "outro n",
    "bar %",
    "4 bars %",
    "med beats",
    "begins n",
    "bar %",
    "4 bars %",
    "med beats",
    "ends n",
    "bar %",
    "4 bars %",
    "med beats",
    "s/track",
];

impl AnchorReport {
    /// The report as a text table for a terminal: one header line, then one
    /// line per row. The `labels` column says which source the row counts,
    /// or `all`.
    pub fn table(&self) -> String {
        let mut cells: Vec<[String; COLUMNS.len()]> = Vec::with_capacity(self.rows.len() + 1);
        cells.push(COLUMNS.map(str::to_owned));
        for row in &self.rows {
            let position = |summary: &PositionSummary| {
                [
                    summary.scored.to_string(),
                    percent(summary.within_bar),
                    percent(summary.within_four_bars),
                    beats(summary.median_beats),
                ]
            };
            let mut line: Vec<String> = vec![
                row.analyzer.clone(),
                row.source.map_or("all", Source::name).to_owned(),
                row.tracks.to_string(),
                row.failures.to_string(),
            ];
            for summary in [&row.intro, &row.outro, &row.begins, &row.ends] {
                line.extend(position(summary));
            }
            line.push(format!("{:.3}", row.seconds_per_track));
            let line: [String; COLUMNS.len()] =
                line.try_into().expect("a line has one cell per column");
            cells.push(line);
        }

        let mut widths = [0usize; COLUMNS.len()];
        for line in &cells {
            for (width, cell) in widths.iter_mut().zip(line) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let mut table = String::new();
        for line in &cells {
            let mut text = String::new();
            for (column, (cell, width)) in line.iter().zip(widths).enumerate() {
                if column > 0 {
                    text.push_str("  ");
                }
                if column < 2 {
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

    /// Every track's errors for one analyzer as text, one line per track:
    /// the name, then each labeled position's error in beats with its
    /// source, and the confidence. A position the analyzer did not score
    /// shows a dash. Returns an empty string for an analyzer not in the report.
    pub fn details(&self, analyzer: &str) -> String {
        let Some(scores) = self.per_track.get(analyzer) else {
            return String::new();
        };
        let cell = |score: Option<PositionScore>| match score {
            Some(score) => format!("{:+.1} ({})", score.error, score.source.name()),
            None => "-".to_owned(),
        };
        let mut text = String::new();
        for score in scores {
            let confidence = match score.confidence {
                Some(confidence) => format!("{confidence:.2}"),
                None => "failed".to_owned(),
            };
            text.push_str(&format!(
                "{}: intro {}, outro {}, begins {}, ends {}, confidence {confidence}\n",
                score.name,
                cell(score.intro),
                cell(score.outro),
                cell(score.begins),
                cell(score.ends),
            ));
        }
        text
    }
}

/// Summarizes one analyzer's scores, over every source or over one.
fn summarize(analyzer: &str, source: Option<Source>, scores: &[AnchorTrackScore]) -> AnchorRow {
    let errors = |pick: fn(&AnchorTrackScore) -> Option<PositionScore>| -> Vec<f64> {
        scores
            .iter()
            .filter_map(pick)
            .filter(|score| source.is_none_or(|wanted| score.source == wanted))
            .map(|score| score.error)
            .collect()
    };
    let total: f64 = scores.iter().map(|score| score.seconds).sum();
    AnchorRow {
        analyzer: analyzer.to_owned(),
        source,
        tracks: scores.len(),
        failures: scores
            .iter()
            .filter(|score| score.confidence.is_none())
            .count(),
        intro: PositionSummary::of(&errors(|score| score.intro)),
        outro: PositionSummary::of(&errors(|score| score.outro)),
        begins: PositionSummary::of(&errors(|score| score.begins)),
        ends: PositionSummary::of(&errors(|score| score.ends)),
        seconds_per_track: if scores.is_empty() {
            0.0
        } else {
            total / scores.len() as f64
        },
    }
}

/// Runs every anchor analyzer over every ground-truth track and scores it.
///
/// Each audio file is decoded once and handed to every analyzer along with
/// the track's labeled grid, so the scores measure anchor placement alone
/// and not the beat tracker. An analyzer's anchors are turned into times
/// through that grid; the extent is already in samples. An analyzer that
/// returns an error on a track is counted as a failure on that track and
/// the run continues.
pub fn run_anchors(
    truth: &[AnchorTruth],
    analyzers: &[&dyn AnchorAnalyzer],
) -> Result<AnchorReport, ScoreboardError> {
    let mut per_track: BTreeMap<String, Vec<AnchorTrackScore>> = BTreeMap::new();
    for track in truth {
        let decoded =
            dermixen_media::decode(&track.audio).map_err(|error| ScoreboardError::Decode {
                path: track.audio.clone(),
                message: error.to_string(),
            })?;
        for analyzer in analyzers {
            let started = Instant::now();
            let result = analyzer.analyze(&decoded.audio, &track.grid);
            let seconds = started.elapsed().as_secs_f64();
            let bpm = track.grid.bpm;
            let score = match result {
                Ok(analysis) => {
                    let position = |label: Option<Label>, estimate: Seconds| {
                        label.map(|label| PositionScore {
                            source: label.source,
                            error: error_in_beats(estimate, label.at, bpm),
                        })
                    };
                    let sample_time = |sample: Samples| sample.to_seconds();
                    AnchorTrackScore {
                        name: track.name.clone(),
                        intro: position(track.intro, track.grid.time_of(analysis.anchors.intro)),
                        outro: position(track.outro, track.grid.time_of(analysis.anchors.outro)),
                        begins: position(track.begins, sample_time(analysis.extent.begins)),
                        ends: position(track.ends, sample_time(analysis.extent.ends)),
                        confidence: Some(analysis.confidence),
                        seconds,
                    }
                }
                Err(_) => AnchorTrackScore {
                    name: track.name.clone(),
                    intro: None,
                    outro: None,
                    begins: None,
                    ends: None,
                    confidence: None,
                    seconds,
                },
            };
            per_track
                .entry(analyzer.name().to_owned())
                .or_default()
                .push(score);
        }
    }

    let mut rows = Vec::new();
    for analyzer in analyzers {
        let scores = per_track
            .get(analyzer.name())
            .map(Vec::as_slice)
            .unwrap_or_default();
        rows.push(summarize(analyzer.name(), None, scores));
        for source in Source::ALL {
            let row = summarize(analyzer.name(), Some(source), scores);
            let labeled = row.intro.scored + row.outro.scored + row.begins.scored + row.ends.scored;
            if labeled > 0 {
                rows.push(row);
            }
        }
    }
    Ok(AnchorReport { rows, per_track })
}

/// How one beat analyzer's grid compares with the labeled grid of one track.
#[derive(Debug, Clone, PartialEq)]
pub struct GridTrackScore {
    /// The track's name.
    pub name: String,
    /// The tempo the analyzer reported, or `None` if it failed.
    pub bpm: Option<Bpm>,
    /// Whether the tempo was within four percent of the labeled tempo.
    pub accuracy1: Option<bool>,
    /// Whether the tempo was within four percent allowing octave errors.
    pub accuracy2: Option<bool>,
    /// How far the tempo was from the labeled tempo, as a fraction of it.
    pub tempo_error: Option<f64>,
    /// Whether the analyzer's first beat lies within the beat tolerance of
    /// a beat of the labeled grid.
    pub on_beat: Option<bool>,
    /// Whether the analyzer's first beat lies within the beat tolerance of
    /// a beat of the labeled grid that starts a bar.
    pub on_downbeat: Option<bool>,
    /// Where the analyzer's first beat falls on the labeled grid, in beats
    /// from the labeled beat zero, or `None` if the analyzer failed.
    pub first_beat: Option<Beats>,
    /// The confidence the analyzer reported, or `None` if it failed.
    pub confidence: Option<f64>,
    /// How long the analyzer took on this track.
    pub seconds: f64,
}

/// An analyzer reporting at least this much confidence counts as sure of
/// its tempo when the grid report splits hits from misses by confidence.
pub const SURE_CONFIDENCE: f64 = 0.5;

/// How the tracks an analyzer was sure about and the tracks it was not
/// each divide into tempo hits and misses, where a hit is a tempo within
/// four percent of the labeled one and sure means a confidence of at
/// least [`SURE_CONFIDENCE`]. An honest confidence puts the misses in the
/// unsure half.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfidenceSplit {
    /// Tracks the analyzer was sure about and got within four percent.
    pub sure_hits: usize,
    /// Tracks the analyzer was sure about and got wrong.
    pub sure_misses: usize,
    /// Tracks the analyzer was unsure about and got within four percent.
    pub unsure_hits: usize,
    /// Tracks the analyzer was unsure about and got wrong.
    pub unsure_misses: usize,
}

/// One beat analyzer's line on the grid scoreboard.
#[derive(Debug, Clone, PartialEq)]
pub struct GridRow {
    /// The analyzer's name.
    pub analyzer: String,
    /// How many tracks it was run on.
    pub tracks: usize,
    /// How many tracks it failed on.
    pub failures: usize,
    /// The share of tracks within four percent of the labeled tempo.
    pub tempo_accuracy1: Option<f64>,
    /// The share within four percent allowing octave errors.
    pub tempo_accuracy2: Option<f64>,
    /// The median tempo error as a fraction of the labeled tempo.
    pub tempo_error: Option<f64>,
    /// The share of tracks whose first beat lies on a beat of the labeled grid.
    pub on_beat: Option<f64>,
    /// The share of tracks whose first beat lies on a downbeat of the labeled grid.
    pub on_downbeat: Option<f64>,
    /// How the analyzer's sure and unsure tracks divide into tempo hits and misses.
    pub confidence_split: ConfidenceSplit,
    /// The mean seconds per track.
    pub seconds_per_track: f64,
    /// The score on every track, in the order the ground truth was given.
    pub per_track: Vec<GridTrackScore>,
}

/// The grid scoreboard for one run: how each beat analyzer's tempo and
/// beat zero compare with the labeled grids.
#[derive(Debug, Clone, PartialEq)]
pub struct GridReport {
    /// One row per analyzer, in the order the analyzers were given.
    pub rows: Vec<GridRow>,
}

/// The column headings of the grid table.
const GRID_COLUMNS: [&str; 13] = [
    "analyzer",
    "tracks",
    "failures",
    "tempo 1",
    "tempo 2",
    "error %",
    "on beat %",
    "downbeat %",
    "sure hits",
    "sure misses",
    "unsure hits",
    "unsure misses",
    "s/track",
];

impl GridReport {
    /// The report as a text table: one header line, then one line per
    /// analyzer with its name, track count, failures, the two tempo
    /// accuracies as percentages, the median tempo error as a percentage
    /// with two decimals, the shares of tracks whose first beat lies on a
    /// beat and on a downbeat of the labeled grid, the counts of tempo hits
    /// and misses among the tracks the analyzer was sure and unsure about,
    /// and the seconds per track.
    pub fn table(&self) -> String {
        let mut cells: Vec<[String; GRID_COLUMNS.len()]> = Vec::with_capacity(self.rows.len() + 1);
        cells.push(GRID_COLUMNS.map(str::to_owned));
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
                percent(row.on_beat),
                percent(row.on_downbeat),
                row.confidence_split.sure_hits.to_string(),
                row.confidence_split.sure_misses.to_string(),
                row.confidence_split.unsure_hits.to_string(),
                row.confidence_split.unsure_misses.to_string(),
                format!("{:.3}", row.seconds_per_track),
            ]);
        }
        let mut widths = [0usize; GRID_COLUMNS.len()];
        for line in &cells {
            for (width, cell) in widths.iter_mut().zip(line) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let mut table = String::new();
        for line in &cells {
            let mut text = String::new();
            for (column, (cell, width)) in line.iter().zip(widths).enumerate() {
                if column > 0 {
                    text.push_str("  ");
                }
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

    /// Every track's result for one analyzer as text, one line per track:
    /// the name, the tempo found against the labeled tempo, and whether the
    /// first beat lies on a beat and on a downbeat. Returns an empty string
    /// for an analyzer not in the report.
    pub fn details(&self, analyzer: &str) -> String {
        let Some(row) = self.rows.iter().find(|row| row.analyzer == analyzer) else {
            return String::new();
        };
        let mut text = String::new();
        for score in &row.per_track {
            let yes_no = |answer: Option<bool>| match answer {
                Some(true) => "yes",
                Some(false) => "no",
                None => "-",
            };
            match score.bpm {
                Some(bpm) => text.push_str(&format!(
                    "{}: {:.3} beats per minute, error {:+.2} %, first beat at labeled beat {:.2}, on a beat {}, on a downbeat {}, confidence {:.2}\n",
                    score.name,
                    bpm.0,
                    score.tempo_error.unwrap_or(0.0) * 100.0,
                    score.first_beat.map_or(f64::NAN, |beat| beat.0),
                    yes_no(score.on_beat),
                    yes_no(score.on_downbeat),
                    score.confidence.unwrap_or(0.0),
                )),
                None => text.push_str(&format!("{}: failed\n", score.name)),
            }
        }
        text
    }
}

/// The median of the values, or `None` if there are none.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    })
}

/// The share of the answers that are true, or `None` if there are none.
fn share(answers: impl Iterator<Item = bool>) -> Option<f64> {
    let mut total = 0usize;
    let mut count = 0usize;
    for answer in answers {
        total += usize::from(answer);
        count += 1;
    }
    (count > 0).then(|| total as f64 / count as f64)
}

/// Runs every beat analyzer over every ground-truth track and compares the
/// grid each one finds with the labeled grid.
///
/// The labeled grid's beat zero starts a bar, so whether an analyzer's
/// first beat lands on a beat, and on a beat that starts a bar, can be
/// read off the labeled grid: the first beat is on a beat when it lies
/// within the beat tolerance of the nearest labeled beat, and on a downbeat
/// when that labeled beat is a whole number of bars from beat zero. Each
/// audio file is decoded once and handed to every analyzer, and an analyzer
/// that returns an error on a track is counted as a failure on that track.
pub fn run_grids(
    truth: &[AnchorTruth],
    analyzers: &[&dyn BeatAnalyzer],
) -> Result<GridReport, ScoreboardError> {
    let mut rows: Vec<GridRow> = analyzers
        .iter()
        .map(|analyzer| GridRow {
            analyzer: analyzer.name().to_owned(),
            tracks: truth.len(),
            failures: 0,
            tempo_accuracy1: None,
            tempo_accuracy2: None,
            tempo_error: None,
            on_beat: None,
            on_downbeat: None,
            confidence_split: ConfidenceSplit::default(),
            seconds_per_track: 0.0,
            per_track: Vec::with_capacity(truth.len()),
        })
        .collect();

    for track in truth {
        let decoded =
            dermixen_media::decode(&track.audio).map_err(|error| ScoreboardError::Decode {
                path: track.audio.clone(),
                message: error.to_string(),
            })?;
        for (row, analyzer) in rows.iter_mut().zip(analyzers) {
            let started = Instant::now();
            let result = analyzer.analyze(&decoded.audio);
            let seconds = started.elapsed().as_secs_f64();
            let score = match result {
                Ok(analysis) => {
                    let reference = track.grid.bpm;
                    let first = analysis.beats.first().map(|beat| {
                        let at = track.grid.beat_at_position(*beat);
                        let nearest = at.round();
                        let distance = (at - nearest).at(reference).0.abs();
                        let on_beat = distance <= BEAT_TOLERANCE.0;
                        let bars = nearest.0 / f64::from(BEATS_PER_BAR);
                        (on_beat, on_beat && bars.fract() == 0.0, at)
                    });
                    GridTrackScore {
                        name: track.name.clone(),
                        bpm: Some(analysis.bpm),
                        accuracy1: Some(tempo_accuracy1(analysis.bpm, reference)),
                        accuracy2: Some(tempo_accuracy2(analysis.bpm, reference)),
                        tempo_error: Some((analysis.bpm.0 - reference.0).abs() / reference.0),
                        on_beat: first.map(|(on_beat, _, _)| on_beat),
                        on_downbeat: first.map(|(_, on_downbeat, _)| on_downbeat),
                        first_beat: first.map(|(_, _, at)| at),
                        confidence: Some(analysis.confidence),
                        seconds,
                    }
                }
                Err(_) => {
                    row.failures += 1;
                    GridTrackScore {
                        name: track.name.clone(),
                        bpm: None,
                        accuracy1: None,
                        accuracy2: None,
                        tempo_error: None,
                        on_beat: None,
                        on_downbeat: None,
                        first_beat: None,
                        confidence: None,
                        seconds,
                    }
                }
            };
            row.per_track.push(score);
        }
    }

    for row in &mut rows {
        row.tempo_accuracy1 = share(row.per_track.iter().filter_map(|score| score.accuracy1));
        row.tempo_accuracy2 = share(row.per_track.iter().filter_map(|score| score.accuracy2));
        let errors: Vec<f64> = row
            .per_track
            .iter()
            .filter_map(|score| score.tempo_error)
            .collect();
        row.tempo_error = median(&errors);
        row.on_beat = share(row.per_track.iter().filter_map(|score| score.on_beat));
        row.on_downbeat = share(row.per_track.iter().filter_map(|score| score.on_downbeat));
        let mut split = ConfidenceSplit::default();
        for score in &row.per_track {
            let (Some(confidence), Some(hit)) = (score.confidence, score.accuracy1) else {
                continue;
            };
            let count = match (confidence >= SURE_CONFIDENCE, hit) {
                (true, true) => &mut split.sure_hits,
                (true, false) => &mut split.sure_misses,
                (false, true) => &mut split.unsure_hits,
                (false, false) => &mut split.unsure_misses,
            };
            *count += 1;
        }
        row.confidence_split = split;
        let total: f64 = row.per_track.iter().map(|score| score.seconds).sum();
        row.seconds_per_track = if row.tracks == 0 {
            0.0
        } else {
            total / row.tracks as f64
        };
    }
    Ok(GridReport { rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_in_beats_and_positive_when_late() {
        // At 120 beats per minute a beat is half a second.
        assert_eq!(error_in_beats(Seconds(2.0), Seconds(1.0), Bpm(120.0)), 2.0);
        assert_eq!(error_in_beats(Seconds(1.0), Seconds(2.0), Bpm(120.0)), -2.0);
    }

    #[test]
    fn a_summary_counts_shares_and_takes_the_median() {
        let summary = PositionSummary::of(&[0.0, -4.0, 5.0, 20.0]);
        assert_eq!(summary.scored, 4);
        assert_eq!(summary.within_bar, Some(0.5));
        assert_eq!(summary.within_four_bars, Some(0.75));
        assert_eq!(summary.median_beats, Some(4.5));
        assert_eq!(PositionSummary::of(&[]).median_beats, None);
    }

    #[test]
    fn sources_are_read_by_name() {
        assert_eq!("ear".parse::<Source>(), Ok(Source::Ear));
        assert_eq!("mixmeister".parse::<Source>(), Ok(Source::MixMeister));
        assert_eq!("plot".parse::<Source>(), Ok(Source::Plot));
        assert!("guess".parse::<Source>().is_err());
    }
}
