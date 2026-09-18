//! The two ways every crate touches a file a person cares about: a bounded
//! read that refuses anything but a regular file, and an atomic write.
//!
//! A path in a mix document, an environment variable, or a folder another
//! person can write to may name a device, a named pipe, or a symbolic link to
//! either. Reading `/dev/zero` never ends, opening a named pipe waits for a
//! writer, and a temporary file with a predictable name can be a link another
//! person planted. The functions here are how the app avoids all three.

use std::ffi::{OsStr, OsString};
use std::fs::{File, Permissions};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The largest mix document or autosave the app reads, in bytes.
pub const LARGEST_DOCUMENT: u64 = 16 * 1024 * 1024;

/// The largest settings file or playlist the app reads, in bytes.
pub const LARGEST_SETTINGS: u64 = 1024 * 1024;

/// How many names [`AtomicFile::create`] tries before it gives up. Each name
/// ends in 64 bits nobody can predict, so a name that is already taken is a
/// coincidence rather than something anybody can arrange twice.
const NAME_ATTEMPTS: u32 = 16;

/// The longest destination name a temporary name includes, in bytes. A
/// destination with a longer name is left out of the temporary name, so that
/// the temporary name stays well inside the 255 bytes a file name may be on
/// macOS and Linux.
const LONGEST_INCLUDED_NAME: usize = 120;

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
    let file = open_without_waiting(path).map_err(|source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let kind = file
        .metadata()
        .map_err(|source| ReadError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .file_type();
    if !kind.is_file() {
        return Err(ReadError::NotARegularFile {
            path: path.to_path_buf(),
        });
    }
    Ok(file)
}

/// Opens `path` for reading without waiting for anything.
///
/// On macOS and Linux the open asks for the non-blocking mode, which is what
/// makes a named pipe with no writer fail at once instead of waiting for one.
/// The mode changes nothing about reading a regular file.
#[cfg(unix)]
fn open_without_waiting(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

/// Opens `path` for reading on a system that has no non-blocking open.
#[cfg(not(unix))]
fn open_without_waiting(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

/// Reads a whole text file of at most `limit` bytes.
///
/// The file is opened as [`open_regular`] opens it, and no more than `limit`
/// bytes and one more are ever read, whatever size the file system states.
pub fn read_text(path: &Path, limit: u64) -> Result<String, ReadError> {
    let io = |source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    };
    let file = open_regular(path)?;
    // The read stops one byte past the limit. A file that reaches that byte is
    // over the limit, and the size the file system states is never trusted.
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(ReadError::TooLarge {
            path: path.to_path_buf(),
            limit,
        });
    }
    String::from_utf8(bytes).map_err(|error| {
        io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            error.utf8_error(),
        ))
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
    /// The open temporary file.
    file: File,
    /// Where the temporary file is.
    temporary: PathBuf,
    /// The file the temporary file replaces.
    destination: PathBuf,
    /// The permissions the destination has, when a file is already there.
    /// [`AtomicFile::commit`] puts these on the temporary file before the
    /// rename, so that a saved file keeps the permissions it had.
    kept_permissions: Option<Permissions>,
    /// Whether [`AtomicFile::commit`] has renamed the temporary file onto the
    /// destination, which is what stops `Drop` from removing a file that is
    /// no longer temporary.
    committed: bool,
}

impl AtomicFile {
    /// Starts a write that will replace `destination`.
    ///
    /// `private` decides the permissions of a destination that does not exist
    /// yet: read and write for the owner alone when it is true, and the
    /// process's default when it is false.
    pub fn create(destination: &Path, private: bool) -> std::io::Result<AtomicFile> {
        let Some(name) = destination.file_name() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{} does not name a file", destination.display()),
            ));
        };
        let folder = folder_of(destination);
        let kept_permissions = std::fs::metadata(destination)
            .ok()
            .map(|data| data.permissions());

        for _ in 0..NAME_ATTEMPTS {
            let temporary = folder.join(temporary_name(name));
            match create_new(&temporary, private) {
                Ok(file) => {
                    return Ok(AtomicFile {
                        file,
                        temporary,
                        destination: destination.to_path_buf(),
                        kept_permissions,
                        committed: false,
                    });
                }
                // Another file already has the name, which is either a
                // coincidence or a name somebody guessed. The next name is
                // drawn afresh either way.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::other(format!(
            "cannot find an unused temporary name beside {}",
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
    pub fn commit(mut self) -> std::io::Result<()> {
        self.file.sync_all()?;
        if let Some(permissions) = self.kept_permissions.clone() {
            std::fs::set_permissions(&self.temporary, permissions)?;
        }
        std::fs::rename(&self.temporary, &self.destination)?;
        self.committed = true;
        // The rename is a change to the folder rather than to either file, so
        // the folder is flushed as well. A file system that does not flush a
        // folder answers with an error, and the save still stands.
        if let Ok(folder) = File::open(folder_of(&self.destination)) {
            let _ = folder.sync_all();
        }
        Ok(())
    }
}

impl Drop for AtomicFile {
    /// Removes the temporary file, unless [`AtomicFile::commit`] has already
    /// renamed it onto the destination.
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}

/// Replaces `destination` with `bytes` in one step, as [`AtomicFile`] does.
pub fn write_atomically(destination: &Path, private: bool, bytes: &[u8]) -> std::io::Result<()> {
    let writing = AtomicFile::create(destination, private)?;
    writing.file().write_all(bytes)?;
    writing.commit()
}

/// The folder `path` is in, as a path a file can be made in. A path with no
/// folder in it is in the folder the process is running in.
fn folder_of(path: &Path) -> &Path {
    match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    }
}

/// Creates a file that nothing else has the name of, and opens it for reading
/// and writing.
///
/// The file is new or the call fails, which is what keeps a write from
/// following a symbolic link somebody planted at the name. `private` asks for
/// read and write for the owner alone.
#[cfg(unix)]
fn create_new(path: &Path, private: bool) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(if private { 0o600 } else { 0o666 })
        .open(path)
}

/// Creates a file that nothing else has the name of on a system with no
/// permission bits to ask for.
#[cfg(not(unix))]
fn create_new(path: &Path, _private: bool) -> std::io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

/// A name for the temporary file beside a destination named `destination_name`.
///
/// The name is hidden, names the destination so that a person who finds one
/// after a crash knows what it was, and ends in 64 bits nobody can predict.
fn temporary_name(destination_name: &OsStr) -> OsString {
    let mut name = OsString::from(".");
    if destination_name.as_encoded_bytes().len() <= LONGEST_INCLUDED_NAME {
        name.push(destination_name);
    } else {
        name.push("dermixen");
    }
    name.push(format!(".{:016x}.part", unpredictable()));
    name
}

/// A number another process cannot guess, drawn afresh on every call.
///
/// The standard library keys each [`RandomState`] with a random value the
/// operating system gave this process and a counter it bumps on every call,
/// so hashing anything at all through a fresh `RandomState` gives a number
/// that differs from call to call and from run to run. The clock, the process
/// id, and a counter of this process's own writes go through it, so that two
/// writes a moment apart cannot land on one name.
fn unpredictable() -> u64 {
    /// How many names this process has drawn.
    static DRAWN: AtomicU64 = AtomicU64::new(0);

    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0),
    );
    hasher.write_u64(DRAWN.fetch_add(1, Ordering::Relaxed));
    hasher.finish()
}
