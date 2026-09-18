//! Finding the files of a mix again after they have moved.
//!
//! A mix document names each track by a path and by the hash of the file's
//! bytes, and `docs/project-file.md` says which is which: the path is where
//! the file was last seen and the hash is its identity. Relinking is what
//! turns that into the graceful handling of moved files `DESIGN.md` asks for
//! under "Reliability plumbing": every track whose file is not where the
//! document says, or is not the file the document names, is looked for by
//! its hash, first in the library and then under folders the person names,
//! and the document is pointed at wherever the file turns up.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use dermixen_core::{ContentHash, Mix};
use dermixen_media::hash_file;

use crate::index::{Index, IndexError, TrackRecord};
use crate::scan::{ScanError, ScanOptions, scan};

/// What relinking did with one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relink {
    /// The file is where the document says, and its bytes are the ones the
    /// document names.
    Kept,
    /// A file with the track's bytes was found elsewhere, and the document
    /// now names it there.
    Relinked {
        /// The path the document named before.
        from: PathBuf,
        /// The path it names now.
        to: PathBuf,
    },
    /// No file with the track's bytes was found. The document still names
    /// the path it did.
    Missing {
        /// The path the document names.
        path: PathBuf,
        /// Where the file was looked for, in words for a person.
        reason: String,
    },
}

/// What relinking did with a mix: one entry per track, in playlist order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Relinked {
    /// One entry per track of the mix.
    pub tracks: Vec<Relink>,
}

impl Relinked {
    /// How many tracks now name a new path.
    pub fn relinked(&self) -> usize {
        self.tracks
            .iter()
            .filter(|track| matches!(track, Relink::Relinked { .. }))
            .count()
    }

    /// How many tracks still have no file.
    pub fn missing(&self) -> usize {
        self.tracks
            .iter()
            .filter(|track| matches!(track, Relink::Missing { .. }))
            .count()
    }
}

/// The reason relinking could not finish.
#[derive(Debug, thiserror::Error)]
pub enum RelinkError {
    /// The library could not be read or written.
    #[error(transparent)]
    Index(#[from] IndexError),
    /// A folder given to search could not be scanned at all.
    #[error(transparent)]
    Scan(#[from] ScanError),
}

/// Points every track of `mix` whose file is missing or changed at a file
/// with the track's bytes, and says what became of each track.
///
/// Every folder in `under` is listed with [`scan`](crate::scan) and the
/// default options before any track is looked at, so a folder that cannot be
/// scanned at all is an error even when every file is in place. A folder
/// below one of those roots that cannot be read is passed over as a scan
/// passes it over. Then each track in turn: a track whose path reads and
/// hashes to the hash the document names is kept, and nothing else is looked
/// at for it. For any other track the search goes, in order, to the library
/// record for the hash, when a library file is given and that record's path
/// reads and hashes to the same hash, since the library's own path may be out
/// of date, and then to the listed files of each folder in the order the scan
/// gave them, until one hashes to the track's hash. A library row that cannot
/// be read answers nothing and the search goes on to the folders, so one
/// damaged row costs no other track. A file under the folders
/// is hashed at most once in one call, however many tracks are missing, and
/// `progress` is called with each such file before it is hashed, so a caller
/// can say what is being read.
///
/// A track that is found is relinked in the document with the path exactly as
/// the library record named it or as the scan listed it, which is the root
/// joined with the file's path below it. The `mix relink` command is what
/// makes that path canonical before it writes the document. The library, when
/// one is given, is pointed at the file too. A track that is not found is
/// missing, with a reason that names the folders searched or, when `under` is
/// empty, says that no folder was given. A track whose own file is at the
/// path the document names and could not be read, as an unmounted volume or a
/// permission leaves it, is searched for like any other, and its reason names
/// that path and the reason the operating system gave before it says where
/// else was searched, since mounting the volume or changing the permission is
/// what that person needs to do. The document is changed only where a track
/// is relinked. A library that cannot be read or written is an error.
pub fn relink(
    mix: &mut Mix,
    mut index: Option<&mut Index>,
    under: &[PathBuf],
    progress: &mut dyn FnMut(&Path),
) -> Result<Relinked, RelinkError> {
    // The folders are listed before any track is looked at, so a folder that
    // cannot be scanned stops the call whether or not a file is missing.
    let mut search = Search::list(under)?;
    let searched = places_searched(index.is_some(), under);

    let mut tracks = Vec::with_capacity(mix.tracks.len());
    for track in &mut mix.tracks {
        // A file that is there and cannot be read is searched for like a file
        // that is gone, and the reason a missing track ends up with names that
        // failure, because the remedy for it is not the one relinking offers.
        let unreadable = match look_at(&track.path, track.hash) {
            AtThePath::TheTrack => {
                tracks.push(Relink::Kept);
                continue;
            }
            AtThePath::NotTheTrack => None,
            AtThePath::Unreadable(problem) => Some(problem),
        };

        // The library record names where a scan last saw the bytes, and that
        // path can be as out of date as the document's, so this crate hashes
        // the file there before it believes the record.
        let mut found = None;
        if let Some(index) = index.as_deref()
            && let Some(record) = readable_record(index, track.hash)?
            && contains(&record.path, track.hash)
        {
            found = Some(record.path);
        }
        if found.is_none() {
            found = search.find(track.hash, progress);
        }

        match found {
            Some(to) => {
                if let Some(index) = index.as_deref_mut() {
                    index.set_path(track.hash, &to)?;
                }
                let from = std::mem::replace(&mut track.path, to.clone());
                tracks.push(Relink::Relinked { from, to });
            }
            None => tracks.push(Relink::Missing {
                path: track.path.clone(),
                reason: match &unreadable {
                    Some(problem) => format!(
                        "{} could not be read: {problem}, and {searched}",
                        track.path.display()
                    ),
                    None => searched.clone(),
                },
            }),
        }
    }
    Ok(Relinked { tracks })
}

/// The library's record for a hash, with a row the library cannot read
/// counting as no record.
///
/// Relinking looks for the bytes of one track in several places, and the
/// library is the first. A row that cannot be read is one place that answers
/// nothing, and the search goes on to the folders, so one damaged row costs
/// that track its shortcut rather than costing every track of the mix the
/// whole pass. A scan of the file's folder is what replaces such a row.
fn readable_record(index: &Index, hash: ContentHash) -> Result<Option<TrackRecord>, IndexError> {
    match index.get(hash) {
        Err(IndexError::Record { .. }) => Ok(None),
        other => other,
    }
}

/// Whether the file at `path` can be read and contains the bytes `hash`
/// names.
///
/// Every failure to read the file answers false, whatever the failure was:
/// no file at the path, a permission that stops the read, an unmounted
/// volume, or a device that reports an error. The question is whether the
/// bytes are there to be used, and in all four cases they are not.
fn contains(path: &Path, hash: ContentHash) -> bool {
    hash_file(path).is_ok_and(|found| found == hash)
}

/// What the file at the path a track names turned out to be.
enum AtThePath {
    /// The file is there and contains the bytes the document names.
    TheTrack,
    /// There is no file at the path, or the file there contains other bytes.
    NotTheTrack,
    /// A file is at the path and could not be read, with the reason the
    /// operating system gave.
    Unreadable(String),
}

/// Reads the file at the path a track names and says which of the three
/// [`AtThePath`] answers it gives.
fn look_at(path: &Path, hash: ContentHash) -> AtThePath {
    match hash_file(path) {
        Ok(found) if found == hash => AtThePath::TheTrack,
        Ok(_) => AtThePath::NotTheTrack,
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => AtThePath::NotTheTrack,
        Err(problem) => AtThePath::Unreadable(problem.to_string()),
    }
}

/// The files under the folders a caller named, hashed one at a time as the
/// search for a track's bytes reaches them.
///
/// The files are listed once for the whole call, and each file is hashed the
/// first time the search reaches it. What a file's bytes turned out to be is
/// kept by hash, so a track whose file the search has already gone past is
/// matched without reading anything again, and no file is hashed twice in
/// one call.
struct Search {
    /// Every file under the folders, in the order the scans gave them, with
    /// no path twice.
    files: Vec<PathBuf>,
    /// How many of those files have been hashed.
    next: usize,
    /// The first file found to contain each set of bytes.
    known: HashMap<ContentHash, PathBuf>,
}

impl Search {
    /// Lists the files under every folder in `under`, in the order the
    /// folders were given.
    fn list(under: &[PathBuf]) -> Result<Search, ScanError> {
        let options = ScanOptions::default();
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for root in under {
            // A folder given twice, or one that lies under a folder given
            // earlier, would otherwise list its files a second time.
            for path in scan(root, &options)?.files {
                if seen.insert(path.clone()) {
                    files.push(path);
                }
            }
        }
        Ok(Search {
            files,
            next: 0,
            known: HashMap::new(),
        })
    }

    /// The first file under the folders whose bytes are the ones `hash`
    /// names, or `None` when no file there contains those bytes.
    ///
    /// A file that cannot be read is passed over, since the search is for the
    /// bytes and another file may still contain them.
    fn find(&mut self, hash: ContentHash, progress: &mut dyn FnMut(&Path)) -> Option<PathBuf> {
        if let Some(path) = self.known.get(&hash) {
            return Some(path.clone());
        }
        while self.next < self.files.len() {
            let path = self.files[self.next].clone();
            self.next += 1;
            progress(&path);
            if let Ok(found) = hash_file(&path) {
                self.known.entry(found).or_insert_with(|| path.clone());
                if found == hash {
                    return Some(path);
                }
            }
        }
        None
    }
}

/// Where a missing track's file was looked for, in words for a person.
fn places_searched(index: bool, under: &[PathBuf]) -> String {
    match (index, folder_list(under)) {
        (true, Some(folders)) => format!("searched the library and {folders}"),
        (true, None) => "searched the library, and no folder was given to search".to_owned(),
        (false, Some(folders)) => format!("searched {folders}"),
        (false, None) => "no folder was given to search".to_owned(),
    }
}

/// The folders as one phrase, or `None` when the caller named none: one
/// folder on its own, two joined by "and", and three or more as a list.
fn folder_list(under: &[PathBuf]) -> Option<String> {
    let named: Vec<String> = under
        .iter()
        .map(|folder| folder.display().to_string())
        .collect();
    match named.len() {
        0 => None,
        1 => Some(named[0].clone()),
        2 => Some(format!("{} and {}", named[0], named[1])),
        _ => {
            let (last, rest) = named.split_last().expect("the list has three or more");
            Some(format!("{}, and {last}", rest.join(", ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dermixen_core::{
        Anchors, BeatGrid, Beats, Bpm, Decibels, Envelope, EqEnvelopes, Samples, Track,
    };

    /// A mix of one track naming `path`, with a hash no file in the test has.
    fn one_track(path: &Path) -> Mix {
        Mix {
            tracks: vec![Track {
                path: path.to_path_buf(),
                hash: ContentHash([7; 32]),
                length: Samples(44_100),
                grid: BeatGrid {
                    first_beat: Samples::ZERO,
                    bpm: Bpm(130.0),
                },
                anchors: Anchors {
                    intro: Beats::ZERO,
                    outro: Beats(64.0),
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
    fn a_track_whose_file_is_not_there_gets_the_plain_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut mix = one_track(&dir.path().join("gone.wav"));
        let done = relink(&mut mix, None, &[], &mut |_| {}).unwrap();
        match &done.tracks[0] {
            Relink::Missing { reason, .. } => {
                assert_eq!(reason, "no folder was given to search");
            }
            other => panic!("expected the track to be missing, got {other:?}"),
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_track_whose_file_cannot_be_read_says_so_before_it_says_where_it_looked() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        std::fs::write(&path, b"some bytes").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            // A user who reads a file whatever its mode, like the root
            // user, cannot bring this about, so there is nothing to check.
            return;
        }

        let mut mix = one_track(&path);
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        let done = relink(
            &mut mix,
            None,
            std::slice::from_ref(&elsewhere),
            &mut |_| {},
        )
        .unwrap();
        match &done.tracks[0] {
            Relink::Missing { reason, .. } => {
                assert!(reason.starts_with(&path.display().to_string()), "{reason}");
                assert!(reason.contains("could not be read"), "{reason}");
                assert!(reason.contains("elsewhere"), "{reason}");
            }
            other => panic!("expected the track to be missing, got {other:?}"),
        }
        // The document still names the path, so the person can mount the
        // volume or change the permission and render without relinking.
        assert_eq!(mix.tracks[0].path, path);
    }
}
