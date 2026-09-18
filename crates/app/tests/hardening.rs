//! Tests for the rules that keep the window inside the document limits and
//! on the shared file helpers: the comparison of a decoded file's hash with
//! the document's, which tracks reach the thread that reads them, what a
//! relink does to the history, who may enter a folder the window makes, and
//! what the status line says about a document the transport refused.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dermixen_app::audio::{AudioCache, Decoding, Wanted, not_the_track, to_be_read};
use dermixen_app::{Timeline, make_folder, restart_note};
use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Edit, Envelope, EqEnvelopes, Mix,
    Samples, Track,
};
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

/// A one-track mix whose track has `hash` and names `path`.
fn a_mix(hash: ContentHash, path: &Path) -> Mix {
    Mix {
        tracks: vec![Track {
            path: path.to_path_buf(),
            hash,
            length: Samples(44_100),
            grid: BeatGrid {
                first_beat: Samples::ZERO,
                bpm: Bpm(140.0),
            },
            anchors: Anchors {
                intro: Beats(0.0),
                outro: Beats(16.0),
            },
            keylock: true,
            gain: Decibels::UNITY,
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        }],
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
fn a_track_the_pass_relinks_is_read_at_its_new_path_and_only_once() {
    let hash = ContentHash([3; 32]);
    let gone = PathBuf::from("/music/goa/gone/lsd.mp3");
    let found = PathBuf::from("/music/goa/kept/lsd.mp3");
    let mut handed = HashSet::new();

    // While the pass has not placed the track, the window has no file for it
    // and hands the reading thread nothing, so nothing is read at the path
    // the document named.
    let with_no_file = HashSet::from([hash]);
    assert!(to_be_read(&a_mix(hash, &gone), &with_no_file, &mut handed).is_empty());
    assert!(handed.is_empty(), "a track with no file is not recorded");

    // The pass found the file, so the track is handed over at the path the
    // pass wrote, once.
    let relinked = a_mix(hash, &found);
    assert_eq!(
        to_be_read(&relinked, &HashSet::new(), &mut handed),
        vec![Wanted {
            hash,
            path: found.clone()
        }]
    );
    assert!(
        to_be_read(&relinked, &HashSet::new(), &mut handed).is_empty(),
        "a track already handed to the thread is not handed again"
    );

    // One file at two positions of the playlist is read once.
    let twice = Mix {
        tracks: vec![relinked.tracks[0].clone(), relinked.tracks[0].clone()],
    };
    let mut fresh = HashSet::new();
    assert_eq!(to_be_read(&twice, &HashSet::new(), &mut fresh).len(), 1);
}

#[test]
fn pointing_a_track_at_another_file_leaves_the_history_where_it_was() {
    let hash = ContentHash([4; 32]);
    let gone = PathBuf::from("/music/goa/gone/lsd.mp3");
    let found = PathBuf::from("/music/goa/kept/lsd.mp3");
    let mut timeline = Timeline::new(a_mix(hash, &gone));
    timeline
        .apply(Edit::SetKeylock {
            track: 0,
            keylock: false,
        })
        .expect("the track is in the playlist");

    let moved = HashMap::from([(hash, found.clone())]);
    assert!(timeline.set_paths(&moved));
    assert_eq!(timeline.mix().tracks[0].path, found);
    assert!(!timeline.mix().tracks[0].keylock);

    // The relink is no step of its own: the undo takes back the keylock, and
    // the document it takes the window to names the file that was found.
    assert!(timeline.can_undo());
    assert!(timeline.undo());
    assert!(timeline.mix().tracks[0].keylock);
    assert_eq!(
        timeline.mix().tracks[0].path,
        found,
        "the path the pass found stands through an undo"
    );
    assert!(timeline.redo());
    assert_eq!(timeline.mix().tracks[0].path, found);
    assert!(!timeline.mix().tracks[0].keylock);

    assert!(
        !timeline.set_paths(&moved),
        "a track already at that path is no change"
    );
    assert!(
        !timeline.set_paths(&HashMap::from([(ContentHash([9; 32]), gone)])),
        "a hash the mix does not hold changes nothing"
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
