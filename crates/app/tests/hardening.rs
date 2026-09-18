//! Tests for the rules that keep the window inside the document limits and
//! on the shared file helpers: the comparison of a decoded file's hash with
//! the document's, what the library panel says about a row it could not read,
//! where a relative library path lands, who may enter a folder the window
//! makes, and what the status line says about a document the transport
//! refused.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dermixen_app::audio::{AudioCache, Decoding, Finding, Reading, Wanted, not_the_track};
use dermixen_app::{absolute_against, make_folder, restart_note, skipped_note};
use dermixen_core::{ContentHash, Samples};
use dermixen_engine::{TransportState, TransportStatus};

/// How long a test waits for a thread. The fixtures decode in well under a
/// second, so a wait this long is a thread that is not answering.
const PATIENCE: Duration = Duration::from_secs(30);

/// Where the synthetic audio fixtures are.
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

/// An empty store for the audio a thread decodes.
fn cache() -> Arc<Mutex<AudioCache>> {
    Arc::new(Mutex::new(AudioCache::default()))
}

/// Waits for the reading of one track to finish, and answers what it found.
fn wait_for(decoding: &Decoding) -> Result<Arc<dermixen_media::Audio>, String> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        if let Some(read) = decoding.finished() {
            return read;
        }
        assert!(Instant::now() < deadline, "the file was still being read");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_file_whose_bytes_are_not_the_tracks_is_refused_by_the_path_that_names_it() {
    let path = fixture("sine-440-44k.wav");
    let hash = dermixen_media::hash_file(&path).expect("the fixture's hash");

    // The file the document names decodes as that track.
    let store = cache();
    let read = wait_for(&Decoding::start(
        hash,
        path.clone(),
        Arc::clone(&store),
        || {},
    ));
    assert!(read.is_ok(), "{read:?}");

    // Another recording at the same path is refused, and the message names
    // the path and says what is wrong with the file there.
    let other = ContentHash([7; 32]);
    let said = wait_for(&Decoding::start(other, path.clone(), cache(), || {})).unwrap_err();
    assert!(said.contains(&path.display().to_string()), "{said}");
    assert!(
        said.contains("is not the track the document names"),
        "{said}"
    );
    assert_eq!(
        not_the_track(Path::new("/music/goa/alpha.mp3")),
        "The file at /music/goa/alpha.mp3 is not the track the document names. Run `dermixen mix \
         relink` with the folders to search."
    );
}

#[test]
fn the_reading_thread_reports_the_hash_of_the_bytes_it_decoded() {
    let path = fixture("sine-440-44k.wav");
    let hash = dermixen_media::hash_file(&path).expect("the fixture's hash");
    // The thread is asked for another track's bytes at that path, which is
    // what the window compares: the hash it asked for against the hash of
    // what came back.
    let asked = ContentHash([9; 32]);
    let reading = Reading::start(
        vec![Wanted {
            hash: asked,
            path: path.clone(),
        }],
        None,
        cache(),
        || {},
    );
    let deadline = Instant::now() + PATIENCE;
    loop {
        for finding in reading.findings() {
            if let Finding::Overview {
                hash: named,
                decoded,
                ..
            } = finding
            {
                assert_eq!(named, asked, "the finding is keyed by what was asked for");
                assert_eq!(decoded, hash, "the bytes that were decoded are the file's");
                return;
            }
        }
        assert!(Instant::now() < deadline, "the thread sent no overview");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_panel_says_how_many_rows_it_could_not_read_and_what_puts_them_back() {
    assert_eq!(skipped_note(0), "");
    let one = skipped_note(1);
    assert!(
        one.starts_with("1 row of the library could not be read"),
        "{one}"
    );
    assert!(one.contains("Library > Scan music folder"), "{one}");
    let many = skipped_note(4);
    assert!(
        many.starts_with("4 rows of the library could not be read"),
        "{many}"
    );
    assert!(many.contains("Library > Scan music folder"), "{many}");
}

#[test]
fn a_relative_library_file_is_read_against_the_folder_the_window_was_started_in() {
    let here = Path::new("/music/sets");
    assert_eq!(
        absolute_against(here, Path::new("library.sqlite")),
        PathBuf::from("/music/sets/library.sqlite")
    );
    assert_eq!(
        absolute_against(here, Path::new("below/library.sqlite")),
        PathBuf::from("/music/sets/below/library.sqlite")
    );
    assert_eq!(
        absolute_against(here, Path::new("/elsewhere/library.sqlite")),
        PathBuf::from("/elsewhere/library.sqlite"),
        "a path in full names the file itself"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_the_window_makes_is_for_its_owner_alone_and_one_that_is_there_is_left_as_it_is() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let mode_of = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

    let data = dir.path().join("dermixen");
    let corrections = data.join("corrections");
    make_folder(&corrections).unwrap();
    assert_eq!(mode_of(&data), 0o700);
    assert_eq!(mode_of(&corrections), 0o700);

    // A folder somebody made before keeps the permissions that person gave
    // it, since the window changes no folder it did not make.
    let shared = dir.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
    make_folder(&shared).unwrap();
    assert_eq!(mode_of(&shared), 0o755);
}

/// A transport report with the count of restarts and the last word on a
/// replacement, which are the two fields the status line reads.
fn status(restarts: u64, last_restart: Option<&str>) -> TransportStatus {
    TransportStatus {
        state: TransportState::Playing,
        position: Samples(0),
        length: Samples(44_100),
        underruns: 0,
        reached: Samples(0),
        restarts,
        last_restart: last_restart.map(str::to_owned),
    }
}

#[test]
fn a_document_the_transport_refused_says_the_edit_was_not_applied() {
    // The count of restarts stands still for a document the transport turned
    // away, so what a person hears is no longer the mix in the window.
    let refused = restart_note(
        Some(&status(3, None)),
        &status(3, Some("tracks[0].gain_db: a level is at most 24 decibels")),
    )
    .expect("a refusal is worth saying");
    assert_eq!(
        refused,
        "The edit was not applied to the preview: tracks[0].gain_db: a level is at most 24 decibels"
    );

    // A replacement the render could not continue through starts the render
    // over, which the count reports, and the line says so.
    let started_over = restart_note(
        Some(&status(3, None)),
        &status(
            4,
            Some("the mix tempo at 6:32.1 changed from 135.90 to 135.87"),
        ),
    )
    .expect("a restart is worth saying");
    assert_eq!(
        started_over,
        "Started over: the mix tempo at 6:32.1 changed from 135.90 to 135.87"
    );

    assert_eq!(
        restart_note(Some(&status(3, Some("older"))), &status(3, None)),
        None,
        "a replacement the render went on through leaves nothing to say"
    );
    assert_eq!(restart_note(None, &status(0, None)), None);
}
