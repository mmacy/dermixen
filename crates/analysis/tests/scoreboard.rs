//! Acceptance tests for the scoreboard. A coder agent makes these pass without editing them.
//!
//! Ground truth for these tests is written into a temporary directory from
//! synthetic kick tracks, so every expected number follows from the tempo the
//! kicks were generated at.

use std::fs;
use std::path::Path;

use dermixen_analysis::{
    AnalysisError, BeatAnalysis, BeatAnalyzer, FixedTempo, GroundTruth, Key, Mode, PitchClass,
    ScoreboardError, beat_f_measure, load_directory, load_giantsteps, parse_key, run,
    tempo_accuracy1, tempo_accuracy2,
};
use dermixen_core::{Bpm, Seconds};
use dermixen_media::{Audio, WavDepth, write_wav};
use dermixen_testkit::synth;

const TEMPOS: [f64; 3] = [120.0, 130.0, 140.0];

fn kick_track(bpm: f64) -> Audio {
    synth::kicks(Bpm(bpm), Seconds::ZERO, Seconds(10.0))
}

fn beat_times(bpm: f64) -> Vec<f64> {
    (0..)
        .map(|n| f64::from(n) * 60.0 / bpm)
        .take_while(|t| *t < 10.0)
        .collect()
}

/// Writes three annotated kick tracks and one unannotated tone in Dermixen's own layout.
fn write_truth(dir: &Path) {
    for bpm in TEMPOS {
        let name = format!("kicks-{bpm}");
        write_wav(
            &dir.join(format!("{name}.wav")),
            &kick_track(bpm),
            WavDepth::Int16,
        )
        .unwrap();
        fs::write(dir.join(format!("{name}.bpm")), format!("{bpm}\n")).unwrap();
        let beats: Vec<String> = beat_times(bpm)
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{t:.6} {}", i % 4 + 1))
            .collect();
        fs::write(dir.join(format!("{name}.beats")), beats.join("\n") + "\n").unwrap();
    }
    fs::write(dir.join("kicks-130.key"), "F minor\n").unwrap();
    write_wav(
        &dir.join("unannotated.wav"),
        &synth::sine(440.0, 0.5, Seconds(1.0)),
        WavDepth::Int16,
    )
    .unwrap();
}

#[test]
fn tempo_accuracy_one_allows_four_percent() {
    assert!(tempo_accuracy1(Bpm(130.0), Bpm(130.0)));
    assert!(tempo_accuracy1(Bpm(134.0), Bpm(130.0)));
    assert!(tempo_accuracy1(Bpm(125.0), Bpm(130.0)));
    assert!(!tempo_accuracy1(Bpm(136.0), Bpm(130.0)));
    assert!(!tempo_accuracy1(Bpm(65.0), Bpm(130.0)));
    assert!(!tempo_accuracy1(Bpm(260.0), Bpm(130.0)));
}

#[test]
fn tempo_accuracy_two_forgives_octave_errors() {
    assert!(tempo_accuracy2(Bpm(130.0), Bpm(130.0)));
    assert!(tempo_accuracy2(Bpm(65.0), Bpm(130.0)));
    assert!(tempo_accuracy2(Bpm(260.0), Bpm(130.0)));
    assert!(tempo_accuracy2(Bpm(390.0), Bpm(130.0)));
    assert!(tempo_accuracy2(Bpm(43.3), Bpm(130.0)));
    assert!(tempo_accuracy2(Bpm(67.0), Bpm(130.0)));
    assert!(!tempo_accuracy2(Bpm(100.0), Bpm(130.0)));
    assert!(!tempo_accuracy2(Bpm(70.0), Bpm(130.0)));
}

#[test]
fn beat_f_measure_counts_hits_within_the_tolerance_once() {
    let reference: Vec<Seconds> = (0..20).map(|n| Seconds(f64::from(n) * 0.5)).collect();
    let tolerance = Seconds(0.07);
    assert_eq!(beat_f_measure(&reference, &reference, tolerance), 1.0);
    let shifted: Vec<Seconds> = reference.iter().map(|s| Seconds(s.0 + 0.05)).collect();
    assert_eq!(beat_f_measure(&shifted, &reference, tolerance), 1.0);
    let far: Vec<Seconds> = reference.iter().map(|s| Seconds(s.0 + 0.1)).collect();
    assert_eq!(beat_f_measure(&far, &reference, tolerance), 0.0);
    // Half the beats found: precision one, recall a half, F two thirds.
    let half: Vec<Seconds> = reference.iter().step_by(2).copied().collect();
    let f = beat_f_measure(&half, &reference, tolerance);
    assert!((f - 2.0 / 3.0).abs() < 1e-9, "{f}");
    // Two estimates on one reference beat count as one hit and one miss.
    let doubled: Vec<Seconds> = reference
        .iter()
        .flat_map(|s| [Seconds(s.0 - 0.01), Seconds(s.0 + 0.01)])
        .collect();
    let f = beat_f_measure(&doubled, &reference, tolerance);
    assert!((f - 2.0 / 3.0).abs() < 1e-9, "{f}");
    assert_eq!(beat_f_measure(&[], &[], tolerance), 1.0);
    assert_eq!(beat_f_measure(&[], &reference, tolerance), 0.0);
    assert_eq!(beat_f_measure(&reference, &[], tolerance), 0.0);
}

#[test]
fn key_annotations_are_read_in_their_common_spellings() {
    let f_minor = Key {
        tonic: PitchClass::F,
        mode: Mode::Minor,
    };
    assert_eq!(parse_key("F minor"), Some(f_minor));
    assert_eq!(parse_key("f min"), Some(f_minor));
    assert_eq!(parse_key("Fm"), Some(f_minor));
    assert_eq!(parse_key("  F minor \n"), Some(f_minor));
    let cs_major = Key {
        tonic: PitchClass::Cs,
        mode: Mode::Major,
    };
    assert_eq!(parse_key("C# major"), Some(cs_major));
    assert_eq!(parse_key("Db maj"), Some(cs_major));
    assert_eq!(parse_key("C#"), Some(cs_major));
    assert_eq!(
        parse_key("Bb minor"),
        Some(Key {
            tonic: PitchClass::As,
            mode: Mode::Minor
        })
    );
    assert_eq!(parse_key(""), None);
    assert_eq!(parse_key("H major"), None);
    assert_eq!(parse_key("F dorian"), None);
}

#[test]
fn ground_truth_loads_from_a_directory_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    write_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let names: Vec<&str> = truth.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["kicks-120", "kicks-130", "kicks-140"]);
    for (entry, bpm) in truth.iter().zip(TEMPOS) {
        assert_eq!(entry.audio, dir.path().join(format!("kicks-{bpm}.wav")));
        assert_eq!(entry.bpm, Some(Bpm(bpm)));
        let beats = entry.beats.as_ref().unwrap();
        let expected = beat_times(bpm);
        assert_eq!(beats.len(), expected.len());
        for (got, want) in beats.iter().zip(&expected) {
            assert!((got.0 - want).abs() < 1e-5);
        }
    }
    assert_eq!(truth[0].key, None);
    assert_eq!(
        truth[1].key,
        Some(Key {
            tonic: PitchClass::F,
            mode: Mode::Minor
        })
    );
}

#[test]
fn ground_truth_loads_from_a_giantsteps_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("audio")).unwrap();
    fs::create_dir_all(root.join("annotations/tempo")).unwrap();
    fs::create_dir_all(root.join("annotations/key")).unwrap();
    for (id, bpm) in [("1001", 130.0), ("1002", 140.0)] {
        write_wav(
            &root.join(format!("audio/{id}.LOFI.wav")),
            &kick_track(bpm),
            WavDepth::Int16,
        )
        .unwrap();
        fs::write(
            root.join(format!("annotations/tempo/{id}.LOFI.bpm")),
            format!("{bpm}"),
        )
        .unwrap();
    }
    fs::write(root.join("annotations/key/1002.LOFI.key"), "A minor").unwrap();
    let truth = load_giantsteps(root).unwrap();
    assert_eq!(truth.len(), 2);
    assert_eq!(truth[0].name, "1001.LOFI");
    assert_eq!(truth[0].bpm, Some(Bpm(130.0)));
    assert_eq!(truth[0].beats, None);
    assert_eq!(truth[0].key, None);
    assert_eq!(truth[1].bpm, Some(Bpm(140.0)));
    assert_eq!(
        truth[1].key,
        Some(Key {
            tonic: PitchClass::A,
            mode: Mode::Minor
        })
    );
}

#[test]
fn bad_ground_truth_is_reported_by_path() {
    let dir = tempfile::tempdir().unwrap();
    write_truth(dir.path());
    let bad = dir.path().join("kicks-120.bpm");
    fs::write(&bad, "fast\n").unwrap();
    match load_directory(dir.path()) {
        Err(ScoreboardError::Annotation { path, message }) => {
            assert_eq!(path, bad);
            assert!(!message.is_empty());
        }
        other => panic!("expected an annotation error, got {other:?}"),
    }
    match load_directory(&dir.path().join("missing")) {
        Err(ScoreboardError::Read { .. }) => {}
        other => panic!("expected a read error, got {other:?}"),
    }
    let empty = tempfile::tempdir().unwrap();
    match load_directory(empty.path()) {
        Err(ScoreboardError::Empty(path)) => assert_eq!(path, empty.path()),
        other => panic!("expected an empty error, got {other:?}"),
    }
}

/// An analyzer that fails on every track.
struct Broken;

impl BeatAnalyzer for Broken {
    fn name(&self) -> &str {
        "broken"
    }

    fn analyze(&self, _: &Audio) -> Result<BeatAnalysis, AnalysisError> {
        Err(AnalysisError::Failed("always".to_owned()))
    }
}

#[test]
fn a_run_scores_every_analyzer_on_every_track() {
    let dir = tempfile::tempdir().unwrap();
    write_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let at_130 = FixedTempo(Bpm(130.0));
    let at_65 = FixedTempo(Bpm(65.0));
    let report = run(&truth, &[&at_130, &at_65, &Broken]).unwrap();
    assert_eq!(report.rows.len(), 3);

    let row = &report.rows[0];
    assert_eq!(row.analyzer, "fixed tempo");
    assert_eq!(row.tracks, 3);
    assert_eq!(row.failures, 0);
    // Only the 130 BPM track is within four percent of 130, with or without octave errors.
    assert!((row.tempo_accuracy1.unwrap() - 1.0 / 3.0).abs() < 1e-9);
    assert!((row.tempo_accuracy2.unwrap() - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(row.per_track.len(), 3);
    let track_130 = &row.per_track[1];
    assert_eq!(track_130.name, "kicks-130");
    assert_eq!(track_130.bpm, Some(Bpm(130.0)));
    assert_eq!(track_130.accuracy1, Some(true));
    // Beats every 60/130 seconds from time zero are exactly the annotated beats.
    assert!(
        track_130.f_measure.unwrap() > 0.99,
        "{:?}",
        track_130.f_measure
    );
    assert_eq!(row.per_track[0].accuracy1, Some(false));
    assert!(row.per_track[0].f_measure.unwrap() < 0.9);
    let mean = row
        .per_track
        .iter()
        .map(|t| t.f_measure.unwrap())
        .sum::<f64>()
        / 3.0;
    assert!((row.beat_f_measure.unwrap() - mean).abs() < 1e-9);
    assert!(row.seconds_per_track >= 0.0);

    // At 65 BPM the 130 track is an octave error: wrong for accuracy one, right for accuracy two.
    let row = &report.rows[1];
    assert!((row.tempo_accuracy1.unwrap() - 0.0).abs() < 1e-9);
    assert!((row.tempo_accuracy2.unwrap() - 1.0 / 3.0).abs() < 1e-9);
    assert_eq!(row.per_track[1].accuracy2, Some(true));

    // The broken analyzer fails everywhere and has no metrics.
    let row = &report.rows[2];
    assert_eq!(row.analyzer, "broken");
    assert_eq!(row.failures, 3);
    assert_eq!(row.tempo_accuracy1, None);
    assert_eq!(row.beat_f_measure, None);
    assert!(
        row.per_track
            .iter()
            .all(|t| t.bpm.is_none() && t.f_measure.is_none())
    );
}

#[test]
fn the_table_has_one_line_per_analyzer_after_a_header() {
    let dir = tempfile::tempdir().unwrap();
    write_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let at_130 = FixedTempo(Bpm(130.0));
    let report = run(&truth, &[&at_130, &Broken]).unwrap();
    let table = report.table();
    let lines: Vec<&str> = table.lines().collect();
    assert_eq!(lines.len(), 3, "table:\n{table}");
    assert!(lines[1].starts_with("fixed tempo"), "table:\n{table}");
    assert!(lines[2].starts_with("broken"), "table:\n{table}");
    assert!(lines[1].contains("33"), "table:\n{table}");
    assert!(lines[2].contains('-'), "table:\n{table}");
    assert!(lines[0].to_lowercase().contains("tempo"), "table:\n{table}");
}

#[test]
fn a_run_over_no_tracks_gives_empty_rows() {
    let at_130 = FixedTempo(Bpm(130.0));
    let truth: Vec<GroundTruth> = Vec::new();
    let report = run(&truth, &[&at_130]).unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(report.rows[0].tracks, 0);
    assert_eq!(report.rows[0].tempo_accuracy1, None);
}

/// Writes three tracks annotated at 125, 130, and 140 beats per minute.
fn write_tempo_truth(dir: &Path) {
    let audio = Audio {
        frames: vec![[0.1, 0.1]; 44_100],
    };
    for (name, bpm) in [("a", 125), ("b", 130), ("c", 140)] {
        write_wav(&dir.join(format!("{name}.wav")), &audio, WavDepth::Int16).unwrap();
        fs::write(dir.join(format!("{name}.bpm")), format!("{bpm}\n")).unwrap();
    }
}

#[test]
fn the_tempo_error_is_the_median_distance_from_the_annotation() {
    let dir = tempfile::tempdir().unwrap();
    write_tempo_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let at_130 = FixedTempo(Bpm(130.0));
    let report = run(&truth, &[&at_130]).unwrap();
    let row = &report.rows[0];
    // Five in 125 is four percent, 130 is exact, and ten in 140 is a little
    // over seven percent; the median of the three is four percent.
    let errors: Vec<f64> = row
        .per_track
        .iter()
        .map(|track| track.tempo_error.unwrap())
        .collect();
    assert!((errors[0] - 0.04).abs() < 1e-12, "{errors:?}");
    assert!(errors[1].abs() < 1e-12, "{errors:?}");
    assert!((errors[2] - 10.0 / 140.0).abs() < 1e-12, "{errors:?}");
    assert!(
        (row.tempo_error.unwrap() - 0.04).abs() < 1e-12,
        "{:?}",
        row.tempo_error
    );
    let table = report.table();
    assert!(table.contains("4.00"), "table:\n{table}");
    assert!(
        table.lines().next().unwrap().contains("error"),
        "table:\n{table}"
    );
}

#[test]
fn the_tempo_error_of_an_even_count_is_the_mean_of_the_two_middle_values() {
    let dir = tempfile::tempdir().unwrap();
    write_tempo_truth(dir.path());
    let audio = Audio {
        frames: vec![[0.1, 0.1]; 44_100],
    };
    write_wav(&dir.path().join("d.wav"), &audio, WavDepth::Int16).unwrap();
    fs::write(dir.path().join("d.bpm"), "65\n").unwrap();
    let truth = load_directory(dir.path()).unwrap();
    let at_130 = FixedTempo(Bpm(130.0));
    let report = run(&truth, &[&at_130]).unwrap();
    // The errors are four percent, zero, a little over seven percent, and
    // one hundred percent; the two middle values are four and seven.
    let expected = (0.04 + 10.0 / 140.0) / 2.0;
    assert!(
        (report.rows[0].tempo_error.unwrap() - expected).abs() < 1e-12,
        "{:?}",
        report.rows[0].tempo_error
    );
}

#[test]
fn a_failed_track_has_no_tempo_error() {
    let dir = tempfile::tempdir().unwrap();
    write_tempo_truth(dir.path());
    let truth = load_directory(dir.path()).unwrap();
    let report = run(&truth, &[&Broken]).unwrap();
    assert_eq!(report.rows[0].tempo_error, None);
    assert!(
        report.rows[0]
            .per_track
            .iter()
            .all(|t| t.tempo_error.is_none())
    );
}
