//! The anchor scoreboard's ground-truth format, metrics, and report, checked
//! on a directory built in a temporary folder.

use std::fs;
use std::path::Path;

use dermixen_analysis::{
    AnalysisError, AnchorAnalysis, AnchorAnalyzer, EdgeAnchors, Extent, FixedTempo, PulseGrid,
    ScoreboardError, Source, error_in_beats, load_anchor_truth, run_anchors, run_grids,
};
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, Samples, Seconds};
use dermixen_media::{Audio, WavDepth, write_wav};

/// Writes forty seconds of kicks at 120 beats per minute, one every half
/// second from one second in, as a playable file.
fn write_kicks(path: &Path) {
    let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds(1.0), Seconds(40.0));
    write_wav(path, &audio, WavDepth::Int16).unwrap();
}

/// An annotation for the kick track: beat zero one second in, the intro
/// anchor at bar four (beat 16, nine seconds in) confirmed by ear, and the
/// outro anchor at bar twelve (beat 48, twenty-five seconds in) from a
/// MixMeister mix, with the music beginning at one second and ending at
/// thirty-nine.
const ANNOTATION: &str = "# a synthetic kick track
file synthetic/kicks.wav
bpm 120
first_beat 1.0
begins 1.0 plot
ends 39.0 plot
intro 9.0 ear
outro 25.0 mixmeister
";

/// An anchor analyzer that answers fixed beats, so the metrics can be checked
/// against errors chosen in advance.
struct Fixed {
    intro: Beats,
    outro: Beats,
}

impl AnchorAnalyzer for Fixed {
    fn name(&self) -> &str {
        "fixed"
    }

    fn analyze(&self, _audio: &Audio, grid: &BeatGrid) -> Result<AnchorAnalysis, AnalysisError> {
        Ok(AnchorAnalysis {
            extent: Extent {
                begins: grid.position_of(Beats::ZERO),
                ends: grid.position_of(Beats(76.0)),
            },
            anchors: Anchors {
                intro: self.intro,
                outro: self.outro,
            },
            confidence: 0.5,
        })
    }
}

#[test]
fn an_annotation_beside_its_audio_loads_with_its_grid_and_labels() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write_kicks(&dir.join("kicks.wav"));
    fs::write(dir.join("kicks.anchors"), ANNOTATION).unwrap();
    // An annotation with no audio beside it is reported, not loaded.
    fs::write(dir.join("elsewhere.anchors"), ANNOTATION).unwrap();

    let truth = load_anchor_truth(dir).unwrap();
    assert_eq!(truth.without_audio, vec![dir.join("elsewhere.anchors")]);
    assert!(truth.warnings.is_empty(), "{:?}", truth.warnings);
    assert_eq!(truth.tracks.len(), 1);
    let track = &truth.tracks[0];
    assert_eq!(track.name, "kicks");
    assert_eq!(track.audio, dir.join("kicks.wav"));
    assert_eq!(track.grid.bpm, Bpm(120.0));
    assert_eq!(track.grid.first_beat, Samples(44_100));
    let intro = track.intro.unwrap();
    assert_eq!(intro.at, Seconds(9.0));
    assert_eq!(intro.source, Source::Ear);
    assert_eq!(track.outro.unwrap().source, Source::MixMeister);
    assert_eq!(track.begins.unwrap().at, Seconds(1.0));
    assert_eq!(track.ends.unwrap().source, Source::Plot);
}

#[test]
fn a_label_without_a_source_is_an_error_that_names_the_line() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write_kicks(&dir.join("kicks.wav"));
    let annotation = dir.join("kicks.anchors");
    fs::write(&annotation, "bpm 120\nfirst_beat 1.0\nintro 9.0\n").unwrap();
    match load_anchor_truth(dir) {
        Err(ScoreboardError::Annotation { path, message }) => {
            assert_eq!(path, annotation);
            assert!(message.contains("line 3"), "{message}");
            assert!(message.contains("source"), "{message}");
        }
        other => panic!("expected an annotation error, got {other:?}"),
    }
}

#[test]
fn an_outro_after_the_ending_is_kept_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write_kicks(&dir.join("kicks.wav"));
    // MixMeister's plot file for one track puts the outro range past the
    // effective ending it gives; the label stays as the source gave it.
    fs::write(
        dir.join("kicks.anchors"),
        "bpm 120\nfirst_beat 1.0\nends 30.0 plot\noutro 35.0 plot\n",
    )
    .unwrap();
    let truth = load_anchor_truth(dir).unwrap();
    assert_eq!(truth.tracks[0].outro.unwrap().at, Seconds(35.0));
    assert_eq!(truth.warnings.len(), 1);
    assert!(truth.warnings[0].contains("kicks"), "{}", truth.warnings[0]);
    assert!(
        truth.warnings[0].contains("35.000"),
        "{}",
        truth.warnings[0]
    );
}

#[test]
fn a_directory_with_no_annotation_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    write_kicks(&dir.path().join("kicks.wav"));
    assert!(matches!(
        load_anchor_truth(dir.path()),
        Err(ScoreboardError::Empty(_))
    ));
}

#[test]
fn errors_are_measured_in_beats_of_the_labeled_grid() {
    // At 120 beats per minute a beat is half a second, so an estimate one
    // second late is two beats late.
    assert_eq!(error_in_beats(Seconds(10.0), Seconds(9.0), Bpm(120.0)), 2.0);
}

#[test]
fn the_report_scores_each_position_and_splits_rows_by_source() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write_kicks(&dir.join("kicks.wav"));
    fs::write(dir.join("kicks.anchors"), ANNOTATION).unwrap();
    let truth = load_anchor_truth(dir).unwrap();

    // Intro one bar late (beat 20 against 16), outro exactly right (beat 48).
    let fixed = Fixed {
        intro: Beats(20.0),
        outro: Beats(48.0),
    };
    let edges = EdgeAnchors;
    let report = run_anchors(&truth.tracks, &[&fixed, &edges]).unwrap();

    let fixed_scores = &report.per_track["fixed"];
    assert_eq!(fixed_scores.len(), 1);
    let score = &fixed_scores[0];
    assert_eq!(score.intro.unwrap().error, 4.0);
    assert_eq!(score.intro.unwrap().source, Source::Ear);
    assert_eq!(score.outro.unwrap().error, 0.0);
    // The extent from the fixed analyzer begins at beat zero, which is the
    // labeled beginning, and ends at beat 76, thirty-nine seconds in.
    assert_eq!(score.begins.unwrap().error, 0.0);
    assert!(score.ends.unwrap().error.abs() < 0.01);
    assert_eq!(score.confidence, Some(0.5));

    // The rows: one over every source per analyzer, then one per source
    // with a label, in analyzer order.
    let rows: Vec<(&str, Option<Source>)> = report
        .rows
        .iter()
        .map(|row| (row.analyzer.as_str(), row.source))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("fixed", None),
            ("fixed", Some(Source::Ear)),
            ("fixed", Some(Source::MixMeister)),
            ("fixed", Some(Source::Plot)),
            ("edges", None),
            ("edges", Some(Source::Ear)),
            ("edges", Some(Source::MixMeister)),
            ("edges", Some(Source::Plot)),
        ]
    );
    let all = &report.rows[0];
    assert_eq!(all.tracks, 1);
    assert_eq!(all.failures, 0);
    assert_eq!(all.intro.scored, 1);
    // One bar late is within a bar and within four bars.
    assert_eq!(all.intro.within_bar, Some(1.0));
    assert_eq!(all.intro.within_four_bars, Some(1.0));
    assert_eq!(all.intro.median_beats, Some(4.0));
    let ear = &report.rows[1];
    assert_eq!(ear.intro.scored, 1);
    assert_eq!(ear.outro.scored, 0);
    assert_eq!(ear.outro.within_bar, None);

    // The edges analyzer puts the intro on the first sound, which is beat
    // zero, sixteen beats early, so it is outside a bar but not outside four.
    let edge_row = &report.rows[4];
    assert_eq!(edge_row.intro.within_bar, Some(0.0));
    assert_eq!(edge_row.intro.within_four_bars, Some(1.0));

    let table = report.table();
    assert!(table.starts_with("analyzer  labels"), "{table}");
    assert!(table.contains("fixed     all"), "{table}");
    assert!(table.contains("edges     ear"), "{table}");
    let details = report.details("fixed");
    assert!(
        details.contains("kicks: intro +4.0 (ear), outro +0.0 (mixmeister)"),
        "{details}"
    );
    assert_eq!(report.details("nobody"), "");
}

#[test]
fn the_grid_report_says_whether_beat_zero_is_on_a_beat_and_on_a_downbeat() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write_kicks(&dir.join("kicks.wav"));
    fs::write(dir.join("kicks.anchors"), ANNOTATION).unwrap();
    let truth = load_anchor_truth(dir).unwrap();

    // The fixed analyzer starts its beats at the first sample, which is two
    // beats before the labeled beat zero: on a beat, but not on a downbeat.
    let fixed = FixedTempo(Bpm(120.0));
    let pulse = PulseGrid;
    let report = run_grids(&truth.tracks, &[&fixed, &pulse]).unwrap();
    let fixed_row = &report.rows[0];
    assert_eq!(fixed_row.tempo_accuracy1, Some(1.0));
    assert_eq!(fixed_row.tempo_error, Some(0.0));
    assert_eq!(fixed_row.on_beat, Some(1.0));
    assert_eq!(fixed_row.on_downbeat, Some(0.0));

    // The pulse analyzer reads the tempo off the kicks and starts on one.
    let pulse_row = &report.rows[1];
    assert_eq!(pulse_row.failures, 0);
    assert!(
        pulse_row.tempo_error.unwrap() < 0.001,
        "{:?}",
        pulse_row.tempo_error
    );
    assert_eq!(pulse_row.on_beat, Some(1.0));

    let table = report.table();
    assert!(table.starts_with("analyzer"), "{table}");
    assert!(
        table.contains("tempo 1  tempo 2  error %  on beat %  downbeat %"),
        "{table}"
    );
    assert!(
        report
            .details("fixed tempo")
            .contains("on a beat yes, on a downbeat no")
    );
    assert_eq!(report.details("nobody"), "");
}
