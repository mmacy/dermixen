//! The autosave file: what the window writes after every committed edit,
//! and what it offers back when the mix is opened again. A mix that has a
//! project file is protected by a file beside that project file, and an
//! untitled mix by one file in the user's data folder.
//!
//! `DESIGN.md` puts "never loses work" among the two values that outrank
//! every feature. Not every way out of the window asks the person anything:
//! the quit keystroke on macOS reaches the application rather than the
//! window, and a machine that loses power asks nobody. So the window keeps
//! the whole document in a file beside the project file after every edit,
//! and the next opening of the mix offers that document back when it says
//! something the project file does not. `docs/window.md` describes what the
//! person sees. This module is the rules, which are tested without a window.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dermixen_core::Mix;

/// The folder below the user's data folder that Dermixen keeps its own files
/// in, which `docs/cli.md` names for the library file.
const DATA_FOLDER: &str = "dermixen";

/// The name of the autosave file for a mix that has no file of its own,
/// below the `dermixen` folder in the user's data folder.
const UNTITLED_AUTOSAVE: &str = "untitled.autosave";

/// The autosave file beside `project`: the project's path with `.autosave`
/// appended to its file name, so `set.dmx` is protected by `set.dmx.autosave`
/// in the same folder.
pub fn autosave_path(project: &Path) -> PathBuf {
    beside(project, ".autosave")
}

/// The autosave file for a mix that has no file of its own: `untitled.autosave`
/// in the `dermixen` folder below `data_folder`, which is the user's data
/// folder, so an untitled mix is kept the same way as a named one.
///
/// `dirs::data_dir` gives the folder to pass, and a user for whom it gives
/// nothing has no such file at all. The folder is not made here, so a caller
/// about to write the file makes the folder first. For a mix that has a file
/// of its own, [`autosave_path`] names the file to write instead, and
/// [`offered_untitled`] is what reads back what this one names.
pub fn untitled_autosave_path(data_folder: &Path) -> PathBuf {
    data_folder.join(DATA_FOLDER).join(UNTITLED_AUTOSAVE)
}

/// Reads the untitled autosave file at `file` and says what it contains.
/// An untitled mix has nothing saved, so any mix document the file
/// contains other than the empty mix is offered back, with no `saved`
/// time, and a file that contains the empty mix is removed and nothing is
/// offered. Everything else is as [`offered`] has it.
///
/// [`untitled_autosave_path`] names the file to pass. Call this whenever an
/// untitled mix comes under the window, which is a launch with no mix named
/// and every **New**, and call [`offered`] instead for a mix that has a file
/// of its own, since that one has a project file to compare against. A
/// [`Offer::Restore`] leaves the file where it is, so the caller removes it
/// with [`forget_file`] once the person has said what should become of the
/// document it holds.
pub fn offered_untitled(file: &Path) -> Offer {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => {
            return Offer::Nothing;
        }
        Err(problem) => {
            return Offer::Unreadable(format!("cannot read {}: {problem}", file.display()));
        }
    };
    let autosaved = std::fs::metadata(file)
        .and_then(|about| about.modified())
        .ok();
    match Mix::from_json(&text) {
        Ok(mix) if mix == Mix::new() => {
            let _ = std::fs::remove_file(file);
            Offer::Nothing
        }
        Ok(mix) => Offer::Restore {
            mix,
            autosaved,
            // An untitled mix was never saved, so there is no project file
            // whose age the dialog could set beside the autosave file's.
            saved: None,
        },
        Err(problem) => Offer::Unreadable(format!(
            "{} contains unsaved changes that could not be read as a mix document: {problem}",
            file.display()
        )),
    }
}

/// The temporary file [`write_atomically`] writes through, beside `path`:
/// the whole name with `.part` appended.
fn temporary_path(path: &Path) -> PathBuf {
    beside(path, ".part")
}

/// A path with `suffix` appended to its whole name rather than substituted
/// for its extension, which leaves the new path in the same folder as
/// `path`.
fn beside(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// What the autosave file beside a project file contains for the person to
/// decide about.
#[derive(Debug, Clone, PartialEq)]
pub enum Offer {
    /// There is nothing to offer: either there is no autosave file, or the
    /// autosave file contained the same document as the project file and has
    /// been removed, since it protected nothing.
    Nothing,
    /// The autosave file contains a document that differs from the project
    /// file's.
    Restore {
        /// The autosaved document.
        mix: Mix,
        /// When the autosave file was last written, when the file system
        /// says.
        autosaved: Option<SystemTime>,
        /// When the project file was last written, when the file system
        /// says.
        saved: Option<SystemTime>,
    },
    /// The autosave file exists but cannot be read at all, or contains text
    /// that is not a mix document. Either way it has been left where it is,
    /// so that whatever it contains can be looked at by hand, and this is
    /// the message for the status line, which names the file and the
    /// problem.
    Unreadable(String),
}

/// Reads the autosave file beside `project` and says what it contains, given
/// `current`, the document the project file contains.
///
/// Documents are compared, not bytes: a project file that `dermixen mix add`
/// has rewritten with the same document in different bytes contains the same
/// document, and so does an autosave written by another build with other
/// spacing. An autosave that contains the current document is removed and
/// nothing is offered.
///
/// A missing autosave file is nothing to offer. One that exists but cannot
/// be read, because a permission is denied or a mount has gone away, is left
/// where it is and reported, the same as one that cannot be parsed as a mix
/// document: a person who is not told stands to lose the file's contents to
/// the next edit, which overwrites it.
pub fn offered(project: &Path, current: &Mix) -> Offer {
    let file = autosave_path(project);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => {
            return Offer::Nothing;
        }
        Err(problem) => {
            return Offer::Unreadable(format!("cannot read {}: {problem}", file.display()));
        }
    };
    let autosaved = std::fs::metadata(&file)
        .and_then(|about| about.modified())
        .ok();
    match Mix::from_json(&text) {
        Ok(mix) if mix == *current => {
            let _ = std::fs::remove_file(&file);
            Offer::Nothing
        }
        Ok(mix) => {
            let saved = std::fs::metadata(project)
                .and_then(|about| about.modified())
                .ok();
            Offer::Restore {
                mix,
                autosaved,
                saved,
            }
        }
        Err(problem) => Offer::Unreadable(format!(
            "{} contains unsaved changes that could not be read as a mix document: {problem}",
            file.display()
        )),
    }
}

/// Writes `mix` to the autosave file beside `project`, as
/// [`write_atomically`] writes it.
pub fn write(project: &Path, mix: &Mix) -> Result<(), String> {
    write_atomically(&autosave_path(project), &mix.to_json())
}

/// Removes the autosave file beside `project`, once the project file contains
/// the same document or the person has said what should become of the
/// changes. A file that is not there is already forgotten, and is not an
/// error.
pub fn forget(project: &Path) -> Result<(), String> {
    forget_file(&autosave_path(project))
}

/// Removes the autosave file at `file`, whichever mix it protected. A file
/// that is not there is already forgotten, and is not an error.
pub fn forget_file(file: &Path) -> Result<(), String> {
    match std::fs::remove_file(file) {
        Ok(()) => Ok(()),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(problem) => Err(format!("cannot remove {}: {problem}", file.display())),
    }
}

/// Writes `text` to `path` so that the file contains either what it contained
/// before or the whole of `text`, never part of it.
///
/// The text goes to a temporary file beside `path`, named after it with
/// `.part` appended, the bytes are flushed from the machine's write cache to
/// the device, and only then is that file renamed over `path`, which a file
/// system carries out in one step. A disk that fills up or a machine that
/// stops partway therefore leaves `path` as it was. A write that fails
/// removes its temporary file and says why, naming the path.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    let temporary = temporary_path(path);
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        // Without this the rename can reach the device before the bytes do,
        // and a machine that stops in between would leave an empty file
        // where the document was.
        file.sync_all()
    })();
    if let Err(problem) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("cannot write {}: {problem}", path.display()));
    }
    if let Err(problem) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("cannot write {}: {problem}", path.display()));
    }
    Ok(())
}
