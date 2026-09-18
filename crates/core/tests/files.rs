//! Acceptance tests for the bounded read and the atomic write. A coder agent
//! makes these pass without editing them.
#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::time::{Duration, Instant};

use dermixen_core::Settings;
use dermixen_core::files::{
    AtomicFile, LARGEST_DOCUMENT, LARGEST_SETTINGS, ReadError, open_regular, read_text,
    write_atomically,
};

/// Runs `work` on a thread and panics when it has not answered in two
/// seconds, so that a read that never ends fails the test instead of hanging
/// the run.
fn within_two_seconds<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(work()));
    receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("the call did not return within two seconds")
}

fn make_pipe(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn a_regular_file_within_the_limit_is_read() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("set.dmx");
    fs::write(&path, "0123456789").unwrap();
    assert_eq!(read_text(&path, 10).unwrap(), "0123456789");
    assert_eq!(read_text(&path, LARGEST_DOCUMENT).unwrap(), "0123456789");

    let link = folder.path().join("link.dmx");
    symlink(&path, &link).unwrap();
    assert_eq!(read_text(&link, 10).unwrap(), "0123456789");
}

#[test]
fn a_file_one_byte_over_the_limit_is_refused_by_size() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("set.dmx");
    fs::write(&path, "0123456789").unwrap();
    match read_text(&path, 9) {
        Err(ReadError::TooLarge { limit: 9, .. }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(LARGEST_DOCUMENT, 16 * 1024 * 1024);
    assert_eq!(LARGEST_SETTINGS, 1024 * 1024);
}

#[test]
fn a_missing_file_is_an_input_error_that_says_not_found() {
    let folder = tempfile::tempdir().unwrap();
    match read_text(&folder.path().join("nothing.dmx"), 10) {
        Err(ReadError::Io { source, .. }) => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound)
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn text_that_is_not_utf8_is_an_input_error() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("set.dmx");
    fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
    assert!(matches!(read_text(&path, 10), Err(ReadError::Io { .. })));
}

#[test]
fn a_device_a_pipe_and_a_folder_are_refused_at_once() {
    let folder = tempfile::tempdir().unwrap();
    let pipe = folder.path().join("pipe.dmx");
    make_pipe(&pipe);
    let link = folder.path().join("zero.dmx");
    symlink("/dev/zero", &link).unwrap();
    let link_to_pipe = folder.path().join("link-to-pipe.dmx");
    symlink(&pipe, &link_to_pipe).unwrap();

    for path in [
        Path::new("/dev/zero").to_path_buf(),
        link,
        pipe,
        link_to_pipe,
        folder.path().to_path_buf(),
    ] {
        let shown = path.display().to_string();
        let (read, opened) = within_two_seconds(move || {
            (
                read_text(&path, LARGEST_DOCUMENT),
                open_regular(&path).map(|_| ()),
            )
        });
        assert!(
            matches!(read, Err(ReadError::NotARegularFile { .. })),
            "{shown}: {read:?}"
        );
        assert!(
            matches!(opened, Err(ReadError::NotARegularFile { .. })),
            "{shown}: {opened:?}"
        );
    }
}

#[test]
fn an_atomic_write_replaces_the_destination_and_leaves_nothing_else() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("set.dmx");
    write_atomically(&path, false, b"first").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"first");
    write_atomically(&path, false, b"second").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"second");
    assert_eq!(names_in(folder.path()), ["set.dmx"]);
}

#[test]
fn a_write_that_is_not_committed_changes_nothing_and_leaves_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("set.dmx");
    fs::write(&path, "the work").unwrap();
    {
        let writing = AtomicFile::create(&path, false).unwrap();
        writing.file().write_all(b"half a docum").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"the work");
        assert_eq!(names_in(folder.path()).len(), 2);
    }
    assert_eq!(fs::read(&path).unwrap(), b"the work");
    assert_eq!(names_in(folder.path()), ["set.dmx"]);
}

#[test]
fn a_writer_can_own_a_handle_of_its_own() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("out.wav");
    let writing = AtomicFile::create(&path, false).unwrap();
    let mut handle = writing.file().try_clone().unwrap();
    handle.write_all(b"RIFF").unwrap();
    drop(handle);
    writing.commit().unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"RIFF");
}

#[test]
fn a_link_planted_at_a_predictable_temporary_name_is_not_written_through() {
    let folder = tempfile::tempdir().unwrap();
    let victim = folder.path().join("victim.txt");
    fs::write(&victim, "PRECIOUS").unwrap();
    let path = folder.path().join("set.dmx");
    for suffix in ["part", "new", "tmp"] {
        symlink(&victim, folder.path().join(format!("set.dmx.{suffix}"))).unwrap();
    }
    write_atomically(&path, false, b"a document").unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"PRECIOUS");
    assert_eq!(fs::read(&path).unwrap(), b"a document");
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
}

#[test]
fn two_writes_in_one_folder_use_two_temporary_names() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("set.dmx");
    let first = AtomicFile::create(&path, false).unwrap();
    let after_first = names_in(folder.path());
    let second = AtomicFile::create(&path, false).unwrap();
    assert_eq!(after_first.len(), 1);
    assert_eq!(names_in(folder.path()).len(), 2);
    drop(first);
    drop(second);
    // A run of the process later picks another name again.
    let third = AtomicFile::create(&path, false).unwrap();
    assert_ne!(names_in(folder.path()), after_first);
    drop(third);
}

#[test]
fn a_save_keeps_the_permissions_of_the_file_it_replaces() {
    let folder = tempfile::tempdir().unwrap();
    for mode in [0o600, 0o640, 0o444] {
        let path = folder.path().join(format!("set-{mode:o}.dmx"));
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        write_atomically(&path, false, b"new").unwrap();
        assert_eq!(mode_of(&path), mode, "{mode:o}");
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }
}

#[test]
fn a_new_private_file_is_for_its_owner_alone() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("untitled.autosave");
    write_atomically(&path, true, b"work").unwrap();
    assert_eq!(mode_of(&path), 0o600);
    let shared = folder.path().join("set.dmx");
    write_atomically(&shared, false, b"work").unwrap();
    assert_ne!(mode_of(&shared) & 0o400, 0);
}

#[test]
fn the_settings_file_is_read_with_a_limit_and_written_atomically() {
    let folder = tempfile::tempdir().unwrap();

    let zero = folder.path().join("zero.toml");
    symlink("/dev/zero", &zero).unwrap();
    let read = within_two_seconds(move || Settings::read(&zero).map(|_| ()));
    assert!(read.is_err());

    let large = folder.path().join("large.toml");
    let mut text = String::from("# a comment\n");
    while text.len() <= LARGEST_SETTINGS as usize {
        text.push_str("# another line of a settings file that is far too long\n");
    }
    fs::write(&large, text).unwrap();
    let problem = Settings::read(&large).unwrap_err().to_string();
    assert!(problem.contains("1048576"), "{problem}");

    let path = folder.path().join("settings.toml");
    fs::write(&path, "").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let victim = folder.path().join("victim.txt");
    fs::write(&victim, "PRECIOUS").unwrap();
    symlink(&victim, folder.path().join("settings.toml.part")).unwrap();
    Settings::default().write(&path).unwrap();
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(fs::read(&victim).unwrap(), b"PRECIOUS");
    assert_eq!(Settings::read(&path).unwrap(), Settings::default());

    // A missing settings file still means every default.
    let started = Instant::now();
    assert_eq!(
        Settings::read(&folder.path().join("none.toml")).unwrap(),
        Settings::default()
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
#[ignore = "file-helpers-links"]
fn a_destination_that_is_a_link_keeps_the_link_and_replaces_its_target() {
    // A person who keeps a settings file or a mix document in another folder
    // and links to it gets the new contents where the link points.
    let folder = tempfile::tempdir().unwrap();
    let kept = folder.path().join("kept");
    fs::create_dir(&kept).unwrap();
    let target = kept.join("settings.toml");
    fs::write(&target, "old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    let link = folder.path().join("settings.toml");
    symlink(&target, &link).unwrap();

    write_atomically(&link, false, b"new").unwrap();
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert_eq!(mode_of(&target), 0o600);
    assert_eq!(names_in(&kept), ["settings.toml"]);
    assert_eq!(names_in(folder.path()), ["kept", "settings.toml"]);

    // A link whose target is not there yet gets its target made.
    let dangling = folder.path().join("new.dmx");
    symlink(kept.join("new.dmx"), &dangling).unwrap();
    write_atomically(&dangling, false, b"a document").unwrap();
    assert!(
        fs::symlink_metadata(&dangling)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(kept.join("new.dmx")).unwrap(), b"a document");
}
