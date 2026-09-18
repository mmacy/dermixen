//! Acceptance tests for `mix relink`, and for the way `render` points at
//! it when a file is missing. A coder agent makes these pass without editing
//! them.

mod common;

use std::path::Path;

use common::{dermixen, fails, json, ok, stdout, two_track_mix};
use dermixen_core::Mix;

fn read_mix(path: &Path) -> Mix {
    Mix::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

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

#[test]
fn relink_finds_a_moved_file_under_a_folder_and_rewrites_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let mix = two_track_mix(dir.path());
    let before = read_mix(&mix);
    std::fs::create_dir(dir.path().join("moved")).unwrap();
    std::fs::rename(
        dir.path().join("a.wav"),
        dir.path().join("moved/renamed.wav"),
    )
    .unwrap();

    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "relink", "set.dmx", "--under", "moved"],
    );
    ok(&out);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 2, "{text}");
    assert!(
        lines[0].contains("1.")
            && lines[0].contains("relinked")
            && lines[0].contains("renamed.wav"),
        "{text}"
    );
    assert!(
        lines[1].contains("2.") && lines[1].contains("kept"),
        "{text}"
    );

    let after = read_mix(&mix);
    assert_eq!(
        after.tracks[0].path,
        dir.path().join("moved/renamed.wav").canonicalize().unwrap()
    );
    assert_eq!(after.tracks[0].hash, before.tracks[0].hash);
    assert_eq!(after.tracks[1], before.tracks[1]);

    // A render now reads the file where it is.
    ok(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.wav"],
    ));
}

#[test]
fn relink_asks_the_library_before_searching_anywhere() {
    let dir = tempfile::tempdir().unwrap();
    let mix = two_track_mix(dir.path());
    let before = read_mix(&mix);
    std::fs::create_dir(dir.path().join("moved")).unwrap();
    std::fs::rename(dir.path().join("a.wav"), dir.path().join("moved/a.wav")).unwrap();
    // The scan sees the file's bytes under a new path and points the library at it.
    ok(&dermixen(dir.path(), &[], &["library", "scan", "moved"]));

    let out = dermixen(dir.path(), &[], &["mix", "relink", "set.dmx", "--json"]);
    let value = json(&out);
    fits(&value, "mix_relink");
    assert_eq!(value["relinked"], 1);
    assert_eq!(value["missing"], 0);
    let tracks = value["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0]["position"], 1);
    assert_eq!(tracks[0]["outcome"], "relinked");
    assert_eq!(
        tracks[0]["path"],
        dir.path()
            .join("moved/a.wav")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(tracks[0]["from"], before.tracks[0].path.to_str().unwrap());
    assert_eq!(tracks[1]["outcome"], "kept");
    assert_eq!(tracks[1]["from"], serde_json::Value::Null);
    assert_eq!(
        read_mix(&mix).tracks[0].path,
        dir.path().join("moved/a.wav").canonicalize().unwrap()
    );
}

#[test]
fn a_file_found_nowhere_is_reported_and_the_document_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mix = two_track_mix(dir.path());
    let before = std::fs::read_to_string(&mix).unwrap();
    std::fs::remove_file(dir.path().join("a.wav")).unwrap();
    std::fs::create_dir(dir.path().join("elsewhere")).unwrap();

    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "relink", "set.dmx", "--under", "elsewhere", "--json"],
    );
    let value = json(&out);
    fits(&value, "mix_relink");
    assert_eq!(value["relinked"], 0);
    assert_eq!(value["missing"], 1);
    let tracks = value["tracks"].as_array().unwrap();
    assert_eq!(tracks[0]["outcome"], "missing");
    let reason = tracks[0]["reason"].as_str().unwrap();
    assert!(reason.contains("elsewhere"), "{reason}");
    assert_eq!(tracks[1]["outcome"], "kept");
    assert_eq!(tracks[1]["reason"], serde_json::Value::Null);
    assert_eq!(std::fs::read_to_string(&mix).unwrap(), before);

    let out = dermixen(dir.path(), &[], &["mix", "relink", "set.dmx"]);
    ok(&out);
    assert!(stdout(&out).contains("missing"), "{}", stdout(&out));
}

#[test]
fn a_render_with_a_missing_file_says_to_relink() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    std::fs::remove_file(dir.path().join("a.wav")).unwrap();
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.wav"],
    ));
    assert!(
        message.contains("a.wav") && message.contains("mix relink"),
        "{message}"
    );
    assert!(!dir.path().join("out.wav").exists());
}

#[test]
fn a_folder_that_cannot_be_searched_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["mix", "relink", "set.dmx", "--under", "nowhere"],
    ));
    assert!(message.contains("nowhere"), "{message}");
}
