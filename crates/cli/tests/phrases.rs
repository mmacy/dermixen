//! Acceptance tests for phrases behind the commands: `analyze` and the
//! `library` commands report the phrase analysis, and `scoreboard` prints the
//! phrase scoreboard. A coder agent makes these pass without editing them.

mod common;

use std::path::Path;

use common::{dermixen, json, kicks_file, ok, stdout};
use dermixen_core::{Bpm, Seconds};
use dermixen_media::{WavDepth, write_wav};
use dermixen_testkit::synth;

/// Checks a value against one definition in the schema file, failing with
/// every violation the validator found.
fn fits(value: &serde_json::Value, definition: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let whole: serde_json::Value = serde_json::from_str(&text).unwrap();
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

/// Checks that a phrase record describes bars of the grid it was found on:
/// every start and section is a whole beat that begins a bar under the
/// downbeat, the starts are in time order with the lengths the schema
/// allows, and the sections are in time order.
fn check_phrases(phrases: &serde_json::Value) {
    let downbeat = phrases["downbeat"].as_u64().unwrap();
    assert!(downbeat < 4, "{phrases}");
    let starts = phrases["starts"].as_array().unwrap();
    let mut last = f64::NEG_INFINITY;
    for start in starts {
        let beat = start["beat"].as_f64().unwrap();
        assert_eq!(
            beat.fract(),
            0.0,
            "a phrase start off a whole beat in {phrases}"
        );
        assert_eq!(
            (beat as i64 - downbeat as i64).rem_euclid(4),
            0,
            "a phrase start off a bar line in {phrases}"
        );
        assert!(beat > last, "phrase starts out of order in {phrases}");
        last = beat;
        assert!([8, 16, 32].contains(&start["bars"].as_u64().unwrap()));
    }
    let mut last = f64::NEG_INFINITY;
    for section in phrases["sections"].as_array().unwrap() {
        let beat = section.as_f64().unwrap();
        assert_eq!(beat.fract(), 0.0, "a section off a whole beat in {phrases}");
        assert_eq!(
            (beat as i64 - downbeat as i64).rem_euclid(4),
            0,
            "{phrases}"
        );
        assert!(beat > last, "sections out of order in {phrases}");
        last = beat;
    }
}

/// A minute of kicks at 130 beats per minute, silent for its first four
/// bars, as an audio file the analyzers have something to say about.
fn a_minute_of_kicks(dir: &Path, name: &str) {
    let period = 60.0 / 130.0;
    let audio = synth::kicks(Bpm(130.0), Seconds(0.5 + 16.0 * period), Seconds(60.0));
    write_wav(&dir.join(name), &audio, WavDepth::Int16).unwrap();
}

#[test]
fn analyze_reports_the_phrases_the_shifts_analyzer_finds() {
    let dir = tempfile::tempdir().unwrap();
    a_minute_of_kicks(dir.path(), "a.wav");
    let out = dermixen(
        dir.path(),
        &[],
        &[
            "analyze",
            "a.wav",
            "--json",
            "--bpm",
            "130",
            "--first-beat",
            "0.5",
        ],
    );
    let value = json(&out);
    fits(&value, "analyze");
    let phrases = &value["phrases"];
    assert!(phrases.is_object(), "no phrase record in {value}");
    assert_eq!(phrases["analyzer"], "shifts");
    check_phrases(phrases);
    assert!(
        !phrases["starts"].as_array().unwrap().is_empty(),
        "a minute of kicks has at least one phrase start"
    );
    assert_eq!(
        value["grid"]["first_beat_sample"], 22_050,
        "the grid is not moved to put beat zero on a downbeat"
    );

    let text = stdout(&dermixen(
        dir.path(),
        &[],
        &["analyze", "a.wav", "--bpm", "130", "--first-beat", "0.5"],
    ));
    for name in ["phrase_analyzer", "downbeat", "phrase_starts", "sections"] {
        assert!(
            text.lines().any(|line| line.starts_with(name)),
            "no {name} line in\n{text}"
        );
    }
}

#[test]
fn a_scan_stores_the_phrases_and_the_library_commands_print_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lib");
    std::fs::create_dir_all(&root).unwrap();
    kicks_file(dir.path(), "lib/01 Etnica - Alpha.wav", 130.0, 0.5);
    let index = dir.path().join("library.sqlite");
    let env = [("DERMIXEN_LIBRARY_FILE", index.to_str().unwrap())];
    ok(&dermixen(dir.path(), &env, &["library", "scan", "lib"]));

    let value = json(&dermixen(dir.path(), &env, &["library", "query", "--json"]));
    fits(&value, "library_query");
    let records = value.as_array().unwrap();
    assert_eq!(records.len(), 1);
    let phrases = &records[0]["phrases"];
    assert!(phrases.is_object(), "the scan stored no phrases: {value}");
    assert_eq!(phrases["analyzer"], "shifts");
    check_phrases(phrases);

    let value = json(&dermixen(
        dir.path(),
        &env,
        &["library", "find", "etnica alpha", "--json"],
    ));
    fits(&value, "library_find");
    assert_eq!(value[0]["record"]["phrases"], *phrases);
}

/// Writes one annotated track: a minute at 130 beats per minute whose grid
/// starts half a second in, silent for its first four bars and kicks from
/// bar four on, with anchor labels and one phrase label at bar eight.
fn phrase_truth(dir: &Path) {
    let period: f64 = 60.0 / 130.0;
    let intro = 0.5 + 16.0 * period;
    a_minute_of_kicks(dir, "a.wav");
    let last_kick = 0.5 + ((60.0 - 0.5) / period).floor() * period;
    let outro = last_kick - 64.0 * period;
    let phrase = 0.5 + 32.0 * period;
    std::fs::write(
        dir.join("a.anchors"),
        format!(
            "# Synthetic kicks for the phrase scoreboard tests.\nbpm 130\nfirst_beat 0.5\nintro {intro:.6} ear\noutro {outro:.6} ear\nphrase {phrase:.6} ear\n"
        ),
    )
    .unwrap();
}

#[test]
fn the_scoreboard_scores_phrases_when_the_directory_has_anchor_annotations() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    std::fs::create_dir_all(&truth).unwrap();
    phrase_truth(&truth);
    let out = dermixen(dir.path(), &[], &["scoreboard", "truth"]);
    ok(&out);
    let text = stdout(&out);
    for name in ["counted", "shifts", "16 bars"] {
        assert!(text.contains(name), "no {name} in\n{text}");
    }

    let value = json(&dermixen(
        dir.path(),
        &[],
        &["scoreboard", "truth", "--json"],
    ));
    fits(&value, "scoreboard");
    let phrases = value["phrases"].as_array().unwrap();
    for name in ["counted", "shifts"] {
        assert!(
            phrases.iter().any(|row| row["analyzer"] == name),
            "no {name} row in {value}"
        );
    }
    for row in phrases {
        assert_eq!(row["per_track"][0]["name"], "a");
    }
    // The counting analyzer takes beat zero as a bar and a thirty-two-bar
    // phrase and counts from there, so the label at bar eight sits on its
    // eight-bar start and not on a sixteen-bar one.
    let counted = phrases
        .iter()
        .find(|row| row["analyzer"] == "counted" && row["source"].is_null())
        .expect("a counted row over all sources");
    assert_eq!(counted["tracks"], 1);
    assert_eq!(counted["phrase_labels"], 1);
    assert_eq!(counted["on_8_bars"], 1.0);
    assert_eq!(counted["on_16_bars"], 0.0);
    assert_eq!(counted["downbeat"], 1.0);
    assert_eq!(counted["per_track"][0]["downbeat"], true);
    let ear = phrases
        .iter()
        .find(|row| row["analyzer"] == "counted" && row["source"] == "ear")
        .expect("a counted row over the ear labels");
    assert_eq!(ear["phrase_labels"], 1);
    assert!(
        ear["downbeat"].is_null(),
        "only the row over all sources has a downbeat share"
    );
}

#[test]
fn a_directory_without_anchor_annotations_has_no_phrase_rows() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    std::fs::create_dir_all(&truth).unwrap();
    a_minute_of_kicks(&truth, "a.wav");
    std::fs::write(truth.join("a.bpm"), "130\n").unwrap();
    let value = json(&dermixen(
        dir.path(),
        &[],
        &["scoreboard", "truth", "--json"],
    ));
    fits(&value, "scoreboard");
    assert_eq!(value["phrases"], serde_json::json!([]));
}
