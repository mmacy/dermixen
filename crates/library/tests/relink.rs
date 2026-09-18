//! Acceptance tests for relinking: finding a mix's files again by their
//! bytes after they have moved. A coder agent makes these pass without editing
//! them.

use std::fs;
use std::path::{Path, PathBuf};

use dermixen_analysis::Extent;
use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, Decibels, Envelope, EqEnvelopes, Mix, Samples, Seconds, Track,
};
use dermixen_library::{
    Index, Metadata, MetadataSource, Release, Relink, RelinkError, TrackRecord, relink,
};
use dermixen_media::{WavDepth, hash_file, write_wav};
use dermixen_testkit::synth;

/// Writes twenty seconds of kicks at `bpm` to `relative` under `dir`, so
/// two files at different tempos have different bytes.
fn kicks_file(dir: &Path, relative: &str, bpm: f64) -> PathBuf {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let audio = synth::kicks(Bpm(bpm), Seconds(0.5), Seconds(20.0));
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    path
}

/// A track of a mix that names `path` and the hash of its bytes.
fn track_of(path: &Path) -> Track {
    Track {
        path: path.to_path_buf(),
        hash: hash_file(path).unwrap(),
        length: Samples(20 * 44_100),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(130.0),
        },
        anchors: Anchors {
            intro: Beats(0.0),
            outro: Beats(64.0),
        },
        keylock: true,
        gain: Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    }
}

/// An index record for the bytes of `track`, naming `path`, which need not
/// be where the file is.
fn record_of(track: &Track, path: &Path) -> TrackRecord {
    TrackRecord {
        hash: track.hash,
        path: path.to_path_buf(),
        length: track.length,
        grid: track.grid,
        grid_confidence: 0.9,
        grid_analyzer: "given".to_owned(),
        key: None,
        extent: Extent {
            begins: Samples::ZERO,
            ends: track.length,
        },
        anchors: track.anchors,
        anchor_confidence: 0.5,
        anchor_analyzer: "edges".to_owned(),
        metadata: Metadata {
            artist: None,
            title: Some("kicks".to_owned()),
            year: None,
            year_is_approximate: false,
            source: MetadataSource::Filename,
        },
        phrases: None,
        loudness: None,
        release: Release::default(),
    }
}

/// A library folder with two tracks and a mix naming both.
fn a_mix(root: &Path) -> Mix {
    let a = kicks_file(root, "lib/a.wav", 130.0);
    let b = kicks_file(root, "lib/b.wav", 140.0);
    Mix {
        tracks: vec![track_of(&a), track_of(&b)],
    }
}

fn nothing(_: &Path) {}

#[test]
fn tracks_whose_files_are_where_the_document_says_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let before = mix.clone();
    let done = relink(&mut mix, None, &[], &mut nothing).unwrap();
    assert_eq!(done.tracks, vec![Relink::Kept, Relink::Kept]);
    assert_eq!((done.relinked(), done.missing()), (0, 0));
    assert_eq!(mix, before);
}

#[test]
fn a_moved_file_is_found_through_the_index_when_the_index_knows_where_it_went() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let old = mix.tracks[0].path.clone();
    let new = dir.path().join("moved/a.wav");
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::rename(&old, &new).unwrap();
    // A scan after the move would have pointed the record at the new path.
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    index.upsert(&record_of(&mix.tracks[0], &new)).unwrap();

    let done = relink(&mut mix, Some(&mut index), &[], &mut nothing).unwrap();
    assert_eq!(
        done.tracks,
        vec![
            Relink::Relinked {
                from: old,
                to: new.clone()
            },
            Relink::Kept
        ]
    );
    assert_eq!((done.relinked(), done.missing()), (1, 0));
    assert_eq!(mix.tracks[0].path, new);
}

#[test]
fn an_index_path_that_is_also_stale_is_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let old = mix.tracks[0].path.clone();
    let stale = dir.path().join("gone/a.wav");
    let new = kicks_file(dir.path(), "elsewhere/a.wav", 130.0);
    fs::remove_file(&old).unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    index.upsert(&record_of(&mix.tracks[0], &stale)).unwrap();

    // The index names a path with nothing there, so the folder is searched,
    // and the index is pointed at the file too.
    let done = relink(
        &mut mix,
        Some(&mut index),
        &[dir.path().join("elsewhere")],
        &mut nothing,
    )
    .unwrap();
    assert_eq!(
        done.tracks[0],
        Relink::Relinked {
            from: old,
            to: new.clone()
        }
    );
    assert_eq!(mix.tracks[0].path, new);
    assert_eq!(index.get(mix.tracks[0].hash).unwrap().unwrap().path, new);

    // An index record whose path contains other bytes is not trusted either.
    let mut mix = a_mix(dir.path());
    let old = mix.tracks[1].path.clone();
    let decoy = kicks_file(dir.path(), "decoy/b.wav", 125.0);
    let new = kicks_file(dir.path(), "elsewhere/b.wav", 140.0);
    fs::remove_file(&old).unwrap();
    index.upsert(&record_of(&mix.tracks[1], &decoy)).unwrap();
    let done = relink(
        &mut mix,
        Some(&mut index),
        &[dir.path().join("elsewhere")],
        &mut nothing,
    )
    .unwrap();
    assert_eq!(
        done.tracks[1],
        Relink::Relinked {
            from: old,
            to: new.clone()
        }
    );
    assert_eq!(index.get(mix.tracks[1].hash).unwrap().unwrap().path, new);
}

#[test]
fn a_file_is_found_under_a_folder_by_its_bytes_whatever_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let (old_a, old_b) = (mix.tracks[0].path.clone(), mix.tracks[1].path.clone());
    let new_a = dir.path().join("elsewhere/renamed.wav");
    let new_b = dir.path().join("elsewhere/deeper/also renamed.wav");
    fs::create_dir_all(new_b.parent().unwrap()).unwrap();
    fs::rename(&old_a, &new_a).unwrap();
    fs::rename(&old_b, &new_b).unwrap();
    // A decoy that sorts first has to be hashed and passed over.
    let decoy = kicks_file(dir.path(), "elsewhere/aaa decoy.wav", 125.0);
    fs::write(dir.path().join("elsewhere/notes.txt"), "not audio").unwrap();

    let mut hashed = Vec::new();
    let done = relink(
        &mut mix,
        None,
        &[dir.path().join("elsewhere")],
        &mut |path| hashed.push(path.to_path_buf()),
    )
    .unwrap();
    assert_eq!(
        done.tracks,
        vec![
            Relink::Relinked {
                from: old_a,
                to: new_a.clone()
            },
            Relink::Relinked {
                from: old_b,
                to: new_b.clone()
            }
        ]
    );
    assert_eq!(mix.tracks[0].path, new_a);
    assert_eq!(mix.tracks[1].path, new_b);
    // Every audio file under the folder was hashed at most once, the text
    // file not at all, and the decoy first.
    assert_eq!(hashed[0], decoy);
    let mut once = hashed.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), hashed.len(), "{hashed:?}");
    assert!(hashed.iter().all(|path| path.extension().unwrap() == "wav"));
}

#[test]
fn a_track_whose_bytes_are_nowhere_is_missing_with_the_search_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let gone = mix.tracks[0].path.clone();
    fs::remove_file(&gone).unwrap();
    let elsewhere = dir.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    kicks_file(dir.path(), "elsewhere/other.wav", 125.0);
    let before = mix.clone();

    let done = relink(
        &mut mix,
        None,
        std::slice::from_ref(&elsewhere),
        &mut nothing,
    )
    .unwrap();
    match &done.tracks[0] {
        Relink::Missing { path, reason } => {
            assert_eq!(path, &gone);
            assert!(reason.contains("elsewhere"), "{reason}");
        }
        other => panic!("expected the track to be missing, got {other:?}"),
    }
    assert_eq!(done.tracks[1], Relink::Kept);
    assert_eq!((done.relinked(), done.missing()), (0, 1));
    assert_eq!(mix, before, "the document is left as it was");

    // With nowhere to look, the reason says so.
    let done = relink(&mut mix, None, &[], &mut nothing).unwrap();
    match &done.tracks[0] {
        Relink::Missing { reason, .. } => {
            assert!(reason.contains("no folder"), "{reason}");
        }
        other => panic!("expected the track to be missing, got {other:?}"),
    }
}

#[test]
#[cfg(unix)]
fn a_file_that_is_there_but_cannot_be_read_says_so_in_its_reason() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let path = mix.tracks[0].path.clone();
    let readable = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    // A superuser reads the file whatever its mode, and the track is then
    // kept. The rule here concerns a file the library cannot read.
    if fs::read(&path).is_err() {
        let done = relink(&mut mix, None, &[], &mut nothing).unwrap();
        match &done.tracks[0] {
            Relink::Missing {
                path: named,
                reason,
            } => {
                assert_eq!(named, &path);
                let where_the_path_is = reason.find(path.to_str().unwrap()).unwrap();
                let where_the_search_is = reason.find("no folder").unwrap();
                assert!(
                    where_the_path_is < where_the_search_is,
                    "the path and the operating system's reason come first: {reason}"
                );
                assert!(reason.contains("could not be read"), "{reason}");
            }
            other => panic!("expected the track to be missing, got {other:?}"),
        }
        assert_eq!(mix.tracks[0].path, path, "the document is left as it was");
    }
    fs::set_permissions(&path, readable).unwrap();
}

#[test]
fn a_file_replaced_by_other_bytes_is_found_where_its_bytes_went() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    let path = mix.tracks[0].path.clone();
    let new = dir.path().join("elsewhere/a.wav");
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::copy(&path, &new).unwrap();
    // Something else now sits at the document's path.
    fs::copy(&mix.tracks[1].path, &path).unwrap();

    let done = relink(
        &mut mix,
        None,
        &[dir.path().join("elsewhere")],
        &mut nothing,
    )
    .unwrap();
    assert_eq!(
        done.tracks[0],
        Relink::Relinked {
            from: path,
            to: new.clone()
        }
    );
    assert_eq!(mix.tracks[0].path, new);
    assert_eq!(hash_file(&mix.tracks[0].path).unwrap(), mix.tracks[0].hash);
}

#[test]
fn two_tracks_of_one_file_are_both_relinked() {
    let dir = tempfile::tempdir().unwrap();
    let a = kicks_file(dir.path(), "lib/a.wav", 130.0);
    let mut mix = Mix {
        tracks: vec![track_of(&a), track_of(&a)],
    };
    let new = dir.path().join("elsewhere/a.wav");
    fs::create_dir_all(new.parent().unwrap()).unwrap();
    fs::rename(&a, &new).unwrap();
    let done = relink(
        &mut mix,
        None,
        &[dir.path().join("elsewhere")],
        &mut nothing,
    )
    .unwrap();
    assert_eq!(done.relinked(), 2);
    assert!(mix.tracks.iter().all(|track| track.path == new));
}

#[test]
fn a_folder_that_cannot_be_scanned_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut mix = a_mix(dir.path());
    fs::remove_file(&mix.tracks[0].path).unwrap();
    match relink(&mut mix, None, &[dir.path().join("nowhere")], &mut nothing) {
        Err(RelinkError::Scan(problem)) => {
            assert!(problem.to_string().contains("nowhere"), "{problem}");
        }
        other => panic!("expected a scan error, got {other:?}"),
    }
}
