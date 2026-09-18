//! Acceptance tests for the `play` command and for the span flags of
//! `render`. A coder agent makes these pass without editing them.
//!
//! The promise under test is that what `play` sends to the device is what
//! `render` writes for the same span, checked here through `--capture`,
//! which writes those frames to a file instead of a device, so the tests
//! need no audio hardware.

mod common;

use std::path::Path;

use common::*;
use dermixen_media::{Frame, decode};

fn frames(path: &Path) -> Vec<Frame> {
    decode(path).unwrap().audio.frames
}

/// The length of the mix as `mix show` reports it, written the way the
/// commands write lengths: minutes, seconds, and tenths.
fn mix_length_text(dir: &Path) -> String {
    let shown = json(&dermixen(dir, &[], &["mix", "show", "set.dmx", "--json"]));
    let seconds = shown["length_seconds"].as_f64().unwrap();
    let tenths = (seconds * 10.0).round() as i64;
    format!("{}:{:04.1}", tenths / 600, (tenths % 600) as f64 / 10.0)
}

/// Seventeen seconds in, as a frame count: where the span the tests use
/// begins. The span runs to twenty-two seconds, so it holds the last three
/// seconds of the first track, the start of the beatmix at eighteen and a
/// half seconds where the second track is already sounding, and the first
/// track's end at twenty seconds.
const FROM: usize = 749_700;

/// Five seconds, as a frame count: the length of that span.
const LENGTH: usize = 220_500;

#[test]
fn render_from_and_for_write_the_span_of_the_whole_render() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    ok(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "whole.wav"],
    ));
    let whole = frames(&dir.path().join("whole.wav"));
    assert!(
        whole.len() > FROM + LENGTH,
        "the mix is longer than the span"
    );

    let out = dermixen(
        dir.path(),
        &[],
        &[
            "render", "set.dmx", "span.wav", "--from", "17", "--for", "5",
        ],
    );
    ok(&out);
    assert!(stdout(&out).contains("span.wav"));
    assert!(stdout(&out).contains("0:05.0"), "{}", stdout(&out));
    let span = frames(&dir.path().join("span.wav"));
    assert_eq!(span.len(), LENGTH);
    assert_eq!(span, whole[FROM..FROM + LENGTH]);

    ok(&dermixen(
        dir.path(),
        &[],
        &[
            "render",
            "set.dmx",
            "colon.wav",
            "--from",
            "0:17",
            "--for",
            "0:05",
        ],
    ));
    assert_eq!(frames(&dir.path().join("colon.wav")), span);

    ok(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "head.wav", "--for", "5"],
    ));
    assert_eq!(frames(&dir.path().join("head.wav")), whole[..LENGTH]);

    ok(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "tail.wav", "--from", "17"],
    ));
    assert_eq!(frames(&dir.path().join("tail.wav")), whole[FROM..]);

    let out = dermixen(
        dir.path(),
        &[],
        &[
            "render", "set.dmx", "long.wav", "--from", "17", "--for", "99:00",
        ],
    );
    ok(&out);
    assert_eq!(frames(&dir.path().join("long.wav")), whole[FROM..]);
}

#[test]
fn render_and_play_refuse_a_start_past_the_end_and_a_bad_span() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let length = mix_length_text(dir.path());

    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.wav", "--from", "99:00"],
    ));
    assert!(message.contains(&length), "{message}");
    assert!(!dir.path().join("out.wav").exists());

    let message = fails(&dermixen(
        dir.path(),
        &[],
        &[
            "play",
            "set.dmx",
            "--from",
            "99:00",
            "--capture",
            "heard.wav",
        ],
    ));
    assert!(message.contains(&length), "{message}");
    assert!(!dir.path().join("heard.wav").exists());

    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.wav", "--for", "0"],
    ));
    assert!(message.contains("--for"), "{message}");

    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "out.wav", "--from", "soon"],
    ));
    assert!(message.contains("--from"), "{message}");

    let invalid = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mix/invalid/negative-bpm.dmx");
    std::fs::copy(invalid, dir.path().join("bad.dmx")).unwrap();
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["play", "bad.dmx", "--capture", "heard.wav"],
    ));
    assert!(message.contains("tracks[0].grid.bpm"), "{message}");
}

#[test]
fn play_capture_writes_exactly_what_render_writes() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    ok(&dermixen(
        dir.path(),
        &[],
        &[
            "render", "set.dmx", "span.wav", "--from", "17", "--for", "5",
        ],
    ));
    let out = dermixen(
        dir.path(),
        &[],
        &[
            "play",
            "set.dmx",
            "--from",
            "17",
            "--for",
            "5",
            "--capture",
            "heard.wav",
        ],
    );
    ok(&out);
    let said = stdout(&out);
    assert!(said.contains("0:05.0"), "{said}");
    assert!(said.contains("0:17.0"), "{said}");
    assert!(said.contains("no underruns"), "{said}");
    assert!(said.contains("heard.wav"), "{said}");
    assert_eq!(
        frames(&dir.path().join("heard.wav")),
        frames(&dir.path().join("span.wav"))
    );

    ok(&dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "whole.wav"],
    ));
    ok(&dermixen(
        dir.path(),
        &[],
        &["play", "set.dmx", "--capture", "all.wav"],
    ));
    assert_eq!(
        frames(&dir.path().join("all.wav")),
        frames(&dir.path().join("whole.wav"))
    );
}
