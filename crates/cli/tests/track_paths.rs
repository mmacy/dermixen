//! What the commands do with a mix document whose track no longer names an
//! audio file. A folder where the file was is what a person finds when a
//! volume is not mounted or when the file was deleted and a folder took its
//! name, and `mix relink` is the command that repairs it. Every other
//! command refuses the document until it has been repaired.
#![cfg(unix)]

mod common;

use std::fs;

use common::{dermixen, fails, ok, stdout, two_track_mix};

#[test]
fn a_track_that_names_a_folder_is_refused_and_mix_relink_repairs_it() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    two_track_mix(dir);
    // The first track's file moves into a folder, and a folder takes the name
    // the document holds.
    fs::create_dir(dir.join("music")).unwrap();
    fs::rename(dir.join("a.wav"), dir.join("music/a.wav")).unwrap();
    fs::create_dir(dir.join("a.wav")).unwrap();

    for command in [
        vec!["mix", "show", "set.dmx"],
        vec!["render", "set.dmx", "out.wav"],
    ] {
        let message = fails(&dermixen(dir, &[], &command));
        assert!(
            message.contains("a.wav") && message.contains("regular file"),
            "{command:?}: {message}"
        );
    }
    assert!(!dir.join("out.wav").exists());

    let repaired = dermixen(dir, &[], &["mix", "relink", "set.dmx", "--under", "music"]);
    ok(&repaired);
    assert!(
        stdout(&repaired).contains("relinked"),
        "{}",
        stdout(&repaired)
    );
    ok(&dermixen(dir, &[], &["mix", "show", "set.dmx"]));
    ok(&dermixen(dir, &[], &["render", "set.dmx", "out.wav"]));
    assert!(dir.join("out.wav").exists());
}
