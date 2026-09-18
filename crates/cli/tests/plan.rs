//! Acceptance tests for `mix plan`, which predicts the timeline a playlist
//! would build without writing a mix document. A coder agent makes these pass
//! without editing them.
//!
//! The command's one promise is that its prediction is the mix `mix new` and
//! then `mix add` for each path, with no options, would build, laid out as
//! `mix show` lays a document out. The first test builds that mix and
//! compares every number. The rest cover the three warnings, the refusals,
//! and the rule that the plan reads the library and never writes it.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{dermixen, fails, json, kicks_file, ok, stderr, stdout};
use dermixen_analysis::{Camelot, Extent, Loudness};
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, Decibels, Lufs, Samples};
use dermixen_library::{Index, KeyRecord, Metadata, MetadataSource, Release, TrackRecord};
use dermixen_media::hash_file;

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

/// How many fixture files the tests have written so far, which gives each
/// file its own bytes.
static FILES_WRITTEN: AtomicUsize = AtomicUsize::new(0);

/// A track as the library stores one, for a real file in `dir`, so that
/// `mix plan` finds the record by the file's hash as `mix add` does.
///
/// The record says the track is seven minutes at `bpm` with its first beat
/// on its first sample, its anchors at beats 32 and 900, which is long enough
/// for the blend's twenty-eight bars, its key `camelot` scored
/// `key_confidence`, and its loudness twelve LUFS below full scale. The file
/// itself is twenty seconds of kicks, with its first kick a little further
/// in for every file written before it, because two kick files at one tempo
/// and one offset are the same bytes, and the library keeps one record per
/// hash. Nothing under test decodes the file, because `mix plan` and
/// `mix add` both read the record.
fn track(
    dir: &Path,
    name: &str,
    artist: &str,
    title: &str,
    bpm: f64,
    camelot: &str,
    key_confidence: f64,
) -> TrackRecord {
    let written = FILES_WRITTEN.fetch_add(1, Ordering::Relaxed);
    let first_kick = 0.01 * (written + 1) as f64;
    let path = kicks_file(dir, name, bpm, first_kick)
        .canonicalize()
        .unwrap();
    let camelot: Camelot = camelot.parse().unwrap();
    TrackRecord {
        hash: hash_file(&path).unwrap(),
        path,
        length: Samples(420 * 44_100),
        grid: BeatGrid {
            first_beat: Samples(0),
            bpm: Bpm(bpm),
        },
        grid_confidence: 0.9,
        grid_analyzer: "pulse".to_owned(),
        key: Some(KeyRecord {
            key: camelot.key(),
            camelot,
            confidence: key_confidence,
            analyzer: "keyfinder".to_owned(),
        }),
        extent: Extent {
            begins: Samples(0),
            ends: Samples(420 * 44_100),
        },
        anchors: Anchors {
            intro: Beats(32.0),
            outro: Beats(900.0),
        },
        anchor_confidence: 0.8,
        anchor_analyzer: "kick".to_owned(),
        metadata: Metadata {
            artist: Some(artist.to_owned()),
            title: Some(title.to_owned()),
            year: Some(1996),
            year_is_approximate: false,
            source: MetadataSource::Tags,
        },
        phrases: None,
        loudness: Some(Loudness {
            integrated: Lufs(-12.0),
            true_peak: Decibels(-1.0),
        }),
        release: Release::default(),
    }
}

/// Stores the records in `library.sqlite` in `dir`, which is the library file
/// every command these tests run reads.
fn library(dir: &Path, records: &[&TrackRecord]) {
    let mut index = Index::open(&dir.join("library.sqlite")).unwrap();
    for record in records {
        index.upsert(record).unwrap();
    }
}

/// Writes a playlist file naming the records' files in order, one per line,
/// with a blank line among them, which the command skips.
fn playlist(dir: &Path, name: &str, records: &[&TrackRecord]) -> PathBuf {
    let mut text = String::new();
    for (position, record) in records.iter().enumerate() {
        if position == 1 {
            text.push('\n');
        }
        text.push_str(&record.path.display().to_string());
        text.push('\n');
    }
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// The `M:SS.s` length on the last line `mix show` prints, which has the
/// form `3 tracks, 12:34.5 long, 33345000 samples`.
fn shown_length(show_text: &str) -> String {
    let last = show_text.trim_end().lines().last().unwrap();
    let mut parts = last.split(", ");
    parts.next();
    parts
        .next()
        .unwrap()
        .strip_suffix(" long")
        .unwrap()
        .to_owned()
}

/// The names in the `warnings` list of a plan's JSON output.
fn listed_warnings(planned: &serde_json::Value) -> Vec<&str> {
    planned["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning.as_str().unwrap())
        .collect()
}

#[test]
fn plan_predicts_the_timeline_mix_add_builds() {
    let dir = tempfile::tempdir().unwrap();
    let alpha = track(
        dir.path(),
        "01 Alpha.wav",
        "Etnica",
        "Alpha",
        140.0,
        "8A",
        0.8,
    );
    let beta = track(dir.path(), "02 Beta.wav", "Prana", "Beta", 141.0, "9A", 0.8);
    let gamma = track(
        dir.path(),
        "03 Gamma.wav",
        "Doof",
        "Gamma",
        142.0,
        "10A",
        0.8,
    );
    library(dir.path(), &[&alpha, &beta, &gamma]);
    let list = playlist(dir.path(), "playlist.txt", &[&alpha, &beta, &gamma]);

    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--json", "--max-step", "1"],
    );
    let planned = json(&out);
    fits(&planned, "mix_plan");
    assert!(
        planned["playlist"]
            .as_str()
            .unwrap()
            .ends_with("playlist.txt"),
        "{planned}"
    );
    assert_eq!(planned["keys_checked"], true);
    assert_eq!(listed_warnings(&planned), Vec::<&str>::new());
    assert!(
        !dir.path().join("set.dmx").exists(),
        "the plan writes no document"
    );

    // The document `mix add` builds from the same playlist, one add at a time with no
    // options, laid out by mix show.
    ok(&dermixen(dir.path(), &[], &["mix", "new", "set.dmx"]));
    for record in [&alpha, &beta, &gamma] {
        ok(&dermixen(
            dir.path(),
            &[],
            &["mix", "add", "set.dmx", &record.path.display().to_string()],
        ));
    }
    let shown = json(&dermixen(
        dir.path(),
        &[],
        &["mix", "show", "set.dmx", "--json"],
    ));

    assert_eq!(planned["length_samples"], shown["length_samples"]);
    assert_eq!(planned["length_seconds"], shown["length_seconds"]);
    let planned_tracks = planned["tracks"].as_array().unwrap();
    let shown_tracks = shown["tracks"].as_array().unwrap();
    assert_eq!(planned_tracks.len(), 3);
    for (planned, shown) in planned_tracks.iter().zip(shown_tracks) {
        for field in [
            "position",
            "path",
            "hash",
            "length_samples",
            "grid",
            "anchors",
            "keylock",
            "gain_db",
            "start_seconds",
            "end_seconds",
            "tempo_nodes",
            "volume_nodes",
        ] {
            assert_eq!(planned[field], shown[field], "{field} of {planned}");
        }
    }
    // The leveling gain for a track twelve LUFS below full scale with a true
    // peak one decibel below it is minus two decibels, so a plan that skipped
    // leveling would have failed the comparison above.
    assert_eq!(planned_tracks[0]["gain_db"], -2.0);
    // What the plan adds to the layout: the names, the key, and the step.
    assert_eq!(planned_tracks[0]["artist"], "Etnica");
    assert_eq!(planned_tracks[0]["title"], "Alpha");
    assert_eq!(planned_tracks[0]["camelot"], "8A");
    assert_eq!(planned_tracks[0]["tempo_step_bpm"], serde_json::Value::Null);
    assert_eq!(planned_tracks[1]["camelot"], "9A");
    assert_eq!(planned_tracks[1]["tempo_step_bpm"], 1.0);
    assert_eq!(planned_tracks[2]["camelot"], "10A");
    assert_eq!(planned_tracks[2]["tempo_step_bpm"], 1.0);

    // The text says the same, one line per track and a last line with the
    // count and the length mix show gives.
    let show_text = stdout(&dermixen(dir.path(), &[], &["mix", "show", "set.dmx"]));
    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "plan", &list.display().to_string()],
    );
    ok(&out);
    let text = stdout(&out);
    for wanted in [
        "Etnica - Alpha",
        "Prana - Beta",
        "Doof - Gamma",
        "8A",
        "9A",
        "10A",
        "enters",
        "+1.00",
        "opening at 140.00 bpm and closing at 142.00 bpm",
    ] {
        assert!(text.contains(wanted), "no {wanted:?} in:\n{text}");
    }
    let length = shown_length(&show_text);
    assert!(
        text.contains(&format!("3 tracks, {length} long")),
        "no {length:?} in:\n{text}"
    );
    assert!(stderr(&out).is_empty(), "{}", stderr(&out));
}

#[test]
fn plan_lays_out_one_track_as_mix_show_does_and_resolves_a_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let alpha = track(
        dir.path(),
        "01 Alpha.wav",
        "Etnica",
        "Alpha",
        140.0,
        "8A",
        0.8,
    );
    library(dir.path(), &[&alpha]);
    // A relative path in a playlist is resolved against the folder the
    // command runs in, as every other path a command takes is.
    std::fs::write(dir.path().join("playlist.txt"), "01 Alpha.wav\n").unwrap();

    let planned = json(&dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--json"],
    ));
    fits(&planned, "mix_plan");
    ok(&dermixen(dir.path(), &[], &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir.path(),
        &[],
        &["mix", "add", "set.dmx", "01 Alpha.wav"],
    ));
    let shown = json(&dermixen(
        dir.path(),
        &[],
        &["mix", "show", "set.dmx", "--json"],
    ));
    assert_eq!(planned["length_samples"], shown["length_samples"]);
    assert_eq!(planned["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(planned["tracks"][0]["path"], shown["tracks"][0]["path"]);
    assert_eq!(
        planned["tracks"][0]["path"],
        alpha.path.display().to_string()
    );
    assert_eq!(
        planned["tracks"][0]["end_seconds"],
        shown["tracks"][0]["end_seconds"]
    );
    assert_eq!(
        planned["tracks"][0]["tempo_step_bpm"],
        serde_json::Value::Null
    );
    assert_eq!(listed_warnings(&planned), Vec::<&str>::new());

    let out = dermixen(dir.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    let show_text = stdout(&dermixen(dir.path(), &[], &["mix", "show", "set.dmx"]));
    let length = shown_length(&show_text);
    assert!(
        stdout(&out).contains(&format!("1 track, {length} long")),
        "{}",
        stdout(&out)
    );
    assert!(
        stdout(&out).contains("opening at 140.00 bpm and closing at 140.00 bpm"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn plan_warns_when_the_tempo_steps_further_than_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let alpha = track(
        dir.path(),
        "01 Alpha.wav",
        "Etnica",
        "Alpha",
        140.0,
        "8A",
        0.8,
    );
    let beta = track(dir.path(), "02 Beta.wav", "Prana", "Beta", 142.0, "9A", 0.8);
    library(dir.path(), &[&alpha, &beta]);
    playlist(dir.path(), "playlist.txt", &[&alpha, &beta]);

    // A warning is not a failure: the timeline is printed, the exit code is
    // zero, and the warning is on standard error.
    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--max-step", "1"],
    );
    ok(&out);
    assert!(stdout(&out).contains("2 tracks"), "{}", stdout(&out));
    assert!(
        stderr(&out).contains("transition 1 moves the tempo by 2.00 bpm, over the 1.00 allowed"),
        "{}",
        stderr(&out)
    );

    // One beat per minute is the limit when none is given.
    let out = dermixen(dir.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    assert!(
        stderr(&out).contains("moves the tempo by 2.00 bpm"),
        "{}",
        stderr(&out)
    );

    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--max-step", "2"],
    );
    ok(&out);
    assert!(
        !stderr(&out).contains("moves the tempo"),
        "{}",
        stderr(&out)
    );

    let planned = json(&dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--json"],
    ));
    fits(&planned, "mix_plan");
    assert_eq!(
        listed_warnings(&planned),
        vec!["transition 1 moves the tempo by 2.00 bpm, over the 1.00 allowed"]
    );
    assert_eq!(planned["tracks"][1]["tempo_step_bpm"], 2.0);

    // A limit that is not a number from zero up is refused with the option
    // named, since a limit of NaN would switch the check off without a word.
    for bad in ["nan", "-1"] {
        let message = fails(&dermixen(
            dir.path(),
            &[],
            &["mix", "plan", "playlist.txt", "--max-step", bad],
        ));
        assert!(message.contains("--max-step"), "{bad}: {message}");
        assert!(message.contains(bad), "{bad}: {message}");
    }
}

#[test]
fn plan_checks_keys_only_when_the_library_has_real_keys() {
    // Every key in this library scored near zero, so the codes are noise and
    // a clash between them is not a warning. The third record is in the
    // library and not in the playlist, and the median is taken over the
    // library, so with three confidences the median is the middle one.
    let noise = tempfile::tempdir().unwrap();
    let alpha = track(
        noise.path(),
        "01 Alpha.wav",
        "Koxbox",
        "Alpha",
        140.0,
        "8A",
        0.02,
    );
    let beta = track(
        noise.path(),
        "02 Beta.wav",
        "Doof",
        "Beta",
        140.0,
        "3B",
        0.01,
    );
    let gamma = track(
        noise.path(),
        "03 Gamma.wav",
        "Prana",
        "Gamma",
        140.0,
        "5A",
        0.03,
    );
    library(noise.path(), &[&alpha, &beta, &gamma]);
    playlist(noise.path(), "playlist.txt", &[&alpha, &beta]);
    let out = dermixen(noise.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    assert!(
        stderr(&out)
            .contains("keys are not checked: the median key confidence in the library is 0.02"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("keys that do not fit"),
        "{}",
        stderr(&out)
    );
    let planned = json(&dermixen(
        noise.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--json"],
    ));
    fits(&planned, "mix_plan");
    assert_eq!(planned["keys_checked"], false);
    assert_eq!(listed_warnings(&planned), Vec::<&str>::new());

    // The same tracks with keys the analyzer was sure of.
    let real = tempfile::tempdir().unwrap();
    let alpha = track(
        real.path(),
        "01 Alpha.wav",
        "Koxbox",
        "Alpha",
        140.0,
        "8A",
        0.6,
    );
    let beta = track(real.path(), "02 Beta.wav", "Doof", "Beta", 140.0, "3B", 0.7);
    let gamma = track(
        real.path(),
        "03 Gamma.wav",
        "Prana",
        "Gamma",
        140.0,
        "5A",
        0.8,
    );
    library(real.path(), &[&alpha, &beta, &gamma]);
    playlist(real.path(), "playlist.txt", &[&alpha, &beta]);
    let out = dermixen(real.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    assert!(
        stderr(&out).contains("transition 1 goes from 8A to 3B, which are keys that do not fit"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("keys are not checked"),
        "{}",
        stderr(&out)
    );
    let planned = json(&dermixen(
        real.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--json"],
    ));
    assert_eq!(planned["keys_checked"], true);
    assert_eq!(
        listed_warnings(&planned),
        vec!["transition 1 goes from 8A to 3B, which are keys that do not fit"]
    );

    // Whether the analyzer found keys is a fact about the library, not about
    // the two tracks in the playlist: two confident keys in a library of
    // noise are still not checked, and a record with no key does not count.
    let mostly_noise = tempfile::tempdir().unwrap();
    let alpha = track(
        mostly_noise.path(),
        "01 Alpha.wav",
        "Koxbox",
        "Alpha",
        140.0,
        "8A",
        0.6,
    );
    let beta = track(
        mostly_noise.path(),
        "02 Beta.wav",
        "Doof",
        "Beta",
        140.0,
        "3B",
        0.7,
    );
    let mut keyless = track(
        mostly_noise.path(),
        "03 Keyless.wav",
        "Prana",
        "Keyless",
        140.0,
        "5A",
        0.9,
    );
    keyless.key = None;
    let quiet: Vec<TrackRecord> = (0..5)
        .map(|index| {
            track(
                mostly_noise.path(),
                &format!("1{index} Quiet.wav"),
                "Etnica",
                &format!("Quiet {index}"),
                140.0,
                "5A",
                0.01,
            )
        })
        .collect();
    let mut records = vec![&alpha, &beta, &keyless];
    records.extend(quiet.iter());
    library(mostly_noise.path(), &records);
    playlist(mostly_noise.path(), "playlist.txt", &[&alpha, &beta]);
    let out = dermixen(mostly_noise.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    assert!(
        stderr(&out)
            .contains("keys are not checked: the median key confidence in the library is 0.01"),
        "{}",
        stderr(&out)
    );
    assert!(
        !stderr(&out).contains("keys that do not fit"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn plan_warns_when_an_artist_repeats_counting_collaborations_and_remixes() {
    let dir = tempfile::tempdir().unwrap();
    let one = track(
        dir.path(),
        "a1.wav",
        "Total Eclipse",
        "Blade Runner",
        140.0,
        "8A",
        0.8,
    );
    let two = track(
        dir.path(),
        "a2.wav",
        "MFG",
        "New Horizon (Total Eclipse remix)",
        140.0,
        "8A",
        0.8,
    );
    let three = track(
        dir.path(),
        "a3.wav",
        "MFG & Astral Projection",
        "The Sleeper Must Awake",
        140.0,
        "8A",
        0.8,
    );
    let four = track(
        dir.path(),
        "a4.wav",
        "Total Eclipse",
        "Le Lotus Bleu",
        140.0,
        "8A",
        0.8,
    );
    library(dir.path(), &[&one, &two, &three, &four]);
    playlist(dir.path(), "playlist.txt", &[&one, &two, &three, &four]);

    let out = dermixen(dir.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    assert!(stdout(&out).contains("4 tracks"), "{}", stdout(&out));
    let warnings = stderr(&out);
    assert!(
        warnings.contains("Total Eclipse appears on tracks 1, 2, and 4"),
        "{warnings}"
    );
    assert!(
        warnings.contains("MFG appears on tracks 2 and 3"),
        "{warnings}"
    );
    assert!(
        !warnings.contains("Astral Projection appears"),
        "{warnings}"
    );

    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--allow-repeats"],
    );
    ok(&out);
    assert!(
        !stderr(&out).contains("appears on tracks"),
        "{}",
        stderr(&out)
    );

    let planned = json(&dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--json"],
    ));
    fits(&planned, "mix_plan");
    assert_eq!(
        listed_warnings(&planned),
        vec![
            "Total Eclipse appears on tracks 1, 2, and 4",
            "MFG appears on tracks 2 and 3"
        ]
    );
}

#[test]
fn plan_reads_credits_in_any_case_and_a_remixer_with_words_after_the_remix() {
    let dir = tempfile::tempdir().unwrap();
    let one = track(
        dir.path(),
        "b1.wav",
        "Astral Projection Vs. MFG",
        "Sleeper",
        140.0,
        "8A",
        0.8,
    );
    let two = track(
        dir.path(),
        "b2.wav",
        "Cosmosis",
        "Mahadeva (Man With No Name Remix Edit)",
        140.0,
        "8A",
        0.8,
    );
    let three = track(
        dir.path(),
        "b3.wav",
        "man with no name",
        "Floor Essence",
        140.0,
        "8A",
        0.8,
    );
    let four = track(
        dir.path(),
        "b4.wav",
        "Mfg",
        "The Prophecy",
        140.0,
        "8A",
        0.8,
    );
    library(dir.path(), &[&one, &two, &three, &four]);
    playlist(dir.path(), "playlist.txt", &[&one, &two, &three, &four]);

    let out = dermixen(dir.path(), &[], &["mix", "plan", "playlist.txt"]);
    ok(&out);
    let warnings = stderr(&out);
    // The spelling reported is the one in the earliest track's record.
    assert!(
        warnings.contains("MFG appears on tracks 1 and 4"),
        "{warnings}"
    );
    assert!(
        warnings.contains("Man With No Name appears on tracks 2 and 3"),
        "{warnings}"
    );
    assert!(
        !warnings.contains("Astral Projection appears"),
        "{warnings}"
    );
    assert!(!warnings.contains("Cosmosis appears"), "{warnings}");
}

#[test]
fn plan_repeats_the_warning_mix_add_gives_a_track_with_no_loudness() {
    let dir = tempfile::tempdir().unwrap();
    let alpha = track(
        dir.path(),
        "01 Alpha.wav",
        "Etnica",
        "Alpha",
        140.0,
        "8A",
        0.8,
    );
    let mut beta = track(dir.path(), "02 Beta.wav", "Prana", "Beta", 140.0, "9A", 0.8);
    beta.loudness = None;
    library(dir.path(), &[&alpha, &beta]);
    playlist(dir.path(), "playlist.txt", &[&alpha, &beta]);

    // The gain is the one mix add would write, and so is the warning, since
    // the plan gives each track its gain the way mix add does. The warning
    // is not one of the plan's own, so it is not in the JSON list.
    let out = dermixen(dir.path(), &[], &["mix", "plan", "playlist.txt", "--json"]);
    let planned = json(&out);
    fits(&planned, "mix_plan");
    assert_eq!(planned["tracks"][0]["gain_db"], -2.0);
    assert_eq!(planned["tracks"][1]["gain_db"], 0.0);
    assert!(
        stderr(&out).contains("the library has no loudness for"),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("02 Beta.wav"), "{}", stderr(&out));
    assert_eq!(listed_warnings(&planned), Vec::<&str>::new());
}

#[test]
fn plan_refuses_a_file_the_library_does_not_have_and_a_playlist_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let alpha = track(
        dir.path(),
        "01 Alpha.wav",
        "Etnica",
        "Alpha",
        140.0,
        "8A",
        0.8,
    );
    library(dir.path(), &[&alpha]);
    let stranger = kicks_file(dir.path(), "stranger.wav", 140.0, 0.0);

    // A file that is on disk but not in the library is refused rather than
    // analyzed, because a plan runs in a second and never writes the library.
    std::fs::write(
        dir.path().join("unknown.txt"),
        format!("{}\n{}\n", alpha.path.display(), stranger.display()),
    )
    .unwrap();
    let message = fails(&dermixen(dir.path(), &[], &["mix", "plan", "unknown.txt"]));
    assert!(message.contains("stranger.wav"), "{message}");
    assert!(message.contains("library scan"), "{message}");
    let index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    assert_eq!(index.len().unwrap(), 1, "the plan wrote to the library");
    drop(index);

    // A file that is not on disk at all is named.
    std::fs::write(
        dir.path().join("ghost.txt"),
        format!(
            "{}\n{}\n",
            alpha.path.display(),
            dir.path().join("ghost.wav").display()
        ),
    )
    .unwrap();
    let message = fails(&dermixen(dir.path(), &[], &["mix", "plan", "ghost.txt"]));
    assert!(message.contains("ghost.wav"), "{message}");

    // A playlist that is not there is named and said to be unreadable, and
    // one that names nothing is said to name nothing.
    let message = fails(&dermixen(dir.path(), &[], &["mix", "plan", "missing.txt"]));
    assert!(message.contains("missing.txt"), "{message}");
    assert!(message.contains("cannot read"), "{message}");
    std::fs::write(dir.path().join("empty.txt"), "\n\n").unwrap();
    let message = fails(&dermixen(dir.path(), &[], &["mix", "plan", "empty.txt"]));
    assert!(message.contains("empty.txt"), "{message}");
    assert!(message.contains("names no tracks"), "{message}");

    // Under --json a failure is still one line on standard error and nothing
    // on standard output.
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "unknown.txt", "--json"],
    ));
    assert!(message.starts_with("error:"), "{message}");
}

#[test]
fn plan_refuses_a_track_the_blend_does_not_fit_as_mix_add_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut short = track(
        dir.path(),
        "01 Short.wav",
        "Etnica",
        "Short",
        140.0,
        "8A",
        0.8,
    );
    short.anchors = Anchors {
        intro: Beats(32.0),
        outro: Beats(100.0),
    };
    let beta = track(dir.path(), "02 Beta.wav", "Prana", "Beta", 140.0, "8A", 0.8);
    library(dir.path(), &[&short, &beta]);
    playlist(dir.path(), "playlist.txt", &[&short, &beta]);

    let message = fails(&dermixen(dir.path(), &[], &["mix", "plan", "playlist.txt"]));
    assert!(message.contains("01 Short.wav"), "{message}");
    assert!(message.contains("68 beats"), "{message}");
    assert!(message.contains("112 beats"), "{message}");

    // The same words mix add would use, since it is the same rule.
    ok(&dermixen(dir.path(), &[], &["mix", "new", "set.dmx"]));
    ok(&dermixen(
        dir.path(),
        &[],
        &["mix", "add", "set.dmx", &short.path.display().to_string()],
    ));
    let added = fails(&dermixen(
        dir.path(),
        &[],
        &["mix", "add", "set.dmx", &beta.path.display().to_string()],
    ));
    assert_eq!(message, added);
}

#[test]
fn plan_refuses_a_library_file_that_is_not_there_and_creates_none() {
    // A plan writes nothing, and that includes the library file itself: a
    // path that names no file is refused, naming the path and the command
    // that creates a library, and neither the file nor its folder appears.
    let dir = tempfile::tempdir().unwrap();
    let alpha = track(
        dir.path(),
        "01 Alpha.wav",
        "Etnica",
        "Alpha",
        140.0,
        "8A",
        0.8,
    );
    std::fs::write(
        dir.path().join("playlist.txt"),
        format!("{}\n", alpha.path.display()),
    )
    .unwrap();
    let missing = dir.path().join("nowhere").join("library.sqlite");
    let missing_text = missing.display().to_string();

    let message = fails(&dermixen(
        dir.path(),
        &[("DERMIXEN_LIBRARY_FILE", &missing_text)],
        &["mix", "plan", "playlist.txt"],
    ));
    assert!(message.contains(&missing_text), "{message}");
    assert!(message.contains("library scan"), "{message}");
    assert!(!missing.exists(), "the plan created a library file");
    assert!(
        !missing.parent().unwrap().exists(),
        "the plan created the library's folder"
    );

    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["mix", "plan", "playlist.txt", "--library", &missing_text],
    ));
    assert!(message.contains(&missing_text), "{message}");
    assert!(!missing.exists(), "the plan created a library file");
}
