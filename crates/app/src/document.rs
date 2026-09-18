//! Which file the mix under the window is, whether it has changes that
//! were never saved, and what the window asks before it lets those changes
//! go.
//!
//! The window opens with an untitled empty mix when it is given no file,
//! and **File** offers **New**, **Open**, **Save**, **Save as**, and
//! **Quit**. Every one of those that would put another document under the
//! window, or close it, first asks what should become of unsaved changes,
//! because never losing work outranks every feature in `DESIGN.md`. This
//! type is those rules, tested without a window: the window paints the
//! title it gives and the question it asks, and reports the answers.

use std::path::{Path, PathBuf};

/// The name the window shows for a mix that has never been saved.
pub const UNTITLED: &str = "Untitled";

/// The words that follow the name in the window's title while the mix has
/// changes that were never saved.
pub const EDITED: &str = " (edited)";

/// What the person asked the window to do, which unsaved changes may stand
/// in the way of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Put an untitled empty mix under the window.
    New,
    /// Put the mix in this file under the window.
    Open(PathBuf),
    /// Close the window.
    Quit,
}

/// What the window does with an intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Nothing stands in the way: act on the intent.
    Proceed(Intent),
    /// The mix has unsaved changes, so the window asks what should become
    /// of them, and the intent waits for the answer.
    Ask(Intent),
}

/// The three answers to "Save changes to this mix?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Save, then act on the intent. A save that fails leaves the
    /// question standing.
    Save,
    /// Let the changes go and act on the intent. The window's button for
    /// this answer reads **Don't save**.
    Discard,
    /// Do nothing: the mix stays as it is, with its changes.
    Cancel,
}

/// What the window does after an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Save the mix, and act on the intent once the save has succeeded.
    /// The window asks for a file first when the mix has none.
    SaveThen(Intent),
    /// Act on the intent now.
    Proceed(Intent),
    /// Nothing: the question is withdrawn and the mix keeps its changes.
    Stay,
}

/// How far the intent waiting on the question has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    /// The window is asking, and no answer has come yet.
    Question,
    /// The answer was [`Answer::Save`], and the save has not succeeded yet.
    Save,
    /// The save succeeded, so the intent is there for the window to take.
    Ready,
}

/// The file the mix under the window came from, whether the mix has
/// changes that were never saved, and the intent waiting on the answer to
/// the question about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    path: Option<PathBuf>,
    unsaved: bool,
    pending: Option<(Intent, Waiting)>,
}

impl Document {
    /// A mix that has never been saved and has no file.
    pub fn untitled() -> Document {
        Document {
            path: None,
            unsaved: false,
            pending: None,
        }
    }

    /// The mix in the file at `path`, as it was read, with nothing to save.
    pub fn at(path: PathBuf) -> Document {
        Document {
            path: Some(path),
            unsaved: false,
            pending: None,
        }
    }

    /// The file the mix came from, or `None` for an untitled mix.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The file's name, or [`UNTITLED`] for a mix with no file. A path that
    /// ends in no file name is named by the whole path, since a person needs
    /// to read which mix the window holds whatever the path looks like.
    pub fn name(&self) -> String {
        let Some(path) = &self.path else {
            return UNTITLED.to_owned();
        };
        match path.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => path.display().to_string(),
        }
    }

    /// The window's title: `Dermixen · ` and the name, then [`EDITED`]
    /// while the mix has changes that were never saved.
    ///
    /// The window asks for this on every repaint and compares it with the
    /// title the windowing system is showing, and it sends the windowing
    /// system this title when the two differ, so an edit, a save, and a
    /// document coming in each change the title at once.
    pub fn title(&self) -> String {
        let edited = if self.unsaved { EDITED } else { "" };
        format!("Dermixen \u{b7} {}{edited}", self.name())
    }

    /// Whether the mix has changes that were never saved.
    pub fn is_unsaved(&self) -> bool {
        self.unsaved
    }

    /// Whether the window is asking what should become of unsaved changes.
    pub fn asking(&self) -> bool {
        self.pending.is_some()
    }

    /// The intent waiting on the answer, while the window is asking.
    pub fn pending(&self) -> Option<&Intent> {
        self.pending.as_ref().map(|(intent, _)| intent)
    }

    /// An edit was committed, so the mix now has changes that were never
    /// saved.
    ///
    /// The window calls this for every edit the timeline commits, and for a
    /// document the relinking pass changed as it opened. The autosave file
    /// is a separate thing: the window writes the whole document to it
    /// straight after this call, so an exit the window never sees costs at
    /// most the edit in progress.
    pub fn edited(&mut self) {
        self.unsaved = true;
    }

    /// The mix was written to its own file, so nothing is unsaved.
    ///
    /// This also moves an intent that was waiting for a save on to where
    /// [`take_pending`](Document::take_pending) hands it back, which is what
    /// makes **Save** in the question go on to the **New**, the **Open**, or
    /// the **Quit** it was answered for.
    pub fn saved(&mut self) {
        self.unsaved = false;
        self.save_succeeded();
    }

    /// The mix was written to `path`, which is its file from now on, and
    /// nothing is unsaved.
    pub fn saved_as(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.saved();
    }

    /// Takes an intent: it proceeds at once when nothing is unsaved, and
    /// otherwise waits while the window asks. A request made while the
    /// window is already asking replaces the intent that was waiting, so
    /// the newest request is the one the answer applies to.
    ///
    /// On [`Step::Proceed`] the caller does what the intent names. On
    /// [`Step::Ask`] the caller does nothing with the intent and draws the
    /// question, which [`asking`](Document::asking) reports for as long as
    /// it stands, and [`answered`](Document::answered) says what happens
    /// next.
    pub fn request(&mut self, intent: Intent) -> Step {
        if !self.unsaved {
            return Step::Proceed(intent);
        }
        self.pending = Some((intent.clone(), Waiting::Question));
        Step::Ask(intent)
    }

    /// Takes the answer to the question and says what the window does now.
    /// After [`Next::Proceed`] and [`Next::Stay`] the window is no longer
    /// asking. After [`Next::SaveThen`] the intent goes on waiting until
    /// [`saved`](Document::saved) or [`saved_as`](Document::saved_as)
    /// is called, and [`take_pending`](Document::take_pending) then gives
    /// it back, so a save that fails leaves the question where it was.
    /// An answer while nothing is pending is [`Next::Stay`].
    pub fn answered(&mut self, answer: Answer) -> Next {
        let Some((intent, _)) = self.pending.clone() else {
            return Next::Stay;
        };
        match answer {
            Answer::Save => {
                self.pending = Some((intent.clone(), Waiting::Save));
                Next::SaveThen(intent)
            }
            Answer::Discard => {
                self.pending = None;
                Next::Proceed(intent)
            }
            Answer::Cancel => {
                self.pending = None;
                Next::Stay
            }
        }
    }

    /// The intent that was waiting for a save, once the save has succeeded:
    /// `Some` exactly once after a [`Next::SaveThen`] whose save was
    /// followed by [`saved`](Document::saved) or
    /// [`saved_as`](Document::saved_as), and `None` at every other time.
    pub fn take_pending(&mut self) -> Option<Intent> {
        if self.pending.as_ref()?.1 != Waiting::Ready {
            return None;
        }
        self.pending.take().map(|(intent, _)| intent)
    }

    /// Moves an intent that was waiting for a save on to where
    /// [`take_pending`](Document::take_pending) hands it back.
    ///
    /// A save the question did not ask for, such as the command key with S,
    /// leaves an intent waiting on the question where it is, so no save
    /// acts on an intent that the person has not answered for.
    fn save_succeeded(&mut self) {
        if let Some((_, waiting)) = &mut self.pending
            && *waiting == Waiting::Save
        {
            *waiting = Waiting::Ready;
        }
    }
}
