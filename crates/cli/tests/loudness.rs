//! Acceptance tests for loudness in the commands: `analyze` reports it,
//! `library scan` completes records that lack it, and `mix add` writes the
//! leveling gain the loudness gives. A coder agent makes these pass without
//! editing them.

mod common;

use std::path::Path;

use common::{dermixen, json, kicks_file, ok, stderr, stdout};
use dermixen_analysis::measure_loudness;
use dermixen_core::{Decibels, Mix, leveling_gain};
use dermixen_library::{Index, Query};
use dermixen_media::decode;

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

fn read_mix(path: &Path) -> Mix {
    Mix::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn analyze_reports_the_loudness_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = kicks_file(dir.path(), "01 Etnica - Alpha.wav", 130.0, 0.5);
    let expected = measure_loudness(&decode(&a).unwrap().audio).expect("kicks have a loudness");
    let out = dermixen(
        dir.path(),
        &[],
        &["analyze", "01 Etnica - Alpha.wav", "--json", "--bpm", "130"],
    );
    let value = json(&out);
    fits(&value, "analyze");
    assert_eq!(
        value["loudness"]["integrated_lufs"].as_f64().unwrap(),
        expected.integrated.0
    );
    assert_eq!(
        value["loudness"]["true_peak_db"].as_f64().unwrap(),
        expected.true_peak.0
    );

    let out = dermixen(
        dir.path(),
        &[],
        &["analyze", "01 Etnica - Alpha.wav", "--bpm", "130"],
    );
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("LUFS") && text.contains("dBTP"), "{text}");
}

#[test]
fn mix_add_writes_the_leveling_gain_and_show_reports_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = kicks_file(dir.path(), "a.wav", 130.0, 0.0);
    let loudness = measure_loudness(&decode(&a).unwrap().audio).unwrap();
    let expected = leveling_gain(loudness.integrated, loudness.true_peak);
    ok(&dermixen(dir.path(), &[], &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir.path(),
        &[],
        &[
            "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--outro", "40",
        ],
    ));
    let mix = read_mix(&dir.path().join("set.dmx"));
    assert_eq!(mix.tracks[0].gain, expected);

    let out = dermixen(dir.path(), &[], &["mix", "show", "set.dmx", "--json"]);
    let value = json(&out);
    fits(&value, "mix_show");
    assert_eq!(value["tracks"][0]["gain_db"].as_f64().unwrap(), expected.0);
    let out = dermixen(dir.path(), &[], &["mix", "show", "set.dmx"]);
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains(&format!("{:+.1} dB", expected.0)), "{text}");
}

#[test]
fn a_given_gain_replaces_the_leveling_gain() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "a.wav", 130.0, 0.0);
    kicks_file(dir.path(), "b.wav", 140.0, 0.0);
    ok(&dermixen(dir.path(), &[], &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir.path(),
        &[],
        &[
            "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--outro", "40", "--gain", "-3.5",
        ],
    ));
    // The beatmix by name, since a twenty-second file has no room for the
    // blend's twenty-eight bars between its anchors.
    ok(&dermixen(
        dir.path(),
        &[],
        &[
            "mix", "add", "set.dmx", "b.wav", "--bpm", "140", "--intro", "16", "--outro", "80",
            "--gain", "2", "--preset", "beatmix",
        ],
    ));
    let mix = read_mix(&dir.path().join("set.dmx"));
    assert_eq!(mix.tracks[0].gain, Decibels(-3.5));
    assert_eq!(mix.tracks[1].gain, Decibels(2.0));

    let out = dermixen(
        dir.path(),
        &[],
        &[
            "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--gain", "loud",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn a_scan_completes_records_that_have_no_loudness_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "music/a.wav", 130.0, 0.5);
    kicks_file(dir.path(), "music/b.wav", 140.0, 0.5);
    let out = dermixen(dir.path(), &[], &["library", "scan", "music"]);
    ok(&out);
    assert!(stdout(&out).contains("added 2"), "{}", stdout(&out));

    // One record loses its loudness, so the next scan has one to complete.
    let index_path = dir.path().join("library.sqlite");
    let mut index = Index::open(&index_path).unwrap();
    let mut records = index.query(&Query::default()).unwrap();
    assert!(records.iter().all(|record| record.loudness.is_some()));
    let measured = records[0].loudness.take().unwrap();
    index.upsert(&records[0]).unwrap();
    drop(index);

    let out = dermixen(dir.path(), &[], &["library", "scan", "music"]);
    ok(&out);
    let text = stdout(&out);
    assert!(
        text.contains("completed 1") && text.contains("unchanged 1"),
        "{text}"
    );
    assert!(stderr(&out).contains("completed"), "{}", stderr(&out));
    let index = Index::open(&index_path).unwrap();
    assert_eq!(
        index.get(records[0].hash).unwrap().unwrap().loudness,
        Some(measured)
    );
    drop(index);

    let out = dermixen(dir.path(), &[], &["library", "scan", "music", "--json"]);
    let value = json(&out);
    fits(&value, "library_scan");
    assert_eq!(value["completed"], 0);
    assert_eq!(value["unchanged"], 2);
}

#[test]
fn library_query_includes_the_loudness() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "music/a.wav", 130.0, 0.5);
    ok(&dermixen(dir.path(), &[], &["library", "scan", "music"]));
    let out = dermixen(dir.path(), &[], &["library", "query", "--json"]);
    let value = json(&out);
    fits(&value, "library_query");
    let track = &value[0];
    assert!(track["loudness"]["integrated_lufs"].is_number(), "{track}");
    assert!(track["loudness"]["true_peak_db"].is_number(), "{track}");
}
