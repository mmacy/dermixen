//! Acceptance tests for rendering to MP3. A coder agent makes these pass without
//! editing them.
//!
//! These tests exist only in a build with the `mp3` feature, which is on by
//! default: `cargo test -p dermixen-cli --test mp3`.

#![cfg(feature = "mp3")]

mod common;

use std::path::Path;

use common::{dermixen, fails, json, ok, stdout, two_track_mix};
use dermixen_media::decode;
use dermixen_testkit::mp3::mp3_frames;

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
fn render_writes_an_mp3_when_the_output_ends_in_mp3() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    ok(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.wav"],
    ));
    let wav = decode(&dir.path().join("out.wav")).unwrap();

    let out = dermixen(dir.path(), &[], &["render", "set.dmx", "out.mp3"]);
    ok(&out);
    assert!(stdout(&out).contains("out.mp3"), "{}", stdout(&out));
    let bytes = std::fs::read(dir.path().join("out.mp3")).unwrap();
    let frames = mp3_frames(&bytes);
    assert!(
        frames.iter().all(|frame| *frame == (320, 44_100)),
        "{frames:?}"
    );
    let mp3 = decode(&dir.path().join("out.mp3")).unwrap();
    assert_eq!(mp3.source_sample_rate, 44_100);
    assert_eq!(mp3.source_channels, 2);
    let difference = (mp3.audio.len().0 - wav.audio.len().0).abs();
    assert!(
        difference <= 2_400,
        "the lengths differ by {difference} frames"
    );
    assert!(
        !dir.path().join("out.mp3.part").exists(),
        "no temporary file is left behind"
    );

    // The extension is read in either case, and the JSON output names the file.
    let out = dermixen(dir.path(), &[], &["render", "set.dmx", "OUT.MP3", "--json"]);
    let value = json(&out);
    fits(&value, "render");
    assert!(value["path"].as_str().unwrap().ends_with("OUT.MP3"));
    assert_eq!(value["length_samples"], wav.audio.len().0);
    assert!(mp3_frames(&std::fs::read(dir.path().join("OUT.MP3")).unwrap()).len() > 100);
}

#[test]
fn a_failed_mp3_render_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let mix = two_track_mix(dir.path());
    let text = std::fs::read_to_string(&mix).unwrap();
    let hash = dermixen_core::Mix::from_json(&text).unwrap().tracks[1]
        .hash
        .to_string();
    std::fs::write(&mix, text.replace(&hash, &"0".repeat(64))).unwrap();
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.mp3"],
    ));
    assert!(message.contains("b.wav"), "{message}");
    assert!(!dir.path().join("out.mp3").exists());
    assert!(!dir.path().join("out.mp3.part").exists());
}
