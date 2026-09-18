//! Acceptance tests for `decode`, which writes the audio the library and the
//! render read from a file, or a span of it, as a 16-bit WAV at 44.1 kHz. A
//! coder makes these pass without editing them.
//!
//! The profiling programs in `tools/` read audio through this command, so
//! what it writes has to be the frames `dermixen_media::decode` gives, cut
//! at the span asked for and nowhere else.

mod common;

use std::path::{Path, PathBuf};

use common::{dermixen, fails, json, kicks_file, ok, stdout};
use dermixen_media::{Audio, decode};

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

/// One of the committed audio fixtures.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
        .canonicalize()
        .unwrap()
}

/// The largest difference between two decodes of the same length, frame by
/// frame and channel by channel.
fn largest_difference(a: &Audio, b: &Audio) -> f32 {
    assert_eq!(a.len(), b.len(), "the two decodes differ in length");
    a.frames
        .iter()
        .zip(&b.frames)
        .flat_map(|(x, y)| [(x[0] - y[0]).abs(), (x[1] - y[1]).abs()])
        .fold(0.0, f32::max)
}

/// A frame written as a sixteen-bit sample and read back differs from the
/// frame written by at most half a step of that scale, so a whole step is
/// the bound with room for the rounding of the arithmetic itself.
const ONE_STEP: f32 = 1.0 / 32_767.0;

#[test]
fn decode_writes_the_frames_the_media_crate_decodes() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "a.wav", 130.0, 0.5);

    // A sixteen-bit WAV comes back frame for frame.
    let out = dermixen(dir.path(), &[], &["decode", "a.wav", "out.wav", "--json"]);
    let value = json(&out);
    fits(&value, "decode");
    assert!(
        value["file"].as_str().unwrap().ends_with("a.wav"),
        "{value}"
    );
    assert!(
        value["path"].as_str().unwrap().ends_with("out.wav"),
        "{value}"
    );
    assert_eq!(value["from_seconds"], 0.0);
    assert_eq!(value["length_samples"], 20 * 44_100);
    assert_eq!(value["length_seconds"], 20.0);
    let written = decode(&dir.path().join("out.wav")).unwrap().audio;
    let source = decode(&dir.path().join("a.wav")).unwrap().audio;
    assert_eq!(written.frames, source.frames);

    // An MP3 comes back as the media crate decodes it, to the precision of a
    // sixteen-bit sample. The tools read this, and the library stored its
    // grid against the same decode, so the two name the same sample.
    let tone = fixture("sine-440-44k.mp3");
    ok(&dermixen(
        dir.path(),
        &[],
        &["decode", &tone.display().to_string(), "tone.wav"],
    ));
    let written = decode(&dir.path().join("tone.wav")).unwrap().audio;
    let source = decode(&tone).unwrap().audio;
    let apart = largest_difference(&written, &source);
    assert!(apart <= ONE_STEP, "the MP3 decode differs by {apart}");

    // A file at another sample rate comes out at 44.1 kHz, as the library
    // reads it.
    let fast = fixture("sine-440-48k.wav");
    let out = dermixen(
        dir.path(),
        &[],
        &["decode", &fast.display().to_string(), "fast.wav", "--json"],
    );
    let value = json(&out);
    let written = decode(&dir.path().join("fast.wav")).unwrap().audio;
    let source = decode(&fast).unwrap().audio;
    assert_eq!(value["length_samples"], source.len().0);
    let apart = largest_difference(&written, &source);
    assert!(apart <= ONE_STEP, "the resampled decode differs by {apart}");
    assert_eq!(written.len(), source.len());
}

#[test]
fn decode_writes_a_span_and_cuts_it_at_the_end_of_the_file() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "a.wav", 130.0, 0.5);
    let source = decode(&dir.path().join("a.wav")).unwrap().audio;

    // Two seconds from five seconds in, as seconds or as minutes and seconds.
    let out = dermixen(
        dir.path(),
        &[],
        &[
            "decode", "a.wav", "span.wav", "--from", "5", "--for", "2", "--json",
        ],
    );
    let value = json(&out);
    fits(&value, "decode");
    assert_eq!(value["from_seconds"], 5.0);
    assert_eq!(value["length_samples"], 2 * 44_100);
    assert_eq!(value["length_seconds"], 2.0);
    let written = decode(&dir.path().join("span.wav")).unwrap().audio;
    assert_eq!(written.frames, source.frames[5 * 44_100..7 * 44_100]);
    ok(&dermixen(
        dir.path(),
        &[],
        &[
            "decode",
            "a.wav",
            "clock.wav",
            "--from",
            "0:05",
            "--for",
            "0:02",
        ],
    ));
    let clock = decode(&dir.path().join("clock.wav")).unwrap().audio;
    assert_eq!(clock.frames, written.frames);

    // A length that runs past the end is cut there, and the output says how
    // much was written.
    let out = dermixen(
        dir.path(),
        &[],
        &["decode", "a.wav", "tail.wav", "--from", "18", "--for", "5"],
    );
    ok(&out);
    let text = stdout(&out);
    for wanted in ["tail.wav", "0:02.0", "88200 samples"] {
        assert!(text.contains(wanted), "no {wanted:?} in:\n{text}");
    }
    let tail = decode(&dir.path().join("tail.wav")).unwrap().audio;
    assert_eq!(tail.frames, source.frames[18 * 44_100..]);

    // A start at or past the end of the file is refused with the option
    // named, and no file is written.
    for late in ["20", "25"] {
        let message = fails(&dermixen(
            dir.path(),
            &[],
            &["decode", "a.wav", "late.wav", "--from", late],
        ));
        assert!(message.contains("--from"), "{message}");
        assert!(
            !dir.path().join("late.wav").exists(),
            "late.wav was written"
        );
    }
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["decode", "a.wav", "none.wav", "--for", "0"],
    ));
    assert!(message.contains("--for"), "{message}");
}

#[test]
fn decode_refuses_a_missing_file_a_file_that_is_not_audio_and_a_bad_time() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "a.wav", 130.0, 0.5);
    std::fs::write(dir.path().join("notes.txt"), "not audio\n").unwrap();

    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["decode", "missing.mp3", "out.wav"],
    ));
    assert!(message.contains("missing.mp3"), "{message}");
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["decode", "notes.txt", "out.wav"],
    ));
    assert!(message.contains("notes.txt"), "{message}");
    for (option, bad) in [("--from", "fast"), ("--for", "1x")] {
        let message = fails(&dermixen(
            dir.path(),
            &[],
            &["decode", "a.wav", "out.wav", option, bad],
        ));
        assert!(message.contains(option), "{option} {bad}: {message}");
        assert!(message.contains(bad), "{option} {bad}: {message}");
    }
    assert!(!dir.path().join("out.wav").exists(), "out.wav was written");

    // Under --json a failure is still one line on standard error and nothing
    // on standard output.
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["decode", "missing.mp3", "out.wav", "--json"],
    ));
    assert!(message.starts_with("error:"), "{message}");
}
