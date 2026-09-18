//! Helpers shared by the command-line acceptance tests: running the built
//! binary in a temporary folder and making synthetic audio for it to read.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dermixen_core::{Bpm, Seconds};
use dermixen_media::{WavDepth, write_wav};
use dermixen_testkit::synth;

/// Runs `dermixen` in `dir` with the given arguments and environment
/// variables, with `DERMIXEN_LIBRARY_FILE` pointed at `library.sqlite` in `dir`
/// and `DERMIXEN_SETTINGS_FILE` at `settings.toml` in `dir` unless the caller
/// sets them, so that no test reads or writes the settings file of whoever
/// runs it.
pub fn dermixen(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dermixen"));
    command.args(args).current_dir(dir);
    if !env.iter().any(|(name, _)| *name == "DERMIXEN_LIBRARY_FILE") {
        command.env("DERMIXEN_LIBRARY_FILE", dir.join("library.sqlite"));
    }
    if !env
        .iter()
        .any(|(name, _)| *name == "DERMIXEN_SETTINGS_FILE")
    {
        command.env("DERMIXEN_SETTINGS_FILE", dir.join("settings.toml"));
    }
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("the dermixen binary runs")
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub fn ok(output: &Output) {
    assert!(
        output.status.success(),
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout(output),
        stderr(output)
    );
}

/// Asserts the command failed with exit code one and returns what it said.
pub fn fails(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
    assert!(
        stdout(output).is_empty(),
        "a failed command prints nothing on standard output, but printed:\n{}",
        stdout(output)
    );
    stderr(output)
}

/// The one JSON document a command printed, and nothing else.
pub fn json(output: &Output) -> serde_json::Value {
    ok(output);
    let text = stdout(output);
    serde_json::from_str(&text).unwrap_or_else(|problem| {
        panic!("standard output is not one JSON document: {problem}\n{text}")
    })
}

/// Writes twenty seconds of kicks at `bpm`, the first kick `first` seconds in.
pub fn kicks_file(dir: &Path, relative: &str, bpm: f64, first: f64) -> PathBuf {
    let path = dir.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let audio = synth::kicks(Bpm(bpm), Seconds(first), Seconds(20.0));
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    path
}

/// A two-track mix of twenty-second kick tracks at 130 and 140 beats per
/// minute with given grids, so that no beat tracker is needed.
///
/// The first track's outro anchor is beat 40, about eighteen and a half
/// seconds in, so the second track enters about eleven seconds into the
/// mix, the eight-bar beatmix begins at eighteen and a half seconds with
/// both tracks sounding, the first track ends at twenty seconds, and the
/// mix runs about thirty-two seconds.
pub fn two_track_mix(dir: &Path) -> PathBuf {
    kicks_file(dir, "a.wav", 130.0, 0.0);
    kicks_file(dir, "b.wav", 140.0, 0.0);
    ok(&dermixen(dir, &[], &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir,
        &[],
        &[
            "mix",
            "add",
            "set.dmx",
            "a.wav",
            "--bpm",
            "130",
            "--first-beat",
            "0",
            "--intro",
            "0",
            "--outro",
            "40",
            "--no-keylock",
        ],
    ));
    ok(&dermixen(
        dir,
        &[],
        &[
            "mix",
            "add",
            "set.dmx",
            "b.wav",
            "--bpm",
            "140",
            "--intro",
            "16",
            "--outro",
            "80",
            "--no-keylock",
            "--preset",
            "beatmix",
        ],
    ));
    dir.join("set.dmx")
}
