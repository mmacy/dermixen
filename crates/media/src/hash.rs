//! Hashing a file's bytes to identify it independently of its path.

use std::io::Read;
use std::path::Path;

use dermixen_core::ContentHash;
use dermixen_core::files::{ReadError, open_regular};

use crate::LARGEST_AUDIO_FILE;

/// The BLAKE3 hash of every byte of the file at `path`.
///
/// The hash is the same wherever the file is moved or however it is renamed,
/// and different as soon as one byte changes, including a tag edit.
///
/// The path must name a regular file, opened as
/// [`dermixen_core::files::open_regular`] opens one, so a device or a named
/// pipe is an error at once rather than a read that never ends. A file larger
/// than [`LARGEST_AUDIO_FILE`](crate::LARGEST_AUDIO_FILE) is an error too,
/// found when the read passes that size.
pub fn hash_file(path: &Path) -> std::io::Result<ContentHash> {
    let file = open_regular(path).map_err(as_io_error)?;
    let mut hasher = blake3::Hasher::new();
    // The file is consumed in blocks, so a file far larger than memory is
    // still hashed whole. The read stops one byte past the limit, and a file
    // that reaches that byte is over the limit.
    let mut bounded = file.take(LARGEST_AUDIO_FILE.saturating_add(1));
    hasher.update_reader(&mut bounded)?;
    if bounded.limit() == 0 {
        return Err(std::io::Error::other(format!(
            "{} is larger than the {LARGEST_AUDIO_FILE} bytes Dermixen reads from an audio file",
            path.display()
        )));
    }
    Ok(ContentHash(*hasher.finalize().as_bytes()))
}

/// Turns the reason a file was not opened into the `std::io::Error` that
/// [`hash_file`] and [`decode`](crate::decode) report. A path that names a
/// folder, a device, or a named pipe is an input the caller gave, so that
/// path reports as bad input.
pub(crate) fn as_io_error(error: ReadError) -> std::io::Error {
    match error {
        ReadError::Io { source, .. } => source,
        other => std::io::Error::new(std::io::ErrorKind::InvalidInput, other.to_string()),
    }
}
