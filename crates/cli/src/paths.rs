//! Whether a command is about to write over one of the files it reads.
//!
//! `render`, `decode`, and `play --capture` replace whatever file the person
//! names, because a render is made again from its document. A mix document
//! and a track are not: replacing either with audio loses work that cannot
//! be made again. The check here runs before any of the three creates
//! anything, and it holds whether the output names the input directly,
//! through `..`, through a symbolic link, or through a hard link.

use std::path::{Path, PathBuf};

use crate::text::shown_path;

/// A file as this check compares one.
struct Place {
    /// The path with its folder resolved, so that a path written with `..`
    /// or through a linked folder has one spelling.
    at: PathBuf,
    /// Which file the operating system says is at the path: its device and
    /// its inode. A path with nothing at it has none, which is the case of
    /// an output that does not exist yet.
    file: Option<(u64, u64)>,
}

impl Place {
    /// Whether this place and `other` are the same file.
    ///
    /// Two paths that both name a file are the same file when the device and
    /// the inode match, which is what catches a hard link and a symbolic
    /// link. An output with nothing at it yet is compared by its resolved
    /// path instead.
    fn is(&self, other: &Place) -> bool {
        match (self.file, other.file) {
            (Some(one), Some(another)) => one == another,
            _ => self.at == other.at,
        }
    }
}

/// Where a path is and what is there.
fn place(path: &Path) -> Place {
    Place {
        at: resolved(path),
        file: identity(path),
    }
}

/// A path with its folder resolved and the file's own name kept.
///
/// A path that names a file is resolved whole, which follows a symbolic link
/// to the file it points at. A path with nothing at it has its folder
/// resolved and its name joined back on, which is how an output that does
/// not exist yet is compared. A path whose folder cannot be resolved either
/// is left as it was given.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    match (folder.canonicalize(), path.file_name()) {
        (Ok(folder), Some(name)) => folder.join(name),
        _ => path.to_path_buf(),
    }
}

/// The device and the inode of the file at `path`, or `None` when nothing is
/// there. A symbolic link is followed, so a link and the file it points at
/// give one answer.
#[cfg(unix)]
fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let data = std::fs::metadata(path).ok()?;
    Some((data.dev(), data.ino()))
}

/// The device and the inode of a file on a system that reports neither, where
/// the resolved path is what the comparison has.
#[cfg(not(unix))]
fn identity(_path: &Path) -> Option<(u64, u64)> {
    None
}

/// Refuses an output that is one of `inputs`, naming the input it is and
/// what that input is to the command.
///
/// Call this before the command creates anything. `inputs` holds files of
/// one kind, which `role` names in a few words, such as `the mix document`
/// or `a track of the mix`, so a command that reads two kinds of file calls
/// this once for each kind.
pub fn refuse_an_input(out: &Path, role: &str, inputs: &[&Path]) -> Result<(), String> {
    let output = place(out);
    for input in inputs {
        if output.is(&place(input)) {
            return Err(format!(
                "cannot write {}: that is {}, {role}, and dermixen writes over no file it reads. Give the output another name.",
                shown_path(out),
                shown_path(input)
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_named_a_second_way_is_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.wav");
        std::fs::write(&file, b"audio").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let through_dots = dir.path().join("sub/../a.wav");
        let message = refuse_an_input(&through_dots, "a track of the mix", &[&file]).unwrap_err();
        assert!(message.contains("a.wav"), "{message}");
    }

    #[test]
    fn a_name_nothing_is_at_yet_is_not_an_input() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.wav");
        std::fs::write(&file, b"audio").unwrap();
        let out = dir.path().join("out.wav");
        assert!(refuse_an_input(&out, "a track of the mix", &[&file]).is_ok());
        // An earlier render at that name is replaced, which is what render
        // is for.
        std::fs::write(&out, b"earlier").unwrap();
        assert!(refuse_an_input(&out, "a track of the mix", &[&file]).is_ok());
    }
}
