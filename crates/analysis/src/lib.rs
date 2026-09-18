#![forbid(unsafe_code)]

//! Beat grid, key, and anchor analysis with confidence reporting, and the
//! scoreboard that measures each analyzer against ground truth.

pub mod analyzer;
pub mod camelot;
pub mod dsp;
pub mod edm_key;
#[cfg(feature = "keyfinder")]
pub mod keyfinder;
pub mod keys;
pub mod kick_anchors;
pub mod loudness;
pub mod phrase_scoreboard;
pub mod phrased_anchors;
pub mod phrases;
pub mod pulse;
pub mod scoreboard;
pub mod shift_phrases;
pub mod structure;
pub mod structure_scoreboard;

#[cfg(feature = "aubio")]
pub use analyzer::AubioBeats;
pub use analyzer::{
    AnalysisError, BeatAnalysis, BeatAnalyzer, FixedTempo, Key, KeyAnalysis, KeyAnalyzer,
    KeyParseError, Mode, PitchClass, UnknownKey,
};
pub use camelot::{Camelot, CamelotParseError, Letter};
pub use edm_key::EdmKey;
#[cfg(feature = "keyfinder")]
pub use keyfinder::KeyfinderKey;
pub use keys::{KeyReport, KeyRow, KeyTrackScore, key_score, run_keys};
pub use kick_anchors::KickAnchors;
pub use loudness::{Loudness, measure_loudness};
pub use phrase_scoreboard::{
    GridHandling, PHRASE_LENGTHS, PhraseLabelScore, PhraseReport, PhraseRow, PhraseTrackScore,
    SectionLabelScore, handed_grid, run_phrases,
};
pub use phrased_anchors::PhrasedAnchors;
pub use phrases::{
    CountedPhrases, LONGEST_PHRASE_BARS, PhraseAnalysis, PhraseAnalyzer, PhraseStart,
    counted_phrases,
};
pub use pulse::PulseGrid;
pub use scoreboard::{
    GroundTruth, Report, Row, ScoreboardError, TrackScore, beat_f_measure, load_directory,
    load_giantsteps, parse_key, run, tempo_accuracy1, tempo_accuracy2,
};
pub use shift_phrases::ShiftPhrases;
pub use structure::{AnchorAnalysis, AnchorAnalyzer, EdgeAnchors, Extent};
pub use structure_scoreboard::{
    AnchorReport, AnchorRow, AnchorTrackScore, AnchorTruth, AnchorTruthDirectory, ConfidenceSplit,
    GridReport, GridRow, GridTrackScore, Label, PositionScore, PositionSummary, SURE_CONFIDENCE,
    Source, error_in_beats, load_anchor_truth, run_anchors, run_grids,
};
