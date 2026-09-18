//! Acceptance tests for how the `dermixen` command names the library: the
//! file the `library` commands, `mix add`, and `mix relink` read and write is
//! the library file, chosen by `--library` or `DERMIXEN_LIBRARY_FILE`, and nothing
//! the command prints calls that file an index. A coder agent makes these pass
//! without editing them.

mod common;

use common::{dermixen, json, kicks_file, ok, stderr, stdout};

/// Whether the text contains the word "index" in any form or case.
fn says_index(text: &str) -> bool {
    text.to_ascii_lowercase().contains("index")
}

/// Everything a command printed, on both streams.
fn everything(output: &std::process::Output) -> String {
    format!("{}{}", stdout(output), stderr(output))
}

#[test]
fn every_help_text_offers_the_library_option_and_never_says_index() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        &["library", "--help"][..],
        &["library", "scan", "--help"],
        &["library", "query", "--help"],
        &["library", "find", "--help"],
        &["mix", "add", "--help"],
        &["mix", "plan", "--help"],
        &["mix", "relink", "--help"],
    ] {
        let out = dermixen(dir.path(), &[], args);
        ok(&out);
        let text = stdout(&out);
        assert!(
            text.contains("--library <PATH>"),
            "{} does not offer --library <PATH>:\n{text}",
            args.join(" ")
        );
        assert!(
            text.contains("DERMIXEN_LIBRARY_FILE"),
            "{} does not name DERMIXEN_LIBRARY_FILE:\n{text}",
            args.join(" ")
        );
        assert!(!says_index(&text), "{} says index:\n{text}", args.join(" "));
    }
    for args in [&["--help"][..], &["mix", "--help"], &["analyze", "--help"]] {
        let out = dermixen(dir.path(), &[], args);
        ok(&out);
        let text = stdout(&out);
        assert!(!says_index(&text), "{} says index:\n{text}", args.join(" "));
    }
}

#[test]
fn the_index_option_and_variable_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "music/a.wav", 130.0, 0.5);
    let out = dermixen(
        dir.path(),
        &[],
        &["library", "query", "--index", "anywhere.sqlite"],
    );
    assert_eq!(
        out.status.code(),
        Some(2),
        "--index is accepted:\n{}",
        everything(&out)
    );
    // A variable with the old name names nothing.
    let by_library = dir.path().join("by-library.sqlite");
    let by_index = dir.path().join("by-index.sqlite");
    ok(&dermixen(
        dir.path(),
        &[
            ("DERMIXEN_LIBRARY_FILE", by_library.to_str().unwrap()),
            ("DERMIXEN_INDEX", by_index.to_str().unwrap()),
        ],
        &["library", "scan", "music"],
    ));
    assert!(by_library.exists());
    assert!(!by_index.exists());
}

#[test]
fn what_the_library_commands_print_never_says_index() {
    let dir = tempfile::tempdir().unwrap();
    kicks_file(dir.path(), "music/01 Etnica - Alpha.wav", 130.0, 0.5);
    // The same bytes under a second name, so the scan reports a duplicate.
    kicks_file(dir.path(), "music/copy/01 Etnica - Alpha.wav", 130.0, 0.5);
    let out = dermixen(dir.path(), &[], &["library", "scan", "music"]);
    ok(&out);
    let text = everything(&out);
    assert!(text.contains("duplicate"), "no duplicate reported:\n{text}");
    assert!(!says_index(&text), "the scan says index:\n{text}");

    let out = dermixen(dir.path(), &[], &["library", "scan", "music", "--json"]);
    let value = json(&out);
    assert!(
        value["library"]
            .as_str()
            .unwrap()
            .ends_with("library.sqlite"),
        "{value}"
    );
    assert!(value.get("index").is_none(), "{value}");
    assert!(!says_index(&stderr(&out)), "{}", stderr(&out));

    for args in [
        &["library", "query"][..],
        &["library", "find", "etnica alpha"],
    ] {
        let out = dermixen(dir.path(), &[], args);
        ok(&out);
        let text = everything(&out);
        assert!(!says_index(&text), "{} says index:\n{text}", args.join(" "));
    }

    // A library file that is not SQLite is refused with a message that says
    // what to do, without calling the file an index.
    let broken = dir.path().join("broken.sqlite");
    std::fs::write(&broken, b"not a database").unwrap();
    let out = dermixen(
        dir.path(),
        &[],
        &["library", "query", "--library", broken.to_str().unwrap()],
    );
    assert!(!out.status.success());
    let text = everything(&out);
    assert!(!says_index(&text), "the refusal says index:\n{text}");
}

#[test]
fn what_relink_prints_about_a_missing_file_never_says_index() {
    let dir = tempfile::tempdir().unwrap();
    common::two_track_mix(dir.path());
    std::fs::remove_file(dir.path().join("a.wav")).unwrap();
    // The library file exists and names no other file with the same bytes,
    // so the line for the missing track says where the command looked.
    let out = dermixen(dir.path(), &[], &["mix", "relink", "set.dmx"]);
    ok(&out);
    let text = everything(&out);
    assert!(text.contains("missing"), "{text}");
    assert!(!says_index(&text), "relink says index:\n{text}");
    let out = dermixen(dir.path(), &[], &["mix", "relink", "set.dmx", "--json"]);
    ok(&out);
    let text = everything(&out);
    assert!(!says_index(&text), "relink --json says index:\n{text}");
}
