//! The `scoreboard` command, which measures every built-in analyzer against
//! a directory of ground truth.
//!
//! A directory can hold two kinds of annotation and the command prints the
//! tables that follow from each kind it finds: tempo, beat, and key
//! annotations give the beat and key scoreboards, and the anchor annotations
//! that `docs/ground-truth.md` describes give the anchor, grid, and phrase
//! scoreboards.

use std::path::Path;

use dermixen_analysis::{
    AnchorAnalyzer, AnchorReport, AnchorTrackScore, AnchorTruthDirectory, BeatAnalyzer,
    CountedPhrases, EdgeAnchors, FixedTempo, GridHandling, GridReport, GroundTruth, KeyReport,
    KickAnchors, PhraseAnalyzer, PhraseReport, PositionScore, PositionSummary, PulseGrid, Report,
    ScoreboardError, ShiftPhrases, load_anchor_truth, load_directory, load_giantsteps, run_anchors,
    run_grids, run_phrases,
};
use dermixen_core::Bpm;

use crate::analyze::print_json;

/// The tempo the fixed-tempo analyzer answers with. It is a baseline rather
/// than an estimate: it shows what guessing one common tempo scores.
const FIXED_TEMPO: Bpm = Bpm(120.0);

/// The shapes `scoreboard --json` prints.
///
/// They mirror the `scoreboard` definition in
/// `docs/json/dermixen.schema.json` field for field, and they live in a
/// module of their own so that they can keep the schema's names without
/// colliding with the analysis crate's own report types.
mod document {
    use serde::Serialize;

    /// The whole document: one list of rows per kind of scoreboard.
    #[derive(Debug, Serialize)]
    pub struct Scoreboard {
        /// The beat scoreboard, empty when the directory holds no tempo,
        /// beat, or key annotations.
        pub beats: Vec<BeatRow>,
        /// The key scoreboard, empty when no key analyzer is built in.
        pub keys: Vec<KeyRow>,
        /// The anchor scoreboard, empty when the directory holds no anchor
        /// annotations.
        pub anchors: Vec<AnchorRow>,
        /// The grid scoreboard, empty for the same reason.
        pub grids: Vec<GridRow>,
        /// The phrase scoreboard, empty for the same reason.
        pub phrases: Vec<PhraseRow>,
    }

    /// One phrase analyzer's line on the phrase scoreboard, over one label
    /// source or over all of them.
    #[derive(Debug, Serialize)]
    pub struct PhraseRow {
        /// The analyzer's name.
        pub analyzer: String,
        /// The label source the row is restricted to, or null for all sources.
        pub source: Option<String>,
        /// How many tracks it was run on.
        pub tracks: usize,
        /// How many tracks it failed on.
        pub failures: usize,
        /// The share of tracks whose bar lines are the labeled ones; only
        /// the row over all sources has it.
        pub downbeat: Option<f64>,
        /// How many phrase labels were scored.
        pub phrase_labels: usize,
        /// The share of phrase labels on one of the analyzer's bar lines.
        pub on_bar: Option<f64>,
        /// The share of phrase labels on a phrase start of at least eight bars.
        pub on_8_bars: Option<f64>,
        /// The share on a phrase start of at least sixteen bars.
        pub on_16_bars: Option<f64>,
        /// The share on a thirty-two-bar phrase start.
        pub on_32_bars: Option<f64>,
        /// The median distance in bars from a phrase label to the nearest
        /// sixteen-bar phrase start.
        pub median_bars: Option<f64>,
        /// How many section labels were scored.
        pub section_labels: usize,
        /// The share of section labels within one bar of a section change.
        pub sections_within_bar: Option<f64>,
        /// How its sure and unsure tracks divide into hits and misses, where
        /// a hit is a track whose downbeat is right and whose phrase labels
        /// all lie on sixteen-bar phrase starts.
        pub confidence_split: ConfidenceSplit,
        /// The mean seconds per track.
        pub seconds_per_track: f64,
        /// The score on every track, in the order the ground truth was read.
        pub per_track: Vec<PhraseTrack>,
    }

    /// How one phrase analyzer did on one track.
    #[derive(Debug, Serialize)]
    pub struct PhraseTrack {
        /// The track's name.
        pub name: String,
        /// Whether the analyzer's bar lines are the labeled ones, or null
        /// when the analyzer failed on the track.
        pub downbeat: Option<bool>,
        /// How sure the analyzer was, from zero to one.
        pub confidence: Option<f64>,
        /// How long the analyzer took on this track.
        pub seconds: f64,
    }

    /// One analyzer's line on the beat scoreboard.
    #[derive(Debug, Serialize)]
    pub struct BeatRow {
        /// The analyzer's name.
        pub analyzer: String,
        /// How many tracks it was run on.
        pub tracks: usize,
        /// How many tracks it failed on.
        pub failures: usize,
        /// The share of tempo-annotated tracks it got within four percent.
        pub tempo_accuracy1: Option<f64>,
        /// The same share allowing octave errors.
        pub tempo_accuracy2: Option<f64>,
        /// The median tempo error, as a fraction of the annotated tempo.
        pub tempo_error: Option<f64>,
        /// The mean beat F-measure over beat-annotated tracks.
        pub beat_f_measure: Option<f64>,
        /// The mean seconds per track.
        pub seconds_per_track: f64,
        /// The score on every track, in the order the ground truth was read.
        pub per_track: Vec<BeatTrack>,
    }

    /// How one beat analyzer did on one track.
    #[derive(Debug, Serialize)]
    pub struct BeatTrack {
        /// The track's name.
        pub name: String,
        /// The tempo the analyzer reported.
        pub bpm: Option<f64>,
        /// Whether the tempo was within four percent.
        pub accuracy1: Option<bool>,
        /// Whether it was within four percent allowing octave errors.
        pub accuracy2: Option<bool>,
        /// How far the tempo was from the annotation, as a fraction of it.
        pub tempo_error: Option<f64>,
        /// The beat F-measure on this track.
        pub f_measure: Option<f64>,
        /// How long the analyzer took on this track.
        pub seconds: f64,
        /// How sure the analyzer was of its answer, from zero to one.
        pub confidence: Option<f64>,
    }

    /// One analyzer's line on the key scoreboard.
    #[derive(Debug, Serialize)]
    pub struct KeyRow {
        /// The analyzer's name.
        pub analyzer: String,
        /// How many tracks it was run on.
        pub tracks: usize,
        /// How many tracks it failed on.
        pub failures: usize,
        /// The share of the key-annotated tracks it answered on that were exactly right.
        pub exact: Option<f64>,
        /// The mean credit over the key-annotated tracks it answered on.
        pub weighted: Option<f64>,
        /// The mean seconds per track.
        pub seconds_per_track: f64,
        /// The score on every track, in the order the ground truth was read.
        pub per_track: Vec<KeyTrack>,
    }

    /// How one key analyzer did on one track.
    #[derive(Debug, Serialize)]
    pub struct KeyTrack {
        /// The track's name.
        pub name: String,
        /// The key the analyzer reported.
        pub key: Option<String>,
        /// The credit the answer earned against the annotation.
        pub score: Option<f64>,
        /// How long the analyzer took on this track.
        pub seconds: f64,
    }

    /// How well one analyzer placed one kind of position over every track it
    /// was scored on.
    #[derive(Debug, Serialize)]
    pub struct PositionSummary {
        /// How many tracks held a label for this position.
        pub scored: usize,
        /// The share placed within one bar of the label.
        pub within_bar: Option<f64>,
        /// The share placed within four bars of the label.
        pub within_four_bars: Option<f64>,
        /// The median distance from the label, in beats.
        pub median_beats: Option<f64>,
    }

    /// How one analyzer did on one labeled position of one track.
    #[derive(Debug, Serialize)]
    pub struct PositionScore {
        /// Where the label came from.
        pub source: String,
        /// How far the analyzer's position sits from the label, in beats,
        /// negative when early.
        pub error: f64,
    }

    /// One anchor analyzer's line, over one label source or over all of them.
    #[derive(Debug, Serialize)]
    pub struct AnchorRow {
        /// The analyzer's name.
        pub analyzer: String,
        /// The label source the scoreboard measured the analyzer against for this row, or null for all sources together.
        pub source: Option<String>,
        /// How many tracks it was run on.
        pub tracks: usize,
        /// How many tracks it failed on.
        pub failures: usize,
        /// How it placed the intro anchor.
        pub intro: PositionSummary,
        /// How it placed the outro anchor.
        pub outro: PositionSummary,
        /// How it placed the effective beginning.
        pub begins: PositionSummary,
        /// How it placed the effective ending.
        pub ends: PositionSummary,
        /// The mean seconds per track.
        pub seconds_per_track: f64,
        /// Every track the analyzer was run on.
        pub per_track: Vec<AnchorTrack>,
    }

    /// How one anchor analyzer did on one track.
    #[derive(Debug, Serialize)]
    pub struct AnchorTrack {
        /// The track's name.
        pub name: String,
        /// The intro anchor, or null when the track holds no intro label.
        pub intro: Option<PositionScore>,
        /// The outro anchor, or null when the track holds no outro label.
        pub outro: Option<PositionScore>,
        /// The effective beginning, or null when the track holds no label for it.
        pub begins: Option<PositionScore>,
        /// The effective ending, or null when the track holds no label for it.
        pub ends: Option<PositionScore>,
        /// How sure the analyzer was, from zero to one.
        pub confidence: Option<f64>,
        /// How long the analyzer took on this track.
        pub seconds: f64,
    }

    /// How the tracks an analyzer was sure about and the tracks it was not
    /// each divide into hits and misses. On a grid row a hit is a tempo
    /// within four percent of the labeled one; on a phrase row it is a
    /// track whose downbeat is right and whose phrase labels all lie on
    /// sixteen-bar phrase starts.
    #[derive(Debug, Serialize)]
    pub struct ConfidenceSplit {
        /// Tracks it was sure about and got right.
        pub sure_hits: usize,
        /// Tracks it was sure about and got wrong.
        pub sure_misses: usize,
        /// Tracks it was unsure about and got right.
        pub unsure_hits: usize,
        /// Tracks it was unsure about and got wrong.
        pub unsure_misses: usize,
    }

    /// One beat analyzer's line on the grid scoreboard.
    #[derive(Debug, Serialize)]
    pub struct GridRow {
        /// The analyzer's name.
        pub analyzer: String,
        /// How many tracks it was run on.
        pub tracks: usize,
        /// How many tracks it failed on.
        pub failures: usize,
        /// The share within four percent of the labeled tempo.
        pub tempo_accuracy1: Option<f64>,
        /// The same share allowing octave errors.
        pub tempo_accuracy2: Option<f64>,
        /// The median tempo error, as a fraction of the labeled tempo.
        pub tempo_error: Option<f64>,
        /// The share whose beat zero lands on a beat of the labeled grid.
        pub on_beat: Option<f64>,
        /// The share whose beat zero lands on a downbeat of the labeled grid.
        pub on_downbeat: Option<f64>,
        /// How its sure and unsure tracks divide into tempo hits and misses.
        pub confidence_split: ConfidenceSplit,
        /// The mean seconds per track.
        pub seconds_per_track: f64,
        /// The score on every track, in the order the ground truth was read.
        pub per_track: Vec<GridTrack>,
    }

    /// How one beat analyzer's grid compares with one track's labeled grid.
    #[derive(Debug, Serialize)]
    pub struct GridTrack {
        /// The track's name.
        pub name: String,
        /// The tempo the analyzer reported.
        pub bpm: Option<f64>,
        /// Whether the tempo was within four percent of the labeled tempo.
        pub accuracy1: Option<bool>,
        /// Whether it was within four percent allowing octave errors.
        pub accuracy2: Option<bool>,
        /// How far the tempo was from the labeled tempo, as a fraction of it.
        pub tempo_error: Option<f64>,
        /// Whether beat zero lands on a beat of the labeled grid.
        pub on_beat: Option<bool>,
        /// Whether beat zero lands on a downbeat of the labeled grid.
        pub on_downbeat: Option<bool>,
        /// The beat of the labeled grid at which beat zero fell.
        pub first_beat: Option<f64>,
        /// How sure the analyzer was, from zero to one.
        pub confidence: Option<f64>,
        /// How long the analyzer took on this track.
        pub seconds: f64,
    }
}

/// The beat scoreboard as the JSON document holds it.
fn beat_rows(report: &Report) -> Vec<document::BeatRow> {
    report
        .rows
        .iter()
        .map(|row| document::BeatRow {
            analyzer: row.analyzer.clone(),
            tracks: row.tracks,
            failures: row.failures,
            tempo_accuracy1: row.tempo_accuracy1,
            tempo_accuracy2: row.tempo_accuracy2,
            tempo_error: row.tempo_error,
            beat_f_measure: row.beat_f_measure,
            seconds_per_track: row.seconds_per_track,
            per_track: row
                .per_track
                .iter()
                .map(|track| document::BeatTrack {
                    name: track.name.clone(),
                    bpm: track.bpm.map(|bpm| bpm.0),
                    accuracy1: track.accuracy1,
                    accuracy2: track.accuracy2,
                    tempo_error: track.tempo_error,
                    f_measure: track.f_measure,
                    seconds: track.seconds,
                    confidence: track.confidence,
                })
                .collect(),
        })
        .collect()
}

/// The key scoreboard as the JSON document holds it.
fn key_rows(report: &KeyReport) -> Vec<document::KeyRow> {
    report
        .rows
        .iter()
        .map(|row| document::KeyRow {
            analyzer: row.analyzer.clone(),
            tracks: row.tracks,
            failures: row.failures,
            exact: row.exact,
            weighted: row.weighted,
            seconds_per_track: row.seconds_per_track,
            per_track: row
                .per_track
                .iter()
                .map(|track| document::KeyTrack {
                    name: track.name.clone(),
                    key: track.key.map(|key| key.to_string()),
                    score: track.score,
                    seconds: track.seconds,
                })
                .collect(),
        })
        .collect()
}

/// One summary of how an analyzer placed one kind of position.
fn position_summary(summary: &PositionSummary) -> document::PositionSummary {
    document::PositionSummary {
        scored: summary.scored,
        within_bar: summary.within_bar,
        within_four_bars: summary.within_four_bars,
        median_beats: summary.median_beats,
    }
}

/// One labeled position an analyzer was scored on, or nothing when the track
/// holds no label for it.
fn position_score(score: Option<PositionScore>) -> Option<document::PositionScore> {
    score.map(|score| document::PositionScore {
        source: score.source.name().to_owned(),
        error: score.error,
    })
}

/// One track's line under an anchor analyzer.
fn anchor_track(score: &AnchorTrackScore) -> document::AnchorTrack {
    document::AnchorTrack {
        name: score.name.clone(),
        intro: position_score(score.intro),
        outro: position_score(score.outro),
        begins: position_score(score.begins),
        ends: position_score(score.ends),
        confidence: score.confidence,
        seconds: score.seconds,
    }
}

/// The anchor scoreboard as the JSON document holds it.
///
/// An analyzer has one row over all label sources and one row for each
/// source with at least one label, and every one of those rows lists the
/// same tracks, because the analyzer ran once over each track and the rows
/// differ only in which labels the scoreboard measured that run against.
fn anchor_rows(report: &AnchorReport) -> Vec<document::AnchorRow> {
    report
        .rows
        .iter()
        .map(|row| document::AnchorRow {
            analyzer: row.analyzer.clone(),
            source: row.source.map(|source| source.name().to_owned()),
            tracks: row.tracks,
            failures: row.failures,
            intro: position_summary(&row.intro),
            outro: position_summary(&row.outro),
            begins: position_summary(&row.begins),
            ends: position_summary(&row.ends),
            seconds_per_track: row.seconds_per_track,
            per_track: report
                .per_track
                .get(&row.analyzer)
                .map(|scores| scores.iter().map(anchor_track).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// The phrase scoreboard as the JSON document holds it.
///
/// As on the anchor scoreboard, an analyzer has one row over all label
/// sources and one row for each source with at least one label, and every
/// one of those rows lists the same tracks, because the analyzer ran once
/// over each track and the rows differ only in which labels the scoreboard
/// measured that run against.
fn phrase_rows(report: &PhraseReport) -> Vec<document::PhraseRow> {
    report
        .rows
        .iter()
        .map(|row| document::PhraseRow {
            analyzer: row.analyzer.clone(),
            source: row.source.map(|source| source.name().to_owned()),
            tracks: row.tracks,
            failures: row.failures,
            downbeat: row.downbeat,
            phrase_labels: row.phrase_labels,
            on_bar: row.on_bar,
            on_8_bars: row.on_phrase[0],
            on_16_bars: row.on_phrase[1],
            on_32_bars: row.on_phrase[2],
            median_bars: row.median_bars,
            section_labels: row.section_labels,
            sections_within_bar: row.sections_within_bar,
            confidence_split: document::ConfidenceSplit {
                sure_hits: row.confidence_split.sure_hits,
                sure_misses: row.confidence_split.sure_misses,
                unsure_hits: row.confidence_split.unsure_hits,
                unsure_misses: row.confidence_split.unsure_misses,
            },
            seconds_per_track: row.seconds_per_track,
            per_track: report
                .per_track
                .get(&row.analyzer)
                .map(|scores| {
                    scores
                        .iter()
                        .map(|score| document::PhraseTrack {
                            name: score.name.clone(),
                            downbeat: score.downbeat,
                            confidence: score.confidence,
                            seconds: score.seconds,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

/// The grid scoreboard as the JSON document holds it.
fn grid_rows(report: &GridReport) -> Vec<document::GridRow> {
    report
        .rows
        .iter()
        .map(|row| document::GridRow {
            analyzer: row.analyzer.clone(),
            tracks: row.tracks,
            failures: row.failures,
            tempo_accuracy1: row.tempo_accuracy1,
            tempo_accuracy2: row.tempo_accuracy2,
            tempo_error: row.tempo_error,
            on_beat: row.on_beat,
            on_downbeat: row.on_downbeat,
            confidence_split: document::ConfidenceSplit {
                sure_hits: row.confidence_split.sure_hits,
                sure_misses: row.confidence_split.sure_misses,
                unsure_hits: row.confidence_split.unsure_hits,
                unsure_misses: row.confidence_split.unsure_misses,
            },
            seconds_per_track: row.seconds_per_track,
            per_track: row
                .per_track
                .iter()
                .map(|track| document::GridTrack {
                    name: track.name.clone(),
                    bpm: track.bpm.map(|bpm| bpm.0),
                    accuracy1: track.accuracy1,
                    accuracy2: track.accuracy2,
                    tempo_error: track.tempo_error,
                    on_beat: track.on_beat,
                    on_downbeat: track.on_downbeat,
                    first_beat: track.first_beat.map(|beat| beat.0),
                    confidence: track.confidence,
                    seconds: track.seconds,
                })
                .collect(),
        })
        .collect()
}

/// Runs every key analyzer that is built in over the ground truth.
///
/// The electronic-music analyzer is always built in and always runs. The
/// libkeyfinder baseline joins it when the `keyfinder` feature is on, which
/// it is by default, and that feature is what builds the vendored C++
/// library the baseline needs.
fn key_scoreboard(truth: &[GroundTruth]) -> Result<Option<KeyReport>, String> {
    let edm = dermixen_analysis::EdmKey;
    #[cfg(feature = "keyfinder")]
    let keyfinder = dermixen_analysis::KeyfinderKey;
    #[cfg(feature = "keyfinder")]
    let analyzers: Vec<&dyn dermixen_analysis::KeyAnalyzer> = vec![&keyfinder, &edm];
    #[cfg(not(feature = "keyfinder"))]
    let analyzers: Vec<&dyn dermixen_analysis::KeyAnalyzer> = vec![&edm];
    dermixen_analysis::run_keys(truth, &analyzers)
        .map(Some)
        .map_err(|problem| problem.to_string())
}

/// A directory holding no annotation of one kind is not a failure, because
/// the other kind may still be there. Only a directory that cannot be read
/// at all, or an annotation file that cannot be understood, stops the
/// command.
fn no_annotations_is_not_a_failure<T>(
    found: Result<T, ScoreboardError>,
    none: T,
) -> Result<T, String> {
    match found {
        Ok(found) => Ok(found),
        Err(ScoreboardError::Empty(_)) => Ok(none),
        Err(problem) => Err(problem.to_string()),
    }
}

/// The tempo, beat, and key annotations in the directory, which come back
/// empty when it holds none.
fn beat_truth(dir: &Path, giantsteps: bool) -> Result<Vec<GroundTruth>, String> {
    let found = if giantsteps {
        load_giantsteps(dir)
    } else {
        load_directory(dir)
    };
    no_annotations_is_not_a_failure(found, Vec::new())
}

/// An anchor ground truth holding nothing at all.
fn no_anchor_truth() -> AnchorTruthDirectory {
    AnchorTruthDirectory {
        tracks: Vec::new(),
        without_audio: Vec::new(),
        warnings: Vec::new(),
    }
}

/// The anchor annotations in the directory, which come back empty when it
/// holds none. A GiantSteps checkout never holds any, so it is not searched.
fn anchor_truth(dir: &Path, giantsteps: bool) -> Result<AnchorTruthDirectory, String> {
    if giantsteps {
        return Ok(no_anchor_truth());
    }
    no_annotations_is_not_a_failure(load_anchor_truth(dir), no_anchor_truth())
}

/// Carries out the `scoreboard` command.
pub fn run(dir: &Path, giantsteps: bool, json: bool) -> Result<(), String> {
    let truth = beat_truth(dir, giantsteps)?;
    let anchors = anchor_truth(dir, giantsteps)?;
    for path in &anchors.without_audio {
        eprintln!("warning: no audio beside {}", path.display());
    }
    for warning in &anchors.warnings {
        eprintln!("warning: {warning}");
    }
    if truth.is_empty() && anchors.tracks.is_empty() {
        return Err(format!("{} holds no annotated audio", dir.display()));
    }

    // The beat analyzers run over both kinds of annotation: against the
    // tempo and beat annotations on the beat scoreboard, and against the
    // labeled grids on the grid scoreboard.
    let pulse = PulseGrid;
    #[cfg(feature = "aubio")]
    let aubio = dermixen_analysis::AubioBeats;
    let fixed = FixedTempo(FIXED_TEMPO);
    let beat_analyzers: Vec<&dyn BeatAnalyzer> = vec![
        &pulse,
        #[cfg(feature = "aubio")]
        &aubio,
        &fixed,
    ];

    let mut beats = None;
    let mut keys = None;
    if !truth.is_empty() {
        eprintln!(
            "running {} beat analyzers over {} annotated tracks",
            beat_analyzers.len(),
            truth.len()
        );
        beats = Some(
            dermixen_analysis::run(&truth, &beat_analyzers)
                .map_err(|problem| problem.to_string())?,
        );
        keys = key_scoreboard(&truth)?;
    }

    let edges = EdgeAnchors;
    let kick = KickAnchors;
    let anchor_analyzers: Vec<&dyn AnchorAnalyzer> = vec![&edges, &kick];
    // Every phrase analyzer is handed each track's labeled grid as it is,
    // because that grid is the one the labels were written against.
    let counted = CountedPhrases;
    let shifts = ShiftPhrases;
    let phrase_analyzers: Vec<&dyn PhraseAnalyzer> = vec![&counted, &shifts];

    let mut placed = None;
    let mut grids = None;
    let mut phrases = None;
    if !anchors.tracks.is_empty() {
        eprintln!(
            "running {} anchor analyzers over {} labeled tracks",
            anchor_analyzers.len(),
            anchors.tracks.len()
        );
        placed = Some(
            run_anchors(&anchors.tracks, &anchor_analyzers)
                .map_err(|problem| problem.to_string())?,
        );
        eprintln!(
            "running {} beat analyzers over the {} labeled grids",
            beat_analyzers.len(),
            anchors.tracks.len()
        );
        grids = Some(
            run_grids(&anchors.tracks, &beat_analyzers).map_err(|problem| problem.to_string())?,
        );
        eprintln!(
            "running {} phrase analyzers over the {} labeled tracks",
            phrase_analyzers.len(),
            anchors.tracks.len()
        );
        phrases = Some(
            run_phrases(&anchors.tracks, &phrase_analyzers, GridHandling::AsLabeled)
                .map_err(|problem| problem.to_string())?,
        );
    }

    if json {
        print_json(&document::Scoreboard {
            beats: beats.as_ref().map(beat_rows).unwrap_or_default(),
            keys: keys.as_ref().map(key_rows).unwrap_or_default(),
            anchors: placed.as_ref().map(anchor_rows).unwrap_or_default(),
            grids: grids.as_ref().map(grid_rows).unwrap_or_default(),
            phrases: phrases.as_ref().map(phrase_rows).unwrap_or_default(),
        });
        return Ok(());
    }

    // Each table already ends with a newline, so joining them with one more
    // leaves a blank line between one table and the next.
    let mut tables: Vec<String> = Vec::new();
    if let Some(beats) = &beats {
        tables.push(beats.table());
    }
    if let Some(keys) = &keys {
        tables.push(keys.table());
    }
    if let Some(placed) = &placed {
        tables.push(placed.table());
    }
    if let Some(grids) = &grids {
        tables.push(grids.table());
    }
    if let Some(phrases) = &phrases {
        tables.push(phrases.table());
    }
    print!("{}", tables.join("\n"));
    Ok(())
}
