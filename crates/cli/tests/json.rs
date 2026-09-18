//! Acceptance tests for `--json` on every command: each output is one JSON
//! document that fits its definition in `docs/json/dermixen.schema.json`.
//! A coder agent makes these pass without editing them.

mod common;

use std::path::Path;

use common::{dermixen, fails, json, kicks_file, ok, stdout, two_track_mix};
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

#[test]
fn analyze_prints_the_record_the_library_would_store() {
    let dir = tempfile::tempdir().unwrap();
    let a = kicks_file(dir.path(), "01 Etnica - Alpha.wav", 130.0, 0.5);
    let out = dermixen(
        dir.path(),
        &[],
        &[
            "analyze",
            "01 Etnica - Alpha.wav",
            "--json",
            "--bpm",
            "130",
            "--first-beat",
            "0.5",
        ],
    );
    let value = json(&out);
    fits(&value, "analyze");
    assert_eq!(value["path"], a.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(
        value["hash"],
        dermixen_media::hash_file(&a).unwrap().to_string()
    );
    assert_eq!(value["length_samples"], 20 * 44_100);
    assert_eq!(value["grid"]["bpm"], 130.0);
    assert_eq!(value["grid"]["first_beat_sample"], 22_050);
    assert_eq!(value["grid_analyzer"], "given");
    assert_eq!(value["grid_confidence"], 1.0);
    // The kicks begin half a second in, on beat zero of the given grid.
    assert_eq!(value["anchors"]["intro_beat"], 0.0);
    assert!(value["anchors"]["outro_beat"].as_f64().unwrap() > 0.0);
    assert!(value["extent"]["begins_sample"].as_i64().unwrap() >= 22_050);
    assert_eq!(value["metadata"]["artist"], "Etnica");
    assert_eq!(value["metadata"]["title"], "Alpha");
    assert_eq!(value["metadata"]["source"], "filename");

    // Without --json the same facts come as text, not JSON.
    let out = dermixen(
        dir.path(),
        &[],
        &["analyze", "01 Etnica - Alpha.wav", "--bpm", "130"],
    );
    common::ok(&out);
    let text = stdout(&out);
    assert!(text.contains("130") && text.contains("Etnica"), "{text}");
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_err());
}

#[test]
fn mix_new_add_and_show_print_the_document_and_its_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let out = dermixen(dir.path(), &[], &["mix", "new", "set.dmx", "--json"]);
    let value = json(&out);
    fits(&value, "mix_new");
    assert!(value["path"].as_str().unwrap().ends_with("set.dmx"));

    kicks_file(dir.path(), "a.wav", 130.0, 0.0);
    kicks_file(dir.path(), "b.wav", 140.0, 0.0);
    let out = dermixen(
        dir.path(),
        &[],
        &[
            "mix", "add", "set.dmx", "a.wav", "--bpm", "130", "--outro", "32", "--json",
        ],
    );
    let after_one = json(&out);
    fits(&after_one, "mix_show");
    assert_eq!(after_one["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(after_one["tracks"][0]["position"], 1);
    assert_eq!(after_one["length_samples"], 20 * 44_100);

    // The beatmix by name, since a twenty-second file has no room for the
    // blend's twenty-eight bars between its anchors.
    let out = dermixen(
        dir.path(),
        &[],
        &[
            "mix", "add", "set.dmx", "b.wav", "--bpm", "140", "--intro", "16", "--preset",
            "beatmix", "--json",
        ],
    );
    let after_two = json(&out);
    fits(&after_two, "mix_show");
    let tracks = after_two["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[1]["position"], 2);
    assert!(tracks[1]["path"].as_str().unwrap().ends_with("b.wav"));
    assert_eq!(tracks[1]["anchors"]["intro_beat"], 16.0);
    assert!(tracks[1]["start_seconds"].as_f64().unwrap() > 0.0);
    assert!(
        tracks[0]["end_seconds"].as_f64().unwrap() > tracks[1]["start_seconds"].as_f64().unwrap()
    );
    assert!(after_two["length_seconds"].as_f64().unwrap() > 20.0);
    assert_eq!(tracks[0]["tempo_nodes"], 1);
    assert_eq!(tracks[0]["volume_nodes"], 6);

    let out = dermixen(dir.path(), &[], &["mix", "show", "set.dmx", "--json"]);
    let shown = json(&out);
    fits(&shown, "mix_show");
    assert_eq!(shown, after_two);

    let out = dermixen(
        dir.path(),
        &[],
        &[
            "mix",
            "move-anchor",
            "set.dmx",
            "2",
            "--intro",
            "24",
            "--json",
        ],
    );
    let moved = json(&out);
    fits(&moved, "mix_show");
    assert_eq!(moved["tracks"][1]["anchors"]["intro_beat"], 24.0);
    assert!(
        moved["tracks"][1]["start_seconds"].as_f64().unwrap()
            < tracks[1]["start_seconds"].as_f64().unwrap()
    );
}

#[test]
fn render_prints_where_it_wrote_and_how_long_the_mix_is() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let out = dermixen(dir.path(), &[], &["render", "set.dmx", "out.wav", "--json"]);
    let value = json(&out);
    fits(&value, "render");
    assert!(value["path"].as_str().unwrap().ends_with("out.wav"));
    let written = decode(&dir.path().join("out.wav")).unwrap().audio;
    assert_eq!(value["length_samples"], written.len().0);
    assert!((value["length_seconds"].as_f64().unwrap() - written.duration().0).abs() < 1e-9);
}

#[test]
fn play_prints_the_span_it_played() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
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
            "--json",
        ],
    );
    let value = json(&out);
    fits(&value, "play");
    assert!(value["mix"].as_str().unwrap().ends_with("set.dmx"));
    assert!(value["capture"].as_str().unwrap().ends_with("heard.wav"));
    assert_eq!(value["from_seconds"], 17.0);
    assert_eq!(value["length_samples"], 220_500);
    assert_eq!(value["length_seconds"], 5.0);
    let shown = json(&dermixen(
        dir.path(),
        &[],
        &["mix", "show", "set.dmx", "--json"],
    ));
    assert_eq!(value["mix_length_seconds"], shown["length_seconds"]);
    assert_eq!(value["underruns"], 0);
}

#[test]
fn scoreboard_prints_every_row_with_its_per_track_scores() {
    let dir = tempfile::tempdir().unwrap();
    let truth = dir.path().join("truth");
    kicks_file(&truth, "a.wav", 130.0, 0.0);
    std::fs::write(truth.join("a.bpm"), "130\n").unwrap();
    std::fs::write(truth.join("a.key"), "A minor\n").unwrap();
    let out = dermixen(dir.path(), &[], &["scoreboard", "truth", "--json"]);
    let value = json(&out);
    fits(&value, "scoreboard");
    let beats = value["beats"].as_array().unwrap();
    assert!(
        beats.iter().any(|row| row["analyzer"] == "fixed tempo"),
        "{value}"
    );
    for row in beats {
        assert_eq!(row["tracks"], 1);
        assert_eq!(row["per_track"].as_array().unwrap().len(), 1);
        assert_eq!(row["per_track"][0]["name"], "a");
    }
    for row in value["keys"].as_array().unwrap() {
        assert_eq!(row["tracks"], 1);
        assert_eq!(row["per_track"][0]["name"], "a");
    }
}

#[test]
fn open_prints_the_mix_the_app_and_the_process() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let mix = std::fs::canonicalize(dir.path()).unwrap().join("set.dmx");
    let app = dir.path().join("fake-app.sh");
    std::fs::write(&app, "#!/bin/sh\nsleep 1\n").unwrap();
    std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = dermixen(
        dir.path(),
        &[("DERMIXEN_APP", app.to_str().unwrap())],
        &["open", "set.dmx", "--json"],
    );
    let value = json(&out);
    fits(&value, "open");
    assert_eq!(value["mix"], mix.display().to_string());
    assert_eq!(value["app"], app.display().to_string());
    assert!(value["pid"].as_u64().unwrap() > 0);
}

#[test]
fn a_failure_under_json_is_still_a_line_on_standard_error() {
    let dir = tempfile::tempdir().unwrap();
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["analyze", "nothing.wav", "--json"],
    ));
    assert!(message.starts_with("error:"), "{message}");
    assert!(message.contains("nothing.wav"), "{message}");
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["mix", "show", "nothing.dmx", "--json"],
    ));
    assert!(message.contains("nothing.dmx"), "{message}");
    let message = fails(&dermixen(
        dir.path(),
        &[],
        &["render", "nothing.dmx", "out.wav", "--json"],
    ));
    assert!(message.contains("nothing.dmx"), "{message}");
    assert!(!dir.path().join("out.wav").exists());
}

#[cfg(feature = "aubio")]
mod with_a_beat_tracker {
    use super::*;

    #[test]
    fn library_scan_query_and_find_print_their_records() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("music");
        kicks_file(
            &root,
            "artist/Etnica - One/01 Etnica - Alpha.wav",
            130.0,
            0.5,
        );
        kicks_file(&root, "comp/VA - Two/02 Prana - Beta.wav", 140.0, 0.5);
        kicks_file(&root, "mixes/set.wav", 135.0, 0.0);
        let out = dermixen(
            dir.path(),
            &[],
            &["library", "scan", "music", "--exclude", "mixes", "--json"],
        );
        let value = json(&out);
        fits(&value, "library_scan");
        assert_eq!(value["added"], 2);
        assert_eq!(value["failed"].as_array().unwrap().len(), 0);
        assert!(
            value["library"]
                .as_str()
                .unwrap()
                .ends_with("library.sqlite")
        );

        let out = dermixen(dir.path(), &[], &["library", "query", "--json"]);
        let value = json(&out);
        fits(&value, "library_query");
        let records = value.as_array().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["metadata"]["artist"], "Etnica");
        assert_eq!(records[1]["metadata"]["artist"], "Prana");
        assert!(!records[0]["grid_analyzer"].as_str().unwrap().is_empty());

        let out = dermixen(
            dir.path(),
            &[],
            &["library", "find", "prana beta", "--json"],
        );
        let value = json(&out);
        fits(&value, "library_find");
        let matches = value.as_array().unwrap();
        assert!(!matches.is_empty());
        assert!(
            matches[0]["record"]["path"]
                .as_str()
                .unwrap()
                .ends_with("02 Prana - Beta.wav")
        );
        assert!(matches[0]["score"].as_f64().unwrap() >= 0.99);
    }
}

#[test]
fn render_handover_prints_where_the_handover_falls_in_the_clip() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let out = dermixen(
        dir.path(),
        &[],
        &["render", "set.dmx", "clip.wav", "--handover", "2", "--json"],
    );
    ok(&out);
    let value = json(&out);
    fits(&value, "render");
    let handover = &value["handover"];
    assert_eq!(handover["into_track"].as_u64(), Some(2));
    for key in [
        "from_seconds",
        "for_seconds",
        "rise_seconds",
        "anchor_seconds",
        "outgoing_end_seconds",
    ] {
        assert!(
            handover[key].as_f64().is_some_and(|seconds| seconds >= 0.0),
            "{key} is missing or negative: {value}"
        );
    }
    // The fixture joins its tracks with a beatmix, whose fade in starts at
    // the intro anchor, so the rise begins where the anchors meet. The
    // outgoing track ends after that.
    let rise = handover["rise_seconds"].as_f64().unwrap();
    let anchor = handover["anchor_seconds"].as_f64().unwrap();
    assert!(
        (rise - anchor).abs() < 0.002,
        "rise {rise}, anchor {anchor}"
    );
    assert!(anchor < handover["outgoing_end_seconds"].as_f64().unwrap());
}

#[test]
fn mix_set_gain_prints_the_timeline() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let out = dermixen(
        dir.path(),
        &[],
        &["mix", "set-gain", "set.dmx", "2", "-3", "--json"],
    );
    ok(&out);
    let value = json(&out);
    fits(&value, "mix_show");
    assert_eq!(value["tracks"][1]["gain_db"].as_f64(), Some(-3.0));
}
