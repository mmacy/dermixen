//! Acceptance tests for what the command says about a defect it caught.
//!
//! A panic the command catches belongs to the one file it happened on, and
//! the line the command prints about that file is the whole report. The line
//! beginning `error:` is for a defect that stopped the command, so a scan
//! that catches a panic and goes on must not print one.

mod common;

use std::path::{Path, PathBuf};

use common::{dermixen, kicks_file, ok, stderr, stdout};

/// One of the committed audio fixtures.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
        .canonicalize()
        .unwrap()
}

#[test]
fn a_scan_that_catches_a_panic_reports_the_file_and_claims_nothing_about_stopping() {
    // The WAV file states 65,535 channels, which reaches arithmetic in the
    // decoding library that overflows. A debug build of that library panics
    // on the overflow and a release build returns an error, so this test
    // asserts what holds either way: the scan deals with the file, reports it
    // as failed, and goes on to the file after it.
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let music = dir.join("music");
    std::fs::create_dir_all(&music).unwrap();
    kicks_file(&music, "a_first.wav", 140.0, 0.0);
    kicks_file(&music, "c_last.wav", 130.0, 0.0);
    let hostile = music.join("b_hostile.wav");
    std::fs::copy(fixture("wav-65535-channels.wav"), &hostile).unwrap();

    let output = dermixen(dir, &[], &["library", "scan", "music"]);
    ok(&output);
    let said = stdout(&output);
    assert!(
        said.lines()
            .any(|line| line.starts_with("failed ") && line.contains("b_hostile.wav")),
        "the scan does not report the file as failed:\n{said}"
    );
    assert!(
        said.lines().any(|line| line == "added 2"),
        "the scan does not report the other two files as added:\n{said}"
    );
    let told = stderr(&output);
    assert!(
        !told.lines().any(|line| line.starts_with("error:")),
        "the scan that went on to its end reports a defect that stopped it:\n{told}"
    );

    // The two files the scan could read are in the library, and the file it
    // could not read is not.
    let listing = stdout(&dermixen(dir, &[], &["library", "query"]));
    assert!(listing.contains("a_first.wav"), "{listing}");
    assert!(listing.contains("c_last.wav"), "{listing}");
    assert!(!listing.contains("b_hostile.wav"), "{listing}");
}
