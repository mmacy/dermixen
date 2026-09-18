//! Acceptance tests for the phrase analysis in the index: the record holds
//! the analysis, and the library file stores it. A coder agent makes these pass
//! without editing them.

use std::path::{Path, PathBuf};

use dermixen_analysis::{
    AnalysisError, BeatAnalysis, BeatAnalyzer, CountedPhrases, EdgeAnchors, PhraseAnalysis,
    PhraseAnalyzer, PhraseStart,
};
use dermixen_core::{BeatGrid, Beats, Bpm, Samples, Seconds};
use dermixen_library::{
    Analyzers, Index, PhraseRecord, PhraseStartRecord, SCHEMA_VERSION, TrackRecord, analyze_file,
};
use dermixen_media::{Audio, WavDepth, write_wav};
use dermixen_testkit::synth;

/// The version SQLite's own user version field holds for the file.
fn version_of(path: &Path) -> u32 {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

/// A beat analyzer that answers 130 beats per minute from half a second in.
struct BeatsAt130;

impl BeatAnalyzer for BeatsAt130 {
    fn name(&self) -> &str {
        "fixed"
    }

    fn analyze(&self, _audio: &Audio) -> Result<BeatAnalysis, AnalysisError> {
        Ok(BeatAnalysis {
            bpm: Bpm(130.0),
            beats: vec![Seconds(0.5).to_samples()],
            confidence: 1.0,
        })
    }
}

/// A phrase analyzer that fails on every track.
struct NoPhrases;

impl PhraseAnalyzer for NoPhrases {
    fn name(&self) -> &str {
        "none"
    }

    fn analyze(&self, _audio: &Audio, _grid: &BeatGrid) -> Result<PhraseAnalysis, AnalysisError> {
        Err(AnalysisError::Failed("no phrases here".to_owned()))
    }
}

/// Twenty seconds of kicks at 130 beats per minute from half a second in.
fn kicks_file(dir: &Path) -> PathBuf {
    let path = dir.join("01 Etnica - Alpha.wav");
    let audio = synth::kicks(Bpm(130.0), Seconds(0.5), Seconds(20.0));
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    path
}

/// A record with phrases, built by analyzing the kicks file with the
/// counting phrase analyzer, whose answer follows from the grid alone.
fn analyzed(dir: &Path, phrases: &dyn PhraseAnalyzer) -> TrackRecord {
    let path = kicks_file(dir);
    analyze_file(
        &path,
        &Analyzers {
            beats: &BeatsAt130,
            key: None,
            anchors: &EdgeAnchors,
            phrases,
        },
    )
    .unwrap()
}

#[test]
fn analyze_file_stores_what_the_phrase_analyzer_finds() {
    let dir = tempfile::tempdir().unwrap();
    let record = analyzed(dir.path(), &CountedPhrases);
    // Twenty seconds at 130 beats per minute from half a second in is 42.25
    // beats of grid, so counting thirty-two-bar phrases from beat zero gives
    // a start at beat 0 and an eight-bar start at beat 32, and no more.
    assert_eq!(
        record.phrases,
        Some(PhraseRecord {
            analyzer: "counted".to_owned(),
            confidence: 0.0,
            downbeat: 0,
            starts: vec![
                PhraseStartRecord {
                    beat: Beats(0.0),
                    bars: 32
                },
                PhraseStartRecord {
                    beat: Beats(32.0),
                    bars: 8
                },
            ],
            sections: Vec::new(),
        })
    );
}

#[test]
fn a_phrase_analyzer_that_fails_leaves_the_record_without_phrases() {
    let dir = tempfile::tempdir().unwrap();
    let record = analyzed(dir.path(), &NoPhrases);
    assert_eq!(record.phrases, None);
    assert_eq!(
        record.grid.bpm,
        Bpm(130.0),
        "the rest of the record is intact"
    );
}

#[test]
fn a_fresh_index_is_the_current_version_and_a_record_with_phrases_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    let mut index = Index::open(&path).unwrap();
    assert_eq!(version_of(&path), SCHEMA_VERSION);

    let mut record = analyzed(dir.path(), &CountedPhrases);
    record.phrases.as_mut().unwrap().sections = vec![Beats(16.0), Beats(32.0)];
    record.phrases.as_mut().unwrap().confidence = 0.4;
    index.upsert(&record).unwrap();
    assert_eq!(index.get(record.hash).unwrap(), Some(record.clone()));

    let mut without = record.clone();
    without.phrases = None;
    index.upsert(&without).unwrap();
    assert_eq!(
        index.get(record.hash).unwrap(),
        Some(without),
        "a record written without phrases reads back without them"
    );
}

/// A phrase analyzer that reports bars starting two beats past beat zero.
struct DownbeatTwo;

impl PhraseAnalyzer for DownbeatTwo {
    fn name(&self) -> &str {
        "two"
    }

    fn analyze(&self, _audio: &Audio, _grid: &BeatGrid) -> Result<PhraseAnalysis, AnalysisError> {
        Ok(PhraseAnalysis {
            downbeat: 2,
            phrases: vec![PhraseStart {
                at: Beats(2.0),
                bars: 32,
            }],
            sections: vec![Beats(34.0)],
            confidence: 0.5,
        })
    }
}

#[test]
fn a_downbeat_off_beat_zero_is_recorded_and_the_grid_is_left_where_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let record = analyzed(dir.path(), &DownbeatTwo);
    assert_eq!(
        record.grid,
        BeatGrid {
            first_beat: Samples(22_050),
            bpm: Bpm(130.0)
        },
        "beat zero stays where the beat analyzer put it"
    );
    assert_eq!(
        record.phrases,
        Some(PhraseRecord {
            analyzer: "two".to_owned(),
            confidence: 0.5,
            downbeat: 2,
            starts: vec![PhraseStartRecord {
                beat: Beats(2.0),
                bars: 32
            }],
            sections: vec![Beats(34.0)],
        })
    );
}
