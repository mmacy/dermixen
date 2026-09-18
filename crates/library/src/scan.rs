//! Finding the audio files under a folder.

use std::path::{Path, PathBuf};

/// The extensions of the files a scan treats as audio, in lowercase. A file's
/// extension is compared in either case.
pub const AUDIO_EXTENSIONS: [&str; 5] = ["wav", "mp3", "flac", "m4a", "mp4"];

/// What a scan may leave out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanOptions {
    /// Folders not to enter. Each is a path relative to the scan root, or an
    /// absolute path; a folder is left out when its path is one of these or
    /// lies under one of them. A folder of finished mixes is the usual
    /// entry, so finished mixes never surface as source tracks.
    pub exclude: Vec<PathBuf>,
}

/// A folder the scan could not read and therefore passed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// The folder.
    pub path: PathBuf,
    /// Why it could not be read.
    pub reason: String,
}

/// What a scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scanned {
    /// Every audio file found, in ascending order of path.
    pub files: Vec<PathBuf>,
    /// Every folder the scan could not read. The files inside are not in
    /// `files`, and nothing else is affected.
    pub unreadable: Vec<Unreadable>,
}

/// The reason a scan could not start.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// The root is not a folder, or there is nothing at that path.
    #[error("{0} is not a folder, or does not exist")]
    NotAFolder(PathBuf),
    /// The root could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The root.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// Lists every audio file under `root`.
///
/// A file counts as audio by its extension, compared with
/// [`AUDIO_EXTENSIONS`] in either case. Every folder under the root is
/// entered except the excluded ones; symbolic links are not followed. Files
/// come back as the root joined with their path below it, in ascending order
/// of path, so two scans of an unchanged folder give the same list. A folder
/// that cannot be read is reported in [`Scanned::unreadable`] and the scan
/// goes on; only a root that is not a readable folder is an error.
pub fn scan(root: &Path, options: &ScanOptions) -> Result<Scanned, ScanError> {
    match std::fs::metadata(root) {
        Ok(about) if about.is_dir() => {}
        Ok(_) => return Err(ScanError::NotAFolder(root.to_path_buf())),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => {
            return Err(ScanError::NotAFolder(root.to_path_buf()));
        }
        Err(problem) => {
            return Err(ScanError::Read {
                path: root.to_path_buf(),
                source: problem,
            });
        }
    }

    // An exclusion given as a relative path is read against the root; an
    // absolute one stands on its own. Both are compared with the paths the
    // walk builds from the root, which is why neither side is canonicalized.
    let excluded: Vec<PathBuf> = options
        .exclude
        .iter()
        .map(|folder| {
            if folder.is_absolute() {
                folder.clone()
            } else {
                root.join(folder)
            }
        })
        .collect();

    let mut files = Vec::new();
    let mut unreadable = Vec::new();
    let walk = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !entry.file_type().is_dir()
                || !excluded
                    .iter()
                    .any(|folder| entry.path().starts_with(folder))
        });
    for step in walk {
        match step {
            // A symbolic link has its own file type, so a link to a folder is
            // neither entered nor listed, and a link to a file is passed over.
            Ok(entry) => {
                if entry.file_type().is_file() && is_audio(entry.path()) {
                    files.push(entry.into_path());
                }
            }
            Err(problem) => {
                let path = problem.path().unwrap_or(root).to_path_buf();
                let reason = match problem.io_error() {
                    Some(cause) => cause.to_string(),
                    None => problem.to_string(),
                };
                if problem.depth() == 0 {
                    // The root itself could not be read, so there is nothing
                    // to scan, and the scan reports it to the caller as an error.
                    let source = problem
                        .into_io_error()
                        .unwrap_or_else(|| std::io::Error::other(reason));
                    return Err(ScanError::Read { path, source });
                }
                unreadable.push(Unreadable { path, reason });
            }
        }
    }
    files.sort();
    unreadable.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(Scanned { files, unreadable })
}

/// Whether a file's extension is one of [`AUDIO_EXTENSIONS`], in either case.
fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            let extension = extension.to_ascii_lowercase();
            AUDIO_EXTENSIONS.contains(&extension.as_str())
        })
}
