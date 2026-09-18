//! Acceptance tests for the autosave rules: what is written beside a
//! project file after an edit, and what is offered back the next time the
//! mix is opened. A coder agent makes these pass without editing them.
//!
//! These tests are the crash recovery `DESIGN.md` asks for, run without a
//! window: an autosave written and never followed by a save is exactly what
//! a window that was killed leaves behind.

use std::path::{Path, PathBuf};

use dermixen_app::{Offer, autosave_path, forget, offered, write_atomically};
use dermixen_core::{Decibels, Mix};

fn fixture_text() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mix/valid/two-tracks.dmx");
    std::fs::read_to_string(path).unwrap()
}

/// A project file in `dir` containing the fixture's own bytes, and the
/// document it contains.
fn project_in(dir: &Path) -> (PathBuf, Mix) {
    let project = dir.join("set.dmx");
    let text = fixture_text();
    std::fs::write(&project, &text).unwrap();
    (project, Mix::from_json(&text).unwrap())
}

/// The names of every file in `dir`, sorted.
fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn the_autosave_file_sits_beside_the_project_file_under_its_name() {
    assert_eq!(
        autosave_path(Path::new("/music/mixes/set.dmx")),
        PathBuf::from("/music/mixes/set.dmx.autosave")
    );
    assert_eq!(
        autosave_path(Path::new("set.dmx")),
        PathBuf::from("set.dmx.autosave")
    );
}

#[test]
fn an_edit_written_and_never_saved_is_offered_back() {
    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    let mut edited = mix.clone();
    edited.tracks[0].gain = Decibels(-4.0);
    dermixen_app::autosave::write(&project, &edited).unwrap();
    assert!(autosave_path(&project).exists());

    // No save followed, as after a window that was killed.
    match offered(&project, &mix) {
        Offer::Restore {
            mix: found,
            autosaved,
            saved,
        } => {
            assert_eq!(found, edited);
            assert!(
                autosaved.is_some() && saved.is_some(),
                "both files have times"
            );
        }
        other => panic!("expected the edited document back, got {other:?}"),
    }
    assert!(
        autosave_path(&project).exists(),
        "offering the document does not remove the file"
    );
}

#[test]
fn an_autosave_that_says_what_the_project_says_is_removed_and_nothing_is_offered() {
    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    // The project file contains the fixture's own bytes, and the autosave the
    // same document as the window writes it, with other spacing.
    dermixen_app::autosave::write(&project, &mix).unwrap();
    assert_ne!(
        std::fs::read_to_string(autosave_path(&project)).unwrap(),
        fixture_text(),
        "the two files differ as bytes"
    );
    assert_eq!(offered(&project, &mix), Offer::Nothing);
    assert!(
        !autosave_path(&project).exists(),
        "the file protects nothing"
    );
}

#[test]
fn an_autosave_that_is_not_a_mix_is_left_alone_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    std::fs::write(autosave_path(&project), "{ \"version\": 1, \"tracks\": [ {").unwrap();
    match offered(&project, &mix) {
        Offer::Unreadable(message) => {
            assert!(message.contains("set.dmx.autosave"), "{message}");
        }
        other => panic!("expected the file to be reported, got {other:?}"),
    }
    assert!(
        autosave_path(&project).exists(),
        "the file is left for a person"
    );
}

#[test]
#[cfg(unix)]
fn an_autosave_that_cannot_be_read_is_left_alone_and_reported() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    let mut edited = mix.clone();
    edited.tracks[0].gain = Decibels(-4.0);
    dermixen_app::autosave::write(&project, &edited).unwrap();
    let file = autosave_path(&project);
    let readable = std::fs::metadata(&file).unwrap().permissions();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    // A superuser reads the file whatever its mode, and then the file is
    // offered back as it would be otherwise. The rule here concerns a file
    // the window cannot read.
    if std::fs::read(&file).is_err() {
        match offered(&project, &mix) {
            Offer::Unreadable(message) => {
                assert!(message.contains("set.dmx.autosave"), "{message}");
            }
            other => panic!("expected the file to be reported, got {other:?}"),
        }
    }
    std::fs::set_permissions(&file, readable).unwrap();
    assert!(file.exists(), "the file is left for a person");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        edited.to_json(),
        "the unsaved work is untouched"
    );
}

#[test]
fn no_autosave_means_nothing_to_offer() {
    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    assert_eq!(offered(&project, &mix), Offer::Nothing);
}

#[test]
fn forgetting_removes_the_file_and_forgives_its_absence() {
    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    dermixen_app::autosave::write(&project, &mix).unwrap();
    forget(&project).unwrap();
    assert!(!autosave_path(&project).exists());
    forget(&project).unwrap();
    assert_eq!(files_in(dir.path()), vec!["set.dmx".to_owned()]);
}

#[test]
fn writing_leaves_no_partial_file_and_a_failure_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let (project, mix) = project_in(dir.path());
    dermixen_app::autosave::write(&project, &mix).unwrap();
    assert_eq!(
        files_in(dir.path()),
        vec!["set.dmx".to_owned(), "set.dmx.autosave".to_owned()]
    );

    let missing = dir.path().join("no/such/folder/set.dmx");
    let problem = dermixen_app::autosave::write(&missing, &mix).unwrap_err();
    assert!(problem.contains("set.dmx.autosave"), "{problem}");
}

#[test]
fn an_atomic_write_replaces_the_whole_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("set.dmx");
    write_atomically(&path, "one").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
    write_atomically(&path, "two").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
    assert_eq!(files_in(dir.path()), vec!["set.dmx".to_owned()]);

    let missing = dir.path().join("no/such/folder/set.dmx");
    let problem = write_atomically(&missing, "three").unwrap_err();
    assert!(problem.contains("set.dmx"), "{problem}");
    assert_eq!(files_in(dir.path()), vec!["set.dmx".to_owned()]);
}

#[test]
fn an_untitled_mix_is_kept_in_the_data_folder_and_offered_back_unless_empty() {
    use dermixen_app::{forget_file, offered_untitled, untitled_autosave_path};

    let data = Path::new("/Users/dermixenuser/Library/Application Support");
    assert_eq!(
        untitled_autosave_path(data),
        PathBuf::from("/Users/dermixenuser/Library/Application Support/dermixen/untitled.autosave")
    );

    let dir = tempfile::tempdir().unwrap();
    let file = untitled_autosave_path(dir.path());
    assert_eq!(
        offered_untitled(&file),
        Offer::Nothing,
        "no file, nothing to offer"
    );

    // The empty mix protects nothing, so the file is removed.
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    write_atomically(&file, &Mix::new().to_json()).unwrap();
    assert_eq!(offered_untitled(&file), Offer::Nothing);
    assert!(!file.exists(), "an autosave of the empty mix is removed");

    // A mix with tracks in it is offered back, with no saved time, since an
    // untitled mix was never saved.
    let mix = Mix::from_json(&fixture_text()).unwrap();
    write_atomically(&file, &mix.to_json()).unwrap();
    match offered_untitled(&file) {
        Offer::Restore {
            mix: offered,
            autosaved,
            saved,
        } => {
            assert_eq!(offered, mix);
            assert!(autosaved.is_some());
            assert_eq!(saved, None);
        }
        other => panic!("expected the mix back, got {other:?}"),
    }
    assert!(file.exists(), "offering leaves the file where it is");

    // Text that is not a mix document is reported and left alone.
    std::fs::write(&file, "not a mix").unwrap();
    match offered_untitled(&file) {
        Offer::Unreadable(message) => {
            assert!(message.contains(&file.display().to_string()), "{message}");
        }
        other => panic!("expected the file to be reported, got {other:?}"),
    }
    assert!(file.exists());

    forget_file(&file).unwrap();
    assert!(!file.exists());
    forget_file(&file).unwrap();
}

#[cfg(unix)]
mod hardening {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use dermixen_app::{Offer, autosave_path, offered, write_atomically};
    use dermixen_core::Decibels;

    use super::{files_in, project_in};

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_save_does_not_write_through_a_link_planted_at_the_old_temporary_name() {
        let dir = tempfile::tempdir().unwrap();
        let (project, mix) = project_in(dir.path());
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "PRECIOUS").unwrap();
        symlink(&victim, dir.path().join("set.dmx.part")).unwrap();
        symlink(&victim, dir.path().join("set.dmx.autosave.part")).unwrap();

        write_atomically(&project, &mix.to_json()).unwrap();
        dermixen_app::autosave::write(&project, &mix).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"PRECIOUS");
        assert!(
            std::fs::symlink_metadata(&project)
                .unwrap()
                .file_type()
                .is_file()
        );
    }

    #[test]
    fn a_save_keeps_the_documents_permissions_and_an_autosave_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let (project, mix) = project_in(dir.path());
        std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomically(&project, &mix.to_json()).unwrap();
        assert_eq!(mode_of(&project), 0o600);

        std::fs::set_permissions(&project, std::fs::Permissions::from_mode(0o644)).unwrap();
        dermixen_app::autosave::write(&project, &mix).unwrap();
        assert_eq!(mode_of(&autosave_path(&project)), 0o600);
    }

    #[test]
    fn an_autosave_of_a_mix_no_document_may_hold_is_refused_and_the_last_one_stays() {
        let dir = tempfile::tempdir().unwrap();
        let (project, mut mix) = project_in(dir.path());
        mix.tracks[0].gain = Decibels(-3.0);
        dermixen_app::autosave::write(&project, &mix).unwrap();
        let kept = std::fs::read(autosave_path(&project)).unwrap();

        mix.tracks[0].gain = Decibels(f64::NEG_INFINITY);
        let said = dermixen_app::autosave::write(&project, &mix).unwrap_err();
        assert!(said.contains("gain_db"), "{said}");
        assert_eq!(std::fs::read(autosave_path(&project)).unwrap(), kept);
        assert_eq!(files_in(dir.path()), ["set.dmx", "set.dmx.autosave"]);
    }

    #[test]
    fn an_autosave_that_is_a_device_is_unreadable_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let (project, mix) = project_in(dir.path());
        symlink("/dev/zero", autosave_path(&project)).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(offered(&project, &mix)));
        let offer = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("the autosave was still being read after two seconds");
        assert!(matches!(offer, Offer::Unreadable(_)), "{offer:?}");
    }
}
