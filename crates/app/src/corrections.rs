//! The correction loop: an anchor a person moves, or a grid a person fixes,
//! written out as ground truth in the format `docs/ground-truth.md`
//! describes, so that analyzers are measured against it from then on. The
//! timeline writes one whenever it applies an anchor move or a grid change.

use std::path::{Path, PathBuf};

use dermixen_core::Track;
use dermixen_core::files::write_atomically;

/// Writes the track's grid and anchors as an annotation file in `dir` and
/// returns its path.
///
/// The file is named after the audio file without its extension, then a
/// hyphen, then the first eight hexadecimal digits of the track's content
/// hash, then `.anchors`, so that two tracks of one mix that share a file
/// name in different folders keep separate corrections: a track called
/// `a.mp3` whose hash begins with four bytes of one is written as
/// `a-01010101.anchors`. Writing a second correction for the same track
/// replaces the first.
///
/// The file holds, one per line and in this order: a comment line saying
/// the file was written by the Dermixen timeline; `file` with the track's
/// path as the document holds it; `bpm` with the grid's tempo; `first_beat`
/// with beat zero in seconds; and `intro` and `outro` with each anchor in
/// seconds followed by the source `ear`, since a person placed them.
/// Seconds are written with six decimals, the tempo as Rust's default
/// formatting of a float writes it, without trailing zeros, and the file
/// ends with one newline after the last line.
pub fn write_correction(dir: &Path, track: &Track) -> std::io::Result<PathBuf> {
    // The empty path, the root, and a path that ends in `..` have no stem; a
    // path that ends in a separator does, because Rust's path type looks
    // past the separator to the name before it. Where there is no stem the
    // whole path stands in for it, so the correction still goes somewhere
    // predictable.
    let stem = track
        .path
        .file_stem()
        .unwrap_or_else(|| track.path.as_os_str());
    let hash = track.hash.0;
    let mut name = stem.to_os_string();
    name.push(format!(
        "-{:02x}{:02x}{:02x}{:02x}.anchors",
        hash[0], hash[1], hash[2], hash[3]
    ));
    let path = dir.join(name);

    let grid = track.grid;
    let text = format!(
        "# Written by the Dermixen timeline from a correction made by hand.\n\
         file {}\n\
         bpm {}\n\
         first_beat {:.6}\n\
         intro {:.6} ear\n\
         outro {:.6} ear\n",
        track.path.display(),
        grid.bpm.0,
        grid.first_beat.to_seconds().0,
        grid.time_of(track.anchors.intro).0,
        grid.time_of(track.anchors.outro).0,
    );
    // The file is replaced in one step, so a correction that fails partway
    // leaves the last one whole rather than half a file the scoreboard would
    // read as ground truth.
    write_atomically(&path, false, text.as_bytes())?;
    Ok(path)
}
