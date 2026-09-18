//! Acceptance tests for the `library` commands as a person types them. A
//! coder makes these pass without editing them. They need a beat tracker,
//! so they compile only with the `aubio` feature.

#![cfg(feature = "aubio")]

mod common;

use std::path::Path;

use common::{dermixen, fails, json, kicks_file, ok, stderr, stdout};
use dermixen_analysis::Extent;
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, ContentHash, Samples};
use dermixen_library::{Index, Metadata, MetadataSource, Release, TrackRecord};

/// Two tracks in the usual folder layout, a copy of one of them, and a mix in
/// the folder that scans leave out.
fn a_library(root: &Path) {
    kicks_file(
        root,
        "artist/Etnica - One [BF001]/01 Etnica - Alpha.wav",
        130.0,
        0.5,
    );
    kicks_file(
        root,
        "comp/VA - Two [GV002]/02 Prana - Beta.wav",
        140.0,
        0.5,
    );
    std::fs::copy(
        root.join("artist/Etnica - One [BF001]/01 Etnica - Alpha.wav"),
        root.join("comp/VA - Two [GV002]/03 Etnica - Alpha.wav"),
    )
    .unwrap();
    kicks_file(root, "mixes/set.wav", 135.0, 0.0);
}

#[test]
fn a_scan_reports_what_it_did_and_a_second_scan_analyzes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    a_library(&dir.path().join("music"));
    let out = dermixen(
        dir.path(),
        &[],
        &["library", "scan", "music", "--exclude", "mixes"],
    );
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("added 2"), "{text}");
    assert!(
        text.contains("duplicate") && text.contains("03 Etnica - Alpha.wav"),
        "{text}"
    );
    // Progress went to standard error, one line per file.
    let progress = stderr(&out);
    assert!(progress.contains("01 Etnica - Alpha.wav"), "{progress}");
    assert!(dir.path().join("library.sqlite").exists());

    let out = dermixen(
        dir.path(),
        &[],
        &["library", "scan", "music", "--exclude", "mixes"],
    );
    ok(&out);
    let text = stdout(&out);
    assert!(
        text.contains("added 0") && text.contains("unchanged 2"),
        "{text}"
    );

    // Without the exclusion the mix is scanned too.
    let out = dermixen(dir.path(), &[], &["library", "scan", "music", "--json"]);
    let value = json(&out);
    assert_eq!(value["added"], 1);
    assert_eq!(value["unchanged"], 2);
}

#[test]
fn the_library_file_is_chosen_by_option_then_environment_then_default() {
    let dir = tempfile::tempdir().unwrap();
    a_library(&dir.path().join("music"));
    let by_option = dir.path().join("by-option.sqlite");
    let by_env = dir.path().join("by-env.sqlite");
    ok(&dermixen(
        dir.path(),
        &[("DERMIXEN_LIBRARY_FILE", by_env.to_str().unwrap())],
        &[
            "library",
            "scan",
            "music",
            "--exclude",
            "mixes",
            "--library",
            by_option.to_str().unwrap(),
        ],
    ));
    assert!(by_option.exists());
    assert!(!by_env.exists());
    ok(&dermixen(
        dir.path(),
        &[("DERMIXEN_LIBRARY_FILE", by_env.to_str().unwrap())],
        &["library", "scan", "music", "--exclude", "mixes"],
    ));
    assert!(by_env.exists());
    // A library file that does not exist yet is empty rather than an error.
    let out = dermixen(
        dir.path(),
        &[(
            "DERMIXEN_LIBRARY_FILE",
            dir.path().join("fresh.sqlite").to_str().unwrap(),
        )],
        &["library", "query", "--json"],
    );
    assert_eq!(json(&out), serde_json::json!([]));
}

#[test]
fn queries_narrow_the_listing_and_bad_conditions_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    a_library(&dir.path().join("music"));
    ok(&dermixen(
        dir.path(),
        &[],
        &["library", "scan", "music", "--exclude", "mixes"],
    ));

    let out = dermixen(dir.path(), &[], &["library", "query"]);
    ok(&out);
    let text = stdout(&out);
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.contains("Etnica") && text.contains("Prana"), "{text}");
    // Names guessed from the file name are marked as guesses.
    assert!(text.contains('?'), "{text}");

    let paths = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["library", "query", "--json"];
        all.extend_from_slice(args);
        let value = json(&dermixen(dir.path(), &[], &all));
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["path"].as_str().unwrap().to_owned())
            .collect()
    };
    let only = |found: Vec<String>, name: &str| {
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].ends_with(name), "{found:?}");
    };
    only(paths(&["--bpm", "125-135"]), "01 Etnica - Alpha.wav");
    only(paths(&["--bpm", "138-142"]), "02 Prana - Beta.wav");
    only(paths(&["--under", "music/comp"]), "02 Prana - Beta.wav");
    only(paths(&["--artist", "prana"]), "02 Prana - Beta.wav");
    only(paths(&["--title", "ALPHA"]), "01 Etnica - Alpha.wav");
    assert!(paths(&["--year", "1996"]).is_empty());
    assert!(paths(&["--key", "8A"]).is_empty() || paths(&["--key", "8A"]).len() <= 2);
    assert_eq!(paths(&["--bpm", "125-142", "--under", "music"]).len(), 2);

    for (option, value) in [
        ("--bpm", "fast"),
        ("--year", "199x"),
        ("--key", "13A"),
        ("--compatible-with", "8C"),
    ] {
        let message = fails(&dermixen(
            dir.path(),
            &[],
            &["library", "query", option, value],
        ));
        assert!(message.contains(option), "{message}");
    }
}

#[test]
fn find_ranks_matches_and_prints_scores() {
    let dir = tempfile::tempdir().unwrap();
    a_library(&dir.path().join("music"));
    ok(&dermixen(
        dir.path(),
        &[],
        &["library", "scan", "music", "--exclude", "mixes"],
    ));
    let out = dermixen(dir.path(), &[], &["library", "find", "Prana - Beta"]);
    ok(&out);
    let text = stdout(&out);
    let first = text.lines().next().unwrap();
    assert!(first.starts_with("1.00"), "{text}");
    assert!(first.ends_with("02 Prana - Beta.wav"), "{text}");

    let out = dermixen(
        dir.path(),
        &[],
        &["library", "find", "etnica", "--limit", "1", "--json"],
    );
    let value = json(&out);
    assert_eq!(value.as_array().unwrap().len(), 1);
    assert!(
        value[0]["record"]["path"]
            .as_str()
            .unwrap()
            .ends_with("01 Etnica - Alpha.wav")
    );

    let out = dermixen(dir.path(), &[], &["library", "find", "juno reactor"]);
    ok(&out);
    assert!(stdout(&out).trim().is_empty() || !stdout(&out).starts_with("1.00"));
}

#[test]
fn a_length_range_narrows_the_listing_and_is_written_in_minutes_or_seconds() {
    let dir = tempfile::tempdir().unwrap();
    a_library(&dir.path().join("music"));
    ok(&dermixen(
        dir.path(),
        &[],
        &["library", "scan", "music", "--exclude", "mixes"],
    ));
    // Every track is twenty seconds long.
    for range in ["0:15-0:25", "15-25", "0:20-0:20", "20-25"] {
        let value = json(&dermixen(
            dir.path(),
            &[],
            &["library", "query", "--length", range, "--json"],
        ));
        assert_eq!(value.as_array().unwrap().len(), 2, "{range}");
    }
    for range in ["0:30-1:00", "0-19", "21-30"] {
        let value = json(&dermixen(
            dir.path(),
            &[],
            &["library", "query", "--length", range, "--json"],
        ));
        assert_eq!(value.as_array().unwrap().len(), 0, "{range}");
    }
    let value = json(&dermixen(
        dir.path(),
        &[],
        &[
            "library",
            "query",
            "--length",
            "0:10-0:30",
            "--bpm",
            "138-142",
            "--json",
        ],
    ));
    assert_eq!(value.as_array().unwrap().len(), 1);
    for bad in ["long", "20", "7:00", "9:00-7:00", "1:99-2:00"] {
        let message = fails(&dermixen(
            dir.path(),
            &[],
            &["library", "query", "--length", bad],
        ));
        assert!(message.contains("--length"), "{bad}: {message}");
    }
}

/// A record as the library stores one, with the year and the estimate mark
/// given, so a query can be asked about years that no fixture file has.
fn dated(byte: u8, path: &Path, year: Option<u16>, estimated: bool) -> TrackRecord {
    TrackRecord {
        hash: ContentHash([byte; 32]),
        path: path.to_path_buf(),
        length: Samples(300 * 44_100),
        grid: BeatGrid {
            first_beat: Samples(441),
            bpm: Bpm(140.0),
        },
        grid_confidence: 0.9,
        grid_analyzer: "pulse".to_owned(),
        key: None,
        extent: Extent {
            begins: Samples(44_100),
            ends: Samples(290 * 44_100),
        },
        anchors: Anchors {
            intro: Beats(32.0),
            outro: Beats(512.0),
        },
        anchor_confidence: 0.7,
        anchor_analyzer: "kick".to_owned(),
        metadata: Metadata {
            artist: Some("Etnica".to_owned()),
            title: Some(path.file_stem().unwrap().to_string_lossy().into_owned()),
            year,
            year_is_approximate: estimated,
            source: MetadataSource::Tags,
        },
        phrases: None,
        loudness: None,
        release: Release::default(),
    }
}

#[test]
fn a_query_can_leave_estimated_years_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    for record in [
        dated(1, Path::new("/music/01 Stated.mp3"), Some(1996), false),
        dated(2, Path::new("/music/02 Estimated.mp3"), Some(1996), true),
        dated(3, Path::new("/music/03 Undated.mp3"), None, false),
    ] {
        index.upsert(&record).unwrap();
    }
    drop(index);

    let names = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["library", "query", "--json"];
        all.extend_from_slice(args);
        json(&dermixen(dir.path(), &[], &all))
            .as_array()
            .unwrap()
            .iter()
            .map(|record| {
                Path::new(record["path"].as_str().unwrap())
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    };
    // Without the option an estimate counts as the year it estimates.
    assert_eq!(
        names(&["--year", "1996"]),
        vec!["01 Stated", "02 Estimated"]
    );
    // With it, only a year some source states is a year.
    assert_eq!(
        names(&["--year", "1996", "--no-approximate-years"]),
        vec!["01 Stated"]
    );
    // On its own the option leaves out the estimate and nothing else: a
    // track with no year has no estimate to leave out.
    assert_eq!(
        names(&["--no-approximate-years"]),
        vec!["01 Stated", "03 Undated"]
    );

    let out = dermixen(dir.path(), &[], &["library", "query", "--help"]);
    ok(&out);
    assert!(
        stdout(&out).contains("--no-approximate-years"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn every_field_of_the_analyze_report_is_separated_from_its_value() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "music/a.wav", 130.0, 0.5);
    let out = dermixen(dir.path(), &[], &["analyze", "music/a.wav", "--bpm", "130"]);
    ok(&out);
    let text = stdout(&out);
    for line in text.lines() {
        let (name, value) = line
            .split_once(char::is_whitespace)
            .unwrap_or_else(|| panic!("no separator between the name and the value on {line:?}"));
        assert!(!name.is_empty() && !value.trim().is_empty(), "{line:?}");
    }
    // The two longest names are the ones that ran into their values.
    assert!(
        text.lines()
            .any(|line| line.starts_with("year_is_approximate ")),
        "{text}"
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("release_data_source ")),
        "{text}"
    );
}

#[test]
fn the_listing_shows_each_tracks_year_with_an_estimate_marked() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    for record in [
        dated(1, Path::new("/music/01 Stated.mp3"), Some(1996), false),
        dated(2, Path::new("/music/02 Estimated.mp3"), Some(1996), true),
        dated(3, Path::new("/music/03 Undated.mp3"), None, false),
    ] {
        index.upsert(&record).unwrap();
    }
    drop(index);
    let out = dermixen(dir.path(), &[], &["library", "query"]);
    ok(&out);
    let text = stdout(&out);
    // The year is the third cell of a line, after the code and the tempo.
    // Cells are separated by two spaces or more, and a cell's own text has
    // single spaces at most.
    let year_cell = |name: &str| -> String {
        let line = text
            .lines()
            .find(|line| line.ends_with(name))
            .unwrap_or_else(|| panic!("no line ends with {name}:\n{text}"));
        let cells: Vec<&str> = line
            .split("  ")
            .map(str::trim)
            .filter(|cell| !cell.is_empty())
            .collect();
        cells[2].to_owned()
    };
    assert_eq!(year_cell("01 Stated.mp3"), "1996");
    assert_eq!(year_cell("02 Estimated.mp3"), "~1996");
    assert_eq!(year_cell("03 Undated.mp3"), "-");
}

#[test]
fn a_query_can_require_a_least_confidence_and_refuses_one_it_cannot_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    // Three records whose two confidences pull apart, so a condition on one
    // is told from a condition on the other, and from the two being crossed.
    let mut sure_grid = dated(1, Path::new("/music/01 Sure grid.mp3"), Some(1996), false);
    sure_grid.grid_confidence = 0.9;
    sure_grid.anchor_confidence = 0.3;
    let mut middling = dated(2, Path::new("/music/02 Middling.mp3"), Some(1996), false);
    middling.grid_confidence = 0.5;
    middling.anchor_confidence = 0.5;
    let mut sure_anchors = dated(
        3,
        Path::new("/music/03 Sure anchors.mp3"),
        Some(1996),
        false,
    );
    sure_anchors.grid_confidence = 0.1;
    sure_anchors.anchor_confidence = 0.9;
    for record in [&sure_grid, &middling, &sure_anchors] {
        index.upsert(record).unwrap();
    }
    drop(index);

    let names = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["library", "query", "--json"];
        all.extend_from_slice(args);
        json(&dermixen(dir.path(), &[], &all))
            .as_array()
            .unwrap()
            .iter()
            .map(|record| {
                Path::new(record["path"].as_str().unwrap())
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    };
    // The bound is included: a track scoring exactly it stays in.
    assert_eq!(
        names(&["--min-grid-confidence", "0.5"]),
        vec!["01 Sure grid", "02 Middling"]
    );
    assert_eq!(
        names(&["--min-anchor-confidence", "0.5"]),
        vec!["02 Middling", "03 Sure anchors"]
    );
    assert_eq!(
        names(&["--min-anchor-confidence", "0.6"]),
        vec!["03 Sure anchors"]
    );
    assert_eq!(
        names(&[
            "--min-grid-confidence",
            "0.5",
            "--min-anchor-confidence",
            "0.5"
        ]),
        vec!["02 Middling"]
    );
    assert_eq!(
        names(&["--min-grid-confidence", "0"]),
        vec!["01 Sure grid", "02 Middling", "03 Sure anchors"]
    );

    // A confidence runs from zero to one, so text that is not a number and
    // a number outside that range are refused with the option and the text
    // named.
    for (option, bad) in [
        ("--min-grid-confidence", "high"),
        ("--min-anchor-confidence", "1.5"),
        ("--min-grid-confidence", "nan"),
    ] {
        let message = fails(&dermixen(
            dir.path(),
            &[],
            &["library", "query", option, bad],
        ));
        assert!(message.contains(option), "{option} {bad}: {message}");
        assert!(message.contains(bad), "{option} {bad}: {message}");
    }

    let out = dermixen(dir.path(), &[], &["library", "query", "--help"]);
    ok(&out);
    for option in [
        "--min-grid-confidence <CONFIDENCE>",
        "--min-anchor-confidence <CONFIDENCE>",
    ] {
        assert!(stdout(&out).contains(option), "{}", stdout(&out));
    }
}

#[test]
fn a_scan_with_no_folder_scans_the_music_folder_the_setting_names() {
    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    kicks_file(&music, "Etnica - Alpha.wav", 130.0, 0.5);
    // The scan stores the folder as the operating system spells it with
    // symbolic links resolved, which on macOS turns /var into /private/var.
    let music = music.canonicalize().unwrap();
    let settings = dir.path().join("settings.toml");
    let library = dir.path().join("library.sqlite");
    let env = [
        ("DERMIXEN_SETTINGS_FILE", settings.to_str().unwrap()),
        ("DERMIXEN_LIBRARY_FILE", library.to_str().unwrap()),
    ];

    // With no folder given and no music folder set, the scan fails naming
    // the default folder and the setting, and the person is told to give a
    // folder or to set one.
    let missing = dir.path().join("Music/Undefunktis");
    let said = fails(&dermixen(
        dir.path(),
        &[
            ("DERMIXEN_SETTINGS_FILE", settings.to_str().unwrap()),
            ("DERMIXEN_LIBRARY_FILE", library.to_str().unwrap()),
            ("HOME", dir.path().to_str().unwrap()),
        ],
        &["library", "scan"],
    ));
    assert!(
        said.contains(&missing.display().to_string()),
        "the failure names the folder looked for: {said}"
    );
    assert!(
        said.contains("music_folder"),
        "the failure names the setting: {said}"
    );
    assert!(
        !library.exists(),
        "nothing was scanned, so no library was made"
    );

    ok(&dermixen(
        dir.path(),
        &env,
        &["settings", "set", "music_folder", music.to_str().unwrap()],
    ));
    let out = dermixen(dir.path(), &env, &["library", "scan"]);
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("added 1"), "{text}");
    let listed = json(&dermixen(dir.path(), &env, &["library", "query", "--json"]));
    let paths: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        vec![music.join("Etnica - Alpha.wav").to_str().unwrap()],
        "the track under the music folder is in the library"
    );

    // A folder given on the command line is scanned instead of the setting.
    let other = dir.path().join("other");
    kicks_file(&other, "Prana - Beta.wav", 140.0, 0.5);
    let out = dermixen(
        dir.path(),
        &env,
        &["library", "scan", other.to_str().unwrap()],
    );
    ok(&out);
    assert!(stdout(&out).contains("added 1"), "{}", stdout(&out));
    let report = json(&dermixen(dir.path(), &env, &["library", "scan", "--json"]));
    assert_eq!(
        report["root"],
        music.display().to_string(),
        "the JSON report names the music folder as the root that was scanned"
    );
}

/// The library names every audio file a person owns, so a folder the command
/// makes to hold the library file is for its owner alone, and a folder that
/// is already there keeps the permissions it has.
#[test]
#[cfg(unix)]
fn a_folder_the_command_makes_for_the_library_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let music = dir.path().join("music");
    kicks_file(&music, "Etnica - Alpha.wav", 130.0, 0.5);
    let mode_of = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

    let made = dir.path().join("newfolder");
    let inside = made.join("inner");
    let library = inside.join("library.sqlite");
    ok(&dermixen(
        dir.path(),
        &[("DERMIXEN_LIBRARY_FILE", library.to_str().unwrap())],
        &["library", "scan", "music"],
    ));
    assert!(library.exists());
    assert_eq!(mode_of(&made), 0o700, "the folder the command made");
    assert_eq!(mode_of(&inside), 0o700, "the folder below it");

    // A folder that is already there keeps the permissions it has.
    let chosen = dir.path().join("chosen");
    std::fs::create_dir(&chosen).unwrap();
    std::fs::set_permissions(&chosen, std::fs::Permissions::from_mode(0o755)).unwrap();
    ok(&dermixen(
        dir.path(),
        &[(
            "DERMIXEN_LIBRARY_FILE",
            chosen.join("library.sqlite").to_str().unwrap(),
        )],
        &["library", "scan", "music"],
    ));
    assert_eq!(mode_of(&chosen), 0o755);
}

/// A reader that has read enough closes the pipe, and the command stops
/// writing rather than reporting a defect. The listing is larger than the
/// 64 KiB a pipe holds, so the command is still writing when the reader goes.
#[test]
#[cfg(unix)]
fn a_reader_that_closes_the_pipe_is_not_a_failure() {
    use std::process::{Command, Stdio};

    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    for number in 0..500u16 {
        let name =
            format!("{number:03} Etnica - A track with a name long enough to fill a pipe.wav");
        let path = dir.path().join("music").join(&name);
        index
            .upsert(&dated((number % 251) as u8, &path, Some(1996), false))
            .unwrap();
    }
    drop(index);

    let mut child = Command::new(env!("CARGO_BIN_EXE_dermixen"))
        .args(["library", "query"])
        .current_dir(dir.path())
        .env("DERMIXEN_LIBRARY_FILE", dir.path().join("library.sqlite"))
        .env("DERMIXEN_SETTINGS_FILE", dir.path().join("settings.toml"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the dermixen binary runs");
    // The reader closes at once, which is what `| head -c 10` does once it
    // has its ten bytes.
    drop(child.stdout.take().expect("standard output is a pipe"));
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "the command said something about the closed pipe: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
