//! Acceptance tests for what the command does with input made to break it:
//! an output that is one of the command's own inputs, a number far outside
//! any mix, a path that names a device, a link planted where a temporary
//! file goes, and text that would drive a terminal. A coder agent makes
//! these pass without editing them.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

use common::{dermixen, fails, json, kicks_file, ok, stdout, two_track_mix};
use dermixen_core::Mix;

/// Runs the command as [`dermixen`] does, and kills it and panics when it
/// has not ended in `seconds`, which is what a read of a device or a render
/// without end looks like. The kill matters, and so does a short wait: a
/// read of `/dev/zero` takes five gigabytes of memory a second.
fn promptly(seconds: u64, dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_dermixen"));
    command
        .args(args)
        .current_dir(dir)
        .env("DERMIXEN_LIBRARY_FILE", dir.join("library.sqlite"))
        .env("DERMIXEN_SETTINGS_FILE", dir.join("settings.toml"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().expect("the dermixen binary runs");
    let started = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if started.elapsed() > Duration::from_secs(seconds) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("dermixen {args:?} did not end within {seconds} seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
#[ignore = "cli-hardening"]
fn no_writing_command_replaces_one_of_its_own_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    let document = fs::read(dir.join("set.dmx")).unwrap();
    let track = fs::read(dir.join("a.wav")).unwrap();
    symlink(dir.join("a.wav"), dir.join("link.wav")).unwrap();
    fs::hard_link(dir.join("b.wav"), dir.join("hard.wav")).unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    let second = fs::read(dir.join("b.wav")).unwrap();

    for (output, named) in [
        ("set.dmx", "set.dmx"),
        ("./sub/../set.dmx", "set.dmx"),
        ("a.wav", "a.wav"),
        ("link.wav", "a.wav"),
        ("hard.wav", "b.wav"),
    ] {
        for command in [
            vec!["render", "set.dmx", output],
            vec!["play", "set.dmx", "--for", "0:02", "--capture", output],
        ] {
            let message = fails(&dermixen(dir, &[], &command));
            assert!(message.contains(named), "{command:?}: {message}");
            assert_eq!(
                fs::read(dir.join("set.dmx")).unwrap(),
                document,
                "{command:?}"
            );
            assert_eq!(fs::read(dir.join("a.wav")).unwrap(), track, "{command:?}");
            assert_eq!(fs::read(dir.join("b.wav")).unwrap(), second, "{command:?}");
        }
    }

    for output in ["a.wav", "link.wav", "./sub/../a.wav"] {
        let message = fails(&dermixen(dir, &[], &["decode", "a.wav", output]));
        assert!(message.contains("a.wav"), "{message}");
        assert_eq!(fs::read(dir.join("a.wav")).unwrap(), track);
    }

    // Replacing an earlier render is what the command is for.
    ok(&dermixen(dir, &[], &["render", "set.dmx", "out.wav"]));
    ok(&dermixen(dir, &[], &["render", "set.dmx", "out.wav"]));
}

#[test]
#[ignore = "cli-hardening"]
fn an_anchor_far_outside_any_mix_is_refused_and_the_document_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    let before = fs::read(dir.join("set.dmx")).unwrap();
    for arguments in [
        vec!["mix", "move-anchor", "set.dmx", "1", "--outro", "1e308"],
        vec!["mix", "move-anchor", "set.dmx", "2", "--intro=-1e308"],
        vec!["mix", "move-anchor", "set.dmx", "1", "--outro", "10000001"],
        // Within the limit of a beat, and a mix of more than 24 hours.
        vec!["mix", "move-anchor", "set.dmx", "1", "--outro", "300000"],
        vec!["mix", "set-gain", "set.dmx", "1", "1e308"],
        vec!["mix", "set-gain", "set.dmx", "1", "24.5"],
    ] {
        let message = fails(&dermixen(dir, &[], &arguments));
        assert!(!message.contains("panicked"), "{arguments:?}: {message}");
        assert_eq!(
            fs::read(dir.join("set.dmx")).unwrap(),
            before,
            "{arguments:?}"
        );
    }
    ok(&dermixen(dir, &[], &["mix", "show", "set.dmx"]));
}

#[test]
#[ignore = "cli-hardening"]
fn a_number_far_outside_any_track_is_refused_and_names_its_option() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    kicks_file(dir, "c.wav", 140.0, 0.0);
    let before = fs::read(dir.join("set.dmx")).unwrap();
    for (option, arguments) in [
        (
            "--intro",
            vec![
                "mix", "add", "set.dmx", "c.wav", "--bpm", "140", "--intro", "1e300", "--outro",
                "1e301",
            ],
        ),
        (
            "--bpm",
            vec![
                "mix", "add", "set.dmx", "c.wav", "--bpm", "1e308", "--intro", "0", "--outro", "16",
            ],
        ),
        (
            "--bpm",
            vec![
                "mix", "add", "set.dmx", "c.wav", "--bpm", "19.9", "--intro", "0", "--outro", "16",
            ],
        ),
        (
            "--first-beat",
            vec![
                "mix",
                "add",
                "set.dmx",
                "c.wav",
                "--bpm",
                "140",
                "--first-beat",
                "1e15",
                "--intro",
                "0",
                "--outro",
                "16",
            ],
        ),
        ("--bpm", vec!["analyze", "c.wav", "--bpm", "1e308"]),
        ("--bpm", vec!["analyze", "c.wav", "--bpm", "1000"]),
        ("--bpm", vec!["library", "query", "--bpm", "nan"]),
        ("--bpm", vec!["library", "query", "--bpm", "inf"]),
        ("--bpm", vec!["library", "query", "--bpm", "120-inf"]),
    ] {
        let message = fails(&dermixen(dir, &[], &arguments));
        assert!(message.contains(option), "{arguments:?}: {message}");
        assert_eq!(
            fs::read(dir.join("set.dmx")).unwrap(),
            before,
            "{arguments:?}"
        );
    }
}

#[test]
fn a_track_that_overflows_the_loudness_meter_leaves_a_document_that_opens() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(dir.join("hostile.wav"), spec).unwrap();
    for n in 0..(20 * 44_100) {
        let value = if (44_100..44_164).contains(&n) {
            f32::MAX
        } else if n % 20_000 < 200 {
            0.8
        } else {
            0.0
        };
        writer.write_sample(value).unwrap();
        writer.write_sample(value).unwrap();
    }
    writer.finalize().unwrap();

    let added = dermixen(
        dir,
        &[],
        &[
            "mix",
            "add",
            "set.dmx",
            "hostile.wav",
            "--bpm",
            "132",
            "--intro",
            "0",
            "--outro",
            "16",
            "--no-keylock",
            "--preset",
            "cut",
        ],
    );
    assert!(matches!(added.status.code(), Some(0 | 1)), "{added:?}");
    let text = fs::read_to_string(dir.join("set.dmx")).unwrap();
    assert!(!text.contains("null"), "{text}");
    let mix = Mix::from_json(&text).unwrap();
    assert_eq!(mix.tracks.len(), if added.status.success() { 3 } else { 2 });
    ok(&dermixen(dir, &[], &["mix", "show", "set.dmx"]));
}

/// The two-track mix with the first track's outro anchor moved far enough
/// out that the mix lasts about seven and a half hours: 60,000 beats at 130
/// beats per minute is 27,692 seconds.
fn a_mix_of_seven_hours(dir: &Path) {
    two_track_mix(dir);
    let mut mix = Mix::from_json(&fs::read_to_string(dir.join("set.dmx")).unwrap()).unwrap();
    mix.tracks[0].anchors.outro = dermixen_core::Beats(60_000.0);
    mix.tracks[0].tempo.clear();
    mix.tracks[1].tempo.clear();
    fs::write(dir.join("long.dmx"), mix.checked_json().unwrap()).unwrap();
}

#[test]
#[ignore = "cli-hardening"]
fn a_mix_too_long_for_a_wav_file_is_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    a_mix_of_seven_hours(dir);
    let started = Instant::now();
    let message = fails(&promptly(10, dir, &[], &["render", "long.dmx", "long.wav"]));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(message.contains("4 GiB"), "{message}");
    assert!(message.contains(".mp3"), "{message}");
    let left: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("long.wav") || name.contains(".part"))
        .collect();
    assert!(left.is_empty(), "{left:?}");

    let message = fails(&promptly(
        10,
        dir,
        &[],
        &["play", "long.dmx", "--capture", "long.wav"],
    ));
    assert!(message.contains("4 GiB"), "{message}");
}

#[test]
#[ignore = "cli-hardening"]
fn a_path_that_names_a_device_is_an_error_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    symlink("/dev/zero", dir.join("zero.dmx")).unwrap();
    symlink("/dev/zero", dir.join("zero.txt")).unwrap();
    symlink("/dev/zero", dir.join("zero.toml")).unwrap();
    let text = fs::read_to_string(dir.join("set.dmx")).unwrap();
    let device = text.replacen(dir.join("a.wav").to_str().unwrap(), "/dev/zero", 1);
    assert_ne!(device, text, "the document names a.wav by its full path");
    fs::write(dir.join("device.dmx"), &device).unwrap();

    for arguments in [
        vec!["mix", "show", "zero.dmx"],
        vec!["mix", "show", "/dev/zero"],
        vec!["render", "zero.dmx", "out.wav"],
        vec!["mix", "set-gain", "zero.dmx", "1", "0"],
        vec!["mix", "plan", "zero.txt"],
        vec!["render", "device.dmx", "out.wav"],
        vec!["mix", "relink", "device.dmx", "--under", "."],
        vec!["analyze", "/dev/zero"],
        vec!["decode", "/dev/zero", "out.wav"],
    ] {
        let message = fails(&promptly(2, dir, &[], &arguments));
        assert!(!message.contains("panicked"), "{arguments:?}: {message}");
    }
    let zero = dir.join("zero.toml");
    let message = fails(&promptly(
        2,
        dir,
        &[("DERMIXEN_SETTINGS_FILE", zero.to_str().unwrap())],
        &["settings", "show"],
    ));
    assert!(message.contains("zero.toml"), "{message}");
    assert!(!dir.join("out.wav").exists());
}

#[test]
#[ignore = "cli-hardening"]
fn a_document_over_the_size_limit_is_refused_by_size() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let mut text = String::from("{\"version\": 1, \"tracks\": [");
    text.push_str(&" ".repeat(16 * 1024 * 1024));
    text.push_str("]}");
    fs::write(dir.join("large.dmx"), text).unwrap();
    let message = fails(&promptly(10, dir, &[], &["mix", "show", "large.dmx"]));
    assert!(message.contains("16777216"), "{message}");
}

#[test]
#[ignore = "cli-hardening"]
fn a_link_planted_where_a_temporary_file_went_is_not_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    fs::write(dir.join("victim.txt"), "PRECIOUS").unwrap();
    for planted in [
        "out.wav.part",
        "out.mp3.part",
        "heard.wav.part",
        "copy.wav.part",
        "set.dmx.new",
        "set.dmx.part",
    ] {
        symlink(dir.join("victim.txt"), dir.join(planted)).unwrap();
    }
    ok(&dermixen(dir, &[], &["render", "set.dmx", "out.wav"]));
    ok(&dermixen(dir, &[], &["render", "set.dmx", "out.mp3"]));
    ok(&dermixen(
        dir,
        &[],
        &["play", "set.dmx", "--for", "0:02", "--capture", "heard.wav"],
    ));
    ok(&dermixen(dir, &[], &["decode", "a.wav", "copy.wav"]));
    ok(&dermixen(
        dir,
        &[],
        &["mix", "set-gain", "set.dmx", "1", "-1.5"],
    ));
    ok(&dermixen(
        dir,
        &[],
        &["mix", "move-anchor", "set.dmx", "1", "--outro", "44"],
    ));
    ok(&dermixen(
        dir,
        &[],
        &["mix", "relink", "set.dmx", "--under", "."],
    ));

    assert_eq!(fs::read(dir.join("victim.txt")).unwrap(), b"PRECIOUS");
    for written in ["out.wav", "out.mp3", "heard.wav", "copy.wav", "set.dmx"] {
        let kind = fs::symlink_metadata(dir.join(written)).unwrap().file_type();
        assert!(kind.is_file(), "{written} is not a regular file");
    }
}

#[test]
#[ignore = "cli-hardening"]
fn an_edit_keeps_the_permissions_of_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    fs::set_permissions(dir.join("set.dmx"), fs::Permissions::from_mode(0o600)).unwrap();
    ok(&dermixen(
        dir,
        &[],
        &["mix", "set-gain", "set.dmx", "1", "-1.5"],
    ));
    assert_eq!(mode_of(&dir.join("set.dmx")), 0o600);
    ok(&dermixen(
        dir,
        &[],
        &["mix", "move-anchor", "set.dmx", "1", "--outro", "44"],
    ));
    assert_eq!(mode_of(&dir.join("set.dmx")), 0o600);
}

#[test]
#[ignore = "cli-hardening"]
fn text_from_a_file_cannot_drive_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let hostile = "music/\u{1b}[2J\u{1b}]0;owned\u{7}\u{9b}31m.wav";
    kicks_file(dir, hostile, 130.0, 0.0);
    kicks_file(dir, "music/line\nbreak.wav", 140.0, 0.0);

    let scanned = dermixen(dir, &[], &["library", "scan", "music"]);
    ok(&scanned);
    let listed = dermixen(dir, &[], &["library", "query"]);
    ok(&listed);
    let found = dermixen(dir, &[], &["library", "find", "owned"]);
    ok(&found);
    ok(&dermixen(dir, &[], &["mix", "new", "set.dmx"]));
    let added = dermixen(
        dir,
        &[],
        &[
            "mix", "add", "set.dmx", hostile, "--bpm", "130", "--intro", "0", "--outro", "16",
        ],
    );
    ok(&added);
    let shown = dermixen(dir, &[], &["mix", "show", "set.dmx"]);
    ok(&shown);
    fs::remove_file(dir.join(hostile)).unwrap();
    let missing = dermixen(dir, &[], &["render", "set.dmx", "out.wav"]);
    assert_eq!(missing.status.code(), Some(1));

    for (name, output) in [
        ("library scan", &scanned),
        ("library query", &listed),
        ("library find", &found),
        ("mix add", &added),
        ("mix show", &shown),
        ("render", &missing),
    ] {
        for stream in [&output.stdout, &output.stderr] {
            // ESC, BEL, and the one-byte CSI of the C1 set as UTF-8.
            assert!(!stream.contains(&0x1b), "{name} printed an escape");
            assert!(!stream.contains(&0x07), "{name} printed a bell");
            assert!(
                !stream.windows(2).any(|pair| pair == [0xc2, 0x9b]),
                "{name} printed a C1 control character"
            );
        }
    }
    // One track is one line, whatever its name contains.
    assert_eq!(stdout(&listed).lines().count(), 2, "{}", stdout(&listed));
    assert!(stdout(&listed).contains("\\x1b"), "{}", stdout(&listed));

    // JSON output is unchanged, because JSON escapes every control character.
    let records = json(&dermixen(dir, &[], &["library", "query", "--json"]));
    let paths: Vec<&str> = records
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["path"].as_str().unwrap())
        .collect();
    assert!(
        paths.iter().any(|path| path.contains("\u{1b}[2J")),
        "{paths:?}"
    );
}
