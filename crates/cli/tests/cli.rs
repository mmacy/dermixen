//! Acceptance tests for the `dermixen` command. A coder agent makes these pass without editing them.
//!
//! Each test runs the built binary as a user would, on synthetic audio
//! written into a temporary directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use dermixen_core::{Beats, Bpm, Mix, Samples, Seconds, TempoNode, Track};
use dermixen_engine::{Resampler, TimeStretcher, render};
use dermixen_media::{Audio, WavDepth, decode, hash_file, write_wav};
use dermixen_testkit::synth;

/// Runs `dermixen` in `dir`, with the library file kept inside `dir` so a
/// test never touches the library on the machine it runs on.
fn dermixen(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dermixen"))
        .args(args)
        .current_dir(dir)
        .env("DERMIXEN_LIBRARY_FILE", dir.join("library.sqlite"))
        .output()
        .expect("the dermixen binary runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn ok(output: &Output) {
    assert!(
        output.status.success(),
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        stdout(output),
        stderr(output)
    );
}

fn fails(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
    stderr(output)
}

fn kicks_file(dir: &Path, name: &str, bpm: f64) -> PathBuf {
    let path = dir.join(name);
    let audio = synth::kicks(Bpm(bpm), Seconds::ZERO, Seconds(20.0));
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    path
}

/// A two-track mix of kick tracks at 130 and 140 BPM, with keylock off on
/// both and joined by an eight-bar beatmix, which the twenty-second files
/// have room for where they do not have room for a blend.
fn two_track_mix(dir: &Path) -> PathBuf {
    kicks_file(dir, "a.wav", 130.0);
    kicks_file(dir, "b.wav", 140.0);
    ok(&dermixen(dir, &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir,
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
            "64",
            "--no-keylock",
        ],
    ));
    ok(&dermixen(
        dir,
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

fn read_mix(path: &Path) -> Mix {
    Mix::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn help_lists_the_commands() {
    let dir = tempfile::tempdir().unwrap();
    let out = dermixen(dir.path(), &["--help"]);
    ok(&out);
    let text = stdout(&out);
    for command in ["analyze", "mix", "render", "scoreboard"] {
        assert!(
            text.contains(command),
            "--help does not mention {command}:\n{text}"
        );
    }
    let out = dermixen(dir.path(), &["mix", "--help"]);
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("new") && text.contains("add"), "{text}");
    let out = dermixen(dir.path(), &["--version"]);
    ok(&out);
    assert!(stdout(&out).starts_with("dermixen "));
    let out = dermixen(dir.path(), &["no-such-command"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn mix_new_writes_an_empty_document_and_will_not_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
    let mix = read_mix(&dir.path().join("set.dmx"));
    assert!(mix.tracks.is_empty());
    let message = fails(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
    assert!(
        message.contains("set.dmx") && message.contains("exists"),
        "{message}"
    );
}

#[test]
fn mix_add_appends_tracks_with_a_transition() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());
    let mix = read_mix(&path);
    assert_eq!(mix.tracks.len(), 2);

    let a = &mix.tracks[0];
    assert_eq!(a.path, dir.path().join("a.wav").canonicalize().unwrap());
    assert_eq!(a.hash, hash_file(&dir.path().join("a.wav")).unwrap());
    assert_eq!(a.length, Samples(20 * 44_100));
    assert_eq!(a.grid.bpm, Bpm(130.0));
    assert_eq!(a.grid.first_beat, Samples::ZERO);
    assert_eq!(a.anchors.intro, Beats(0.0));
    assert_eq!(a.anchors.outro, Beats(64.0));
    assert!(!a.keylock);
    assert_eq!(
        a.tempo,
        vec![TempoNode {
            at: Beats(64.0),
            bpm: Bpm(130.0)
        }]
    );
    assert_eq!(a.volume.len(), 6);
    assert!(a.eq.low.is_empty() && a.eq.mid.is_empty() && a.eq.high.is_empty());

    let b = &mix.tracks[1];
    assert_eq!(b.grid.bpm, Bpm(140.0));
    assert_eq!(b.anchors.intro, Beats(16.0));
    assert_eq!(b.anchors.outro, Beats(80.0));
    assert_eq!(
        b.tempo,
        vec![TempoNode {
            at: Beats(48.0),
            bpm: Bpm(140.0)
        }]
    );
    assert_eq!(b.volume.len(), 6);
    assert_eq!(b.volume.nodes()[0].at, Beats(16.0));
    assert_eq!(b.volume.nodes()[5].at, Beats(48.0));
}

#[test]
fn mix_add_defaults_and_keylock() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "a.wav", 130.0);
    ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir.path(),
        &[
            "mix",
            "add",
            "set.dmx",
            "a.wav",
            "--bpm",
            "130",
            "--first-beat",
            "0.25",
        ],
    ));
    let mix = read_mix(&dir.path().join("set.dmx"));
    let a = &mix.tracks[0];
    assert!(a.keylock);
    assert_eq!(a.grid.first_beat, Samples(11_025));
    assert_eq!(a.anchors.intro, Beats(0.0));
    // Twenty seconds at 130 BPM, from a first beat a quarter second in, is 42.79 beats.
    assert_eq!(a.anchors.outro, Beats(42.0));
    assert!(a.tempo.is_empty());
    assert!(a.volume.is_empty());
}

#[test]
fn mix_add_refuses_a_missing_file_a_missing_document_and_a_fractional_anchor() {
    let dir = tempfile::tempdir().unwrap();
    ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
    let message = fails(&dermixen(
        dir.path(),
        &["mix", "add", "set.dmx", "nothing.wav", "--bpm", "130"],
    ));
    assert!(message.contains("nothing.wav"), "{message}");
    kicks_file(dir.path(), "a.wav", 130.0);
    let message = fails(&dermixen(
        dir.path(),
        &["mix", "add", "other.dmx", "a.wav", "--bpm", "130"],
    ));
    assert!(message.contains("other.dmx"), "{message}");
    let message = fails(&dermixen(
        dir.path(),
        &[
            "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--intro", "3.5",
        ],
    ));
    assert!(message.contains("whole"), "{message}");
    assert!(read_mix(&dir.path().join("set.dmx")).tracks.is_empty());
}

#[test]
fn mix_move_anchor_moves_the_anchor_and_the_nodes_of_its_transition() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());

    // The second track's intro anchor goes eight beats later, and the fade
    // in and the tempo node the beatmix wrote on it go along.
    let out = dermixen(
        dir.path(),
        &["mix", "move-anchor", "set.dmx", "2", "--intro", "24"],
    );
    ok(&out);
    assert!(stdout(&out).contains("intro       24"), "{}", stdout(&out));
    let mix = read_mix(&path);
    let b = &mix.tracks[1];
    assert_eq!(b.anchors.intro, Beats(24.0));
    assert_eq!(b.anchors.outro, Beats(80.0));
    assert_eq!(b.volume.nodes()[0].at, Beats(24.0));
    assert_eq!(b.volume.nodes()[5].at, Beats(56.0));
    assert_eq!(b.tempo[0].at, Beats(56.0));

    // The first track's outro anchor goes eight beats earlier with its fade
    // out and its tempo node, and both anchors of one track move at once.
    ok(&dermixen(
        dir.path(),
        &["mix", "move-anchor", "set.dmx", "1", "--outro", "56"],
    ));
    let a = &read_mix(&path).tracks[0];
    assert_eq!(a.anchors.outro, Beats(56.0));
    assert_eq!(a.volume.nodes()[0].at, Beats(56.0));
    assert_eq!(a.tempo[0].at, Beats(56.0));
    ok(&dermixen(
        dir.path(),
        &[
            "mix",
            "move-anchor",
            "set.dmx",
            "2",
            "--intro",
            "88",
            "--outro",
            "96",
        ],
    ));
    let b = &read_mix(&path).tracks[1];
    assert_eq!(b.anchors.intro, Beats(88.0));
    assert_eq!(b.anchors.outro, Beats(96.0));
}

#[test]
fn mix_move_anchor_refuses_a_bad_beat_a_bad_track_and_no_anchor_at_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());
    let before = std::fs::read_to_string(&path).unwrap();

    let message = fails(&dermixen(
        dir.path(),
        &["mix", "move-anchor", "set.dmx", "2", "--intro", "16.5"],
    ));
    assert!(
        message.contains("--intro") && message.contains("whole"),
        "{message}"
    );
    let message = fails(&dermixen(
        dir.path(),
        &["mix", "move-anchor", "set.dmx", "2", "--outro", "16"],
    ));
    assert!(
        message.contains("16") && message.contains("later"),
        "{message}"
    );
    let message = fails(&dermixen(
        dir.path(),
        &["mix", "move-anchor", "set.dmx", "3", "--outro", "16"],
    ));
    assert!(
        message.contains("no track 3") && message.contains("2 tracks"),
        "{message}"
    );
    let out = dermixen(dir.path(), &["mix", "move-anchor", "set.dmx", "2"]);
    assert_eq!(out.status.code(), Some(2));

    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn render_writes_the_same_audio_the_engine_renders() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());
    let out = dermixen(dir.path(), &["render", "set.dmx", "out.wav"]);
    ok(&out);
    assert!(stdout(&out).contains("out.wav"));

    let mix = read_mix(&path);
    let sources: Vec<Audio> = mix
        .tracks
        .iter()
        .map(|t: &Track| decode(&t.path).unwrap().audio)
        .collect();
    let mut resamplers = |_: &Track| -> Box<dyn TimeStretcher> { Box::new(Resampler::new()) };
    let expected = render(&mix, &sources, &mut resamplers).unwrap();

    let written = decode(&dir.path().join("out.wav")).unwrap();
    assert_eq!(written.source_sample_rate, 44_100);
    assert_eq!(written.source_channels, 2);
    assert_eq!(written.audio.len(), expected.len());
    let worst = written
        .audio
        .frames
        .iter()
        .zip(&expected.frames)
        .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
        .fold(0.0, f32::max);
    assert!(
        worst <= 2.0 / 32_768.0,
        "the written file differs by up to {worst}"
    );
    assert!(written.audio.len() > Samples(30 * 44_100));
}

#[test]
fn render_refuses_a_changed_file_and_a_bad_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());
    let text = std::fs::read_to_string(&path).unwrap();
    let mix = read_mix(&path);
    let wrong = "0".repeat(64);
    let tampered = text.replace(&mix.tracks[1].hash.to_string(), &wrong);
    std::fs::write(&path, tampered).unwrap();
    let message = fails(&dermixen(dir.path(), &["render", "set.dmx", "out.wav"]));
    assert!(
        message.contains("b.wav") && message.contains("hash"),
        "{message}"
    );
    assert!(!dir.path().join("out.wav").exists());

    let invalid = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/mix/invalid/negative-bpm.dmx");
    std::fs::copy(invalid, dir.path().join("bad.dmx")).unwrap();
    let message = fails(&dermixen(dir.path(), &["render", "bad.dmx", "out.wav"]));
    assert!(message.contains("tracks[0].grid.bpm"), "{message}");
}

#[test]
fn analyze_with_a_given_tempo_reports_the_file_as_json() {
    let dir = tempfile::tempdir().unwrap();
    let a = kicks_file(dir.path(), "a.wav", 130.0);
    let out = dermixen(
        dir.path(),
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
    ok(&out);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(json["path"], a.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(json["hash"], hash_file(&a).unwrap().to_string());
    assert_eq!(json["length_samples"], 20 * 44_100);
    assert_eq!(json["grid"]["bpm"], 130.0);
    assert_eq!(json["grid"]["first_beat_sample"], 22_050);
    assert_eq!(json["grid_analyzer"], "given");
    assert_eq!(json["grid_confidence"], 1.0);

    let out = dermixen(dir.path(), &["analyze", "a.wav", "--bpm", "130"]);
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("130") && text.contains("a.wav"), "{text}");
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_err());
}

#[test]
fn analyze_refuses_a_missing_file_with_exit_code_one() {
    let dir = tempfile::tempdir().unwrap();
    let message = fails(&dermixen(dir.path(), &["analyze", "nothing.wav", "--json"]));
    assert!(message.contains("nothing.wav"), "{message}");
}

#[test]
fn analyze_finds_the_tempo_of_a_kick_track() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "a.wav", 130.0);
    let out = dermixen(dir.path(), &["analyze", "a.wav", "--json"]);
    ok(&out);
    let json: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let bpm = json["grid"]["bpm"].as_f64().unwrap();
    assert!((bpm - 130.0).abs() < 0.04 * 130.0, "bpm {bpm}");
    assert_eq!(json["grid_analyzer"], "pulse");
    ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
    ok(&dermixen(dir.path(), &["mix", "add", "set.dmx", "a.wav"]));
    let mix = read_mix(&dir.path().join("set.dmx"));
    assert!((mix.tracks[0].grid.bpm.0 - 130.0).abs() < 0.04 * 130.0);
}

#[test]
fn scoreboard_prints_a_table_over_a_ground_truth_directory() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    std::fs::create_dir(&truth).unwrap();
    for bpm in [120.0, 130.0] {
        kicks_file(&truth, &format!("kicks-{bpm}.wav"), bpm);
        std::fs::write(truth.join(format!("kicks-{bpm}.bpm")), format!("{bpm}\n")).unwrap();
    }
    let out = dermixen(dir.path(), &["scoreboard", "truth"]);
    ok(&out);
    let text = stdout(&out);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 2, "{text}");
    assert!(lines[0].to_lowercase().contains("tempo"), "{text}");
    assert!(text.contains("fixed tempo"), "{text}");
    let message = fails(&dermixen(dir.path(), &["scoreboard", "nowhere"]));
    assert!(message.contains("nowhere"), "{message}");
}

mod mix_commands {
    //! Acceptance tests for the mix commands: `mix show`, insertion,
    //! presets, and the streaming render. A coder agent makes these pass without
    //! editing them.

    use super::*;
    use dermixen_core::Decibels;

    fn beats_of(track: &Track) -> Vec<f64> {
        track.volume.nodes().iter().map(|n| n.at.0).collect()
    }

    #[test]
    fn mix_show_lists_every_track_and_the_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = two_track_mix(dir.path());
        let out = dermixen(dir.path(), &["mix", "show", "set.dmx"]);
        ok(&out);
        let text = stdout(&out);
        assert!(text.contains("a.wav") && text.contains("b.wav"), "{text}");
        assert!(text.contains("130") && text.contains("140"), "{text}");
        // The mix runs from the first track's first sample to the second
        // track's last: track B enters at mix beat 48 of A's 130 BPM grid.
        let mix = read_mix(&path);
        let timeline = mix.timeline().unwrap();
        let length = (timeline.end() - timeline.start()).0;
        let minutes = (length / 60.0).floor() as i64;
        let seconds = length - minutes as f64 * 60.0;
        assert!(
            text.contains(&format!("{minutes}:{seconds:04.1}")),
            "expected the length {minutes}:{seconds:04.1} in\n{text}"
        );
        let message = fails(&dermixen(dir.path(), &["mix", "show", "missing.dmx"]));
        assert!(message.contains("missing.dmx"), "{message}");
    }

    #[test]
    fn a_track_can_be_inserted_and_the_transitions_around_it_are_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = two_track_mix(dir.path());
        kicks_file(dir.path(), "c.wav", 135.0);
        ok(&dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "set.dmx",
                "c.wav",
                "--bpm",
                "135",
                "--intro",
                "8",
                "--outro",
                "72",
                "--position",
                "2",
                "--preset",
                "beatmix",
                "--bars",
                "2",
            ],
        ));
        let mix = read_mix(&path);
        let names: Vec<String> = mix
            .tracks
            .iter()
            .map(|t| t.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.wav", "c.wav", "b.wav"]);
        let (a, c, b) = (&mix.tracks[0], &mix.tracks[1], &mix.tracks[2]);
        // A's fade out now spans two bars from its outro anchor, and its old
        // eight-bar fade's other nodes are gone with it.
        assert_eq!(beats_of(a), vec![64.0, 66.0, 68.0, 70.0, 71.0, 72.0]);
        // C fades in over two bars from its intro anchor and out over two
        // bars from its outro anchor, with the tempo nodes to match.
        assert_eq!(
            beats_of(c),
            vec![
                8.0, 9.0, 10.0, 12.0, 14.0, 16.0, 72.0, 74.0, 76.0, 78.0, 79.0, 80.0
            ]
        );
        assert_eq!(c.tempo.len(), 2);
        // B's fade in now spans two bars from its intro anchor.
        assert_eq!(beats_of(b), vec![16.0, 17.0, 18.0, 20.0, 22.0, 24.0]);
        assert_eq!(
            b.tempo,
            vec![TempoNode {
                at: Beats(24.0),
                bpm: Bpm(140.0)
            }]
        );

        // Position one is the front, and a position past the end is refused.
        ok(&dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "set.dmx",
                "c.wav",
                "--bpm",
                "135",
                "--outro",
                "72",
                "--position",
                "1",
                "--preset",
                "beatmix",
            ],
        ));
        assert!(read_mix(&path).tracks[0].path.ends_with("c.wav"));
        assert_eq!(read_mix(&path).tracks.len(), 4);
        let message = fails(&dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "set.dmx",
                "c.wav",
                "--bpm",
                "135",
                "--position",
                "9",
            ],
        ));
        assert!(message.contains('9') && message.contains('4'), "{message}");
        assert_eq!(read_mix(&path).tracks.len(), 4);
        let out = dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "set.dmx",
                "c.wav",
                "--bpm",
                "135",
                "--position",
                "0",
            ],
        );
        assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    }

    #[test]
    fn presets_are_chosen_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = two_track_mix(dir.path());
        kicks_file(dir.path(), "c.wav", 135.0);
        ok(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "c.wav", "--bpm", "135", "--intro", "8", "--preset", "cut",
            ],
        ));
        let mix = read_mix(&path);
        let (b, c) = (&mix.tracks[1], &mix.tracks[2]);
        assert_eq!(beats_of(c), vec![7.75, 8.0]);
        assert_eq!(c.volume.nodes()[0].value, Decibels::SILENCE);
        assert_eq!(c.volume.nodes()[1].value, Decibels::UNITY);
        assert!(
            beats_of(b).contains(&79.75) && beats_of(b).contains(&80.0),
            "{:?}",
            beats_of(b)
        );

        kicks_file(dir.path(), "d.wav", 128.0);
        ok(&dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "set.dmx",
                "d.wav",
                "--bpm",
                "128",
                "--intro",
                "16",
                "--preset",
                "bass-swap",
                "--bars",
                "4",
            ],
        ));
        let mix = read_mix(&path);
        let (c, d) = (&mix.tracks[2], &mix.tracks[3]);
        // Four bars is sixteen beats, so the lows swap eight beats in.
        let lows = |t: &Track| -> Vec<f64> { t.eq.low.nodes().iter().map(|n| n.at.0).collect() };
        assert_eq!(lows(d), vec![23.0, 24.0]);
        let outro_c = c.anchors.outro.0;
        assert_eq!(lows(c), vec![outro_c + 7.0, outro_c + 8.0]);

        let message = fails(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "d.wav", "--bpm", "128", "--preset", "fade",
            ],
        ));
        assert!(message.contains("fade"), "{message}");
        for name in ["blend", "beatmix", "bass-swap", "cut"] {
            assert!(message.contains(name), "{message}");
        }
        assert_eq!(read_mix(&path).tracks.len(), 4);
    }

    #[test]
    fn a_failed_render_leaves_no_output_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = two_track_mix(dir.path());
        // Change the second file after it was added: its hash no longer matches.
        kicks_file(dir.path(), "b.wav", 141.0);
        let message = fails(&dermixen(dir.path(), &["render", "set.dmx", "out.wav"]));
        assert!(message.contains("b.wav"), "{message}");
        assert!(!dir.path().join("out.wav").exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("out"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert!(path.exists());
    }

    #[test]
    fn adding_a_file_stores_it_in_the_index_and_reads_it_back_from_there() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("library.sqlite");
        kicks_file(dir.path(), "a.wav", 130.0);
        ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
        ok(&dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "set.dmx",
                "a.wav",
                "--bpm",
                "130",
                "--library",
                index.to_str().unwrap(),
            ],
        ));
        // The record is in the library, and a query finds it.
        let out = dermixen(
            dir.path(),
            &[
                "library",
                "query",
                "--json",
                "--library",
                index.to_str().unwrap(),
            ],
        );
        ok(&out);
        let records: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        assert_eq!(records.as_array().unwrap().len(), 1);
        assert_eq!(
            records[0]["hash"],
            hash_file(&dir.path().join("a.wav")).unwrap().to_string()
        );
        // Anchors given on the command line replace the record's.
        ok(&dermixen(dir.path(), &["mix", "new", "other.dmx"]));
        ok(&dermixen(
            dir.path(),
            &[
                "mix",
                "add",
                "other.dmx",
                "a.wav",
                "--bpm",
                "130",
                "--intro",
                "4",
                "--outro",
                "40",
                "--library",
                index.to_str().unwrap(),
            ],
        ));
        let other = read_mix(&dir.path().join("other.dmx"));
        assert_eq!(other.tracks[0].anchors.intro, Beats(4.0));
        assert_eq!(other.tracks[0].anchors.outro, Beats(40.0));
    }

    #[cfg(feature = "aubio")]
    #[test]
    fn analyzed_anchors_follow_the_transition_length() {
        let dir = tempfile::tempdir().unwrap();
        // A minute of kicks at each tempo, so the analyzed anchors leave
        // room for an eight-bar transition and the shorter ones.
        for (name, bpm) in [("a.wav", 130.0), ("b.wav", 140.0)] {
            let audio = synth::kicks(Bpm(bpm), Seconds::ZERO, Seconds(60.0));
            write_wav(&dir.path().join(name), &audio, WavDepth::Int16).unwrap();
        }
        for (name, bars) in [("eight.dmx", "8"), ("four.dmx", "4"), ("two.dmx", "2")] {
            ok(&dermixen(dir.path(), &["mix", "new", name]));
            ok(&dermixen(
                dir.path(),
                &[
                    "mix", "add", name, "a.wav", "--preset", "beatmix", "--bars", bars,
                ],
            ));
            ok(&dermixen(
                dir.path(),
                &[
                    "mix", "add", name, "b.wav", "--preset", "beatmix", "--bars", bars,
                ],
            ));
        }
        let eight = read_mix(&dir.path().join("eight.dmx"));
        let four = read_mix(&dir.path().join("four.dmx"));
        let two = read_mix(&dir.path().join("two.dmx"));
        // The intro anchor comes from analysis and does not move; the outro
        // anchor moves four beats per bar so the overlap ends where analysis said.
        assert_eq!(eight.tracks[1].anchors.intro, four.tracks[1].anchors.intro);
        assert_eq!(
            four.tracks[0].anchors.outro,
            eight.tracks[0].anchors.outro + Beats(16.0)
        );
        assert_eq!(
            two.tracks[0].anchors.outro,
            eight.tracks[0].anchors.outro + Beats(24.0)
        );
        assert!(
            eight.tracks[0].anchors.intro.is_whole() && eight.tracks[0].anchors.outro.is_whole()
        );
        assert!(eight.tracks[0].anchors.outro > eight.tracks[0].anchors.intro);
    }

    #[test]
    fn the_blend_is_the_default_preset() {
        let dir = tempfile::tempdir().unwrap();
        // A minute of kicks at each tempo: beat zero on the first sample,
        // and the last sample of a.wav on its beat 130. The outro anchor
        // of a.wav is given as beat 112, the twenty-eight bars after its
        // intro anchor that the blend needs, which leaves an eighteen-beat
        // tail.
        for (name, bpm) in [("a.wav", 130.0), ("b.wav", 140.0)] {
            let audio = synth::kicks(Bpm(bpm), Seconds::ZERO, Seconds(60.0));
            write_wav(&dir.path().join(name), &audio, WavDepth::Int16).unwrap();
        }
        ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
        ok(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--intro", "0", "--outro", "112",
            ],
        ));
        ok(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "b.wav", "--bpm", "140", "--intro", "32",
            ],
        ));
        let mix = read_mix(&dir.path().join("set.dmx"));
        let (a, b) = (&mix.tracks[0], &mix.tracks[1]);
        // A eases from its outro anchor to -7 dB at its last sample, 18
        // beats later, with the middle nodes on whole beats, and is never
        // faded out.
        assert_eq!(beats_of(a), vec![112.0, 117.0, 121.0, 126.0, 130.0]);
        assert_eq!(a.volume.nodes()[0].value, Decibels::UNITY);
        assert_eq!(a.volume.nodes()[4].value, Decibels(-7.0));
        // B rises from silence eight bars before its intro anchor, is at
        // -12 dB on the anchor, and is full twenty-eight bars after it.
        assert_eq!(
            beats_of(b),
            vec![0.0, 8.0, 20.0, 32.0, 64.0, 96.0, 128.0, 144.0]
        );
        assert_eq!(b.volume.nodes()[0].value, Decibels::SILENCE);
        assert_eq!(b.volume.nodes()[3].value, Decibels(-12.0));
        assert_eq!(b.volume.nodes()[7].value, Decibels::UNITY);
        // The tempo ramps over the eight bars after the anchors, as a
        // beatmix's does.
        assert_eq!(
            a.tempo,
            vec![TempoNode {
                at: Beats(112.0),
                bpm: Bpm(130.0)
            }]
        );
        assert_eq!(
            b.tempo,
            vec![TempoNode {
                at: Beats(64.0),
                bpm: Bpm(140.0)
            }]
        );

        // A track without twenty-eight bars between its anchors cannot be
        // blended out of, and the refusal names the file and the way out.
        let dir = tempfile::tempdir().unwrap();
        let path = two_track_mix(dir.path());
        kicks_file(dir.path(), "c.wav", 135.0);
        let message = fails(&dermixen(
            dir.path(),
            &["mix", "add", "set.dmx", "c.wav", "--bpm", "135"],
        ));
        assert!(
            message.contains("b.wav") && message.contains("112") && message.contains("preset"),
            "{message}"
        );
        assert_eq!(read_mix(&path).tracks.len(), 2);
    }

    /// Kicks at each tempo in `dir`: `a.wav` at 130 BPM, `seconds` long,
    /// so its last sample is on beat `seconds * 130 / 60`, and `b.wav` at
    /// 140 BPM for ninety seconds, 210 beats, so that a blend into it at
    /// beat 32 ends its rise at beat 144, well inside its own anchors.
    /// Beat zero is on the first sample of each.
    fn minute_of_kicks(dir: &Path, seconds: f64) {
        for (name, bpm, length) in [("a.wav", 130.0, seconds), ("b.wav", 140.0, 90.0)] {
            let audio = synth::kicks(Bpm(bpm), Seconds::ZERO, Seconds(length));
            write_wav(&dir.join(name), &audio, WavDepth::Int16).unwrap();
        }
    }

    #[test]
    fn a_blend_out_of_a_track_anchored_on_its_last_beat_keeps_it_at_full_level() {
        // Without --outro, a track given with --bpm gets its outro anchor
        // on its last whole beat, beat 130 of a 60.3-second file whose last
        // sample is on beat 130.65. The blend out of it has no room for
        // middle nodes, so the track stays at full level to its anchor and
        // eases to -7 dB at its last sample.
        let dir = tempfile::tempdir().unwrap();
        minute_of_kicks(dir.path(), 60.3);
        ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
        ok(&dermixen(
            dir.path(),
            &["mix", "add", "set.dmx", "a.wav", "--bpm", "130"],
        ));
        ok(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "b.wav", "--bpm", "140", "--intro", "32",
            ],
        ));
        let a = &read_mix(&dir.path().join("set.dmx")).tracks[0];
        assert_eq!(a.anchors.outro, Beats(130.0));
        let nodes = a.volume.nodes();
        assert_eq!(nodes.len(), 2, "{:?}", beats_of(a));
        assert_eq!(nodes[0].at, Beats(130.0));
        assert_eq!(nodes[0].value, Decibels::UNITY);
        assert!((nodes[1].at.0 - 130.65).abs() < 1e-6, "{:?}", beats_of(a));
        assert_eq!(nodes[1].value, Decibels(-7.0));
        assert_eq!(a.volume.value_at(Beats(60.0)), Decibels::UNITY);
    }

    #[test]
    fn mix_move_anchor_takes_a_blends_rise_along() {
        // The blend's rise runs from eight bars before the intro anchor to
        // twenty-eight bars after it, and every one of its nodes moves with
        // the anchor, as the tempo node does.
        let dir = tempfile::tempdir().unwrap();
        minute_of_kicks(dir.path(), 60.0);
        ok(&dermixen(dir.path(), &["mix", "new", "set.dmx"]));
        ok(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--intro", "0", "--outro", "112",
            ],
        ));
        ok(&dermixen(
            dir.path(),
            &[
                "mix", "add", "set.dmx", "b.wav", "--bpm", "140", "--intro", "32",
            ],
        ));
        ok(&dermixen(
            dir.path(),
            &["mix", "move-anchor", "set.dmx", "2", "--intro", "40"],
        ));
        let b = &read_mix(&dir.path().join("set.dmx")).tracks[1];
        assert_eq!(b.anchors.intro, Beats(40.0));
        assert_eq!(
            beats_of(b),
            vec![8.0, 16.0, 28.0, 40.0, 72.0, 104.0, 136.0, 152.0]
        );
        assert_eq!(b.volume.nodes()[0].value, Decibels::SILENCE);
        assert_eq!(b.volume.nodes()[3].value, Decibels(-12.0));
        assert_eq!(b.volume.nodes()[7].value, Decibels::UNITY);
        assert_eq!(
            b.tempo,
            vec![TempoNode {
                at: Beats(72.0),
                bpm: Bpm(140.0)
            }]
        );
    }
}

/// Three kick tracks of two and a half minutes each, joined by the blend,
/// so the handover into the third track sits in the middle of the mix with
/// more than a minute of mix on either side of it. Each track keeps a gain
/// of zero rather than the leveling gain, since two kick tracks at that gain
/// add up past full scale where they overlap, and a WAV cannot hold that.
///
/// The second track enters at beat 300 of the first, about two minutes and
/// eighteen seconds in, and the third at beat 300 of the second, so the
/// third track's rise begins at its own beat zero, thirty-two beats before
/// its intro anchor, and the second track keeps playing for twenty seconds
/// after the anchors meet.
fn three_track_mix(dir: &Path) -> PathBuf {
    for (name, bpm) in [("a.wav", 130.0), ("b.wav", 140.0), ("c.wav", 130.0)] {
        let audio = synth::kicks(Bpm(bpm), Seconds::ZERO, Seconds(150.0));
        write_wav(&dir.join(name), &audio, WavDepth::Int16).unwrap();
    }
    ok(&dermixen(dir, &["mix", "new", "long.dmx"]));
    for (name, bpm, intro) in [
        ("a.wav", "130", "0"),
        ("b.wav", "140", "32"),
        ("c.wav", "130", "32"),
    ] {
        ok(&dermixen(
            dir,
            &[
                "mix",
                "add",
                "long.dmx",
                name,
                "--bpm",
                bpm,
                "--first-beat",
                "0",
                "--intro",
                intro,
                "--outro",
                "300",
                "--no-keylock",
                "--gain",
                "0",
            ],
        ));
    }
    dir.join("long.dmx")
}

#[test]
fn render_handover_writes_the_clip_around_one_handover() {
    let dir = tempfile::tempdir().unwrap();
    let path = three_track_mix(dir.path());
    let mix = read_mix(&path);

    // Where the handover into the third track falls, worked out from the
    // document the way `mix show` works out every track's start: the rise
    // begins thirty-two beats before the incoming track's intro anchor, the
    // anchors meet at that anchor, and the outgoing track ends at its last
    // sample. The clip runs from a minute before the rise to a minute after
    // the outgoing track ends, on the clock `mix show` reports.
    let timeline = mix.timeline().unwrap();
    let opening = timeline.start();
    let incoming = &mix.tracks[2];
    let placed_in = timeline.tracks[2];
    let placed_out = timeline.tracks[1];
    let rise = placed_in.mix_time_of(
        &timeline.curve,
        incoming.grid.time_of(incoming.anchors.intro - Beats(32.0)),
    ) - opening;
    let anchor = placed_in.mix_time_of(
        &timeline.curve,
        incoming.grid.time_of(incoming.anchors.intro),
    ) - opening;
    let outgoing_end = placed_out.end(&timeline.curve) - opening;
    let length = timeline.end() - opening;
    let from = Seconds((rise.0 - 60.0).max(0.0));
    let to = Seconds((outgoing_end.0 + 60.0).min(length.0));
    assert!(
        from.0 > 0.0 && to.0 < length.0,
        "the fixture's handover has a minute either side"
    );

    let out = dermixen(
        dir.path(),
        &[
            "render",
            "long.dmx",
            "clip.wav",
            "--handover",
            "3",
            "--json",
        ],
    );
    ok(&out);
    let value: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let handover = &value["handover"];
    let close = |key: &str, expected: f64| {
        let found = handover[key].as_f64().unwrap_or(f64::NAN);
        assert!(
            (found - expected).abs() < 0.002,
            "{key} is {found}, expected {expected}"
        );
    };
    assert_eq!(handover["into_track"].as_u64(), Some(3));
    close("from_seconds", from.0);
    close("for_seconds", to.0 - from.0);
    close("rise_seconds", rise.0 - from.0);
    close("anchor_seconds", anchor.0 - from.0);
    close("outgoing_end_seconds", outgoing_end.0 - from.0);

    // The clip holds exactly the frames a render of the whole mix holds at
    // those positions, since no track has keylock.
    let sources: Vec<Audio> = mix
        .tracks
        .iter()
        .map(|t: &Track| decode(&t.path).unwrap().audio)
        .collect();
    let mut resamplers = |_: &Track| -> Box<dyn TimeStretcher> { Box::new(Resampler::new()) };
    let whole = render(&mix, &sources, &mut resamplers).unwrap();
    let written = decode(&dir.path().join("clip.wav")).unwrap().audio;
    let start = from.to_samples().0 as usize;
    let count = (to.to_samples().0 - from.to_samples().0) as usize;
    assert!(
        (written.len().0 as i64 - count as i64).abs() <= 1,
        "the clip is {} frames, expected {count}",
        written.len().0
    );
    let worst = written
        .frames
        .iter()
        .zip(&whole.frames[start..])
        .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
        .fold(0.0, f32::max);
    assert!(worst <= 2.0 / 32_768.0, "the clip differs by up to {worst}");

    // Without --json the line a person reads names the file and says where
    // the rise, the anchors, and the outgoing track's end fall in the clip.
    let out = dermixen(
        dir.path(),
        &["render", "long.dmx", "clip2.wav", "--handover", "3"],
    );
    ok(&out);
    let text = stdout(&out);
    assert!(
        text.contains("clip2.wav") && text.contains("rise") && text.contains("anchor"),
        "{text}"
    );
}

#[test]
fn render_handover_refuses_the_first_track_a_missing_track_and_a_span_flag() {
    let dir = tempfile::tempdir().unwrap();
    three_track_mix(dir.path());

    let message = fails(&dermixen(
        dir.path(),
        &["render", "long.dmx", "clip.wav", "--handover", "1"],
    ));
    assert!(
        message.contains("track 1") && message.contains("before"),
        "{message}"
    );
    let message = fails(&dermixen(
        dir.path(),
        &["render", "long.dmx", "clip.wav", "--handover", "4"],
    ));
    assert!(
        message.contains("no track 4") && message.contains("3 tracks"),
        "{message}"
    );
    let out = dermixen(
        dir.path(),
        &[
            "render",
            "long.dmx",
            "clip.wav",
            "--handover",
            "3",
            "--from",
            "10",
        ],
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "--handover and --from together are refused"
    );
    assert!(!dir.path().join("clip.wav").exists());
}

#[test]
fn mix_set_gain_writes_the_gain_and_prints_the_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());
    let before = read_mix(&path);

    let out = dermixen(dir.path(), &["mix", "set-gain", "set.dmx", "2", "-2.5"]);
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("-2.5 dB") && text.contains("b.wav"), "{text}");
    let mix = read_mix(&path);
    assert_eq!(mix.tracks[1].gain.0, -2.5);
    assert_eq!(mix.tracks[0].gain.0, before.tracks[0].gain.0);
    assert_eq!(mix.tracks[1].anchors, before.tracks[1].anchors);
    assert_eq!(mix.tracks[1].volume, before.tracks[1].volume);

    ok(&dermixen(
        dir.path(),
        &["mix", "set-gain", "set.dmx", "1", "1.5"],
    ));
    assert_eq!(read_mix(&path).tracks[0].gain.0, 1.5);

    let out = dermixen(dir.path(), &["mix", "--help"]);
    ok(&out);
    assert!(stdout(&out).contains("set-gain"), "{}", stdout(&out));
}

#[test]
fn mix_set_gain_refuses_a_bad_track_and_a_value_that_is_not_a_finite_number() {
    let dir = tempfile::tempdir().unwrap();
    let path = two_track_mix(dir.path());
    let before = std::fs::read_to_string(&path).unwrap();

    let message = fails(&dermixen(
        dir.path(),
        &["mix", "set-gain", "set.dmx", "3", "-1"],
    ));
    assert!(
        message.contains("no track 3") && message.contains("2 tracks"),
        "{message}"
    );
    let message = fails(&dermixen(
        dir.path(),
        &["mix", "set-gain", "set.dmx", "2", "inf"],
    ));
    assert!(message.contains("finite"), "{message}");
    let out = dermixen(dir.path(), &["mix", "set-gain", "set.dmx", "2", "loud"]);
    assert_eq!(out.status.code(), Some(2));

    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}
