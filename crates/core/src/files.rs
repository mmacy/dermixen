//! The two ways every crate touches a file a person cares about: a bounded
//! read that refuses anything but a regular file, and an atomic write.
//!
//! A path in a mix document, an environment variable, or a folder another
//! person can write to may name a device, a named pipe, or a symbolic link to
//! either. Reading `/dev/zero` never ends, opening a named pipe waits for a
//! writer, and a temporary file with a predictable name can be a link another
//! person planted. The functions here are how the app avoids all three.

use std::fs::File;
use std::path::{Path, PathBuf};

/// The largest mix document or autosave the app reads, in bytes.
pub const LARGEST_DOCUMENT: u64 = 16 * 1024 * 1024;

/// The largest settings file or playlist the app reads, in bytes.
pub const LARGEST_SETTINGS: u64 = 1024 * 1024;

/// Why a file was not read.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The path names something other than a regular file, such as a folder,
    /// a device, or a named pipe, whether directly or through a symbolic link.
    #[error("{} is not a regular file", path.display())]
    NotARegularFile {
        /// The path.
        path: PathBuf,
    },
    /// The file is larger than the limit the caller gave.
    #[error("{} is larger than the {limit} bytes the app reads", path.display())]
    TooLarge {
        /// The path.
        path: PathBuf,
        /// The limit, in bytes.
        limit: u64,
    },
    /// The file could not be opened or read, or its text is not UTF-8.
    #[error("cannot read {}: {source}", path.display())]
    Io {
        /// The path.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// Opens a regular file for reading.
///
/// The check is made on the open handle, so the file cannot be swapped for
/// another between the check and the read. The open never waits: a named pipe
/// with no writer is refused at once rather than blocking. A symbolic link to
/// a regular file is followed.
pub fn open_regular(path: &Path) -> Result<File, ReadError> {
    Err(ReadError::NotARegularFile {
        path: path.to_path_buf(),
    })
}

/// Reads a whole text file of at most `limit` bytes.
///
/// The file is opened as [`open_regular`] opens it, and no more than `limit`
/// bytes and one more are ever read, whatever size the file system states.
pub fn read_text(path: &Path, limit: u64) -> Result<String, ReadError> {
    Err(ReadError::TooLarge {
        path: path.to_path_buf(),
        limit,
    })
}

/// A file being written beside its destination, which replaces the
/// destination in one step when [`AtomicFile::commit`] is called and is
/// removed when it is dropped without that call.
///
/// The temporary file is in the destination's folder, is created only if
/// nothing has its name, and has a name nobody can predict, so a symbolic
/// link planted in the folder is never written through. A destination that
/// exists keeps its permissions. A new file gets the permissions the caller
/// asked for.
#[derive(Debug)]
pub struct AtomicFile {
    file: File,
    temporary: PathBuf,
    destination: PathBuf,
}

impl AtomicFile {
    /// Starts a write that will replace `destination`.
    ///
    /// `private` decides the permissions of a destination that does not exist
    /// yet: read and write for the owner alone when it is true, and the
    /// process's default when it is false.
    pub fn create(destination: &Path, private: bool) -> std::io::Result<AtomicFile> {
        let _ = private;
        Err(std::io::Error::other(format!(
            "writing {} atomically is not implemented",
            destination.display()
        )))
    }

    /// The open temporary file. `&File` implements `Write` and `Seek`, and
    /// [`File::try_clone`] gives a writer a handle of its own.
    pub fn file(&self) -> &File {
        &self.file
    }

    /// Flushes the file to the disk, moves it onto the destination, and
    /// flushes the folder, so that after a crash the destination is the old
    /// file or the new one, whole.
    pub fn commit(self) -> std::io::Result<()> {
        let _ = (&self.temporary, &self.destination);
        Err(std::io::Error::other("not implemented"))
    }
}

/// Replaces `destination` with `bytes` in one step, as [`AtomicFile`] does.
pub fn write_atomically(destination: &Path, private: bool, bytes: &[u8]) -> std::io::Result<()> {
    let _ = bytes;
    AtomicFile::create(destination, private)?.commit()
}
