//! Acceptance tests for the bespoke analyzers behind the commands and for
//! the anchor and grid scoreboards. A coder agent makes these pass without
//! editing them.

mod common;

use std::path::Path;

use common::{dermixen, json, ok, stderr, stdout};
use dermixen_core::{Bpm, Seconds};
use dermixen_media::{WavDepth, write_wav};
use dermixen_testkit::synth;

/// Checks a value against one definition in the schema file.
fn fits(value: &serde_json::Value, definition: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let whole: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": format!("#/$defs/{definition}"),
        "$defs": whole["$defs"],
    });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let problems: Vec<String> = validator
        .iter_errors(value)
        .map(|problem| format!("{} at {}", problem, problem.instance_path()))
        .collect();
    assert!(
        problems.is_empty(),
        "does not fit {definition}:\n{}\n{}",
        problems.join("\n"),
        serde_json::to_string_pretty(value).unwrap()
    );
}

/// A minute of kicks at 130 beats per minute, the first kick half a second in.
fn a_minute_of_kicks(dir: &Path, name: &str) {
    let audio = synth::kicks(Bpm(130.0), Seconds(0.5), Seconds(60.0));
    write_wav(&dir.join(name), &audio, WavDepth::Int16).unwrap();
}

#[test]
fn analyze_uses_the_bespoke_grid_and_anchor_analyzers() {
    let dir = tempfile::tempdir().unwrap();
    a_minute_of_kicks(dir.path(), "a.wav");
    let value = json(&dermixen(dir.path(), &[], &["analyze", "a.wav", "--json"]));
    fits(&value, "analyze");
    assert_eq!(value["grid_analyzer"], "pulse");
    assert_eq!(value["anchor_analyzer"], "kick");
    // The pulse analyzer reads the tempo to well within a tenth of a percent
    // on kicks this clean, and puts beat zero on a kick.
    let bpm = value["grid"]["bpm"].as_f64().unwrap();
    assert!((bpm - 130.0).abs() < 0.13, "bpm {bpm}");
    let first = value["grid"]["first_beat_sample"].as_i64().unwrap() as f64 / 44_100.0;
    let period = 60.0 / 130.0;
    let phase = ((first - 0.5) / period).round() * period + 0.5;
    assert!(
        (first - phase).abs() < 0.01,
        "beat zero at {first} s is not on a kick"
    );
    let confidence = value["grid_confidence"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&confidence));
    assert!(
        value["anchors"]["outro_beat"].as_f64().unwrap()
            > value["anchors"]["intro_beat"].as_f64().unwrap()
    );
}

#[test]
fn a_scan_needs_no_foreign_beat_tracker() {
    // This test runs with any feature set; the grid comes from pulse.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    std::fs::create_dir_all(&root).unwrap();
    a_minute_of_kicks(&root, "01 Etnica - Alpha.wav");
    let value = json(&dermixen(
        dir.path(),
        &[],
        &["library", "scan", "music", "--json"],
    ));
    assert_eq!(value["added"], 1, "{value}");
    let records = json(&dermixen(dir.path(), &[], &["library", "query", "--json"]));
    assert_eq!(records[0]["grid_analyzer"], "pulse");
    assert_eq!(records[0]["anchor_analyzer"], "kick");
}

/// Writes one annotated track: a minute at 130 beats per minute whose grid
/// starts half a second in, silent for its first four bars and kicks from
/// bar four on, with an anchors file whose intro anchor is bar four, where
/// the kick starts, and whose outro anchor is sixteen bars before the last kick.
fn anchor_truth(dir: &Path) {
    let period: f64 = 60.0 / 130.0;
    let intro = 0.5 + 16.0 * period;
    let audio = synth::kicks(Bpm(130.0), Seconds(intro), Seconds(60.0));
    write_wav(&dir.join("a.wav"), &audio, WavDepth::Int16).unwrap();
    let last_kick = 0.5 + ((60.0 - 0.5) / period).floor() * period;
    let outro = last_kick - 64.0 * period;
    std::fs::write(
        dir.join("a.anchors"),
        format!(
            "# Synthetic kicks for the scoreboard tests.\nbpm 130\nfirst_beat 0.5\nintro {intro:.6} ear\noutro {outro:.6} ear\n"
        ),
    )
    .unwrap();
}

#[test]
fn the_scoreboard_scores_anchors_and_grids_when_the_directory_has_anchor_annotations() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    std::fs::create_dir_all(&truth).unwrap();
    anchor_truth(&truth);
    let out = dermixen(dir.path(), &[], &["scoreboard", "truth"]);
    ok(&out);
    let text = stdout(&out);
    for name in ["edges", "kick", "pulse"] {
        assert!(text.contains(name), "no {name} row in\n{text}");
    }
    assert!(text.contains("downbeat"), "{text}");
    assert!(text.contains("intro"), "{text}");

    let value = json(&dermixen(
        dir.path(),
        &[],
        &["scoreboard", "truth", "--json"],
    ));
    fits(&value, "scoreboard");
    let anchors = value["anchors"].as_array().unwrap();
    assert!(
        anchors.iter().any(|row| row["analyzer"] == "kick"),
        "{value}"
    );
    assert!(
        anchors.iter().any(|row| row["analyzer"] == "edges"),
        "{value}"
    );
    for row in anchors {
        assert_eq!(row["per_track"][0]["name"], "a");
    }
    let grids = value["grids"].as_array().unwrap();
    let pulse = grids
        .iter()
        .find(|row| row["analyzer"] == "pulse")
        .expect("a pulse row");
    assert_eq!(pulse["tracks"], 1);
    assert_eq!(pulse["per_track"][0]["name"], "a");
    assert_eq!(pulse["per_track"][0]["accuracy1"], true);
    // The kick analyzer puts the intro anchor at the first bar that starts
    // eight kick bars in a row, which here is the bar the label names.
    let kick = anchors
        .iter()
        .find(|row| row["analyzer"] == "kick" && row["source"].is_null())
        .expect("a kick row over all sources");
    assert_eq!(kick["intro"]["scored"], 1);
    assert!(
        kick["intro"]["median_beats"].as_f64().unwrap().abs() <= 4.0,
        "{kick}"
    );
    // The beat rows now record the analyzer's confidence per track.
    let beats = value["beats"].as_array().unwrap();
    assert!(
        beats
            .iter()
            .all(|row| row["per_track"][0].get("confidence").is_some())
    );
}

#[test]
fn a_directory_without_anchor_annotations_has_no_anchor_or_grid_rows() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    std::fs::create_dir_all(&truth).unwrap();
    a_minute_of_kicks(&truth, "a.wav");
    std::fs::write(truth.join("a.bpm"), "130\n").unwrap();
    let out = dermixen(dir.path(), &[], &["scoreboard", "truth"]);
    ok(&out);
    assert!(!stdout(&out).contains("downbeat"), "{}", stdout(&out));
    let value = json(&dermixen(
        dir.path(),
        &[],
        &["scoreboard", "truth", "--json"],
    ));
    fits(&value, "scoreboard");
    assert_eq!(value["anchors"], serde_json::json!([]));
    assert_eq!(value["grids"], serde_json::json!([]));
    assert!(
        value["beats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["analyzer"] == "pulse"),
        "{value}"
    );
}

#[test]
fn an_annotation_that_contradicts_itself_is_a_warning_on_standard_error() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    std::fs::create_dir_all(&truth).unwrap();
    a_minute_of_kicks(&truth, "a.wav");
    std::fs::write(
        truth.join("a.anchors"),
        "bpm 130\nfirst_beat 0.5\nends 30.0 plot\noutro 50.0 plot\n",
    )
    .unwrap();
    let out = dermixen(dir.path(), &[], &["scoreboard", "truth"]);
    ok(&out);
    let warning = stderr(&out);
    assert!(
        warning.contains("outro") && warning.contains("ending"),
        "{warning}"
    );
}
