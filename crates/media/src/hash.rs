//! Hashing a file's bytes to identify it independently of its path.

use std::fs::File;
use std::path::Path;

use dermixen_core::ContentHash;

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
    let file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    // The reader is consumed in blocks, so a file far larger than memory is
    // still hashed whole.
    hasher.update_reader(file)?;
    Ok(ContentHash(*hasher.finalize().as_bytes()))
}
