//! Acceptance tests for the key scoreboard. A coder agent makes these pass without editing them.

use std::path::Path;

use dermixen_analysis::{
    AnalysisError, Key, KeyAnalysis, KeyAnalyzer, Mode, PitchClass, UnknownKey, key_score,
    load_directory, run_keys,
};
use dermixen_media::{Audio, WavDepth, write_wav};

fn key(tonic: PitchClass, mode: Mode) -> Key {
    Key { tonic, mode }
}

#[test]
fn a_key_estimate_earns_credit_by_its_relation_to_the_annotation() {
    let c_major = key(PitchClass::C, Mode::Major);
    for (estimate, expected, relation) in [
        (key(PitchClass::C, Mode::Major), 1.0, "the same key"),
        (key(PitchClass::G, Mode::Major), 0.5, "a fifth above"),
        (key(PitchClass::F, Mode::Major), 0.5, "a fifth below"),
        (key(PitchClass::A, Mode::Minor), 0.3, "the relative minor"),
        (key(PitchClass::C, Mode::Minor), 0.2, "the parallel minor"),
        (key(PitchClass::D, Mode::Major), 0.0, "a whole tone away"),
        (
            key(PitchClass::G, Mode::Minor),
            0.0,
            "a fifth above in the other mode",
        ),
        (
            key(PitchClass::E, Mode::Minor),
            0.0,
            "the relative minor of the fifth",
        ),
    ] {
        let score = key_score(estimate, c_major);
        assert!(
            (score - expected).abs() < 1e-12,
            "{relation}: expected {expected}, got {score}"
        );
    }
    // The relations hold from a minor annotation too.
    let a_minor = key(PitchClass::A, Mode::Minor);
    assert_eq!(key_score(key(PitchClass::E, Mode::Minor), a_minor), 0.5);
    assert_eq!(key_score(key(PitchClass::D, Mode::Minor), a_minor), 0.5);
    assert_eq!(key_score(key(PitchClass::C, Mode::Major), a_minor), 0.3);
    assert_eq!(key_score(key(PitchClass::A, Mode::Major), a_minor), 0.2);
    // The wheel wraps around: B and F sharp are a fifth apart.
    assert_eq!(
        key_score(
            key(PitchClass::Fs, Mode::Major),
            key(PitchClass::B, Mode::Major)
        ),
        0.5
    );
}

/// A key analyzer that fails on every track.
struct Broken;

impl KeyAnalyzer for Broken {
    fn name(&self) -> &str {
        "broken"
    }

    fn analyze(&self, _audio: &Audio) -> Result<KeyAnalysis, AnalysisError> {
        Err(AnalysisError::Failed("broken on purpose".to_owned()))
    }
}

/// Writes three annotated tracks: one in C major, one in A minor, and one
/// with a tempo annotation but no key.
fn write_truth(dir: &Path) {
    let audio = Audio {
        frames: vec![[0.1, 0.1]; 44_100],
    };
    for name in ["a", "b", "c"] {
        write_wav(&dir.join(format!("{name}.wav")), &audio, WavDepth::Int16).unwrap();
    }
    std::fs::write(dir.join("a.key"), "C major\n").unwrap();
    std::fs::write(dir.join("b.key"), "A minor\n").unwrap();
    std::fs::write(dir.join("c.bpm"), "128\n").unwrap();
}

#[test]
fn the_key_report_scores_every_analyzer_on_every_annotated_track() {
    let dir = tempfile::tempdir().unwrap();
    write_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let report = run_keys(&truth, &[&UnknownKey, &Broken]).unwrap();
    assert_eq!(report.rows.len(), 2);

    // The unknown-key analyzer always answers C major, which is right for
    // the first track and the relative major of the second.
    let row = &report.rows[0];
    assert_eq!(row.analyzer, "unknown key");
    assert_eq!(row.tracks, 3);
    assert_eq!(row.failures, 0);
    assert!((row.exact.unwrap() - 0.5).abs() < 1e-12, "{:?}", row.exact);
    assert!(
        (row.weighted.unwrap() - 0.65).abs() < 1e-12,
        "{:?}",
        row.weighted
    );
    assert_eq!(row.per_track.len(), 3);
    assert_eq!(row.per_track[0].name, "a");
    assert_eq!(row.per_track[0].score, Some(1.0));
    assert_eq!(row.per_track[1].score, Some(0.3));
    assert_eq!(row.per_track[2].score, None);
    assert!(row.per_track[2].key.is_some());
    assert!(row.seconds_per_track >= 0.0);

    let row = &report.rows[1];
    assert_eq!(row.analyzer, "broken");
    assert_eq!(row.failures, 3);
    assert_eq!(row.exact, None);
    assert_eq!(row.weighted, None);
    assert!(
        row.per_track
            .iter()
            .all(|t| t.key.is_none() && t.score.is_none())
    );
}

#[test]
fn the_key_table_has_one_line_per_analyzer_after_a_header() {
    let dir = tempfile::tempdir().unwrap();
    write_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let report = run_keys(&truth, &[&UnknownKey, &Broken]).unwrap();
    let table = report.table();
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 3, "table:\n{table}");
    assert!(
        lines[0].to_lowercase().contains("analyzer"),
        "table:\n{table}"
    );
    assert!(lines[1].starts_with("unknown key"), "table:\n{table}");
    assert!(lines[1].contains("50.0"), "table:\n{table}");
    assert!(lines[1].contains("65.0"), "table:\n{table}");
    assert!(lines[2].starts_with("broken"), "table:\n{table}");
    assert!(lines[2].contains('-'), "table:\n{table}");
}

#[test]
fn a_key_run_over_no_tracks_gives_empty_rows() {
    let report = run_keys(&[], &[&UnknownKey]).unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].tracks, 0);
    assert_eq!(report.rows[0].exact, None);
    assert_eq!(report.rows[0].weighted, None);
}
