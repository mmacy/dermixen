//! Acceptance tests for folder scanning. A coder agent makes these pass without editing them.

use std::fs;
use std::path::{Path, PathBuf};

use dermixen_library::{ScanError, ScanOptions, scan};

/// Writes an empty file, creating the folders above it. A scan judges audio
/// by extension alone, so the files need no contents.
fn touch(root: &Path, relative: &str) -> PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, b"").unwrap();
    path
}

fn a_library(root: &Path) {
    touch(
        root,
        "artist/Etnica - Alien Protein [BF001]/01 Etnica - Alien Protein.mp3",
    );
    touch(
        root,
        "artist/Etnica - Alien Protein [BF001]/02 Etnica - Vimana.MP3",
    );
    touch(root, "artist/Etnica - Alien Protein [BF001]/cover.jpg");
    touch(root, "artist/Etnica - Alien Protein [BF001]/notes.txt");
    touch(
        root,
        "comp/VA - Goa Vibes [GV001]/CD1/03 Slinky Wizard - Lunar Juice.wav",
    );
    touch(
        root,
        "comp/VA - Goa Vibes [GV001]/CD2/01 Prana - Boundless.flac",
    );
    touch(
        root,
        "comp/VA - Goa Vibes [GV001]/CD2/02 Prana - Scarab.m4a",
    );
    touch(root, "misc/tape.mp4");
    touch(root, "misc/README");
    touch(root, "mixes/slow ascent.wav");
    touch(root, "mixes/slow ascent.mmp");
    touch(root, "mixes/published/global goa party 3.mp3");
}

#[test]
fn every_audio_file_is_listed_in_path_order_and_nothing_else_is() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    a_library(root);
    let found = scan(root, &ScanOptions::default()).unwrap();
    let relative: Vec<String> = found
        .files
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        relative,
        vec![
            "artist/Etnica - Alien Protein [BF001]/01 Etnica - Alien Protein.mp3",
            "artist/Etnica - Alien Protein [BF001]/02 Etnica - Vimana.MP3",
            "comp/VA - Goa Vibes [GV001]/CD1/03 Slinky Wizard - Lunar Juice.wav",
            "comp/VA - Goa Vibes [GV001]/CD2/01 Prana - Boundless.flac",
            "comp/VA - Goa Vibes [GV001]/CD2/02 Prana - Scarab.m4a",
            "misc/tape.mp4",
            "mixes/published/global goa party 3.mp3",
            "mixes/slow ascent.wav",
        ]
    );
    assert!(found.unreadable.is_empty());
    // Every path is the root joined with the file's path below it.
    assert!(found.files.iter().all(|path| path.starts_with(root)));
}

#[test]
fn excluded_folders_are_not_entered() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    a_library(root);
    // A relative exclusion covers the folder and everything below it.
    let options = ScanOptions {
        exclude: vec![PathBuf::from("mixes")],
    };
    let found = scan(root, &options).unwrap();
    assert_eq!(found.files.len(), 6, "{:?}", found.files);
    assert!(
        found
            .files
            .iter()
            .all(|path| !path.starts_with(root.join("mixes")))
    );

    // An absolute exclusion works the same way, and a nested one leaves its parent alone.
    let options = ScanOptions {
        exclude: vec![
            root.join("mixes/published"),
            PathBuf::from("comp/VA - Goa Vibes [GV001]/CD2"),
        ],
    };
    let found = scan(root, &options).unwrap();
    let names: Vec<String> = found
        .files
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            "01 Etnica - Alien Protein.mp3",
            "02 Etnica - Vimana.MP3",
            "03 Slinky Wizard - Lunar Juice.wav",
            "tape.mp4",
            "slow ascent.wav",
        ]
    );
}

#[test]
fn a_root_that_is_not_a_folder_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = touch(dir.path(), "track.mp3");
    match scan(&file, &ScanOptions::default()) {
        Err(ScanError::NotAFolder(path)) => assert_eq!(path, file),
        other => panic!("expected a not-a-folder error, got {other:?}"),
    }
    let missing = dir.path().join("nowhere");
    assert!(matches!(
        scan(&missing, &ScanOptions::default()),
        Err(ScanError::NotAFolder(_))
    ));
}

#[test]
fn an_empty_folder_scans_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let found = scan(dir.path(), &ScanOptions::default()).unwrap();
    assert!(found.files.is_empty());
    assert!(found.unreadable.is_empty());
}

#[cfg(unix)]
#[test]
fn a_folder_that_cannot_be_read_is_reported_and_passed_over() {
    use std::os::unix::fs::PermissionsExt;

    // The superuser can read anything, so this test has nothing to show there.
    if std::process::Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "0")
    {
        eprintln!("skipping: running as the superuser");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "open/a.mp3");
    touch(root, "shut/b.mp3");
    let shut = root.join("shut");
    fs::set_permissions(&shut, fs::Permissions::from_mode(0o000)).unwrap();
    let found = scan(root, &ScanOptions::default());
    // Whatever happened, the folder must be readable again so it can be removed.
    fs::set_permissions(&shut, fs::Permissions::from_mode(0o755)).unwrap();
    let found = found.unwrap();
    assert_eq!(found.files, vec![root.join("open/a.mp3")]);
    assert_eq!(found.unreadable.len(), 1);
    assert_eq!(found.unreadable[0].path, shut);
    assert!(!found.unreadable[0].reason.is_empty());
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "real/a.mp3");
    std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
    let found = scan(root, &ScanOptions::default()).unwrap();
    assert_eq!(found.files, vec![root.join("real/a.mp3")]);
}

#[test]
fn an_exclusion_written_with_two_dots_still_excludes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    std::fs::create_dir_all(&root).unwrap();
    a_library(&root);
    let everything = scan(&root, &ScanOptions::default()).unwrap().files.len();

    for exclusion in [
        PathBuf::from("../music/mixes"),
        PathBuf::from("comp/../mixes"),
        PathBuf::from("./mixes/"),
        root.join("comp").join("..").join("mixes"),
    ] {
        let options = ScanOptions {
            exclude: vec![exclusion.clone()],
        };
        let found = scan(&root, &options).unwrap();
        assert!(
            found.files.len() < everything,
            "{exclusion:?} excluded nothing"
        );
        assert!(
            found
                .files
                .iter()
                .all(|path| !path.starts_with(root.join("mixes"))),
            "{exclusion:?}: {:?}",
            found.files
        );
    }
}
