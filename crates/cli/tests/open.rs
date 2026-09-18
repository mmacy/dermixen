//! Acceptance tests for `dermixen open`, which hands a mix to the window. A
//! coder makes these pass without editing them.
//!
//! The window itself is never started here: `DERMIXEN_APP` points the
//! command at a shell script that records the arguments it was started
//! with, so the tests see what the window would have been given.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{dermixen, fails, ok, stdout, two_track_mix};

/// Writes a script that records its arguments in the file the
/// `DERMIXEN_OPEN_RECORD` environment variable names, one per line, then
/// sleeps for `seconds` so that a command waiting for it would be seen to
/// wait.
fn fake_app(dir: &Path, seconds: u32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-app.sh");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$DERMIXEN_OPEN_RECORD\"\nsleep {seconds}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Waits for the record file to appear and returns its lines.
fn recorded(record: &Path) -> Vec<String> {
    let began = Instant::now();
    while !record.exists() {
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the app was never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    std::fs::read_to_string(record)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn open_starts_the_app_on_the_absolute_path_of_the_mix_and_does_not_wait() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let app = fake_app(dir.path(), 3);
    let record = dir.path().join("record.txt");
    let began = Instant::now();
    let out = dermixen(
        dir.path(),
        &[
            ("DERMIXEN_APP", app.to_str().unwrap()),
            ("DERMIXEN_OPEN_RECORD", record.to_str().unwrap()),
        ],
        &["open", "set.dmx"],
    );
    ok(&out);
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "open waited for the app, which sleeps three seconds"
    );
    // The path is made absolute with symbolic links resolved, which on
    // macOS turns a temporary folder under /var into one under /private/var.
    let mix = std::fs::canonicalize(dir.path()).unwrap().join("set.dmx");
    assert_eq!(recorded(&record), vec![mix.display().to_string()]);
    let text = stdout(&out);
    assert!(text.contains("set.dmx"), "{text}");
    assert!(text.contains("fake-app.sh"), "{text}");
}

#[test]
fn open_refuses_a_mix_it_cannot_read_and_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = fake_app(dir.path(), 0);
    let record = dir.path().join("record.txt");
    let env = [
        ("DERMIXEN_APP", app.to_str().unwrap()),
        ("DERMIXEN_OPEN_RECORD", record.to_str().unwrap()),
    ];

    let message = fails(&dermixen(dir.path(), &env, &["open", "missing.dmx"]));
    assert!(message.contains("missing.dmx"), "{message}");

    std::fs::write(
        dir.path().join("broken.dmx"),
        "{\"version\": 1, \"tracks\": [{}]}",
    )
    .unwrap();
    let message = fails(&dermixen(dir.path(), &env, &["open", "broken.dmx"]));
    assert!(
        message.contains("broken.dmx") || message.contains("tracks"),
        "{message}"
    );

    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !record.exists(),
        "the app was started on a mix that could not be read"
    );
}

#[test]
fn open_says_which_app_it_could_not_start() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let nowhere = dir.path().join("no-such-app");
    let message = fails(&dermixen(
        dir.path(),
        &[("DERMIXEN_APP", nowhere.to_str().unwrap())],
        &["open", "set.dmx"],
    ));
    assert!(message.contains("no-such-app"), "{message}");
    assert!(message.contains("DERMIXEN_APP"), "{message}");

    // A path that exists but cannot be started fails too, naming it.
    let folder = dir.path().join("a-folder");
    std::fs::create_dir(&folder).unwrap();
    let message = fails(&dermixen(
        dir.path(),
        &[("DERMIXEN_APP", folder.to_str().unwrap())],
        &["open", "set.dmx"],
    ));
    assert!(message.contains("a-folder"), "{message}");
}
