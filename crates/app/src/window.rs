//! The window: the state one open mix needs, the controls above the
//! timeline, and the routing of the mouse and the keyboard to the
//! view-models.
//!
//! Nothing here changes the mix document on its own. Every change goes
//! through [`Timeline`], which records it in a history that undo and redo
//! walk. The window's part is to paint what the timeline describes, to hand
//! it the mouse and the keyboard, and to take the requests the timeline
//! leaves behind afterwards and carry them out on the transport.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use dermixen_analysis::{Camelot, Letter};
use dermixen_app::{
    Answer, Column, Document, Filters, Finish, GridEditor, GridScene, GridView, Intent,
    LibraryPanel, LibraryRow, MAX_LANE_PX, MIN_LANE_PX, Next, Offer, Playback, Point, Report, Scan,
    Selection, Sort, Step, TAPS_FOR_A_TEMPO, TempoField, Timeline, TransportOrder, View,
    WheelGesture, arrow_step, autosave, autosave_path, forget_file, offered, offered_untitled,
    open_the_library, parse_buffer_frames, read_the_mix, untitled_autosave_path, wheel_gesture,
};
use dermixen_core::{
    BeatGrid, Beats, Bpm, ContentHash, Curve, Edit, Mix, Preset, Samples, Seconds, Settings,
    TempoNode,
};
use dermixen_engine::{
    Audition, AuditionState, Transport, TransportState, TransportStatus, mix_length,
};
use dermixen_library::{
    Change, Index, IndexError, PhraseRecord, Query, Relink, RelinkError, ScanSummary, TrackRecord,
    relink,
};
use dermixen_media::{Audio, Overview};
use egui::{Color32, Key, KeyboardShortcut, Modifiers, Sense, Vec2};
use egui_extras::{Column as TableColumn, TableBuilder};

use dermixen_app::audio::{self, AudioCache, Decoding, Finding, Reading, Wanted};

use crate::paint::{self, NoFile};

/// The tallest a lane the person has not resized is fitted to, so that a
/// mix of two tracks does not fill the window with two enormous lanes. A
/// lane resized by a drag of its bottom edge is as tall as the drag left
/// it, up to [`dermixen_app::MAX_LANE_PX`].
const FITTED_LANE_PX: f32 = 120.0;

/// How tall the ruler above the lanes is.
const RULER_HEIGHT: f32 = 20.0;

/// How far the pointer must move after a press before the window treats the
/// press as a drag, so that a plain click on a node never nudges it.
const DRAG_PX: f32 = 2.0;

/// How much the zoom buttons narrow or widen the view.
const ZOOM_STEP: f64 = 1.15;

/// The shortcut for **New**. egui's command modifier is the command key on
/// macOS and the control key on Linux, so every shortcut below reads as a
/// person on either machine expects, and the menu shows it their way.
const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);

/// The shortcut for **Open...**.
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);

/// The shortcut for **Save**.
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);

/// The shortcut for **Save as...**.
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);

/// The shortcut for **Quit**.
const QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);

/// The shortcut for **Undo**.
const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);

/// The shortcut for **Redo**.
const REDO: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);

/// The shortcut for **Settings...**.
const SETTINGS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma);

/// The shortcut for **Zoom in**.
const ZOOM_IN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Equals);

/// The shortcut for **Zoom out**.
const ZOOM_OUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus);

/// The shortcut for **Whole mix**.
const WHOLE_MIX: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Num0);

/// The shortcut for **Show library** and **Hide library**, which are the two
/// names of one item.
const LIBRARY_PANEL: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::L);

/// How tall the grid editor's waveform strip is when the window opens,
/// which is tall enough to see a kick stand out from the bar around it. A
/// drag of the strip's bottom edge makes it anywhere from [`MIN_LANE_PX`]
/// to [`MAX_LANE_PX`] tall.
const GRID_STRIP_PX: f32 = 140.0;

/// How tall the band under the grid editor's strip is, which is the handle
/// the strip's bottom edge is dragged by. The band is under the strip
/// rather than over its last rows, so a press that resizes the strip never
/// reaches the strip, where a press slides the grid.
const STRIP_EDGE_PX: f32 = 6.0;

/// Whether a press at a point on the timeline would take hold of a lane's
/// bottom edge, which is what the pointer shows the vertical resize arrow
/// over.
///
/// [`Timeline::edge_at`] names the lane whose bottom edge lies under the
/// point whatever else lies there, and [`Timeline::hit`] names nothing
/// where a press would take hold of that edge, so the two answers together
/// tell an edge from a node, an anchor, or a tempo node drawn beside one.
fn takes_hold_of_an_edge(timeline: &Timeline, at: Point) -> bool {
    timeline.edge_at(at).is_some() && timeline.hit(at) == Selection::Nothing
}

/// How wide the library panel beside the timeline is when the window opens,
/// which is wide enough for the table's ten columns. The panel can be
/// dragged wider or narrower by its edge.
const LIBRARY_WIDTH_PX: f32 = 800.0;

/// How many digits after the point the master tempo field and the selected
/// tempo node's field show.
const TEMPO_DECIMALS: usize = 1;

/// How many digits after the point the grid editor's tempo field shows. A
/// beat grid is judged over a whole track, where a thousandth of a beat per
/// minute is a beat's worth of drift by the end, so the field shows more of
/// the number than the mix tempo fields do.
const GRID_DECIMALS: usize = 3;

/// How long the window waits before repainting while the preview runs, which
/// keeps the playhead moving smoothly without repainting faster than a
/// screen shows.
const PLAYING_REPAINT: Duration = Duration::from_millis(16);

/// How long the status line shows why the render last started over on a
/// replacement, which is long enough to read beside the buffering it caused
/// and short enough to be gone before the next edit.
const RESTART_SHOWN: Duration = Duration::from_secs(5);

/// How long the window leaves between two readings of the library while a
/// scan adds tracks to it, which is short enough that the panel fills in as
/// the scan goes and long enough that reading a library of thousands of
/// tracks again does not take the window's whole repaint.
const REFRESH_GAP: Duration = Duration::from_secs(1);

/// The folder below the user's data folder that Dermixen keeps its own files
/// in, which `docs/cli.md` names for the library file.
const DATA_FOLDER: &str = "dermixen";

/// The folder every anchor move and grid fix is written to.
const CORRECTIONS_FOLDER: &str = "corrections";

/// The environment variable that names the library file.
const LIBRARY_VARIABLE: &str = "DERMIXEN_LIBRARY_FILE";

/// Where the corrections a person makes are written: `dermixen/corrections`
/// in the user's data folder, made if it is not there yet.
fn corrections_dir() -> Result<PathBuf, String> {
    let data = dirs::data_dir().ok_or_else(|| {
        "There is no data folder for this user, so corrections cannot be written".to_owned()
    })?;
    let dir = data.join(DATA_FOLDER).join(CORRECTIONS_FOLDER);
    std::fs::create_dir_all(&dir)
        .map_err(|problem| format!("Cannot make the folder {}: {problem}", dir.display()))?;
    Ok(dir)
}

/// Which library file the window opens: the `DERMIXEN_LIBRARY_FILE`
/// environment variable, else the `library_file` setting, else
/// `dermixen/library.sqlite` in the user's data folder. That is the order
/// the `dermixen` command uses after its own `--library` option, which the
/// window does not have.
///
/// A user with no data folder gets the sentence the status line shows, since
/// the default file lies below that folder. A user with no home folder gets
/// that sentence only when the `library_file` setting is a relative path,
/// which is the one case where the home folder is needed to work the library
/// file out. The music folder is another matter: its default lies below the
/// home folder, so a user with no home folder can open a library and still
/// be told, when a scan starts, that the music folder cannot be worked out.
fn library_location(settings: &Settings) -> Result<PathBuf, String> {
    if let Some(path) = named_library_file() {
        return Ok(path);
    }
    let data = dirs::data_dir().ok_or_else(|| {
        format!(
            "There is no data folder for this user to keep the library file in, so name one with \
             {LIBRARY_VARIABLE} or with the library_file setting."
        )
    })?;
    // Only a setting that is a relative path is worked out below the home
    // folder, so a user with no home folder still gets the default file and
    // a setting written as a path in full.
    let home = match settings.library_file.as_deref() {
        Some(setting) if setting.is_relative() && !setting.as_os_str().is_empty() => home_folder()?,
        _ => PathBuf::new(),
    };
    Ok(settings.library_file(&home, &data))
}

/// The library file the `DERMIXEN_LIBRARY_FILE` environment variable names,
/// while the variable is set to something. A variable set to empty text
/// counts as unset, as it does for the `dermixen` command.
fn named_library_file() -> Option<PathBuf> {
    std::env::var_os(LIBRARY_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The user's home folder, which the music folder and a relative library
/// file setting are worked out below.
fn home_folder() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| {
        "There is no home folder for this user, so a path below the home folder cannot be worked \
         out. Write music_folder and library_file as paths in full."
            .to_owned()
    })
}

/// The folder the window scans: the `music_folder` setting, else
/// `Music/Undefunktis` below the user's home folder. The folder need not
/// exist, and the caller says what to do when it does not.
fn music_folder(settings: &Settings) -> Result<PathBuf, String> {
    Ok(settings.music_folder(&home_folder()?))
}

/// The music folder as a scan reads it: `named` with the links along it
/// resolved and made absolute, which is what `dermixen library scan` stores.
/// A folder reached through a symbolic link is therefore stored under one
/// spelling whichever of the two scans reads it, and a later scan reports
/// its tracks as unchanged rather than moved.
///
/// A path with nothing at it, a path that names something other than a
/// folder, and a folder the operating system cannot resolve each come back
/// as a sentence for the status line, which the caller puts the remedy
/// after.
fn folder_to_scan(named: &Path) -> Result<PathBuf, String> {
    if !named.is_dir() {
        return Err(match named.exists() {
            true => format!("{} is not a folder.", named.display()),
            false => format!("There is no folder at {} to scan.", named.display()),
        });
    }
    named.canonicalize().map_err(|problem| {
        format!(
            "The folder {} could not be read: {problem}.",
            named.display()
        )
    })
}

/// Which library file the window has open, and what it took to open it.
struct LibraryFile {
    /// The file, or nothing when the window could work out no path or could
    /// make no file.
    file: Option<PathBuf>,
    /// Whether the window made the file, which is what starts the scan of
    /// the music folder that fills a library nobody has scanned into yet.
    made: bool,
    /// What the status line says about the library file, empty when there
    /// is nothing to say.
    message: String,
}

/// Works out which library file to open, makes it and the folders above it
/// when it is not there yet, and says which file it is.
///
/// The window opens the library this way for every mix it puts under itself,
/// so a `library_file` setting changed in the settings dialog takes effect
/// the next time a mix is opened or a new one started.
fn library_file_for(settings: &Settings) -> LibraryFile {
    let path = match library_location(settings) {
        Ok(path) => path,
        Err(problem) => {
            return LibraryFile {
                file: None,
                made: false,
                message: problem,
            };
        }
    };
    // The library file is made by opening it, which is what a scan does
    // before it lists the folder, so the window and the scan make the file
    // the same way. The index closes again at the end of this call, since
    // the window opens the library afresh for every read.
    let made = !path.exists();
    match open_the_library(&path) {
        Ok(_) => LibraryFile {
            file: Some(path),
            made,
            message: String::new(),
        },
        Err(problem) => LibraryFile {
            file: None,
            made: false,
            message: capitalized(problem),
        },
    }
}

/// What a scan of `folder` did, in one sentence for the status line: the
/// counts of every outcome, and whether the scan ran to the end or was
/// stopped. Folders the scan could not read are named at the end, when it
/// met any, since the tracks in such a folder are missing from the library
/// and nothing else says so.
fn scan_summary(folder: &Path, summary: &ScanSummary) -> String {
    let ending = match summary.stopped {
        true => "stopped",
        false => "finished",
    };
    let mut sentence = format!(
        "The scan of {} {ending}: {} added, {} moved, {} unchanged, {} completed, {} duplicates, \
         {} failed",
        folder.display(),
        summary.added,
        summary.moved,
        summary.unchanged,
        summary.completed,
        summary.duplicates.len(),
        summary.failed.len()
    );
    if !summary.unreadable.is_empty() {
        sentence.push_str(&format!(
            ", and {} that could not be read",
            folder_count(summary.unreadable.len())
        ));
    }
    sentence.push('.');
    sentence
}

/// A count of folders with the noun that agrees with it.
fn folder_count(count: usize) -> String {
    match count {
        1 => "1 folder".to_owned(),
        _ => format!("{count} folders"),
    }
}

/// The reason a library failure gives, without the sentence the library crate
/// wraps it in.
///
/// A read or write that fails names no file, because the library crate does
/// not know which file the caller opened. The window does know, so it names
/// the file itself and shows only the reason. Every other failure names the
/// file in its own text, and the caller shows that text whole.
fn reason_only(problem: &IndexError) -> String {
    match problem {
        IndexError::Storage(reason) => reason.clone(),
        named => named.to_string(),
    }
}

/// Every track the library at `path` contains, and anything the person
/// should read about the library.
///
/// The panel is empty in three cases: when the window has no library file,
/// when the library file cannot be opened, and when the library cannot be
/// read. Why the window has no library file is already in the status line,
/// put there when the window worked the path out, so nothing is said about
/// it a second time here.
fn library_records(path: Option<&Path>) -> (Vec<TrackRecord>, String) {
    let Some(path) = path else {
        return (Vec::new(), String::new());
    };
    let index = match Index::open(path) {
        Ok(index) => index,
        Err(problem) => {
            return (Vec::new(), format!("The library panel is empty: {problem}"));
        }
    };
    match index.query(&Query::default()) {
        Ok(records) => (records, String::new()),
        Err(problem) => (
            Vec::new(),
            format!(
                "The library panel is empty: the library file at {} could not be read: {}",
                path.display(),
                reason_only(&problem)
            ),
        ),
    }
}

/// Which of the four curves the window shows, in the order the selector lists
/// them.
const CURVES: [Curve; 4] = [Curve::Volume, Curve::Low, Curve::Mid, Curve::High];

/// What became of a save.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Saved {
    /// The mix is in its file.
    Written,
    /// The person cancelled the dialog that asked where to write the mix, so
    /// nothing was written and nothing changed.
    Cancelled,
    /// The write failed, and the status line says why.
    Failed,
}

/// What the file dialogs call a mix document, and the extension they filter
/// on, which `docs/project-file.md` gives as the convention.
const MIX_FILTER: (&str, &str) = ("Mix document", "dmx");

/// Asks the operating system for a mix document to open, and gives back what
/// the person chose. A dialog the person cancels gives nothing.
fn ask_for_a_mix_to_open() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open a mix")
        .add_filter(MIX_FILTER.0, &[MIX_FILTER.1])
        .pick_file()
}

/// Asks the operating system for the file to write the mix to, offering
/// `name` as the name to save under, and gives back what the person chose. A
/// dialog the person cancels gives nothing.
fn ask_where_to_save(name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Save the mix as")
        .add_filter(MIX_FILTER.0, &[MIX_FILTER.1])
        .set_file_name(name)
        .save_file()
}

/// Asks the operating system for a folder, under `title`, and gives back
/// what the person chose. A dialog the person cancels gives nothing.
fn ask_for_a_folder(title: &str) -> Option<PathBuf> {
    rfd::FileDialog::new().set_title(title).pick_folder()
}

/// Asks the operating system for a library file, and gives back what the
/// person chose. A dialog the person cancels gives nothing.
///
/// The dialog shows the files that are there, so a library file that does
/// not exist yet is named by typing its path in the field beside the button.
fn ask_for_a_library_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose the library file")
        .add_filter("Library file", &["sqlite"])
        .pick_file()
}

/// How wide a path field of the settings dialog is.
const PATH_FIELD_PX: f32 = 360.0;

/// The autosave file for a mix that has no file of its own, which is one
/// file in the user's data folder. A user with no data folder has nowhere to
/// keep it, and an untitled mix is then not protected.
fn untitled_autosave_file() -> Option<PathBuf> {
    Some(untitled_autosave_path(&dirs::data_dir()?))
}

/// What the autosave file of `document` contains, given `mix`, the document
/// as its file gives it.
///
/// A mix with a file of its own is protected beside that file, and an
/// untitled mix by the one file in the user's data folder, so which file is
/// read follows from the document rather than from the caller.
fn offer_for(document: &Document, mix: &Mix) -> Offer {
    match document.path() {
        Some(project) => offered(project, mix),
        None => match untitled_autosave_file() {
            Some(file) => offered_untitled(&file),
            None => Offer::Nothing,
        },
    }
}

/// Draws one item of a menu, with the keys that do the same thing beside its
/// name, and says whether the person chose it. An item that is not `enabled`
/// is drawn greyed and cannot be chosen.
fn menu_item(ui: &mut egui::Ui, name: &str, keys: &KeyboardShortcut, enabled: bool) -> bool {
    let shown = ui.ctx().format_shortcut(keys);
    ui.add_enabled(enabled, egui::Button::new(name).shortcut_text(shown))
        .clicked()
}

/// Draws one item of a menu that has no keys of its own, and says whether the
/// person chose it.
fn menu_choice(ui: &mut egui::Ui, name: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(name)).clicked()
}

/// Prints the one line on standard error that names the mix the window has
/// just opened, which is how a launch from the desktop is checked.
fn say_which_file_was_opened(path: Option<&Path>) {
    match path {
        Some(path) => eprintln!("Opened {}", path.display()),
        None => eprintln!("Opened an untitled mix"),
    }
}

/// A file's name for the status line, or its whole path when the path ends
/// in no file name.
fn file_name(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.display().to_string(),
    }
}

/// A count of tracks with the noun that agrees with it, so that one track
/// and several read alike in a sentence.
fn track_count(count: usize) -> String {
    match count {
        1 => "1 track".to_owned(),
        _ => format!("{count} tracks"),
    }
}

/// One status line message made of `first` and then `second`, or of
/// whichever of the two is not empty.
///
/// The status line shows one message at a time, so two things a person needs
/// to read on the same repaint are shown as two sentences of one message
/// rather than one of them taking the place of the other.
fn say_both(first: &str, second: &str) -> String {
    match (first.is_empty(), second.is_empty()) {
        (true, _) => second.to_owned(),
        (_, true) => first.to_owned(),
        (false, false) => format!("{first} {second}"),
    }
}

/// A message that begins with a capital letter, for the status line, where
/// each message is a sentence of its own. The rest of the message is left
/// exactly as it came, so a path inside it reads as it is written.
fn capitalized(message: String) -> String {
    let mut letters = message.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
        None => message,
    }
}

/// What relinking did to a mix.
struct Relinking {
    /// Whether any track was pointed at a new path, which is a change to the
    /// document that a save keeps.
    relinked: bool,
    /// What is wrong with the file of every track the window has no audio
    /// for, by content hash.
    no_file: HashMap<ContentHash, NoFile>,
    /// What the status line says about the relinking, empty when there is
    /// nothing to say.
    message: String,
}

/// What is wrong with the file of a track the library could not place, and
/// the reason the operating system gave where it gave one.
///
/// The library has already established that the file at `path` is not the
/// track's, so a file that opens is one whose content hash has changed. A
/// file the operating system will not open, as an unmounted volume or a
/// permission leaves it, is the third case: the remedy for it is
/// mounting the volume or changing the permission rather than relinking, so
/// the window says which file it was and what the operating system said. The
/// library draws the same distinction and keeps it in a sentence written for
/// a person, so the window asks the file system itself rather than reading
/// that sentence back.
fn what_is_wrong_with(path: &Path) -> (NoFile, Option<String>) {
    match std::fs::File::open(path) {
        Ok(_) => (NoFile::Changed, None),
        Err(problem) if problem.kind() == std::io::ErrorKind::NotFound => (NoFile::Missing, None),
        Err(problem) => (NoFile::Unreadable, Some(problem.to_string())),
    }
}

/// Points every track of `mix` whose file is not where the document says, or
/// is not the file the document names, at the file the library file at
/// `index` names for that track's content hash, and says what became of
/// every track.
///
/// The window runs this over the mix it opens before it reads any file, and
/// over the autosaved document when a person restores it, so a track the
/// library places is read, drawn, and played from its new path as though the
/// document had named that path all along. Every track's file is read and
/// hashed, which is what tells a file that has been replaced from the one
/// the document names, so this takes a few seconds for a mix of lossless
/// files.
///
/// No folder is searched, which is what `dermixen mix relink` does when it
/// is given none: the library is the only place the window looks, and a
/// person whose files are somewhere the library has never seen runs
/// `dermixen mix relink` with the folders to search. With no library file,
/// and with a library file that cannot be opened, every track's file is
/// still read and hashed, so a track with no file to play is still noted on
/// its lane and in the status line.
///
/// A found path is written into the library as well as into the document, so
/// the window opens the library file for writing here. A `dermixen library
/// scan` running in another process at the same time keeps the library file
/// to itself, and the pass then fails, with the reason SQLite reported in
/// the status line.
///
/// The library crate writes a found path as the library's record names it,
/// and `dermixen mix relink` makes such a path absolute with its symbolic
/// links resolved before it writes the document, so the window resolves it
/// the same way. A path the operating system cannot resolve is kept as the
/// library's record names it, which is still the file that was found.
fn relink_from_the_index(index: Option<&Path>, mix: &mut Mix) -> Relinking {
    let mut relinking = Relinking {
        relinked: false,
        no_file: HashMap::new(),
        message: String::new(),
    };
    // A library file that cannot be opened is reported by the library panel,
    // which opens the same file, so nothing is said about that file a second
    // time here. The pass goes on without it, because whether each track's
    // own file is there is worth knowing with no library to look in.
    let mut opened = index.and_then(|path| Index::open(path).ok());
    // Nothing reports progress, because no folder is searched and so no file
    // outside the mix is read.
    let mut quiet = |_: &Path| {};
    let before: Vec<PathBuf> = mix.tracks.iter().map(|track| track.path.clone()).collect();
    let done = match relink(mix, opened.as_mut(), &[], &mut quiet) {
        Ok(done) => done,
        Err(problem) => {
            // Relinking points each track at its file as it goes, so a pass
            // that stops partway may already have changed the document, and
            // a save has to keep whatever it changed.
            relinking.relinked = mix
                .tracks
                .iter()
                .zip(&before)
                .any(|(track, was)| track.path != *was);
            let reason = match &problem {
                RelinkError::Index(problem) => reason_only(problem),
                RelinkError::Scan(problem) => problem.to_string(),
            };
            // Only an open library file can fail this pass, since no folder
            // is searched, so `index` names a file here. The other arm keeps
            // the sentence true rather than naming a file there is none of.
            let which = match index {
                Some(path) => format!("the library file at {}", path.display()),
                None => "the library".to_owned(),
            };
            relinking.message =
                format!("Not every track was looked for: {which} could not be read: {reason}");
            return relinking;
        }
    };

    let mut relinked = Vec::new();
    let mut gone = Vec::new();
    let mut changed = Vec::new();
    let mut unreadable = Vec::new();
    for (at, outcome) in done.tracks.iter().enumerate() {
        let track = &mut mix.tracks[at];
        match outcome {
            Relink::Kept => {}
            Relink::Relinked { .. } => {
                if let Ok(resolved) = track.path.canonicalize() {
                    track.path = resolved;
                }
                relinked.push(file_name(&track.path));
            }
            Relink::Missing { .. } => {
                let (note, reason) = what_is_wrong_with(&track.path);
                relinking.no_file.insert(track.hash, note);
                match note {
                    NoFile::Missing => gone.push(file_name(&track.path)),
                    NoFile::Changed => changed.push(file_name(&track.path)),
                    NoFile::Unreadable => {
                        let reason = reason.unwrap_or_else(|| "no reason given".to_owned());
                        unreadable.push(format!("{} ({reason})", file_name(&track.path)));
                    }
                }
            }
        }
    }

    relinking.relinked = !relinked.is_empty();
    let mut sentences = Vec::new();
    if !relinked.is_empty() {
        sentences.push(format!(
            "The window relinked {} from the library: {}.",
            track_count(relinked.len()),
            relinked.join(", ")
        ));
    }
    if !gone.is_empty() {
        sentences.push(format!(
            "The file of {} is missing: {}.",
            track_count(gone.len()),
            gone.join(", ")
        ));
    }
    if !changed.is_empty() {
        sentences.push(format!(
            "The file of {} has changed: {}.",
            track_count(changed.len()),
            changed.join(", ")
        ));
    }
    if !unreadable.is_empty() {
        // Relinking is no remedy for a file that is there and closed to this
        // user, so the reason the operating system gave stands on its own,
        // without the advice below.
        sentences.push(format!(
            "The file of {} could not be read: {}.",
            track_count(unreadable.len()),
            unreadable.join(", ")
        ));
    }
    if !gone.is_empty() || !changed.is_empty() {
        let where_it_looked = match (opened.is_some(), index.is_some()) {
            (true, _) => "The library contains no other file with the same bytes.",
            (false, true) => "The library file could not be opened, so there was nowhere to look.",
            (false, false) => "There is no library to look in.",
        };
        sentences.push(format!(
            "{where_it_looked} Run `dermixen mix relink` with the folders to search."
        ));
    }
    relinking.message = sentences.join(" ");
    relinking
}

/// The tracks of `mix` the reading thread has not been given yet, with the
/// content hash of each of them added to `handed`.
///
/// A track the window has no file for is left out, since there is nothing to
/// read: the read could only fail, and its failure would take the place of
/// the advice in the status line to run `dermixen mix relink`. A file that is
/// in the mix at two positions is given once, because everything the reading
/// thread finds is keyed by content hash.
fn to_be_read(
    mix: &Mix,
    no_file: &HashMap<ContentHash, NoFile>,
    handed: &mut HashSet<ContentHash>,
) -> Vec<Wanted> {
    let mut wanted = Vec::new();
    for track in &mix.tracks {
        if no_file.contains_key(&track.hash) || !handed.insert(track.hash) {
            continue;
        }
        wanted.push(Wanted {
            hash: track.hash,
            path: track.path.clone(),
        });
    }
    wanted
}

/// The autosaved document the window is offering back, and the moments the
/// two files were last written.
struct Restore {
    /// The document the autosave file contains.
    mix: Mix,
    /// When the autosave file was last written, when the file system says.
    autosaved: Option<SystemTime>,
    /// When the project file was last written, when the file system says.
    saved: Option<SystemTime>,
}

/// How long ago a file was written, in words, from the moment the file
/// system reported for it.
///
/// The dialog that offers unsaved changes back shows this for the autosave
/// file and for the project file, so that a person can tell an autosave from
/// a session that ended hours ago from a project file `dermixen mix add` has
/// rewritten since, and choose knowing which is the newer. Where the file
/// system reported no moment at all, the dialog says that this machine
/// cannot report when the file was written.
fn how_long_ago(written: Option<SystemTime>) -> String {
    let Some(written) = written else {
        return "at a time this machine cannot report".to_owned();
    };
    let Ok(age) = SystemTime::now().duration_since(written) else {
        return "at a time in the future by this machine's clock".to_owned();
    };
    let seconds = age.as_secs();
    let (count, unit) = match seconds {
        0..=59 => return "less than a minute ago".to_owned(),
        60..=3599 => (seconds / 60, "minute"),
        3600..=86_399 => (seconds / 3600, "hour"),
        _ => (seconds / 86_400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// What the status line says about a tempo field that will not read as a
/// tempo. An empty field is named as empty, since a message beginning with
/// nothing at all leaves the person with no subject.
fn not_a_tempo(typed: &str) -> String {
    if typed.is_empty() {
        return "The tempo field is empty".to_owned();
    }
    format!("{typed} is not a tempo")
}

/// The text a path field of the settings dialog shows for a setting: the
/// path as the settings file has it, and empty text where the setting is
/// unset, in which case the field's hint text shows the path in use.
fn path_field_text(setting: Option<&Path>) -> String {
    setting
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

/// The setting the text of a path field stands for. Text that is empty or
/// only spaces leaves the setting unset, which means the default.
fn path_setting(text: &str) -> Option<PathBuf> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// The text the settings dialog's audio buffer field shows for `frames`,
/// which is empty text for the device's own size, as `docs/settings.md`
/// says the field shows an unset buffer.
fn buffer_field_text(frames: Option<NonZeroU32>) -> String {
    match frames {
        Some(frames) => frames.to_string(),
        None => String::new(),
    }
}

/// A beat grid being corrected by hand, and which track it belongs to.
///
/// A [`GridEditor`] knows its track by that track's position in the
/// playlist, and positions move: removing a track, moving one, and undoing
/// or redoing either can leave a different track at the position the editor
/// was opened on. The content hash of the track the editor was opened on is
/// kept beside it so the window can see that happen and close the editor,
/// rather than applying one track's grid to another and writing a correction
/// for the wrong file.
struct GridEdit {
    /// The editor over a copy of the track's grid.
    editor: GridEditor,
    /// The content hash of the track the editor was opened on.
    hash: ContentHash,
    /// The track's length in frames, as the mix document holds it.
    length: Samples,
    /// The track's decoded audio, once it is in hand. The strip is drawn
    /// without a waveform until it arrives.
    audio: Option<Arc<Audio>>,
    /// The thread reading the track's file, while the window did not
    /// already hold its audio.
    decoding: Option<Decoding>,
    /// The audition, while the track is playing on its own.
    audition: Option<Audition>,
    /// The grid the audition was last given, so that the window hands over
    /// only a grid the audition does not already have.
    given: BeatGrid,
    /// The grid the editor last put on the track through
    /// [`Timeline::set_grid`], which is the grid the track had when the
    /// editor opened until the first adjustment is made.
    ///
    /// The window compares this grid with the track's own grid on every
    /// repaint. A difference means something other than the editor changed
    /// the track's grid, which an undo or a redo of the correction does, and
    /// the editor then takes the track's grid as its own.
    on_the_track: BeatGrid,
    /// The track frame the audition last reached, which is where the strip
    /// draws its playhead and where playing again starts.
    playhead: Option<Samples>,
    /// Whether the strip moves itself to keep the audition's playhead in
    /// view.
    ///
    /// Playing turns this on and a scroll or a zoom of the strip turns it
    /// off, so that a person who moves the view while the track plays keeps
    /// the part of the track they moved to rather than being taken back to
    /// the playhead a moment later.
    following: bool,
    /// The strip as it was last drawn, and everything it was drawn from.
    ///
    /// Working a scene out reads every frame of audio the strip covers, so
    /// the window keeps the last one and asks the editor for another only
    /// when something the scene is made of has changed.
    drawn: Option<Drawn>,
}

/// A strip the window has drawn, and everything the editor made it from.
struct Drawn {
    /// The part of the track that was on the strip.
    view: GridView,
    /// The grid whose beats were drawn.
    grid: BeatGrid,
    /// The column the audition's playhead was drawn at, if it was on the
    /// strip.
    playhead: Option<f32>,
    /// How many frames of the track's audio were in hand, which is zero
    /// until the file has been read and the track's length after that.
    frames: usize,
    /// What was drawn.
    scene: GridScene,
}

impl GridEdit {
    /// The track frame the audition's playhead sits at, when it is on the
    /// strip.
    ///
    /// The strip draws a playhead only for a frame within the view, which is
    /// also the frame playing again starts from, so scrolling the strip
    /// somewhere else is how a person chooses to hear another part of the
    /// track.
    fn playhead_on_the_strip(&self) -> Option<Samples> {
        let frame = self.playhead?;
        let x = self.editor.x_of(frame);
        (x >= 0.0 && x < self.editor.view().width_px).then_some(frame)
    }

    /// The frame an audition starts at: the strip's playhead when one is
    /// drawn and the track still has frames after it, and otherwise the
    /// frame at the left edge of the strip.
    fn audition_from(&self) -> Samples {
        match self.playhead_on_the_strip() {
            Some(frame) if frame < self.length => frame,
            _ => self.editor.view().start,
        }
    }

    /// What the strip draws for the view as it stands, worked out again only
    /// when the view, the grid, the playhead's column, or the audio in hand
    /// has changed since the last time.
    fn scene(&mut self) -> &GridScene {
        let view = self.editor.view();
        let grid = self.editor.grid();
        let playhead = self.playhead.map(|frame| self.editor.x_of(frame));
        let audio: &[dermixen_media::Frame] = match &self.audio {
            Some(audio) => &audio.frames,
            None => &[],
        };
        let same = self.drawn.as_ref().is_some_and(|drawn| {
            drawn.view == view
                && drawn.grid == grid
                && drawn.playhead == playhead
                && drawn.frames == audio.len()
        });
        if !same {
            let scene = self.editor.scene(audio, self.playhead);
            self.drawn = Some(Drawn {
                view,
                grid,
                playhead,
                frames: audio.len(),
                scene,
            });
        }
        &self
            .drawn
            .as_ref()
            .expect("the scene was just worked out")
            .scene
    }
}

/// Ends `open`'s audition, if one is running, and gives the audio device
/// back. Says whether one was running.
fn end_the_audition(open: &mut GridEdit) -> bool {
    match open.audition.take() {
        Some(audition) => {
            audition.stop();
            true
        }
        None => false,
    }
}

/// The color of a library row whose key mixes well with the key of the
/// track selected on the timeline.
const COMPATIBLE: Color32 = Color32::from_rgb(150, 220, 150);

/// How large the icon of the library panel's toggle is drawn, which is the
/// outline of a window with a panel down its right third.
const SIDEBAR_ICON_PX: Vec2 = Vec2::new(18.0, 14.0);

/// Draws the one control that hides and shows the library panel, and says
/// whether a person clicked it.
///
/// `shown` is whether the panel stands beside the timeline at the moment.
/// The icon is an outline of a window with a vertical line dividing off its
/// right third, and that third is filled while the panel is shown and empty
/// while the panel is hidden, which is how other applications draw a toggle
/// for a panel down one side. The colors are the ones the widget's visuals
/// give for the state the control is in, so the icon brightens under the
/// pointer as a button does.
///
/// The hover text names what a click does and gives the shortcut for the
/// same thing, which `egui::Context::format_shortcut` writes with the keys
/// of the machine the window is running on.
fn library_toggle(ui: &mut egui::Ui, shown: bool) -> bool {
    let padding = ui.spacing().button_padding;
    let (whole, response) =
        ui.allocate_exact_size(SIDEBAR_ICON_PX + padding + padding, Sense::click());
    if ui.is_rect_visible(whole) {
        let visuals = *ui.style().interact(&response);
        let icon = egui::Rect::from_center_size(whole.center(), SIDEBAR_ICON_PX);
        // The divider stands a third of the way in from the right edge, so
        // the panel the icon stands for is the narrower part, as the library
        // panel is beside the timeline.
        let divider = icon.right() - icon.width() / 3.0;
        let painter = ui.painter();
        if shown {
            let panel = egui::Rect::from_min_max(
                egui::pos2(divider, icon.top()),
                egui::pos2(icon.right(), icon.bottom()),
            );
            painter.rect_filled(panel, 0.0, visuals.fg_stroke.color);
        }
        painter.vline(divider, icon.y_range(), visuals.fg_stroke);
        painter.rect_stroke(icon, 2.0, visuals.fg_stroke, egui::StrokeKind::Inside);
    }
    let name = if shown {
        "Hide library"
    } else {
        "Show library"
    };
    let keys = ui.ctx().format_shortcut(&LIBRARY_PANEL);
    response.on_hover_text(format!("{name} ({keys})")).clicked()
}

/// The mark the table draws on a track that is already in the mix, in the
/// narrow column before the ten headings.
const IN_MIX_MARK: &str = "●";

/// How wide the column that holds the mark on a track in the mix is.
const MIX_MARK_PX: f32 = 18.0;

/// The narrowest a column of the table can be dragged.
const MIN_COLUMN_PX: f32 = 32.0;

/// The name the library table is built under, which egui_extras mixes into
/// the identifier of every one of the table's parts. The window builds the
/// identifier of a column's resize handle from this name, so that a
/// double-click on a divider reaches [`fitted_column`].
const TABLE_SALT: &str = "library table";

/// How far past the width of its own text an italic cell is drawn, as a
/// share of the height of a line.
///
/// egui draws italics by leaning the top of each glyph to the right rather
/// than by laying the text out in an italic font, so an italic cell's text
/// is exactly as wide as the same text upright and only the drawing reaches
/// further right. `tessellate_glyphs` in epaint's `text/text_layout.rs`
/// moves the top of a glyph right by a quarter of that glyph's own height,
/// and a glyph is never taller than the line it sits on, so a quarter of a
/// line's height covers the lean of a cell's last glyph.
const ITALIC_LEAN: f32 = 0.25;

/// How much taller than its text a row of the table is drawn, which keeps
/// one row's last line off the next row's first.
const ROW_PADDING_PX: f32 = 2.0;

/// How wide a text field of the filters section is.
const FILTER_FIELD_PX: f32 = 110.0;

/// How wide one end of a range field of the filters section is.
const RANGE_FIELD_PX: f32 = 50.0;

/// How wide a column of the table is before anyone drags the divider beside
/// its heading.
///
/// Every width holds that column's heading with the space and the sort arrow
/// after it, in the body text style and with the padding a selectable label
/// puts around its text, so the arrow on the sorted column is never the part
/// that the clipping cuts. `Catalog no.` needs the most, 88.3 pixels of the
/// 90 it has, and `Artist` and `Title` take what is left over, since those
/// two hold the longest text. The ten widths come to 676 pixels. The panel
/// opens at [`LIBRARY_WIDTH_PX`], which leaves the table 783 pixels, enough
/// for the ten columns, the mark column, and the eight pixels of gap after
/// each of the eleven.
fn column_width(column: Column) -> f32 {
    match column {
        Column::Artist => 76.0,
        Column::Title => 84.0,
        Column::Year => 50.0,
        Column::Bpm => 52.0,
        Column::Key => 46.0,
        Column::Label => 56.0,
        Column::CatalogNumber => 90.0,
        Column::Release => 70.0,
        Column::TrackNumber => 76.0,
        Column::Duration => 76.0,
    }
}

/// The text in each field of the library panel's filters, and the keys
/// chosen on the key picker.
///
/// The ends of the year and tempo ranges are kept as text, because half of a
/// number typed is not a number. A field whose text is not a number, or is a
/// tempo that is not finite, leaves that end of its range open.
#[derive(Default)]
struct FilterFields {
    /// The artist field.
    artist: String,
    /// The title field.
    title: String,
    /// The label field.
    label: String,
    /// The path field.
    path: String,
    /// The lower end of the year range.
    year_from: String,
    /// The upper end of the year range.
    year_to: String,
    /// The lower end of the tempo range.
    bpm_from: String,
    /// The upper end of the tempo range.
    bpm_to: String,
    /// The keys chosen on the key picker.
    keys: Vec<Camelot>,
}

impl FilterFields {
    /// The conditions the fields stand for.
    fn filters(&self) -> Filters {
        Filters {
            artist: self.artist.clone(),
            title: self.title.clone(),
            keys: self.keys.clone(),
            label: self.label.clone(),
            year_from: self.year_from.trim().parse().ok(),
            year_to: self.year_to.trim().parse().ok(),
            bpm_from: tempo_bound(&self.bpm_from),
            bpm_to: tempo_bound(&self.bpm_to),
            path: self.path.clone(),
        }
    }
}

/// One end of the tempo range as the text in its field gives it. Text that
/// is not a number, and a number that is not finite, like `nan` or `inf`,
/// leave that end open.
fn tempo_bound(text: &str) -> Option<Bpm> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|tempo| tempo.is_finite())
        .map(Bpm)
}

/// Draws one text field of the filters and says whether the person changed
/// it.
fn filter_field(ui: &mut egui::Ui, text: &mut String, id: &str) -> bool {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(FILTER_FIELD_PX)
            .id_salt(id),
    )
    .changed()
}

/// Draws one end of a range field of the filters and says whether the person
/// changed it.
fn range_field(ui: &mut egui::Ui, text: &mut String, id: &str) -> bool {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(RANGE_FIELD_PX)
            .id_salt(id),
    )
    .changed()
}

/// What a person clicked in the library table during one repaint.
#[derive(Default)]
struct TableClicks {
    /// The heading clicked, if one was.
    heading: Option<Column>,
    /// The row clicked, if one was, counted among the rows shown.
    row: Option<usize>,
}

/// How tall each row of the library table is, and what the heights were
/// worked out from.
///
/// The window works the heights out only while the setting
/// `library_word_wrap` is on. A cell then wraps its text onto as many lines
/// as its column is wide enough for, so a row is as tall as its tallest
/// cell, and the table widget is given every row's height before it draws
/// the few rows that are on screen. Each track's height is worked out once
/// at the current column widths and kept under the track's content hash,
/// which is what the library keys a record by, so a search keystroke, a
/// filter, a heading click, and a track added to the mix are a lookup per
/// row rather than a layout per cell. Dragging a column drops the kept
/// heights, since every one of them was worked out at the old widths.
///
/// While the setting is off, every cell is one line and every row is one
/// line tall, so the window asks for no height here at all.
#[derive(Default)]
struct RowHeights {
    /// The widths the heights were worked out at, the mark column first.
    widths: Vec<f32>,
    /// How tall each row is, in the order the panel gives the rows.
    heights: Vec<f32>,
    /// How tall a track's row is at the widths in `widths`, by the track's
    /// content hash.
    at_widths: HashMap<ContentHash, f32>,
    /// How wide each cell's text would be on one line, by the track's
    /// content hash. What a track's cells hold never changes while the
    /// window is open, so a width worked out once stands for the rest of the
    /// session, and a cell whose text holds no line break and is no wider
    /// than its column is one line tall whatever that column's width. A
    /// column dragged wider or narrower therefore lays out again only the
    /// cells whose text no longer fits. A cell whose text does hold a line
    /// break is laid out every time, because the width kept here is that of
    /// its widest line and says nothing about how many lines it has.
    natural: HashMap<ContentHash, [f32; Column::ALL.len()]>,
    /// Whether `heights` describes the rows the panel gives now.
    fresh: bool,
}

impl RowHeights {
    /// Forgets the heights, which the window does whenever it works the
    /// library's rows out again.
    fn forget(&mut self) {
        self.fresh = false;
    }

    /// How tall each row of `rows` is at the column widths in `widths`,
    /// worked out again only when the rows or the widths have changed.
    fn of(&mut self, ui: &egui::Ui, rows: &[LibraryRow], widths: &[f32]) -> &[f32] {
        if self.fresh && self.heights.len() == rows.len() && self.widths == widths {
            return &self.heights;
        }
        if self.widths != widths {
            self.at_widths.clear();
            self.widths = widths.to_vec();
        }
        let font = egui::TextStyle::Body.resolve(ui.style());
        let line = ui.text_style_height(&egui::TextStyle::Body);
        let mut heights = Vec::with_capacity(rows.len());
        for row in rows {
            if let Some(height) = self.at_widths.get(&row.hash) {
                heights.push(*height);
                continue;
            }
            let natural = self.natural.entry(row.hash).or_insert_with(|| {
                let mut natural = [0.0; Column::ALL.len()];
                for (at, column) in Column::ALL.iter().enumerate() {
                    natural[at] = ui
                        .painter()
                        .layout_no_wrap(row.cell(*column), font.clone(), Color32::PLACEHOLDER)
                        .size()
                        .x;
                }
                natural
            });
            let mut height = line;
            for (at, column) in Column::ALL.iter().enumerate() {
                // The mark on a track that is in the mix takes the first
                // width, so a column's own width comes one later.
                let width = widths.get(at + 1).copied().unwrap_or(0.0);
                let text = row.cell(*column);
                // Text with a line break in it takes more than one line
                // however narrow its widest line is, so only text without a
                // line break is taken as one line tall on its width alone.
                if natural[at] <= width && !text.contains('\n') {
                    continue;
                }
                let galley = ui
                    .painter()
                    .layout(text, font.clone(), Color32::PLACEHOLDER, width);
                height = height.max(galley.size().y);
            }
            let height = height + ROW_PADDING_PX;
            self.at_widths.insert(row.hash, height);
            heights.push(height);
        }
        self.heights = heights;
        self.fresh = true;
        &self.heights
    }
}

/// The text of one column's heading, which is the column's name, and the
/// arrow for the direction of the sort after it while the rows are sorted by
/// that column.
///
/// The heading row draws this text, and [`fitted_column`] measures it, so a
/// column fitted to its widest cell has room for the arrow whether or not
/// the arrow is showing at the moment of the fit.
fn heading_text(column: Column, sort: Sort) -> String {
    let way = if sort.descending { "⏷" } else { "⏶" };
    format!("{} {way}", column.heading())
}

/// The column whose divider a person double-clicked, and how wide that
/// column has to be to hold its widest cell, when a double-click landed on a
/// divider since the last repaint.
///
/// A double-click on the divider at the right edge of a column's heading
/// fits that column to its widest cell, as a spreadsheet does.
/// [`library_table`] calls this before it builds the table, and the table
/// draws at the fitted width from the repaint after the double-click, since
/// egui_extras takes the width into its own state only once the rows have
/// drawn.
///
/// egui_extras builds the resize handle of the table's column `i` under the
/// identifier of the table's own `egui::Ui`, the name the table is built
/// under, the text `resize_column`, and `i`, and the mark column before the
/// ten headed ones is column zero. The handle is drawn after the rows, so
/// this reads the handle's answer from the repaint before this one, which is
/// what [`egui::Context::read_response`] gives.
///
/// The width is the widest single-line layout of that column's text over
/// every row `rows` gives, not only the rows on screen, and the heading with
/// its sort arrow laid out the way the heading row draws it. A cell puts no
/// padding around its text and a heading is a selectable label, which puts
/// the button padding on each side of its own. The artist and the title of a
/// row whose metadata was guessed are drawn in italics, which reach
/// [`ITALIC_LEAN`] of a line's height past the width of the text, so those
/// two cells of such a row are measured with that much added. Every row is
/// laid out here, which happens once per double-click rather than once per
/// repaint.
///
/// A fit narrower than [`MIN_COLUMN_PX`], which every column of text as
/// short as a Camelot code is, gives that width instead, so the fit never
/// takes a column below the width a drag can take it to.
fn fitted_column(ui: &egui::Ui, rows: &[LibraryRow], sort: Sort) -> Option<(Column, f32)> {
    // egui_extras turns the name into an `IdSalt` of its own before it mixes
    // the name into the table's identifier, and an `IdSalt` hashes as itself
    // rather than as the text it was made from, so the same step here is
    // what makes the two identifiers the same.
    let table = ui.id().with(egui::IdSalt::new(TABLE_SALT));
    let column = Column::ALL.iter().enumerate().find_map(|(at, column)| {
        let handle = table.with("resize_column").with(at + 1);
        let hit = ui
            .ctx()
            .read_response(handle)
            .is_some_and(|divider| divider.double_clicked());
        hit.then_some(*column)
    })?;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let width = |text: String| {
        ui.painter()
            .layout_no_wrap(text, font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    let heading = width(heading_text(column, sort)) + 2.0 * ui.spacing().button_padding.x;
    let leans = matches!(column, Column::Artist | Column::Title);
    let lean = ui.text_style_height(&egui::TextStyle::Body) * ITALIC_LEAN;
    let widest = rows
        .iter()
        .map(|row| {
            let italics = if leans && row.guessed { lean } else { 0.0 };
            width(row.cell(column)) + italics
        })
        .fold(heading, f32::max);
    Some((column, widest.max(MIN_COLUMN_PX)))
}

/// Draws the library as a table and answers what the mouse clicked.
///
/// The heading row holds a heading per column, and the sorted column's
/// heading shows an arrow for the direction of the sort. Before the ten
/// columns is a narrow one with no heading that marks a track already in the
/// mix. For a row whose artist and title were guessed from the file name
/// rather than read from its tags, the panel draws those two cells in
/// italics. A row whose key mixes well with the track selected on the
/// timeline is drawn in green, and the selected row is drawn selected. A
/// click anywhere on a row chooses it.
///
/// `wrap` is the setting `library_word_wrap`, which the window reads when it
/// opens and the settings dialog's **Wrap library cells** checkbox changes.
/// With it on, a cell wraps its text onto further lines and a row is as tall
/// as its tallest cell, which `heights` works out. With it off, every cell
/// is one line, every row is one line tall, and `heights` is left alone.
fn library_table(
    ui: &mut egui::Ui,
    rows: &[LibraryRow],
    sort: Sort,
    heights: &mut RowHeights,
    wrap: bool,
) -> TableClicks {
    let mut clicks = TableClicks::default();
    let line = ui.text_style_height(&egui::TextStyle::Body);
    let fit = fitted_column(ui, rows, sort);
    if fit.is_some() {
        // egui_extras takes the fitted width into the table's own state at
        // the end of this repaint, so the table draws at that width on the
        // next one. Asking for a repaint here is what brings the fitted
        // column up at once rather than whenever a person next moves the
        // pointer or presses a key.
        ui.ctx().request_repaint();
    }
    let mut builder = TableBuilder::new(ui)
        .id_salt(TABLE_SALT)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .auto_shrink([false, false])
        .column(TableColumn::exact(MIX_MARK_PX).resizable(false));
    for column in Column::ALL {
        // Every column clips, which cuts off at the column's edge the text
        // that neither wrapping nor one line can fit, like a title with no
        // space in it that is wider than its column. Clipping also keeps
        // a column at the width a person left it at, since a column that
        // does not clip grows to whatever its widest drawn cell used, and a
        // wide title scrolling into view would widen the column under the
        // pointer.
        let mut spec = TableColumn::initial(column_width(column))
            .at_least(MIN_COLUMN_PX)
            .clip(true);
        // A fitted column is given a width range of exactly the fit width
        // for this one repaint, because egui_extras keeps a table's column
        // widths where the window cannot reach them and clamps every width
        // to its column's range once the rows of that repaint have drawn.
        // The column then keeps that width under its ordinary range from the
        // next repaint on, as a column a person has dragged does.
        if let Some((fitted, width)) = fit
            && fitted == column
        {
            spec = spec.at_least(width).at_most(width);
        }
        builder = builder.column(spec);
    }
    let table = builder.header(line + ROW_PADDING_PX, |mut row| {
        row.col(|_| {});
        for column in Column::ALL {
            let sorted_here = sort.column == column;
            let heading = if sorted_here {
                heading_text(column, sort)
            } else {
                column.heading().to_owned()
            };
            let mut hit = false;
            let (_, cell) = row.col(|ui| {
                hit = ui.selectable_label(sorted_here, heading).clicked();
            });
            if hit || cell.clicked() {
                clicks.heading = Some(column);
            }
        }
    });
    table.body(|mut body| {
        let draw = |mut table_row: egui_extras::TableRow<'_, '_>| {
            let index = table_row.index();
            let row = rows.get(index)?;
            table_row.set_selected(row.selected);
            table_row.col(|ui| {
                if row.in_mix {
                    let mut mark = egui::RichText::new(IN_MIX_MARK);
                    if row.compatible {
                        mark = mark.color(COMPATIBLE);
                    }
                    ui.label(mark);
                }
            });
            for column in Column::ALL {
                table_row.col(|ui| {
                    let mut text = egui::RichText::new(row.cell(column));
                    if row.guessed && matches!(column, Column::Artist | Column::Title) {
                        text = text.italics();
                    }
                    if row.compatible {
                        text = text.color(COMPATIBLE);
                    }
                    let cell = egui::Label::new(text);
                    // Truncating is what makes a cell one line: the text
                    // that does not fit the column is cut off at the
                    // column's edge rather than wrapped onto a second line.
                    ui.add(if wrap { cell.wrap() } else { cell.truncate() });
                });
            }
            table_row.response().clicked().then_some(index)
        };
        if wrap {
            let widths = body.widths().to_vec();
            let sizes: Vec<f32> = heights.of(body.ui_mut(), rows, &widths).to_vec();
            body.heterogeneous_rows(sizes.into_iter(), |table_row| {
                if let Some(index) = draw(table_row) {
                    clicks.row = Some(index);
                }
            });
        } else {
            // Every row is one line tall, so nothing here lays a cell out to
            // find a height and the table takes the whole list at one
            // height.
            body.rows(line + ROW_PADDING_PX, rows.len(), |table_row| {
                if let Some(index) = draw(table_row) {
                    clicks.row = Some(index);
                }
            });
        }
    });
    clicks
}

/// Draws a tempo field and reports what finishing the typing did.
///
/// The field shows `bpm` whenever nobody is typing in it. Gaining the
/// keyboard settles `target`, which is what a tempo typed here is written
/// for, however the selection moves while it is being typed, and losing the
/// keyboard or pressing return is what writes it.
fn tempo_field<T: Clone>(
    ui: &mut egui::Ui,
    field: &mut TempoField<T>,
    bpm: Bpm,
    target: T,
    id: &str,
    width: f32,
) -> Finish<T> {
    field.show(bpm);
    let mut text = field.text().to_owned();
    let widget = ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(width)
            .id_salt(id),
    );
    // A field the window finished itself, on the path that closes the grid
    // editor, still holds the keyboard as far as egui is concerned.
    // Beginning whenever the field holds the keyboard, rather than only on
    // the frame it took the keyboard, is what lets such a field take typing
    // again. Beginning a second time while a typing is under way changes
    // nothing.
    if widget.has_focus() {
        field.begin(target);
    }
    if widget.changed() {
        field.edit(&text);
    }
    if widget.lost_focus() {
        return field.finish();
    }
    Finish::Unchanged
}

/// One open mix.
pub struct Window {
    /// Which file the mix is, whether it has changes that were never saved,
    /// and the intent waiting on the question about them.
    document: Document,
    /// The title the window is showing, kept so that the window sends the
    /// windowing system a title only when the title has changed.
    shown_title: String,
    /// The autosave file for a mix with no file of its own, which every
    /// untitled mix this window opens is protected by. A user with no data
    /// folder has none.
    untitled_autosave: Option<PathBuf>,
    /// Whether the person has settled what becomes of unsaved changes and
    /// the window is on its way out, which is what lets the next request to
    /// close it through.
    quitting: bool,
    /// The mix laid out for drawing, and everything that edits it.
    timeline: Timeline,
    /// The preview, while one is running.
    transport: Option<Transport>,
    /// Where the preview stood at the last repaint.
    status: Option<TransportStatus>,
    /// The playhead, the start point, and what the transport is doing. The
    /// window carries out the orders it gives and tells it what it hears
    /// back from the transport.
    playback: Playback,
    /// The decoded audio the window and the render thread share.
    cache: Arc<Mutex<AudioCache>>,
    /// The thread reading the mix's files, kept so that its findings keep
    /// arriving.
    reading: Reading,
    /// The content hash of every track the window has handed the reading
    /// thread, for the window's whole life. A file can enter the mix more
    /// than once, in the opening read, by being added again from the
    /// library, or at two positions of a restored document, and its hash is
    /// handed to the thread once.
    handed: HashSet<ContentHash>,
    /// The grid being corrected by hand, while one is.
    editor: Option<GridEdit>,
    /// How far the grid editor's drag has moved the grid, in milliseconds.
    nudge_ms: f64,
    /// How far the grid editor's drag had moved the grid when the window last
    /// acted on it.
    nudged_ms: f64,
    /// The grid editor's exact tempo field.
    grid_tempo: TempoField<()>,
    /// Whether the grid editor's metronome clicks on the beats of the grid.
    ///
    /// The setting belongs to the window rather than to one editor, so it
    /// stands for as long as the window is open and opening the editor on
    /// another track keeps the click as the person left it. It starts as the
    /// settings file has it, and turning it on or off writes that file.
    metronome: bool,
    /// Whether the grid editor's strip is collapsed, so that the editor
    /// shows its row of controls alone.
    ///
    /// Like the metronome, the state belongs to the window rather than to
    /// one editor, so the editor opens on any track as the person left it.
    /// It starts as the settings file has it, and every click of the
    /// editor's control or the settings dialog's checkbox writes that file.
    strip_collapsed: bool,
    /// How tall the grid editor's strip is drawn, which a drag of its bottom
    /// edge sets. The height lasts for as long as the window is open and no
    /// file records it.
    strip_px: f32,
    /// The settings file this window read when it opened and writes a
    /// changed setting back to.
    settings_file: PathBuf,
    /// Every setting, as the settings file gives them and as the person has
    /// changed them since the window opened.
    settings: Settings,
    /// Whether the settings dialog is open.
    settings_open: bool,
    /// The text of the settings dialog's audio buffer field.
    ///
    /// The window reads the text as a buffer size when the person presses
    /// return or the field loses the keyboard, rather than on every
    /// keystroke, so half a number on the way to a whole one is never
    /// refused.
    buffer_text: String,
    /// The selected tempo node's field. What it writes goes to the node it
    /// names, which is settled when the typing begins, so that typing a
    /// tempo and then clicking another node never writes the number onto the
    /// node that was clicked.
    node_tempo: TempoField<(usize, Beats)>,
    /// The master tempo field, which stands for the mix tempo at the
    /// playhead and so needs no target of its own.
    master_tempo: TempoField<()>,
    /// The library beside the timeline.
    library: LibraryPanel,
    /// The rows the library panel last described.
    ///
    /// A row holds the track's file name and its artist and title, so
    /// building the whole list is work worth doing when the search, the
    /// highlighting, the selection, or the mix changes rather than on every
    /// repaint.
    rows: Vec<LibraryRow>,
    /// The search text the panel was last given, so that the window hands it
    /// over only when a person has changed it.
    searched: String,
    /// The text in each field of the library panel's filters, and the keys
    /// chosen on its key picker.
    filters: FilterFields,
    /// Whether the library panel is hidden, so that the timeline has the
    /// width of the window. The field takes the setting the settings file
    /// gives when the window opens, and every click of the panel's toggle in
    /// the first row of controls, of **View > Show library** or
    /// **View > Hide library**, or of the settings dialog's **Library
    /// hidden** checkbox sets the field and writes that file.
    library_collapsed: bool,
    /// Whether a cell of the library table wraps its text onto further
    /// lines. The field takes the setting the settings file gives when the
    /// window opens, and a click of the settings dialog's **Wrap library
    /// cells** checkbox sets the field and writes that file. While the field
    /// is false, every row is one line tall, and a cell shows the first line
    /// of its value as far as the column's edge and cuts off the rest, which
    /// is what `egui::Label::truncate` asks epaint for.
    library_word_wrap: bool,
    /// How tall each row of the library table is, kept so that a repaint
    /// that changes neither the rows nor the column widths draws them
    /// without working every row's height out again.
    heights: RowHeights,
    /// Each track's Camelot code by content hash, as the library gave it,
    /// which is what the selected track on the timeline highlights the
    /// library against.
    keys: HashMap<ContentHash, Camelot>,
    /// The content hash of the track selected on the timeline when the
    /// library was last given a key to highlight against.
    reference_hash: Option<ContentHash>,
    /// Whether the window has set the timeline's selection since the library
    /// last took a key from it.
    ///
    /// Clicking a library row highlights the rows against that row's own
    /// key, so the timeline has to be able to take the highlighting back
    /// even when the person clicks the track that was already selected,
    /// which leaves the selection exactly as it was.
    reselected: bool,
    /// The clock the tap tempo counts taps on.
    clock: Instant,
    /// Where the pointer was when the press being held began, if one is.
    pressed_at: Option<egui::Pos2>,
    /// Whether the press being held has moved far enough to be a drag.
    dragging: bool,
    /// Whether the press being held took hold of a lane's bottom edge, which
    /// is what keeps the resize cursor while the lane is being resized.
    resizing: bool,
    /// Whether the view has been fitted to the whole mix, which is done once,
    /// as soon as the window knows how wide it is.
    fitted: bool,
    /// The message the status line shows, empty when there is none.
    message: String,
    /// Why the render last started over on a replacement, and when this
    /// window first saw that reason, so that the status line can show it for
    /// a few seconds and then drop it.
    started_over: Option<(String, Instant)>,
    /// The folder corrections are written to, kept so that a document put
    /// under the window later can be given it too.
    corrections: Option<PathBuf>,
    /// Every waveform overview the reading thread has found, kept so that a
    /// document put under the window later can be given them again.
    overviews: HashMap<ContentHash, Overview>,
    /// Every phrase record the reading thread has found, kept for the same
    /// reason as the overviews.
    phrases: HashMap<ContentHash, PhraseRecord>,
    /// The autosaved document and the two files' times, while the window is
    /// asking whether to restore that document.
    restore: Option<Restore>,
    /// What is wrong with the file of every track the window has no audio
    /// for, by content hash.
    ///
    /// The lane of such a track says what is wrong there in place of a
    /// waveform. The relinking pass writes this, and every pass writes the
    /// whole of it, so a track a restored document gives a file that reads
    /// loses its note. Everything the window keeps per track is keyed by
    /// content hash, so a note stays with its track as the playlist is
    /// reordered.
    no_file: HashMap<ContentHash, NoFile>,
    /// Why the last autosave failed, while the last one did.
    ///
    /// This stands in the status line until an autosave succeeds, rather
    /// than passing by as one message among others, because autosaving that
    /// fails once usually fails all session (a folder that cannot be
    /// written, a disk that is full), and a person whose work is no longer
    /// being kept needs to know it for as long as it is true.
    autosave_trouble: Option<String>,
    /// The paths of the documents macOS asks the window to open, which are
    /// the `.dmx` files double-clicked in the Finder and the ones dropped on
    /// the Dermixen icon. [`open`] starts the listening before the event
    /// loop and hands the window the channel. On Linux a `.dmx` file opened
    /// on the desktop reaches the window as its argument instead, so nothing
    /// ever arrives on the channel there.
    documents: Option<Receiver<PathBuf>>,
    /// The library file the window has open for the mix under it, or
    /// nothing when the window could work out no path or could make no
    /// file, in which case the status line says why.
    library_file: Option<PathBuf>,
    /// The text of the settings dialog's music folder field.
    ///
    /// The window reads the text as a folder when the person presses return
    /// or the field loses the keyboard, rather than on every keystroke, so
    /// half a path on the way to a whole one is never taken.
    music_text: String,
    /// The text of the settings dialog's library file field, read the same
    /// way as the music folder field.
    library_text: String,
    /// The scan the window has running, while one is.
    scanning: Option<Scanning>,
    /// Whether a file the running scan dealt with has changed what the
    /// library panel would show, so the window owes the panel a refresh.
    refresh_due: bool,
    /// When the window last read the library again for the panel, which is
    /// what holds the refreshes to one a second while a scan runs.
    last_refresh: Instant,
    /// The window's own handle on the screen, which the window hands to
    /// every thread it starts so that a thread with something to report
    /// can ask for a repaint.
    repaint: egui::Context,
}

/// A scan the window has running, and the folder that scan reads.
struct Scanning {
    /// The thread doing the scanning.
    scan: Scan,
    /// The folder the scan reads, which the status line names when the scan
    /// is over.
    folder: PathBuf,
}

impl Window {
    /// Opens `mix`, which is the mix `document` names, with the corrections
    /// folder and the library file the command line would use, and with
    /// `settings`, which [`open`] read from the file at `settings_file`.
    ///
    /// Every track whose file has moved is looked for in that library before
    /// the reading thread is started, so a track the library places gets its
    /// waveform, its phrase marks, and its audio like any other track of the
    /// mix.
    ///
    /// [`open`] is what reads the mix, reads the settings, and calls this,
    /// and it hands the window that comes back to `eframe`, so nothing else
    /// need build one. `ctx` is the window's own handle on the screen, which
    /// every thread the window starts is given so that a thread with
    /// something to report can ask for a repaint. The autosave file of the
    /// mix is not read here: the caller reads it before the window is built
    /// and hands what it found to the window, so the dialog offering that
    /// document back stands over the window from its first repaint.
    ///
    /// The library file the settings name is opened here, and made when it
    /// is not there yet. A library the window has just made and a music
    /// folder that exists start a scan of that folder, which is how a first
    /// launch after a build or an install ends with a library.
    pub fn new(
        document: Document,
        mut mix: Mix,
        settings_file: PathBuf,
        settings: Settings,
        ctx: &egui::Context,
    ) -> Window {
        let wake = {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        };
        let library_file = library_file_for(&settings);
        let relinking = relink_from_the_index(library_file.file.as_deref(), &mut mix);
        let mut timeline = Timeline::new(mix);
        let mut message = String::new();
        let mut corrections = None;
        match corrections_dir() {
            Ok(dir) => {
                timeline.set_corrections_dir(dir.clone());
                corrections = Some(dir);
            }
            Err(problem) => message = problem,
        }
        if let Some(note) = audio::keylock_note() {
            message = note;
        }
        if !library_file.message.is_empty() {
            message = library_file.message.clone();
        }

        let (records, trouble) = library_records(library_file.file.as_deref());
        if !trouble.is_empty() {
            message = trouble;
        }
        // A library file that could not be opened is reported by the library
        // panel, and the relinking pass says nothing about that file, so the
        // two messages stand together rather than one taking the place of
        // the other.
        message = say_both(&message, &relinking.message);
        let keys = records
            .iter()
            .filter_map(|record| Some((record.hash, record.key.as_ref()?.camelot)))
            .collect();
        let mut library = LibraryPanel::new(records);
        library.set_mix(timeline.mix());

        let mut handed = HashSet::new();
        let wanted = to_be_read(timeline.mix(), &relinking.no_file, &mut handed);
        let cache = Arc::new(Mutex::new(AudioCache::default()));
        let reading = Reading::start(wanted, library_file.file.clone(), Arc::clone(&cache), wake);

        // A track pointed at a new file is a document that no longer says
        // what the project file says, so a save is what keeps the new path.
        let mut document = document;
        if relinking.relinked {
            document.edited();
        }

        // The two path fields show the settings as the settings file has
        // them, which the window reads here because the settings themselves
        // move into the window below.
        let settings_music = settings.music_folder.clone();
        let settings_library = settings.library_file.clone();

        let mut window = Window {
            shown_title: document.title(),
            document,
            untitled_autosave: untitled_autosave_file(),
            quitting: false,
            timeline,
            transport: None,
            status: None,
            playback: Playback::new(Samples::ZERO),
            cache,
            reading,
            handed,
            editor: None,
            nudge_ms: 0.0,
            nudged_ms: 0.0,
            grid_tempo: TempoField::new(Bpm(f64::NAN), GRID_DECIMALS),
            metronome: settings.metronome(),
            strip_collapsed: settings.grid_strip_collapsed(),
            strip_px: GRID_STRIP_PX,
            settings_file,
            settings_open: false,
            buffer_text: buffer_field_text(settings.audio_buffer_frames),
            node_tempo: TempoField::new(Bpm(f64::NAN), TEMPO_DECIMALS),
            master_tempo: TempoField::new(Bpm(f64::NAN), TEMPO_DECIMALS),
            rows: library.rows(),
            library,
            searched: String::new(),
            filters: FilterFields::default(),
            library_collapsed: settings.library_collapsed(),
            library_word_wrap: settings.library_word_wrap(),
            settings,
            heights: RowHeights::default(),
            keys,
            reference_hash: None,
            reselected: false,
            clock: Instant::now(),
            pressed_at: None,
            dragging: false,
            resizing: false,
            fitted: false,
            message,
            started_over: None,
            corrections,
            overviews: HashMap::new(),
            phrases: HashMap::new(),
            restore: None,
            no_file: relinking.no_file,
            autosave_trouble: None,
            documents: None,
            library_file: library_file.file,
            music_text: path_field_text(settings_music.as_deref()),
            library_text: path_field_text(settings_library.as_deref()),
            scanning: None,
            refresh_due: false,
            last_refresh: Instant::now(),
            repaint: ctx.clone(),
        };
        if library_file.made {
            window.scan_the_music_folder(true);
        }
        window
    }

    /// Closes the grid editor when the track it was opened on is no longer
    /// the track at its position in the playlist.
    ///
    /// Removing a track, moving one, and undoing or redoing either all move
    /// the tracks after it, so this runs on every repaint, before anything
    /// reads the editor.
    fn close_a_stale_editor(&mut self) {
        let Some(open) = &self.editor else {
            return;
        };
        let at = self.timeline.mix().tracks.get(open.editor.track());
        if at.map(|track| track.hash) != Some(open.hash) {
            // Closing the editor gives back the audio device an audition of
            // its track was holding, and ends the correction, so a
            // correction started after the editor is opened again is a step
            // of its own.
            self.stop_the_audition();
            self.editor = None;
            self.timeline.end_grid_correction();
            self.message =
                "The beat grid editor was closed because its track moved in the playlist"
                    .to_owned();
        }
    }

    /// Takes in everything the reading thread has finished since the last
    /// repaint.
    ///
    /// An overview is proof that the file of the track it belongs to was
    /// read, so a note the relinking pass left about that track's file is
    /// dropped when one arrives. A track added again from the library after
    /// the pass could not find its file therefore shows its waveform without
    /// a note beside it.
    fn collect_findings(&mut self) {
        let mut findings = Vec::new();
        findings.extend(self.reading.findings());
        for finding in findings {
            match finding {
                Finding::Overview { hash, overview } => {
                    self.no_file.remove(&hash);
                    self.overviews.insert(hash, overview.clone());
                    self.timeline.set_overview(hash, overview);
                }
                Finding::Phrases { hash, phrases } => {
                    self.phrases.insert(hash, phrases.clone());
                    self.timeline.set_phrases(hash, phrases);
                }
                Finding::Trouble(problem) => self.message = problem,
            }
        }
    }

    /// Whether a scan of the music folder is running.
    fn a_scan_is_running(&self) -> bool {
        // The thread is asked as well as the window's own record of it, so a
        // thread that has ended without saying so leaves no scan standing in
        // the way of the next one.
        self.scanning
            .as_ref()
            .is_some_and(|running| running.scan.is_running())
    }

    /// Starts a scan of the music folder into the library the window has
    /// open, which is what **Library > Scan music folder** does and what a
    /// library the window has just made begins with.
    ///
    /// `made_the_library` says which of the two this is, because a music
    /// folder that is not there leaves a library the window has just made
    /// empty, and a person who has never chosen a music folder needs to be
    /// told where to choose one. Either way the window names the folder and
    /// the two ways to put that right.
    ///
    /// A window with no library file scans nothing, since there would be
    /// nowhere to put what the scan found, and the status line already says
    /// why there is no library file.
    fn scan_the_music_folder(&mut self, made_the_library: bool) {
        let Some(library) = self.library_file.clone() else {
            return;
        };
        let named = match music_folder(&self.settings) {
            Ok(folder) => folder,
            Err(problem) => {
                self.message = problem;
                return;
            }
        };
        let folder = match folder_to_scan(&named) {
            Ok(folder) => folder,
            Err(trouble) => {
                let empty = match made_the_library {
                    true => format!("The library at {} is empty. ", library.display()),
                    false => String::new(),
                };
                self.message = format!(
                    "{empty}{trouble} Choose a music folder in the settings dialog, then choose \
                     Library > Scan music folder."
                );
                return;
            }
        };
        let ctx = self.repaint.clone();
        let scan = Scan::start(&library, &folder, move || ctx.request_repaint());
        self.message = format!("Scanning {} into {}", folder.display(), library.display());
        self.scanning = Some(Scanning { scan, folder });
    }

    /// Asks the running scan to stop, which is what **Library > Stop scan**
    /// does. The scan finishes the file it is on, and the library keeps
    /// every file dealt with up to there.
    fn stop_the_scan(&mut self) {
        if let Some(running) = &self.scanning {
            running.scan.stop();
            self.message = "Stopping the scan after the file it is on".to_owned();
        }
    }

    /// Takes in everything the scan has reported since the last repaint:
    /// the progress on the status line, the library read again as tracks
    /// land, and the summary when the scan is over.
    ///
    /// A thread that ends without its last report, which a panic leaves
    /// behind, is noted on the status line and dropped, so the next scan can
    /// be started.
    ///
    /// A refresh is due after a file that was added, moved, or completed,
    /// since those are the three that change what the panel would show. A
    /// refresh reads every record in the library, sorts them all, and builds
    /// every row again, which is work worth doing a few times rather than
    /// once per file on a library of thousands of tracks, so while the scan
    /// runs the window refreshes at most once every [`REFRESH_GAP`]. The
    /// scan ending refreshes whatever the clock says, so the panel is right
    /// when the scan is over.
    fn collect_the_scan(&mut self) {
        let Some(running) = &self.scanning else {
            return;
        };
        let folder = running.folder.clone();
        let mut reports: Vec<Report> = running.scan.reports().collect();
        // A thread that has returned sent everything it had to send before
        // it returned, so a second look after that answer leaves nothing in
        // the channel unseen.
        let thread_ended = !running.scan.is_running();
        if thread_ended {
            reports.extend(running.scan.reports());
        }
        let mut over = false;
        for report in reports {
            match report {
                Report::File {
                    done,
                    total,
                    path,
                    change,
                } => {
                    self.message = format!("Scanning {done} of {total}: {}", file_name(&path));
                    self.refresh_due |=
                        matches!(change, Change::Added | Change::Moved | Change::Completed);
                }
                Report::Finished(Ok(summary)) => {
                    self.message = scan_summary(&folder, &summary);
                    self.refresh_due = true;
                    over = true;
                }
                Report::Finished(Err(problem)) => {
                    self.message = capitalized(problem);
                    over = true;
                }
            }
        }
        // A thread that ends without its last report has stopped in a way it
        // has no words for, so the window says that much, drops the scan so
        // that another can be started, and reads the library again for
        // whatever the scan did finish.
        if thread_ended && !over {
            self.message = format!(
                "The scan of {} ended without reporting what it did. The library keeps every file \
                 the scan had finished.",
                folder.display()
            );
            self.refresh_due = true;
        }
        if over || thread_ended {
            self.scanning = None;
        }
        if !self.refresh_due {
            return;
        }
        let over = over || thread_ended;
        if over || self.last_refresh.elapsed() >= REFRESH_GAP {
            self.refresh_due = false;
            self.last_refresh = Instant::now();
            self.read_the_library_again();
        } else {
            // Nothing else need repaint the window for the refresh that is
            // waiting to happen when the gap is up.
            self.repaint.request_repaint_after(REFRESH_GAP);
        }
    }

    /// Reads the library the window has open and hands its records to the
    /// panel, which keeps its search, its filters, its sort, and its
    /// selection.
    ///
    /// The keys come from the same records, and the rows are highlighted
    /// again against the track selected on the timeline, so a track a scan
    /// has just added goes green like any other track that mixes with the
    /// selected one. A key taken from a library row that was clicked stands
    /// until the next refresh or the next change of the timeline's
    /// selection, whichever comes first.
    fn read_the_library_again(&mut self) {
        let (records, trouble) = library_records(self.library_file.as_deref());
        if !trouble.is_empty() {
            self.message = say_both(&self.message, &trouble);
        }
        self.keys = records
            .iter()
            .filter_map(|record| Some((record.hash, record.key.as_ref()?.camelot)))
            .collect();
        self.library.set_records(records);
        // The panel drops the key it was highlighting against when the track
        // that key came from is no longer in the library, and it knows
        // nothing of the timeline's selection, so the window gives it that
        // key again from the track selected on the timeline.
        self.reference_hash = self.selected_hash();
        let key = self
            .reference_hash
            .and_then(|hash| self.keys.get(&hash).copied());
        self.library.set_reference(key);
        self.refresh_rows();
    }

    /// The content hash of the track selected on the timeline, which is the
    /// track the library rows are highlighted against.
    fn selected_hash(&self) -> Option<ContentHash> {
        self.selected_track()
            .and_then(|track| self.timeline.mix().tracks.get(track))
            .map(|track| track.hash)
    }

    /// Carries out what [`Playback`] asks of the transport, in the order it
    /// asks.
    ///
    /// The window keeps the transport and the audio device, and the
    /// playback state keeps the playhead, the start point, and what the
    /// window shows the person about the transport. This is the one place
    /// the window and the playback state meet.
    fn carry_out(&mut self, orders: Vec<TransportOrder>) {
        for order in orders {
            match order {
                TransportOrder::Start { at } => self.start_transport(at),
                TransportOrder::Resume => {
                    if let Some(transport) = &mut self.transport {
                        transport.resume();
                    }
                }
                TransportOrder::Pause => {
                    if let Some(transport) = &mut self.transport {
                        transport.pause();
                    }
                }
                TransportOrder::Seek { at } => {
                    if let Some(transport) = &mut self.transport {
                        transport.seek(at);
                    }
                }
                TransportOrder::Stop => {
                    if let Some(transport) = self.transport.take() {
                        transport.stop();
                    }
                    self.status = None;
                }
            }
        }
        self.timeline.set_playhead(self.playback.playhead());
    }

    /// Starts the preview at `at`, and tells the playback state when no
    /// transport could be started, so that the buttons go back to showing a
    /// mix that is not playing.
    ///
    /// The machine has one audio output, so an audition of a track on its
    /// own gives it up here rather than the two of them fighting over it.
    fn start_transport(&mut self, at: Samples) {
        let stopped_the_audition = self.stop_the_audition();
        let took_the_device =
            "The audition stopped so that the mix could use the audio device".to_owned();
        if stopped_the_audition {
            self.message = took_the_device.clone();
        }
        let output = match audio::output(self.settings.audio_buffer_frames) {
            Ok(output) => output,
            Err(problem) => {
                self.message = problem;
                self.playback.could_not_start();
                return;
            }
        };
        let started = Transport::start(
            self.timeline.mix().clone(),
            at,
            audio::loader(Arc::clone(&self.cache)),
            audio::stretchers(),
            output,
            audio::LOOKAHEAD.to_samples(),
        );
        match started {
            Ok(transport) => {
                self.transport = Some(transport);
                // A start that worked clears whatever the last message
                // was, except the one saying where the audio device came
                // from, which is the news of this very start.
                self.message = if stopped_the_audition {
                    took_the_device
                } else {
                    String::new()
                };
            }
            Err(problem) => {
                self.message = problem.to_string();
                self.playback.could_not_start();
            }
        }
    }

    /// Plays: starts the preview at the playhead, resumes a paused one, or
    /// takes a preview that has reached the end back to the start point and
    /// plays the same stretch again.
    fn play(&mut self) {
        let orders = self.playback.play(mix_length(self.timeline.mix()));
        self.carry_out(orders);
    }

    /// Holds the preview where it is.
    fn pause(&mut self) {
        let orders = self.playback.pause();
        self.carry_out(orders);
    }

    /// Ends the preview, gives the device back, and returns the playhead to
    /// the start point, so that playing again plays the same stretch again.
    fn stop(&mut self) {
        let orders = self.playback.stop(mix_length(self.timeline.mix()));
        self.carry_out(orders);
    }

    /// What the space bar does while no grid editor is open: stops a preview
    /// that is playing, returning the playhead to the start point, and
    /// otherwise plays. With an editor open the space bar plays and stops
    /// that editor's audition instead, through
    /// [`toggle_the_audition`](Window::toggle_the_audition).
    ///
    /// Playing covers both a preview that has never run and one that is
    /// paused, so the space bar resumes a pause rather than starting the
    /// stretch again. **Pause** is what keeps a position to come back to.
    fn toggle_play(&mut self) {
        let orders = self.playback.space(mix_length(self.timeline.mix()));
        self.carry_out(orders);
    }

    /// Writes the mix back to the file it was opened from, and says what
    /// became of the save. A mix with no file of its own asks for one, as
    /// **Save as...** does.
    ///
    /// A save that succeeds also removes the autosave file, which has nothing
    /// left to protect once the project file contains the same document.
    fn save(&mut self) -> Saved {
        let Some(path) = self.document.path().map(Path::to_path_buf) else {
            return self.save_as();
        };
        if !self.write_the_mix_to(&path) {
            return Saved::Failed;
        }
        self.forget_the_autosave();
        self.document.saved();
        self.message = format!("Saved {}", path.display());
        Saved::Written
    }

    /// Asks the operating system for a file, writes the mix to the file the
    /// person chose, and makes that file the mix's own from then on. Says
    /// what became of the save.
    ///
    /// A dialog the person cancels writes nothing and changes nothing, and a
    /// question standing over the window goes on standing, since nothing has
    /// become of the changes yet.
    fn save_as(&mut self) -> Saved {
        let Some(path) = ask_where_to_save(&self.saving_name()) else {
            return Saved::Cancelled;
        };
        if !self.write_the_mix_to(&path) {
            return Saved::Failed;
        }
        // The file that protected the changes protected the mix as it was
        // before this save, which is the untitled autosave file for a mix
        // that had no file and the file beside the old project file
        // otherwise. Either way the changes are in the chosen file now.
        self.forget_the_autosave();
        self.document.saved_as(path.clone());
        self.message = format!("Saved {}", path.display());
        Saved::Written
    }

    /// The name the save dialog offers to write under: the mix's own file
    /// name, and otherwise the untitled name with the extension a mix
    /// document goes by.
    fn saving_name(&self) -> String {
        match self.document.path() {
            Some(path) => file_name(path),
            None => format!("{}.{}", self.document.name(), MIX_FILTER.1),
        }
    }

    /// Writes the mix to `path` and says whether it was written, putting the
    /// reason in the status line when it was not.
    fn write_the_mix_to(&mut self, path: &Path) -> bool {
        // The write names the file itself when it fails, so the status line
        // takes the reason as it comes and only begins it as a sentence.
        match autosave::write_atomically(path, &self.timeline.mix().to_json()) {
            Ok(()) => true,
            Err(problem) => {
                self.message = capitalized(problem);
                false
            }
        }
    }

    /// The file the mix under the window is protected by: the file beside
    /// its project file, and for an untitled mix the one file in the user's
    /// data folder. A user with no data folder has no file for an untitled
    /// mix.
    fn autosave_file(&self) -> Option<PathBuf> {
        match self.document.path() {
            Some(project) => Some(autosave_path(project)),
            None => self.untitled_autosave.clone(),
        }
    }

    /// Writes the mix to its autosave file.
    ///
    /// This runs on every committed edit. An exit the window never sees (the
    /// quit keystroke on macOS, which reaches the application without
    /// passing through the window, or a machine that loses power) therefore
    /// costs at most the edit in progress, and the next time the mix is
    /// opened the window offers the autosaved document back.
    fn write_the_autosave(&mut self) {
        let Some(file) = self.autosave_file() else {
            self.autosave_trouble =
                Some("there is no data folder for this user to keep an untitled mix in".to_owned());
            return;
        };
        // The folder beside a project file is the folder the project file
        // was read from, and the data folder may never have been made, so
        // the window makes the folder the file goes in before it writes.
        if let Some(folder) = file.parent()
            && let Err(problem) = std::fs::create_dir_all(folder)
        {
            self.autosave_trouble = Some(format!(
                "cannot make the folder {}: {problem}",
                folder.display()
            ));
            return;
        }
        match autosave::write_atomically(&file, &self.timeline.mix().to_json()) {
            Ok(()) => self.autosave_trouble = None,
            Err(problem) => self.autosave_trouble = Some(problem),
        }
    }

    /// Removes the autosave file, which happens once the project file contains
    /// the same document or the person has said what should become of the
    /// changes. A file that is not there is already forgotten.
    fn forget_the_autosave(&mut self) {
        let Some(file) = self.autosave_file() else {
            return;
        };
        if let Err(problem) = forget_file(&file) {
            self.message = capitalized(problem);
        }
    }

    /// Draws the dialog that offers the autosaved document back, while there
    /// is one to offer.
    ///
    /// The mouse and the keyboard do not reach the timeline while it is
    /// open, so the first thing a person does with a mix whose last session
    /// ended without a save is to say what should become of that work.
    fn restore_dialog(&mut self, ctx: &egui::Context) {
        // The two moments come from the offer, which read them when it read
        // the autosave file, so the dialog says the same thing however long
        // it stands open.
        let Some(offer) = &self.restore else {
            return;
        };
        let autosaved = how_long_ago(offer.autosaved);
        let saved = offer.saved.map(|written| how_long_ago(Some(written)));
        egui::Modal::new(egui::Id::new("restore")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Restore unsaved changes?");
            let file = self.autosave_file().unwrap_or_default();
            let which = match self.document.path() {
                Some(path) => format!("the mix {}", path.display()),
                None => "an untitled mix".to_owned(),
            };
            ui.label(format!(
                "The last session with {which} ended without saving, and the window kept the work."
            ));
            ui.add_space(4.0);
            ui.label(format!(
                "The unsaved changes are in {}, written {autosaved}.",
                file.display()
            ));
            // An untitled mix has no project file to set beside the autosave
            // file, so the dialog says nothing about one.
            if let (Some(path), Some(saved)) = (self.document.path(), &saved) {
                ui.label(format!(
                    "The mix document {} was written {saved}.",
                    path.display()
                ));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Restore unsaved changes").clicked()
                    && let Some(offer) = self.restore.take()
                {
                    let library = self.library_file.clone();
                    self.restore_the_document(library.as_deref(), offer.mix);
                }
                if ui.button("Discard them").clicked() {
                    self.restore = None;
                    self.forget_the_autosave();
                    self.message = "Discarded the unsaved changes".to_owned();
                }
            });
        });
    }

    /// Puts the autosaved document under the window, after looking again
    /// for the files of the tracks it names.
    ///
    /// The autosaved document names the paths the last session wrote, so a
    /// track this session relinked as it opened the mix would go back to the
    /// path its file has left. The same pass over the restored document
    /// points those tracks at their files again, and it finds a track whose
    /// file has moved since the window opened. What the pass finds is what
    /// the lanes and the status line then say, and a path it wrote is kept
    /// by a save.
    fn restore_the_document(&mut self, index: Option<&Path>, mut mix: Mix) {
        let relinking = relink_from_the_index(index, &mut mix);
        self.no_file = relinking.no_file;
        self.put_under_the_window(mix);
        // The restored document is not what the project file contains, so it is
        // unsaved work like any other, and the autosave file goes on
        // protecting it.
        self.document.edited();
        self.message = say_both(
            "Restored the unsaved changes. Save to keep them.",
            &relinking.message,
        );
    }

    /// Puts a different document under the window.
    ///
    /// A timeline holds one document for the whole of its life, so restoring
    /// the autosaved document means a new timeline. It is given the
    /// corrections folder and every overview and phrase record the reading
    /// thread has found so far, so the lanes are drawn as fully as they were,
    /// and the view is fitted to the restored mix. Any track of the document
    /// whose content hash the window has not already handed the reading
    /// thread, and whose file the window found, is asked for, so its lane
    /// fills in once the thread gets to it.
    fn put_under_the_window(&mut self, mix: Mix) {
        let mut timeline = Timeline::new(mix);
        if let Some(dir) = &self.corrections {
            timeline.set_corrections_dir(dir.clone());
        }
        for (hash, overview) in &self.overviews {
            timeline.set_overview(*hash, overview.clone());
        }
        for (hash, phrases) in &self.phrases {
            timeline.set_phrases(*hash, phrases.clone());
        }
        for want in to_be_read(timeline.mix(), &self.no_file, &mut self.handed) {
            self.reading.want(want);
        }
        self.timeline = timeline;
        self.stop_the_audition();
        self.editor = None;
        self.fitted = false;
        self.library.set_mix(self.timeline.mix());
        self.refresh_rows();
        let mix = self.timeline.mix().clone();
        if let Some(transport) = &mut self.transport {
            transport.replace(mix);
        }
    }

    /// Whether a dialog is open, which is when the mouse and the keyboard do
    /// not reach the timeline.
    fn a_dialog_is_open(&self) -> bool {
        self.restore.is_some() || self.document.asking() || self.settings_open
    }

    /// Takes what the person asked for, which is **New**, **Open**, or
    /// **Quit**: it happens at once when the mix has no unsaved changes, and
    /// otherwise the window asks what should become of them first.
    fn ask_or_do(&mut self, intent: Intent, ctx: &egui::Context) {
        match self.document.request(intent) {
            Step::Proceed(intent) => self.act_on_the_intent(intent, ctx),
            Step::Ask(_) => {
                // One dialog stands over the window at a time. The settings
                // dialog is drawn after the question, so a settings dialog
                // left open would cover it. Closing the settings dialog here
                // reads its three text fields, as the dialog's own **Close**
                // button does, so a size or a path typed and left in a field
                // is kept.
                if self.settings_open {
                    self.take_the_buffer();
                    self.take_the_music_folder();
                    self.take_the_library_file();
                    self.settings_open = false;
                }
            }
        }
    }

    /// Opens every document macOS has asked the window to open since the
    /// last repaint, which are the `.dmx` files double-clicked in the Finder
    /// and the ones dropped on the Dermixen icon.
    ///
    /// Each file takes the route **File > Open...** takes, so the window
    /// asks what should become of unsaved changes first when the mix it
    /// holds has any, prints its `Opened` line for the file, and puts a
    /// file that is no mix document in the status line instead of under the
    /// window. Of two files that arrive together while the mix has unsaved
    /// changes, the second is the one waiting on the question, as it is
    /// when the menu is used twice, because the newest request is the one
    /// the answer applies to.
    ///
    /// A file that arrives while one of the two dialogs that ask a question
    /// stands open waits in the channel until that dialog is answered.
    fn open_what_macos_sent(&mut self, ctx: &egui::Context) {
        // Neither question may be answered by a file arriving. Opening a
        // document while the question about unsaved changes stands would
        // put another intent in the place of the one the question was asked
        // for, and opening one while the dialog offering an autosave back
        // stands would take that dialog off the screen with nothing said
        // about the work it holds. The paths stay in the channel, which the
        // repaint after the answer reads.
        if self.document.asking() || self.restore.is_some() {
            return;
        }
        let Some(documents) = &self.documents else {
            return;
        };
        let paths: Vec<PathBuf> = documents.try_iter().collect();
        for path in paths {
            self.ask_or_do(Intent::Open(path), ctx);
        }
    }

    /// Does what the person asked for, once nothing stands in the way.
    fn act_on_the_intent(&mut self, intent: Intent, ctx: &egui::Context) {
        match intent {
            Intent::New => self.put_a_document_under_the_window(Document::untitled(), Mix::new()),
            Intent::Open(path) => match read_the_mix(&path) {
                Ok(mix) => self.put_a_document_under_the_window(Document::at(path), mix),
                // The mix under the window stays where it is, with its
                // changes, since the file the person chose is no mix to put
                // in its place.
                Err(problem) => self.message = capitalized(problem),
            },
            Intent::Quit => self.close(ctx),
        }
    }

    /// Puts `mix`, which is the mix `document` names, under the window in
    /// place of the mix that is there.
    ///
    /// The window sets the new document up as it sets up the document it
    /// opens with: the same relinking pass, the same corrections folder, the
    /// same library rows with the new document's tracks marked, the same
    /// reading thread, the same offer of an autosave, and the same title,
    /// while the history, the preview, and the grid editor that belonged to
    /// the mix before are dropped. A document whose tracks the relinking
    /// pass moved starts with unsaved changes, as it does when the window
    /// opens. The file that protected the mix before is removed, because the
    /// person has already said what should become of its changes.
    fn put_a_document_under_the_window(&mut self, document: Document, mut mix: Mix) {
        self.stop();
        // One dialog stands over the window at a time, and a settings dialog
        // left open would cover the mix coming in. Closing it here reads its
        // three text fields, as the dialog's own **Close** button does and
        // as the question about unsaved changes does, so a size or a path
        // typed and left in a field is kept. The fields are read before the
        // library file below is worked out, so a library file typed and left
        // in its field is the one the mix coming in opens.
        if self.settings_open {
            self.take_the_buffer();
            self.take_the_music_folder();
            self.take_the_library_file();
            self.settings_open = false;
        }
        // Nothing the mix before it put in the status line holds for the mix
        // coming in, except a reason the file that protected it could not be
        // removed, which the removal below leaves here.
        self.message = String::new();
        self.forget_the_autosave();
        self.document = document;
        self.restore = None;
        self.autosave_trouble = None;
        self.started_over = None;
        self.playback = Playback::new(Samples::ZERO);
        // A tempo field writes what was typed into it when it notices that it
        // has lost the keyboard, and the menu bar is drawn above the row the
        // fields are on, so a number typed and left in a field when **New**
        // or **Open...** was chosen would land on the new document's
        // timeline. The three fields start again, empty of any typing, with
        // the mix that comes in.
        self.master_tempo = TempoField::new(Bpm(f64::NAN), TEMPO_DECIMALS);
        self.node_tempo = TempoField::new(Bpm(f64::NAN), TEMPO_DECIMALS);
        self.grid_tempo = TempoField::new(Bpm(f64::NAN), GRID_DECIMALS);
        // What the autosave file holds is read against the mix as its own
        // file gives it, before the relinking pass changes any path.
        let offer = offer_for(&self.document, &mix);
        // The library file the settings name is opened again for every mix
        // the window puts under itself, so a `library_file` setting changed
        // in the settings dialog takes effect here.
        let library_file = library_file_for(&self.settings);
        self.library_file = library_file.file.clone();
        if !library_file.message.is_empty() {
            self.message = library_file.message;
        }
        self.read_the_library_again();
        self.start_reading_again();
        let relinking = relink_from_the_index(self.library_file.as_deref(), &mut mix);
        self.no_file = relinking.no_file;
        self.put_under_the_window(mix);
        if relinking.relinked {
            self.document.edited();
        }
        self.message = say_both(&self.message, &relinking.message);
        say_which_file_was_opened(self.document.path());
        self.take_the_offer(offer);
        // A scan already running goes on running, since a scan belongs to
        // the library rather than to the mix under the window. The library
        // this mix opened is empty all the same, and a person who reads
        // nothing about it would take an empty panel for a library that
        // could not be read.
        if library_file.made {
            match (self.a_scan_is_running(), self.library_file.clone()) {
                (true, Some(library)) => {
                    self.message = format!(
                        "The library at {} is empty. Library > Scan music folder fills it once \
                         the running scan ends.",
                        library.display()
                    );
                }
                _ => self.scan_the_music_folder(true),
            }
        }
    }

    /// Starts the reading thread again on the library file the window has
    /// open now, and forgets which tracks were handed to the thread before.
    ///
    /// A thread looks its phrase marks up in the one library file it was
    /// started with and keeps that file open for its whole life, so a mix
    /// opened after the `library_file` setting changed needs a thread of its
    /// own. The decoded audio is kept, because the two threads share the
    /// same cache, so a track the window has already read is played from the
    /// audio in hand while its file is read again for the new library's
    /// phrase marks. The thread that was reading before ends once it has
    /// finished the track it is on, since nothing is left to ask it for.
    fn start_reading_again(&mut self) {
        let ctx = self.repaint.clone();
        self.reading = Reading::start(
            Vec::new(),
            self.library_file.clone(),
            Arc::clone(&self.cache),
            move || ctx.request_repaint(),
        );
        self.handed.clear();
    }

    /// Takes what the autosave file of the mix just opened contains: a
    /// document to offer back, a reason the file could not be read, or
    /// nothing.
    fn take_the_offer(&mut self, offer: Offer) {
        match offer {
            Offer::Nothing => {}
            Offer::Restore {
                mix,
                autosaved,
                saved,
            } => {
                self.restore = Some(Restore {
                    mix,
                    autosaved,
                    saved,
                });
            }
            // Both what the relinking pass found and the autosave file that
            // cannot be read are worth reading, so neither takes the place
            // of the other.
            Offer::Unreadable(problem) => {
                self.message = say_both(&self.message, &capitalized(problem));
            }
        }
    }

    /// Holds the window open when the person closes it while the mix holds
    /// changes that were never saved, and opens the dialog that asks what to
    /// do with them.
    ///
    /// Never losing work is part of "bulletproof", the first of the two
    /// values `DESIGN.md` puts above every feature, and a window that closed
    /// here would lose every edit made since the last save without saying so.
    fn consider_closing(&mut self, ctx: &egui::Context) {
        if !ctx.input(|input| input.viewport().close_requested()) {
            return;
        }
        if self.quitting {
            // The person has said what should become of the changes, and
            // [`close`](Window::close) has already removed the autosave file.
            return;
        }
        if self.restore.is_some() {
            // The unsaved work is the autosave file itself, and the dialog
            // offering it back has not been answered, so the window neither
            // closes nor removes the file. Answering the dialog is what makes
            // a close possible.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        self.ask_or_do(Intent::Quit, ctx);
        if self.document.asking() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    /// Draws the question about changes that were never saved, while the
    /// window is asking it, and takes the answer.
    fn question_dialog(&mut self, ctx: &egui::Context) {
        if !self.document.asking() {
            return;
        }
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading(format!("Save changes to {}?", self.document.name()));
            ui.label(match self.document.path() {
                Some(path) => format!("{} has unsaved changes.", path.display()),
                None => "This mix has never been saved to a file.".to_owned(),
            });
            ui.add_space(8.0);
            // Every button is drawn on every repaint, so the three answers
            // stand together however the person answers.
            let answer = ui
                .horizontal(|ui| {
                    let save = ui.button("Save").clicked();
                    let discard = ui.button("Don't save").clicked();
                    let cancel = ui.button("Cancel").clicked();
                    match (save, discard, cancel) {
                        (true, _, _) => Some(Answer::Save),
                        (_, true, _) => Some(Answer::Discard),
                        (_, _, true) => Some(Answer::Cancel),
                        _ => None,
                    }
                })
                .inner;
            if let Some(answer) = answer {
                self.answer_the_question(answer, ctx);
            }
        });
    }

    /// Takes one answer to the question about changes that were never saved.
    ///
    /// A save that fails leaves the question standing with the reason in the
    /// status line, so the work is still there to save somewhere else. So
    /// does a save of an untitled mix whose dialog the person cancels, and
    /// the status line then says that nothing was saved, since a question
    /// that comes back with nothing said would look like a window that had
    /// not heard the answer.
    fn answer_the_question(&mut self, answer: Answer, ctx: &egui::Context) {
        match self.document.answered(answer) {
            Next::Stay => {}
            Next::Proceed(intent) => self.act_on_the_intent(intent, ctx),
            Next::SaveThen(_) => match self.save() {
                Saved::Written => {
                    if let Some(intent) = self.document.take_pending() {
                        self.act_on_the_intent(intent, ctx);
                    }
                }
                Saved::Cancelled => {
                    self.message = "No file was chosen, so nothing was saved".to_owned();
                }
                Saved::Failed => {}
            },
        }
    }

    /// Closes the window, and removes the autosave file on the way out,
    /// since the person has said what should become of the changes.
    fn close(&mut self, ctx: &egui::Context) {
        self.forget_the_autosave();
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Writes every setting to the settings file, and says why on the
    /// status line when the write fails.
    ///
    /// A write that fails leaves the window holding the setting the person
    /// chose, so the metronome and the buffer are what they asked for until
    /// the window closes even where the file cannot keep the choice for the
    /// next window. The answer is whether the file was written, which is
    /// what lets the caller put its own message up in place of this one.
    fn write_the_settings(&mut self) -> bool {
        match self.settings.write(&self.settings_file) {
            Ok(()) => true,
            Err(problem) => {
                self.message = format!(
                    "The settings file {} {problem}",
                    self.settings_file.display()
                );
                false
            }
        }
    }

    /// Takes the metronome being turned on or off, in the grid editor's
    /// checkbox or in the settings dialog's, and keeps the choice in the
    /// settings file, so the click is as the person left it the next time
    /// the window opens.
    ///
    /// A running audition is told by whoever holds it rather than here,
    /// since the grid editor holds its audition while it draws itself.
    fn metronome_changed(&mut self, on: bool) {
        self.metronome = on;
        self.settings.metronome = Some(on);
        self.write_the_settings();
    }

    /// Takes the grid editor's strip being collapsed or expanded, by the
    /// editor's own control or by the settings dialog's checkbox, and keeps
    /// the choice in the settings file, so the strip is as the person left
    /// it the next time the window opens.
    ///
    /// An editor that is open reads this on its next repaint, so the strip
    /// goes away or comes back at once. A strip hidden after it was shown
    /// comes back at the height and the view it had. An editor opened while
    /// the strip is collapsed was never told a width, since
    /// [`grid_strip`](Window::grid_strip) draws nothing and calls
    /// [`GridEditor::set_width`] not at all, so the first width it is given
    /// frames the whole track and its strip opens showing the whole track.
    ///
    /// A file that cannot be written leaves the strip as the person set it
    /// for as long as the window is open, with the reason on the status
    /// line, as a metronome that cannot be written does.
    fn strip_collapsed_changed(&mut self, collapsed: bool) {
        self.strip_collapsed = collapsed;
        self.settings.grid_strip_collapsed = Some(collapsed);
        self.write_the_settings();
    }

    /// Takes the library panel being hidden or shown, by the panel's toggle
    /// in the first row of controls, by **View > Show library** or
    /// **View > Hide library**, or by the settings dialog's **Library
    /// hidden** checkbox, and keeps the choice in the settings file, so the
    /// panel is as the person left it the next time the window opens.
    ///
    /// The window keeps reading the library at open whether the panel is
    /// hidden or shown, and keeps the panel's rows and the key they are
    /// highlighted against up to date while the panel is hidden, so showing
    /// the panel again puts it back with nothing to wait for. egui keeps the
    /// panel's width, so a panel shown after it was hidden comes back at the
    /// width it had.
    ///
    /// A file that cannot be written leaves the panel as the person set it
    /// for as long as the window is open, with the reason on the status
    /// line, as a strip that cannot be written does.
    fn library_collapsed_changed(&mut self, collapsed: bool) {
        self.library_collapsed = collapsed;
        self.settings.library_collapsed = Some(collapsed);
        self.write_the_settings();
    }

    /// Takes the library table's cells being set to wrap their text or to
    /// stand on one line, by the settings dialog's **Wrap library cells**
    /// checkbox, and keeps the choice in the settings file, so the table is
    /// as the person left it the next time the window opens.
    ///
    /// The table follows on its next repaint. The list of row heights is
    /// marked stale, so the next repaint that wraps works the list out
    /// again. Each track's own height and each cell's own width stay, since
    /// both are right for the column widths they were worked out at.
    ///
    /// A file that cannot be written leaves the table as the person set it
    /// for as long as the window is open, with the reason on the status
    /// line, as a panel that cannot be written does.
    fn library_word_wrap_changed(&mut self, wrap: bool) {
        self.library_word_wrap = wrap;
        self.settings.library_word_wrap = Some(wrap);
        self.heights.forget();
        self.write_the_settings();
    }

    /// Reads the settings dialog's audio buffer field as the buffer setting
    /// and writes the settings file.
    ///
    /// Text the field takes becomes the setting, and the status line says
    /// that the size takes effect the next time playing starts, since a
    /// device already open keeps the size it was opened with. Text that
    /// reads as the size the setting already has writes nothing and says
    /// nothing, as the tempo fields write nothing for a tempo they already
    /// show, so a field a person clicked into and left alone changes
    /// neither the file nor the status line. Text the field refuses leaves
    /// the setting as it was and puts the refusal on the status line, and
    /// the field goes back to showing the setting, again as the tempo
    /// fields do with text they refuse.
    fn take_the_buffer(&mut self) {
        match parse_buffer_frames(&self.buffer_text) {
            Ok(frames) if frames == self.settings.audio_buffer_frames => {
                self.buffer_text = buffer_field_text(frames);
            }
            Ok(frames) => {
                self.settings.audio_buffer_frames = frames;
                self.buffer_text = buffer_field_text(frames);
                if self.write_the_settings() {
                    self.message = match frames {
                        Some(frames) => format!(
                            "The audio buffer is {frames} frames, which takes effect the next time playing starts"
                        ),
                        None => "The audio buffer is the device's own size, which takes effect the next time playing starts"
                            .to_owned(),
                    };
                }
            }
            Err(problem) => {
                self.buffer_text = buffer_field_text(self.settings.audio_buffer_frames);
                self.message = problem;
            }
        }
    }

    /// Reads the settings dialog's music folder field as the music folder
    /// setting and writes the settings file.
    ///
    /// An empty field leaves the setting unset, which is `Music/Undefunktis`
    /// below the user's home folder. Text that names the folder the setting
    /// already has writes nothing and says nothing, as the audio buffer
    /// field does for a size it already has. The status line names the
    /// folder in use and the menu item that scans it, because nothing is
    /// scanned until the person asks.
    fn take_the_music_folder(&mut self) {
        let chosen = path_setting(&self.music_text);
        self.music_text = path_field_text(chosen.as_deref());
        if chosen == self.settings.music_folder {
            return;
        }
        self.settings.music_folder = chosen;
        if self.write_the_settings() {
            self.message = match music_folder(&self.settings) {
                Ok(folder) => format!(
                    "The music folder is {}. Choose Library > Scan music folder to scan it.",
                    folder.display()
                ),
                Err(problem) => problem,
            };
        }
    }

    /// Reads the settings dialog's library file field as the library file
    /// setting and writes the settings file.
    ///
    /// An empty field leaves the setting unset, which is
    /// `dermixen/library.sqlite` in the user's data folder. The window goes
    /// on reading the library it already has open, so the status line says
    /// that the file chosen here takes effect the next time a mix is opened
    /// or a new one started.
    fn take_the_library_file(&mut self) {
        let chosen = path_setting(&self.library_text);
        self.library_text = path_field_text(chosen.as_deref());
        if chosen == self.settings.library_file {
            return;
        }
        self.settings.library_file = chosen;
        if !self.write_the_settings() {
            return;
        }
        // The environment variable comes before the setting, so while the
        // variable names a file the window would otherwise report a path the
        // person did not type.
        if let Some(named) = named_library_file() {
            self.message = format!(
                "The library file setting was written. {LIBRARY_VARIABLE} names {}, and the \
                 setting is not read until that variable is unset.",
                named.display()
            );
            return;
        }
        self.message = match library_location(&self.settings) {
            Ok(file) => format!(
                "The library file is {}, which takes effect the next time you open a mix or start \
                 a new one.",
                file.display()
            ),
            Err(problem) => problem,
        };
    }

    /// Draws the settings dialog, while it is open.
    ///
    /// The dialog shows the settings file's path and every setting, and a
    /// change made here is written to that file at once, which is what
    /// `docs/settings.md` describes. It is a dialog like the two that ask
    /// about unsaved changes, so neither the mouse nor the keyboard reaches
    /// the timeline while it stands open.
    fn settings_dialog(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        // A field that is empty shows the path in use, which is what the
        // setting would name if it were set, so the person reads which
        // folder and which file the window is working with either way.
        let music_hint = music_folder(&self.settings)
            .map(|folder| folder.display().to_string())
            .unwrap_or_default();
        let library_hint = library_location(&self.settings)
            .map(|file| file.display().to_string())
            .unwrap_or_default();
        egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.heading("Settings");
            ui.label(format!(
                "These settings are kept in {}.",
                self.settings_file.display()
            ));
            ui.add_space(8.0);
            let typing = ui
                .horizontal(|ui| {
                    ui.label("Audio buffer");
                    let mut text = self.buffer_text.clone();
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(72.0)
                            .id_salt("audio buffer"),
                    );
                    if field.changed() {
                        self.buffer_text = text;
                    }
                    ui.label("frames");
                    // The text is read once, when the person presses return
                    // or the field loses the keyboard, so that a number half
                    // typed is never refused on its way to being whole.
                    if field.lost_focus() {
                        self.take_the_buffer();
                    }
                    field.has_focus()
                })
                .inner;
            ui.label(
                "An empty field leaves the size to the device. \
                 The size takes effect the next time playing starts.",
            );
            ui.add_space(8.0);
            let mut metronome = self.metronome;
            if ui.checkbox(&mut metronome, "Metronome").changed() {
                self.metronome_changed(metronome);
                // The grid editor's audition, when one is running, hears the
                // change from the next frame the audio device takes, as it
                // does when the editor's own checkbox is clicked.
                if let Some(open) = &mut self.editor
                    && let Some(audition) = &mut open.audition
                {
                    audition.set_click(metronome);
                }
            }
            ui.label("The metronome clicks on the beats of the grid in the beat grid editor.");
            ui.add_space(8.0);
            let mut collapsed = self.strip_collapsed;
            if ui
                .checkbox(&mut collapsed, "Grid strip collapsed")
                .changed()
            {
                self.strip_collapsed_changed(collapsed);
            }
            ui.label(
                "The beat grid editor shows its row of controls alone, \
                 without the waveform strip beneath them.",
            );
            ui.add_space(8.0);
            let mut hidden = self.library_collapsed;
            if ui.checkbox(&mut hidden, "Library hidden").changed() {
                self.library_collapsed_changed(hidden);
            }
            ui.label(
                "The library panel is hidden, so the timeline has the whole width of the window.",
            );
            ui.add_space(8.0);
            let mut wrap = self.library_word_wrap;
            if ui.checkbox(&mut wrap, "Wrap library cells").changed() {
                self.library_word_wrap_changed(wrap);
            }
            ui.label(
                "A cell of the library table wraps its text onto further lines, \
                 and a row is as tall as its tallest cell. Without this, every row \
                 is one line and text too wide for its column is cut off at the \
                 column's edge.",
            );
            ui.add_space(8.0);
            let music_typing = ui
                .horizontal(|ui| {
                    ui.label("Music folder");
                    let mut text = self.music_text.clone();
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(PATH_FIELD_PX)
                            .id_salt("music folder")
                            .hint_text(&music_hint),
                    );
                    if field.changed() {
                        self.music_text = text;
                    }
                    if field.lost_focus() {
                        self.take_the_music_folder();
                    }
                    if ui.button("Choose...").clicked()
                        && let Some(folder) = ask_for_a_folder("Choose the music folder")
                    {
                        self.music_text = folder.display().to_string();
                        self.take_the_music_folder();
                    }
                    field.has_focus()
                })
                .inner;
            ui.label("Library > Scan music folder scans this folder into the library.");
            ui.add_space(8.0);
            let library_typing = ui
                .horizontal(|ui| {
                    ui.label("Library file");
                    let mut text = self.library_text.clone();
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(PATH_FIELD_PX)
                            .id_salt("library file")
                            .hint_text(&library_hint),
                    );
                    if field.changed() {
                        self.library_text = text;
                    }
                    if field.lost_focus() {
                        self.take_the_library_file();
                    }
                    if ui.button("Choose...").clicked()
                        && let Some(file) = ask_for_a_library_file()
                    {
                        self.library_text = file.display().to_string();
                        self.take_the_library_file();
                    }
                    field.has_focus()
                })
                .inner;
            ui.label(
                "The file chosen here takes effect the next time you open a mix or start a new \
                 one. The window makes the file when it is not there yet.",
            );
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                // Reading a field here is what keeps a size or a path typed
                // into it and followed at once by **Close**. Reading the
                // same text a second time writes nothing, since the text
                // then reads as the setting the window already has.
                if typing {
                    self.take_the_buffer();
                }
                if music_typing {
                    self.take_the_music_folder();
                }
                if library_typing {
                    self.take_the_library_file();
                }
                self.settings_open = false;
            }
        });
    }

    /// Widens the view until it holds the whole mix and puts its left edge at
    /// the start.
    fn show_whole_mix(&mut self) {
        // Widening is clamped to the mix's length plus a minute, so any
        // factor small enough reaches that width in one step. The view then
        // holds more than the whole mix, so moving it one screen earlier
        // reaches the start, which is as far back as it goes.
        self.timeline.zoom_by(f64::MIN_POSITIVE, 0.0);
        self.timeline.scroll_by(-self.timeline.view().width_px);
    }

    /// The selected tempo node, named by its track and its beat of that
    /// track, together with the tempo it holds.
    ///
    /// The node is looked up in the committed document by the beat the
    /// selection names, so the field shows the tempo the curve actually
    /// holds rather than anything the window keeps of its own.
    fn selected_tempo(&self) -> Option<((usize, Beats), Bpm)> {
        let Selection::TempoNode { track, at } = self.timeline.selection() else {
            return None;
        };
        let node = self
            .timeline
            .mix()
            .tracks
            .get(track)?
            .tempo
            .iter()
            .find(|node| node.at.0 == at.0)?;
        Some(((track, node.at), node.bpm))
    }

    /// Puts a tempo the person finished typing onto the tempo node the field
    /// settled on when the typing began.
    ///
    /// The node is the one the typing began for rather than the one selected
    /// now, so a person who types a tempo and then clicks elsewhere still
    /// changes the node they meant.
    fn take_node_tempo(&mut self, finish: Finish<(usize, Beats)>) {
        match finish {
            Finish::Written((track, at), bpm) => {
                let moved = Edit::MoveTempoNode {
                    track,
                    from: at,
                    to: TempoNode { at, bpm },
                };
                if let Err(problem) = self.timeline.apply(moved) {
                    self.message = format!("The tempo could not be changed: {problem}");
                }
            }
            Finish::Refused(typed) => self.message = not_a_tempo(typed.trim()),
            Finish::Unchanged => {}
        }
    }

    /// The track the person has selected, whatever part of it is selected.
    fn selected_track(&self) -> Option<usize> {
        match self.timeline.selection() {
            Selection::Track(track)
            | Selection::Node { track, .. }
            | Selection::Anchor { track, .. }
            | Selection::TempoNode { track, .. } => Some(track),
            Selection::Nothing => None,
        }
    }

    /// Reads the keyboard: the space bar plays and pauses, delete and
    /// backspace remove what is selected, and the menu's own shortcuts do
    /// what the menu item does.
    ///
    /// Three of those keys belong to the grid editor while one is open. The
    /// command key with Z takes one adjustment of the correction back and
    /// the command key with shift and Z makes one again, and neither of the
    /// two reaches the mix's own history while the editor is open. The space
    /// bar plays and stops the editor's audition rather than the preview of
    /// the mix, and an audition the space bar starts stops a preview that is
    /// playing, because the machine has one audio output. The editor stays
    /// open whatever the key finds to do.
    ///
    /// The space bar answers the first press of the key and not the repeats
    /// a held key sends, since starting and stopping on every repeat would
    /// take the audio device and give it back many times a second. The
    /// command key with Z answers every repeat, as the arrow keys do, so a
    /// held key walks back through the editor's adjustments one repeat at a
    /// time.
    ///
    /// None of it is read while a field is being typed in. A space in a
    /// tempo field has to stay a space, and the undo shortcut pressed inside
    /// a field has to undo the typing rather than the last edit to the mix.
    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.memory(|memory| memory.focused()).is_some() || self.a_dialog_is_open() {
            return;
        }

        // Every shortcut that holds shift is looked for before the one
        // without it, because a shortcut match ignores a shift the shortcut
        // did not ask for, so the command key with Z would otherwise answer
        // the redo shortcut as well.
        if ctx.input_mut(|input| input.consume_shortcut(&REDO)) {
            if self.editor.is_some() {
                self.redo_an_adjustment();
            } else {
                self.timeline.redo();
            }
        } else if ctx.input_mut(|input| input.consume_shortcut(&UNDO)) {
            if self.editor.is_some() {
                self.undo_an_adjustment();
            } else {
                self.timeline.undo();
            }
        }
        if ctx.input_mut(|input| input.consume_shortcut(&SAVE_AS)) {
            self.save_as();
        } else if ctx.input_mut(|input| input.consume_shortcut(&SAVE)) {
            let _ = self.save();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&NEW)) {
            self.ask_or_do(Intent::New, ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&OPEN))
            && let Some(path) = ask_for_a_mix_to_open()
        {
            self.ask_or_do(Intent::Open(path), ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&QUIT)) {
            self.ask_or_do(Intent::Quit, ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&SETTINGS)) {
            self.open_the_settings_dialog();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&ZOOM_IN)) {
            self.zoom_in();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&ZOOM_OUT)) {
            self.zoom_out();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&WHOLE_MIX)) {
            self.show_whole_mix();
        }
        if ctx.input_mut(|input| input.consume_shortcut(&LIBRARY_PANEL)) {
            self.library_collapsed_changed(!self.library_collapsed);
        }
        // A held space bar repeats, and egui counts every repeat as a press
        // of its own, so the first press of the key is picked out of the
        // events by hand. Answering the repeats would start and stop the
        // preview, or the audition, many times a second, and each start and
        // stop takes the audio device and gives it back.
        let space = ctx.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: Key::Space,
                        pressed: true,
                        repeat: false,
                        ..
                    }
                )
            })
        });
        if space {
            if self.editor.is_some() {
                self.toggle_the_audition();
            } else {
                self.toggle_play();
            }
        }
        if ctx.input(|input| input.key_pressed(Key::Delete) || input.key_pressed(Key::Backspace)) {
            self.timeline.delete_selection();
        }
        self.arrow_keys(ctx);
    }

    /// Takes back the last adjustment made in the open grid editor, which is
    /// what the command key with Z does while an editor is open.
    ///
    /// An adjustment is one nudge, one arrow key press, one halving or
    /// doubling, one typed or tapped tempo, or one drag across the strip, as
    /// [`GridEditor::undo`] describes. The grid the editor is left with goes
    /// on the track through
    /// [`put_the_grid_on_the_track`](Window::put_the_grid_on_the_track), the
    /// way the grid goes on the track after any other adjustment, so
    /// [`Timeline::set_grid`] keeps the whole correction as one step of the
    /// mix's own history rather than stacking another step on it. With no
    /// adjustment left to take back, nothing changes and the mix's history
    /// stays where it is. The editor stays open either way.
    fn undo_an_adjustment(&mut self) {
        let Some(mut open) = self.editor.take() else {
            return;
        };
        if open.editor.undo() {
            self.put_the_grid_on_the_track(&mut open);
        }
        self.editor = Some(open);
    }

    /// Makes the adjustment the last undo in the open grid editor took back
    /// again, which is what the command key with shift and Z does while an
    /// editor is open. It follows the rule
    /// [`undo_an_adjustment`](Window::undo_an_adjustment) follows, with
    /// [`GridEditor::redo`] in place of [`GridEditor::undo`].
    fn redo_an_adjustment(&mut self) {
        let Some(mut open) = self.editor.take() else {
            return;
        };
        if open.editor.redo() {
            self.put_the_grid_on_the_track(&mut open);
        }
        self.editor = Some(open);
    }

    /// Stops the grid editor's audition, or starts one when none is running,
    /// which is what the space bar does while an editor is open.
    ///
    /// An audition started here begins at the frame
    /// [`audition_from`](GridEdit::audition_from) gives, which is where
    /// **Play track** begins as well.
    ///
    /// The editor counts as open whether or not its track is selected,
    /// because an audition goes on playing while the controls of another
    /// track are showing. The space bar then stops that audition without the
    /// person selecting its track again to reach **Stop track**.
    fn toggle_the_audition(&mut self) {
        let Some(mut open) = self.editor.take() else {
            return;
        };
        if !end_the_audition(&mut open) {
            self.start_audition(&mut open);
        }
        self.editor = Some(open);
    }

    /// Moves the grid editor's grid with the left and right arrow keys, by
    /// one frame on its own, ten frames with shift held, and a hundred with
    /// shift and the command or the alt key held.
    ///
    /// Each press is a gesture of its own and puts the grid on the track at
    /// once. A held key repeats, and egui counts every repeat as a press, so
    /// the window moves the grid by the step the modifier keys name on every
    /// repeat, and the whole run of repeats stays one undoable step.
    ///
    /// The window reads no arrow key while a press is held on the strip.
    /// The editor works every move of the pointer out from where beat zero
    /// stood at the press, so a nudge made during a press would be lost at
    /// the next move of the pointer.
    fn arrow_keys(&mut self, ctx: &egui::Context) {
        let Some(mut open) = self.editor.take() else {
            return;
        };
        if open.editor.dragging() {
            self.editor = Some(open);
            return;
        }
        let (left, right, step) = ctx.input(|input| {
            let modifiers = input.modifiers;
            (
                input.key_pressed(Key::ArrowLeft),
                input.key_pressed(Key::ArrowRight),
                arrow_step(modifiers.shift, modifiers.command || modifiers.alt),
            )
        });
        if left {
            open.editor.nudge(Samples(-step.0));
        }
        if right {
            open.editor.nudge(step);
        }
        if left || right {
            self.put_the_grid_on_the_track(&mut open);
        }
        self.editor = Some(open);
    }

    /// Draws the menu bar across the top of the window.
    ///
    /// Every item does what the control of the same name on the rows below
    /// does, or what its keys do, so nothing is reachable only through the
    /// menu bar except **New**, **Open...**, **Save as...**, and **Quit**.
    /// **Scan music folder** stands greyed while a scan is running and while
    /// the window has no library file, and **Stop scan** stands greyed when
    /// no scan is running.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if menu_item(ui, "New", &NEW, true) {
                    self.ask_or_do(Intent::New, &ctx);
                }
                if menu_item(ui, "Open...", &OPEN, true)
                    && let Some(path) = ask_for_a_mix_to_open()
                {
                    self.ask_or_do(Intent::Open(path), &ctx);
                }
                if menu_item(ui, "Save", &SAVE, true) {
                    self.save();
                }
                if menu_item(ui, "Save as...", &SAVE_AS, true) {
                    self.save_as();
                }
                ui.separator();
                if menu_item(ui, "Quit", &QUIT, true) {
                    self.ask_or_do(Intent::Quit, &ctx);
                }
            });
            ui.menu_button("Edit", |ui| {
                if menu_item(ui, "Undo", &UNDO, self.timeline.can_undo()) {
                    self.timeline.undo();
                }
                if menu_item(ui, "Redo", &REDO, self.timeline.can_redo()) {
                    self.timeline.redo();
                }
                ui.separator();
                if menu_item(ui, "Settings...", &SETTINGS, true) {
                    self.open_the_settings_dialog();
                }
            });
            ui.menu_button("View", |ui| {
                if menu_item(ui, "Zoom in", &ZOOM_IN, true) {
                    self.zoom_in();
                }
                if menu_item(ui, "Zoom out", &ZOOM_OUT, true) {
                    self.zoom_out();
                }
                if menu_item(ui, "Whole mix", &WHOLE_MIX, true) {
                    self.show_whole_mix();
                }
                ui.separator();
                let hidden = self.library_collapsed;
                let name = if hidden {
                    "Show library"
                } else {
                    "Hide library"
                };
                if menu_item(ui, name, &LIBRARY_PANEL, true) {
                    self.library_collapsed_changed(!hidden);
                }
            });
            ui.menu_button("Library", |ui| {
                let running = self.a_scan_is_running();
                let ready = self.library_file.is_some() && !running;
                if menu_choice(ui, "Scan music folder", ready) {
                    self.scan_the_music_folder(false);
                }
                if menu_choice(ui, "Stop scan", running) {
                    self.stop_the_scan();
                }
            });
        });
    }

    /// Narrows the view about its middle, which is what **Zoom in** does.
    fn zoom_in(&mut self) {
        self.timeline
            .zoom_by(ZOOM_STEP, self.timeline.view().width_px / 2.0);
    }

    /// Widens the view about its middle, which is what **Zoom out** does.
    fn zoom_out(&mut self) {
        self.timeline
            .zoom_by(1.0 / ZOOM_STEP, self.timeline.view().width_px / 2.0);
    }

    /// Opens the settings dialog.
    ///
    /// The audio buffer field shows the setting as the file has it every time
    /// the dialog opens, so text a person typed and left refused is not still
    /// standing there the next time.
    fn open_the_settings_dialog(&mut self) {
        self.buffer_text = buffer_field_text(self.settings.audio_buffer_frames);
        self.settings_open = true;
    }

    /// Draws the row of transport buttons, the master tempo field, the curve
    /// selector, and the undo, redo, and save buttons.
    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Play").clicked() {
                self.play();
            }
            if ui.button("Pause").clicked() {
                self.pause();
            }
            if ui.button("Stop").clicked() {
                self.stop();
            }
            ui.separator();

            ui.label("Master BPM");
            // The tempo is written onto the curve once, when the person is
            // done typing, so that a turn of the control is one node and one
            // undoable step. A field that was clicked into and left alone
            // writes nothing, so passing through it never adds a node.
            let master = self.timeline.master_tempo();
            match tempo_field(ui, &mut self.master_tempo, master, (), "master tempo", 56.0) {
                Finish::Written((), bpm) => self.timeline.set_master_tempo(bpm),
                Finish::Refused(typed) => self.message = not_a_tempo(typed.trim()),
                Finish::Unchanged => {}
            }
            // A tempo node holds a tempo a person may want to type exactly
            // rather than drag, which is what `DESIGN.md` describes under
            // "Editing the tempo curve". The field appears only while a
            // tempo node is selected, and it holds that node's own tempo.
            if let Some((node, bpm)) = self.selected_tempo() {
                ui.separator();
                ui.label("Tempo");
                let finish = tempo_field(ui, &mut self.node_tempo, bpm, node, "node tempo", 56.0);
                self.take_node_tempo(finish);
            } else if self.node_tempo.editing() {
                // The field is gone because a track, or a node of a curve,
                // was selected instead, so the field never lost the
                // keyboard. A tempo typed into that field before it went
                // still belongs to the node it was typed for.
                let finish = self.node_tempo.finish();
                self.take_node_tempo(finish);
            }
            ui.separator();

            ui.label("Curve");
            let mut curve = self.timeline.curve();
            for choice in CURVES {
                ui.selectable_value(&mut curve, choice, paint::curve_name(choice));
            }
            if curve != self.timeline.curve() {
                self.timeline.select_curve(curve);
            }
            ui.separator();

            if ui
                .add_enabled(self.timeline.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.timeline.undo();
            }
            if ui
                .add_enabled(self.timeline.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.timeline.redo();
            }
            if ui.button("Save").clicked() {
                let _ = self.save();
            }
            ui.separator();

            if ui.button("Zoom in").clicked() {
                self.zoom_in();
            }
            if ui.button("Zoom out").clicked() {
                self.zoom_out();
            }
            if ui.button("Whole mix").clicked() {
                self.show_whole_mix();
            }
            ui.separator();

            if ui.button("Settings").clicked() {
                self.open_the_settings_dialog();
            }
            ui.separator();

            // One control hides the library panel and shows it again, and it
            // stands in the same place whether the panel is hidden or shown,
            // so nobody has to look for it in two places.
            if library_toggle(ui, !self.library_collapsed) {
                self.library_collapsed_changed(!self.library_collapsed);
            }
        });
    }

    /// Moves the track at `from`, whose content hash is `hash`, to `to` in
    /// the playlist, and keeps it selected there so it can be moved several
    /// places without being picked up again each time.
    ///
    /// Reordering the playlist reorders the mix, which `DESIGN.md` names as
    /// the timeline's defining trait under "Feature kernel". A move the
    /// history refuses leaves the selection where the timeline put it.
    fn move_track(&mut self, from: usize, to: usize, hash: ContentHash) {
        self.timeline.move_track(from, to);
        if self.timeline.mix().tracks.get(to).map(|track| track.hash) == Some(hash) {
            self.timeline.select_track(to);
            self.reselected = true;
        }
    }

    /// Draws the row of controls for the selected track: where it sits in the
    /// playlist, its keylock, and the grid editor when one is open.
    fn track_controls(&mut self, ui: &mut egui::Ui) {
        let Some(track) = self.selected_track() else {
            ui.horizontal(|ui| {
                ui.label("Select a track to reach its place in the playlist, its keylock, and its beat grid.");
            });
            return;
        };
        let tracks = self.timeline.mix().tracks.len();
        let Some(held) = self.timeline.mix().tracks.get(track) else {
            return;
        };
        let name = held
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut keylock = held.keylock;
        let grid = held.grid;
        let hash = held.hash;
        let length = held.length;
        let file = held.path.clone();

        ui.horizontal(|ui| {
            ui.label(format!("Track {}: {name}", track + 1));
            // Reordering the playlist reorders the mix, which is what makes
            // the timeline the playlist. The selection follows the track to
            // its new position, so a track can be moved several places
            // without being picked up again each time.
            if ui
                .add_enabled(track > 0, egui::Button::new("Move up"))
                .clicked()
            {
                self.move_track(track, track - 1, hash);
            }
            if ui
                .add_enabled(track + 1 < tracks, egui::Button::new("Move down"))
                .clicked()
            {
                self.move_track(track, track + 1, hash);
            }
            if ui.checkbox(&mut keylock, "Keylock").changed() {
                self.timeline.set_keylock(track, keylock);
            }
            let editing = self
                .editor
                .as_ref()
                .is_some_and(|open| open.editor.track() == track);
            if !editing && ui.button("Edit grid").clicked() {
                self.open_the_grid_editor(track, grid, hash, length, file.clone(), ui.ctx());
            }
        });

        if self
            .editor
            .as_ref()
            .is_some_and(|open| open.editor.track() == track)
        {
            self.grid_editor(ui);
        }
    }

    /// Opens the beat grid editor on a track, and starts reading the track's
    /// file when the window is not already holding its audio.
    fn open_the_grid_editor(
        &mut self,
        track: usize,
        grid: BeatGrid,
        hash: ContentHash,
        length: Samples,
        file: PathBuf,
        ctx: &egui::Context,
    ) {
        self.stop_the_audition();
        // The editor that was open, if one was, closes here without its
        // **Close** button. Ending the correction it made keeps the
        // correction that starts now out of the same undoable step.
        self.timeline.end_grid_correction();
        let mut editor = GridEditor::new(track, grid);
        // The length comes from the mix document, so the strip is framed to
        // the whole track from the first repaint, before its audio arrives.
        editor.set_length(length);
        let audio = audio::kept(&self.cache, hash);
        let decoding = audio.is_none().then(|| {
            let wake = {
                let ctx = ctx.clone();
                move || ctx.request_repaint()
            };
            Decoding::start(hash, file, Arc::clone(&self.cache), wake)
        });
        self.grid_tempo = TempoField::new(grid.bpm, GRID_DECIMALS);
        self.nudge_ms = 0.0;
        self.nudged_ms = 0.0;
        self.editor = Some(GridEdit {
            editor,
            hash,
            length,
            audio,
            decoding,
            audition: None,
            given: grid,
            on_the_track: grid,
            playhead: None,
            following: false,
            drawn: None,
        });
    }

    /// Takes in the track's audio when the thread reading it has finished,
    /// and reads where the audition has got to, on every repaint while the
    /// grid editor is open.
    ///
    /// The strip follows the audition's playhead here, so it goes on
    /// following even while another track's controls are showing, and an
    /// audition that has played the track out or whose device stopped taking
    /// frames gives the device back here rather than waiting to be stopped
    /// by hand. The audition takes a changed grid here for the same reason:
    /// the metronome follows the grid whichever track's controls are
    /// showing.
    fn watch_the_editor(&mut self, ctx: &egui::Context) {
        let mut trouble = None;
        let mut playing = false;
        let on_the_track = self
            .editor
            .as_ref()
            .and_then(|open| self.timeline.mix().tracks.get(open.editor.track()))
            .map(|track| track.grid);
        if let Some(open) = &mut self.editor {
            // An undo or a redo while the editor is open puts a grid on the
            // track that the editor did not put there. The editor takes that
            // grid as its own, so the strip and the audition follow the
            // document. A press held on the strip is the person's own hold
            // on the grid, and nothing is taken while it lasts.
            if let Some(grid) = on_the_track
                && grid != open.on_the_track
                && !open.editor.dragging()
            {
                open.editor.set_grid(grid);
                open.on_the_track = grid;
            }
            // However the grid was changed (dragged across the strip,
            // nudged, halved, doubled, tapped, typed, moved by an arrow key,
            // or taken from the document by the undo above), the audition
            // uses it from the next frames it makes, so the click follows
            // the grid while the track plays. The handover happens here,
            // rather than where the controls are drawn, because the editor's
            // controls are drawn only while its own track is selected and an
            // audition goes on playing while another track's controls are
            // showing.
            if let Some(audition) = &mut open.audition {
                let grid = open.editor.grid();
                if grid != open.given && audition.set_grid(grid).is_some() {
                    open.given = grid;
                }
            }
            if let Some(decoding) = &open.decoding
                && let Some(read) = decoding.finished()
            {
                open.decoding = None;
                match read {
                    Ok(audio) => {
                        // The document's length framed the strip when the
                        // editor opened. The decoded audio is what the strip
                        // actually draws, so the audio's own length bounds
                        // the view from here on.
                        open.editor.set_length(Samples(audio.frames.len() as i64));
                        open.audio = Some(audio);
                    }
                    Err(problem) => trouble = Some(problem),
                }
            }
            if let Some(audition) = &open.audition {
                let status = audition.status();
                open.playhead = Some(status.position);
                if open.following {
                    open.editor.follow(status.position);
                }
                match status.state {
                    AuditionState::Playing => playing = true,
                    AuditionState::Ended => {
                        end_the_audition(open);
                    }
                    AuditionState::Failed(problem) => {
                        trouble = Some(format!("The audition stopped: {problem}"));
                        end_the_audition(open);
                    }
                }
            }
        }
        if let Some(problem) = trouble {
            self.message = problem;
        }
        if playing {
            ctx.request_repaint_after(PLAYING_REPAINT);
        }
    }

    /// Ends the audition, if one is running, and says whether it did.
    fn stop_the_audition(&mut self) -> bool {
        match &mut self.editor {
            Some(open) => end_the_audition(open),
            None => false,
        }
    }

    /// Starts the audition of the grid editor's track: the track on its own,
    /// at its own speed, with the metronome clicking on every beat of the
    /// grid as it stands.
    ///
    /// The machine has one audio output, so the mix stops here and the
    /// status line says so.
    fn start_audition(&mut self, open: &mut GridEdit) {
        let Some(audio) = open.audio.clone() else {
            self.message =
                "The track is still being read, so there is nothing to audition yet".to_owned();
            return;
        };
        let orders = self.playback.stop(mix_length(self.timeline.mix()));
        let stopped_the_mix = !orders.is_empty();
        self.carry_out(orders);
        let output = match audio::output(self.settings.audio_buffer_frames) {
            Ok(output) => output,
            Err(problem) => {
                self.message = problem;
                return;
            }
        };
        let from = open.audition_from();
        let grid = open.editor.grid();
        match Audition::start(audio, grid, from, self.metronome, output) {
            Ok(audition) => {
                open.audition = Some(audition);
                open.given = grid;
                open.playhead = Some(from);
                open.following = true;
                self.message = if stopped_the_mix {
                    "The mix stopped so that the audition could use the audio device".to_owned()
                } else {
                    String::new()
                };
            }
            Err(problem) => self.message = format!("The audition could not start: {problem}"),
        }
    }

    /// Puts the editor's grid on its track, as one step of the correction,
    /// when the grid differs from the one the editor last put there.
    ///
    /// The window calls this at the end of every gesture that changes the
    /// grid, so the timeline, the correction file, and the mix all show the
    /// grid as it stands. [`Timeline::set_grid`] makes one undoable step of
    /// the whole correction, so a hundred nudges undo in one go.
    ///
    /// The only grid the document refuses that the window can reach is one
    /// for a track that has left the playlist, which
    /// [`close_a_stale_editor`](Window::close_a_stale_editor) catches on the
    /// repaint after the track moves. A refusal leaves the grid in the
    /// editor and the reason in the status line.
    fn put_the_grid_on_the_track(&mut self, open: &mut GridEdit) {
        let grid = open.editor.grid();
        if grid == open.on_the_track {
            return;
        }
        match self.timeline.set_grid(open.editor.track(), grid) {
            Ok(()) => open.on_the_track = grid,
            Err(problem) => self.message = format!("The grid could not be changed: {problem}"),
        }
    }

    /// Draws the grid editor: the strip with the track's waveform and the
    /// grid over it, the drag that slides the grid, the halve and double
    /// buttons, the exact tempo field, the tap button, and the audition.
    ///
    /// Every gesture that changes the grid puts it on the track as the
    /// gesture ends, through
    /// [`put_the_grid_on_the_track`](Window::put_the_grid_on_the_track). A
    /// drag across the strip ends when the person lets go, and the audition
    /// follows the grid throughout the drag.
    fn grid_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut open) = self.editor.take() else {
            return;
        };
        // Whether a gesture that changed the grid has finished this repaint,
        // which is what puts the grid on the track below.
        let mut settled = false;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.label("Beat grid");
            ui.label("Nudge");
            let drag = ui.add(
                egui::DragValue::new(&mut self.nudge_ms)
                    .speed(0.5)
                    .suffix(" ms"),
            );
            if drag.changed() {
                let by = Seconds((self.nudge_ms - self.nudged_ms) / 1000.0).to_samples();
                open.editor.nudge(by);
                self.nudged_ms = self.nudge_ms;
                settled = true;
            }
            if ui.button("Halve tempo").clicked() {
                open.editor.halve();
                settled = true;
            }
            if ui.button("Double tempo").clicked() {
                open.editor.double();
                settled = true;
            }

            ui.label("Tempo");
            // A tempo typed here is taken when the person leaves the field.
            // One that will not read as a positive tempo is refused, and the
            // field goes back to the grid's own tempo.
            let bpm = open.editor.grid().bpm;
            match tempo_field(ui, &mut self.grid_tempo, bpm, (), "grid tempo", 64.0) {
                Finish::Written((), typed) => {
                    settled |= open.editor.set_bpm(typed);
                }
                Finish::Refused(typed) => self.message = not_a_tempo(typed.trim()),
                Finish::Unchanged => {}
            }

            if ui.button("Tap").clicked() {
                // A tap that gives a tempo has changed the grid. The taps
                // before it leave the grid where it was.
                let tapped = open.editor.tap(Seconds(self.clock.elapsed().as_secs_f64()));
                settled |= tapped.is_some();
            }
            let taps = open.editor.taps();
            if taps < TAPS_FOR_A_TEMPO {
                ui.label(format!("{taps} of {TAPS_FOR_A_TEMPO} taps"));
            } else {
                ui.label(format!("{taps} taps"));
            }

            ui.separator();
            let running = open.audition.is_some();
            if ui
                .add_enabled(!running, egui::Button::new("Play track"))
                .clicked()
            {
                self.start_audition(&mut open);
            }
            if ui
                .add_enabled(running, egui::Button::new("Stop track"))
                .clicked()
            {
                end_the_audition(&mut open);
            }
            // The metronome is turned on and off whether or not the track
            // is playing and whether or not the grid is being dragged, which
            // is what `DESIGN.md` asks for under "Manual beatgrid
            // correction".
            let mut metronome = self.metronome;
            if ui.checkbox(&mut metronome, "Metronome").changed() {
                self.metronome_changed(metronome);
                if let Some(audition) = &mut open.audition {
                    audition.set_click(metronome);
                }
            }

            ui.separator();
            // The strip is collapsed and expanded from here, and the state
            // is a setting, so an editor opened later starts as the person
            // left this one.
            let strip = if self.strip_collapsed {
                "Show strip"
            } else {
                "Hide strip"
            };
            if ui.button(strip).clicked() {
                self.strip_collapsed_changed(!self.strip_collapsed);
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });

        settled |= self.grid_strip(ui, &mut open);

        // A tempo typed into the field and followed at once by **Close**
        // never lost the keyboard, because the button takes it only after
        // the field has been drawn. The closing path finishes the typing
        // itself, before the corrected grid goes on the track, so that a
        // tempo typed and the editor closed in one movement is the tempo the
        // track keeps.
        if close && self.grid_tempo.editing() {
            match self.grid_tempo.finish() {
                Finish::Written((), typed) => {
                    settled |= open.editor.set_bpm(typed);
                }
                Finish::Refused(typed) => self.message = not_a_tempo(typed.trim()),
                Finish::Unchanged => {}
            }
        }

        if settled {
            self.put_the_grid_on_the_track(&mut open);
        }

        if close {
            // Closing the editor gives the audio device back and ends the
            // correction, so a correction made after the editor is opened
            // again is an undoable step of its own.
            end_the_audition(&mut open);
            self.timeline.end_grid_correction();
        } else {
            self.editor = Some(open);
        }
    }

    /// Draws the strip below the grid editor's controls: the track's own
    /// waveform with the grid's beats over it, and the audition's playhead
    /// among them.
    ///
    /// The strip takes the same gestures the timeline does. The wheel moves
    /// the view through the track, the command key with the wheel or two
    /// fingers pinching narrows and widens it, down to one frame of audio
    /// per pixel, and a press and drag slides the grid by the frames the
    /// pointer crossed. A press let go before it travelled
    /// [`DRAG_PX`](dermixen_app::DRAG_PX) is a click, which moves the
    /// strip's playhead to the frame under the pointer and moves a running
    /// audition there with [`Audition::seek`], so the track plays on from
    /// the clicked frame without the audio device stopping.
    ///
    /// A collapsed strip is not drawn at all and takes no gesture, so the
    /// editor is its row of controls alone. Every control, the arrow keys,
    /// the audition, and the metronome go on working. A strip hidden after
    /// it was shown comes back at the height and the view it had, and one
    /// that was collapsed when the editor opened shows the whole track the
    /// first time it is shown, since the editor is told a width here and
    /// nowhere else.
    ///
    /// Says whether a drag of the grid ended here, which is what puts the
    /// dragged grid on the track.
    fn grid_strip(&mut self, ui: &mut egui::Ui, open: &mut GridEdit) -> bool {
        if self.strip_collapsed {
            return false;
        }
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, self.strip_px), Sense::click_and_drag());
        open.editor.set_width(rect.width());
        // Whether a drag of the grid was let go here.
        let mut settled = false;

        if response.contains_pointer() && !self.a_dialog_is_open() {
            let (scroll, zoom) = ui.input(|input| (input.smooth_scroll_delta, input.zoom_delta()));
            let at = ui
                .input(|input| input.pointer.interact_pos())
                .map(|at| at.x - rect.left())
                .unwrap_or(rect.width() / 2.0);
            // Moving the view is a person choosing what to look at, so the
            // strip stops taking itself back to the audition's playhead
            // until the next time the track is played.
            if zoom != 1.0 {
                open.editor.zoom_by(f64::from(zoom), at);
                open.following = false;
            } else {
                let by = scroll.x + scroll.y;
                if by != 0.0 {
                    open.editor.scroll_by(-by);
                    open.following = false;
                }
            }
        }

        if !self.a_dialog_is_open() {
            let (pressed, down, released, at) = ui.input(|input| {
                (
                    input.pointer.primary_pressed(),
                    input.pointer.primary_down(),
                    input.pointer.primary_released(),
                    input.pointer.interact_pos(),
                )
            });
            if pressed
                && response.contains_pointer()
                && let Some(at) = at
            {
                open.editor.press(at.x - rect.left());
            } else if down
                && open.editor.dragging()
                && let Some(at) = at
            {
                open.editor.drag_to(at.x - rect.left());
            }
            if released {
                let held = open.editor.dragging();
                match open.editor.release() {
                    // A click names the frame under the pointer. The strip's
                    // playhead goes there, which is where **Play track**
                    // starts from, and a track already playing moves to that
                    // frame, keeping the grid and the metronome setting it
                    // has, so the person hears the part of the track they
                    // pointed at without the audio device stopping. An
                    // audition that has played the track out is dropped by
                    // `watch_the_editor`, so a click after the end finds
                    // none running and **Play track** starts one from the
                    // clicked frame.
                    Some(frame) => {
                        open.playhead = Some(frame);
                        if let Some(audition) = &mut open.audition {
                            audition.seek(frame);
                        }
                    }
                    None => settled = held,
                }
            }
        }

        paint::grid_strip(&ui.painter_at(rect), rect, open.scene());
        self.strip_edge(ui, rect);
        settled
    }

    /// Takes the handle under the strip, which is dragged to make the strip
    /// taller or shorter, from [`MIN_LANE_PX`] to [`MAX_LANE_PX`], the
    /// range a lane of the timeline is dragged over.
    ///
    /// The height is the pointer's row less the top of the strip, as a
    /// lane's height is the pointer's row less the top of the lane. The
    /// handle is a band of its own below the strip, so a press on it never
    /// reaches the strip and never slides the grid, and a press on the strip
    /// slides the grid as it always has. The pointer shows the vertical
    /// resize arrow over the handle and for as long as the handle is
    /// dragged.
    fn strip_edge(&mut self, ui: &mut egui::Ui, strip: egui::Rect) {
        let (rect, handle) =
            ui.allocate_exact_size(Vec2::new(strip.width(), STRIP_EDGE_PX), Sense::drag());
        let held = handle.dragged();
        if held || handle.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if held && let Some(at) = handle.interact_pointer_pos() {
            self.strip_px = (at.y - strip.top()).clamp(MIN_LANE_PX, MAX_LANE_PX);
        }
        let color = if held || handle.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            ui.visuals().widgets.inactive.bg_fill
        };
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(2.0, color),
        );
    }

    /// Takes the strip above the lanes for the ruler.
    ///
    /// Neither the click nor the painting happens here. Both wait until the
    /// lanes have run, because the lanes are what tell the timeline how wide
    /// the view is, and both the ruler's ticks and the mix time a click on it
    /// means are worked out against that width. Reading the click here would
    /// use the width of the frame before it, and painting here would draw the
    /// view as it stood a frame ago.
    fn ruler_strip(ui: &mut egui::Ui) -> (egui::Rect, egui::Response) {
        let width = ui.available_width();
        ui.allocate_exact_size(Vec2::new(width, RULER_HEIGHT), Sense::click())
    }

    /// Hands a click on the ruler to the timeline, which leaves a request to
    /// move the preview there for the window to take.
    ///
    /// This runs after the lanes, so the column is turned into a mix time
    /// against the width the lanes gave the timeline this frame. The lanes
    /// and the ruler start at the same edge, so a column of one is a column
    /// of the other; where a scroll bar has made the lanes narrower than the
    /// strip, the few pixels of strip past the lanes' right edge run past
    /// the end of the view.
    fn read_the_ruler(&mut self, rect: egui::Rect, response: &egui::Response) {
        if response.clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            self.timeline.click_ruler(at.x - rect.left());
        }
    }

    /// Draws the lanes and the tempo lane, and hands every press, drag, and
    /// release to the timeline.
    fn lanes(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        // The tempo lane is stacked under the track lanes, so the mix's
        // tracks and it share the panel.
        let lanes = self.timeline.mix().tracks.len() + 1;
        // A lane nobody has resized is fitted to the panel. A lane resized
        // by a drag of its bottom edge keeps the height the drag gave it.
        let fitted = (ui.available_height() / lanes as f32).clamp(MIN_LANE_PX, FITTED_LANE_PX);
        let view = self.timeline.view();
        // The height every lane comes to is what the rectangle is allocated
        // at, so lanes taller than the panel overflow it and the scroll bar
        // this panel already has appears. The timeline works that height out
        // from the fitted height, so the view is set before the rectangle is
        // allocated, and again afterwards with the width the allocation gave.
        self.timeline.set_view(View {
            width_px: width,
            lane_height_px: fitted,
            from: view.from,
            to: view.to,
        });
        let height = self.timeline.lanes_height();
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        self.timeline.set_view(View {
            width_px: rect.width(),
            lane_height_px: fitted,
            from: view.from,
            to: view.to,
        });
        if !self.fitted && rect.width() > 0.0 {
            self.show_whole_mix();
            self.fitted = true;
        }

        self.wheel(ui, &response, rect);
        self.pointer(ui, &response, rect);

        let scene = self.timeline.scene();
        // The window knows a track with no file by its content hash, and the
        // painter draws lanes in playlist order, so the hashes are turned
        // into positions of the playlist as it stands.
        let no_file: HashMap<usize, NoFile> = self
            .timeline
            .mix()
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(at, track)| Some((at, *self.no_file.get(&track.hash)?)))
            .collect();
        paint::scene(
            &ui.painter_at(rect),
            rect,
            &scene,
            self.timeline.curve(),
            self.timeline.selection(),
            &no_file,
        );
    }

    /// Reads the wheel and a pinch over the timeline. The wheel scrolls the
    /// lanes up and down. A sideways movement scrolls the view through the
    /// mix in time. The command key or the alt key with the wheel narrows or
    /// widens the view around the pointer. The same key with shift scrolls
    /// the view in time. Two fingers pinching narrow or widen the view.
    /// [`wheel_gesture`] applies the rules for the wheel, and a pinch goes
    /// to [`Timeline::zoom_by`] on its own. `ui` belongs to the scroll area
    /// around the lanes, which is the scroll area a vertical movement moves.
    fn wheel(&mut self, ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
        if !response.contains_pointer() || self.a_dialog_is_open() {
            return;
        }
        let at = ui
            .input(|input| input.pointer.interact_pos())
            .map(|at| at.x - rect.left())
            .unwrap_or(rect.width() / 2.0);
        // egui reworks the wheel before it offers the result as a scroll
        // delta and a zoom factor: the command key turns the movement into a
        // zoom factor and no scroll at all, shift turns the movement
        // sideways, and the alt key folds both axes into the vertical one.
        // Neither of those two values tells the command key with shift from
        // the command key alone, and neither zooms on the alt key, so the
        // window reads the wheel's own events instead and turns each
        // movement into points the way egui does. Nothing here reads that
        // scroll delta or that zoom factor, or the command key with the
        // wheel would zoom twice.
        let line_px = ui
            .ctx()
            .options(|options| options.input_options.line_scroll_speed);
        let gestures: Vec<WheelGesture> = ui.input(|input| {
            let page_px = input.viewport_rect().height();
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    // A wheel event whose phase is the start or the end of
                    // a gesture has no movement of its own, and egui leaves
                    // such an event's delta out as well.
                    egui::Event::MouseWheel {
                        unit,
                        delta,
                        phase: egui::TouchPhase::Move,
                        modifiers,
                    } => {
                        let points = match unit {
                            egui::MouseWheelUnit::Point => *delta,
                            egui::MouseWheelUnit::Line => line_px * *delta,
                            egui::MouseWheelUnit::Page => page_px * *delta,
                        };
                        Some(wheel_gesture(
                            points.x,
                            points.y,
                            modifiers.command || modifiers.alt,
                            modifiers.shift,
                        ))
                    }
                    // Two fingers pinching arrive as their own event, which
                    // gives the factor to zoom by.
                    egui::Event::Zoom(factor) => Some(WheelGesture::Zoom {
                        factor: f64::from(*factor),
                    }),
                    _ => None,
                })
                .collect()
        });
        for gesture in gestures {
            match gesture {
                WheelGesture::Scroll { lanes_px, time_px } => {
                    if lanes_px != 0.0 {
                        // The scroll area around the lanes takes the wheel
                        // from nobody else, since its scroll source is its
                        // scroll bar alone, so the movement it scrolls by is
                        // the one handed to it here. It scrolls without an
                        // animation, so the lanes follow the wheel in the
                        // frame the wheel moved in.
                        ui.scroll_with_delta_animation(
                            Vec2::new(0.0, lanes_px),
                            egui::style::ScrollAnimation::none(),
                        );
                    }
                    self.timeline.scroll_by(time_px);
                }
                WheelGesture::Zoom { factor } => self.timeline.zoom_by(factor, at),
                WheelGesture::Time { time_px } => self.timeline.scroll_by(time_px),
            }
        }
    }

    /// Reads the pointer over the timeline: a press takes hold of whatever
    /// lies under it, a drag moves it, and a release commits the move.
    fn pointer(&mut self, ui: &egui::Ui, response: &egui::Response, rect: egui::Rect) {
        if self.a_dialog_is_open() {
            return;
        }
        let (pressed, down, released, at) = ui.input(|input| {
            (
                input.pointer.primary_pressed(),
                input.pointer.primary_down(),
                input.pointer.primary_released(),
                input.pointer.interact_pos(),
            )
        });

        if self.pressed_at.is_none()
            && pressed
            && response.contains_pointer()
            && let Some(at) = at
        {
            let point = paint::point_in(rect, at);
            // What the press takes hold of is worked out before it is made,
            // since a press that adds a node changes what lies under the
            // pointer.
            self.resizing = takes_hold_of_an_edge(&self.timeline, point);
            self.timeline.press(point);
            // A press on the timeline is where its selection is set, and the
            // library takes its key from whatever the press selected.
            self.reselected = true;
            self.pressed_at = Some(at);
            self.dragging = false;
        } else if down
            && let Some(from) = self.pressed_at
            && let Some(at) = at
            // A press that has not moved is a click, and a click must not
            // nudge the node it selected.
            && (self.dragging || (at - from).length() > DRAG_PX)
        {
            self.dragging = true;
            self.timeline.drag_to(paint::point_in(rect, at));
        }
        if self.pressed_at.is_some() && released {
            self.timeline.release();
            self.pressed_at = None;
            self.dragging = false;
            self.resizing = false;
        }

        // A double-click on a lane's bottom edge makes that lane tall or
        // short. Its two presses take hold of the edge and let it go again
        // without moving it, and the release above has let go of the edge by
        // the time the timeline is handed the double-click.
        if response.double_clicked()
            && let Some(at) = response.interact_pointer_pos()
        {
            self.timeline.double_click(paint::point_in(rect, at));
        }

        if self.resizing {
            // A lane being resized keeps the resize cursor wherever the
            // pointer has been dragged to.
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        } else if self.pressed_at.is_none()
            && let Some(at) = at
            && response.contains_pointer()
        {
            let point = paint::point_in(rect, at);
            let cursor = match self.timeline.hit(point) {
                Selection::Anchor { .. } => egui::CursorIcon::ResizeHorizontal,
                Selection::Node { .. } | Selection::TempoNode { .. } => egui::CursorIcon::Grab,
                // The timeline names nothing where a press would take
                // hold of a lane's bottom edge.
                Selection::Nothing if takes_hold_of_an_edge(&self.timeline, point) => {
                    egui::CursorIcon::ResizeVertical
                }
                Selection::Track(_) | Selection::Nothing => egui::CursorIcon::Default,
            };
            ui.ctx().set_cursor_icon(cursor);
        }
    }

    /// Draws the line at the bottom of the window: what the preview is doing,
    /// where it is, and the last message the window recorded.
    fn status_line(&mut self, ui: &mut egui::Ui) {
        let length = mix_length(self.timeline.mix()).to_seconds();
        let at = self.timeline.playhead().to_seconds();
        let (state, underruns) = match &self.status {
            Some(status) => (
                match &status.state {
                    TransportState::Buffering => "Buffering".to_owned(),
                    TransportState::Playing => "Playing".to_owned(),
                    TransportState::Paused => "Paused".to_owned(),
                    TransportState::Ended => "Ended".to_owned(),
                    TransportState::Failed(problem) => format!("Stopped: {problem}"),
                },
                status.underruns,
            ),
            None => ("Stopped".to_owned(), 0),
        };
        ui.horizontal(|ui| {
            ui.label(format!(
                "{state}  ·  {} of {}",
                paint::time_text(at),
                paint::time_text(length)
            ));
            if underruns > 0 {
                ui.label(format!("{underruns} underruns"));
            }
            if self.document.is_unsaved() {
                ui.label("Unsaved changes");
            }
            // A failing autosave stands here for as long as it is failing,
            // ahead of the passing messages, since it is the one thing in
            // this line that says work is no longer being kept.
            if let Some(problem) = &self.autosave_trouble {
                ui.colored_label(
                    Color32::from_rgb(240, 130, 110),
                    format!("Autosave failing: {problem}"),
                );
            }
            // Why the last edit cost the person the buffering they can see
            // in this same line, for as long as RESTART_SHOWN.
            if let Some((reason, since)) = &self.started_over {
                let left = RESTART_SHOWN.checked_sub(since.elapsed());
                if let Some(left) = left {
                    ui.colored_label(
                        Color32::from_rgb(240, 190, 120),
                        format!("Started over: {reason}"),
                    );
                    // Nothing else need repaint the window for this to go
                    // away by itself when the mix is not playing.
                    ui.ctx().request_repaint_after(left);
                }
            }
            if !self.message.is_empty() {
                ui.colored_label(Color32::from_rgb(240, 190, 120), &self.message);
            }
        });
    }

    /// Works the library's rows out again, which the window does whenever
    /// the search, the highlighting, the selection, or the mix has changed.
    fn refresh_rows(&mut self) {
        self.rows = self.library.rows();
        self.heights.forget();
    }

    /// Highlights the library against the track selected on the timeline,
    /// whenever the selection moves to another track.
    ///
    /// `DESIGN.md` names this under "Feature kernel": selecting a track
    /// highlights the tracks in the library that mix harmonically with it. A
    /// track the library does not contain, and one no key analyzer answered
    /// for, leaves the library with nothing to highlight against.
    fn follow_the_selection(&mut self) {
        let hash = self.selected_hash();
        let reselected = std::mem::take(&mut self.reselected);
        if !reselected && hash == self.reference_hash {
            return;
        }
        self.reference_hash = hash;
        let key = hash.and_then(|hash| self.keys.get(&hash).copied());
        self.library.set_reference(key);
        self.refresh_rows();
    }

    /// Adds the track selected in the library to the mix, after the track
    /// selected on the timeline, or at the end of the playlist when no track
    /// is selected there.
    ///
    /// The track is joined to its neighbors with the blend preset, which is
    /// what `dermixen mix add` writes when it is not told otherwise. An
    /// insert the document refuses changes nothing, and the status line
    /// gives the reason. When the insert
    /// succeeds and the window has not already handed the track's content
    /// hash to the reading thread, the thread is asked for it so the new
    /// lane fills in once it has.
    fn add_to_mix(&mut self) {
        let Some(row) = self.library.selected() else {
            self.message = "Choose a track in the library before adding one to the mix".to_owned();
            return;
        };
        let at = match self.selected_track() {
            Some(track) => track + 1,
            None => self.timeline.mix().tracks.len(),
        };
        let preset = Preset::Blend;
        let Some(edit) = self.library.insert_edit(row, at, preset) else {
            self.message = "That track is no longer in the library".to_owned();
            return;
        };
        match self.timeline.apply(edit) {
            Ok(()) => {
                self.timeline.select_track(at);
                self.reselected = true;
                self.message.clear();
                if let Some(track) = self.timeline.mix().tracks.get(at) {
                    let hash = track.hash;
                    let path = track.path.clone();
                    if self.handed.insert(hash) {
                        self.reading.want(Wanted { hash, path });
                    }
                }
            }
            Err(problem) => {
                self.message = format!("The track could not be added to the mix: {problem}");
            }
        }
    }

    /// Draws the library beside the timeline: the search field,
    /// **Add selected to mix**, the count of the rows shown, the filters,
    /// and the table.
    ///
    /// Nothing of this is drawn while the panel is hidden, so
    /// **Add selected to mix** is out of reach until the panel's toggle in
    /// the first row of controls brings the panel back.
    fn library_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Search");
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.searched)
                    .desired_width(160.0)
                    .id_salt("library search"),
            );
            if field.changed() {
                self.library.set_search(&self.searched);
                self.refresh_rows();
            }
            if ui.button("Add selected to mix").clicked() {
                self.add_to_mix();
            }
        });
        ui.weak(format!("{} tracks", self.rows.len()));
        self.library_filters(ui);
        ui.separator();

        let clicks = library_table(
            ui,
            &self.rows,
            self.library.sort(),
            &mut self.heights,
            self.library_word_wrap,
        );
        if let Some(column) = clicks.heading {
            self.library.click_heading(column);
            self.refresh_rows();
        }
        if let Some(row) = clicks.row {
            self.library.select(row);
            self.refresh_rows();
        }
    }

    /// Draws the filters above the library table, in a section that is
    /// closed when the window opens.
    ///
    /// Every change to a field narrows the rows at once. A year or tempo
    /// field whose text is not a number leaves that end of its range open,
    /// so a half-typed number narrows nothing rather than emptying the
    /// table.
    fn library_filters(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        egui::CollapsingHeader::new("Filters")
            .id_salt("library filters")
            .default_open(false)
            .show(ui, |ui| {
                let fields = &mut self.filters;
                egui::Grid::new("library filter fields")
                    .num_columns(4)
                    .show(ui, |ui| {
                        ui.label("Artist");
                        changed |= filter_field(ui, &mut fields.artist, "library artist filter");
                        ui.label("Title");
                        changed |= filter_field(ui, &mut fields.title, "library title filter");
                        ui.end_row();

                        ui.label("Label");
                        changed |= filter_field(ui, &mut fields.label, "library label filter");
                        ui.label("Path");
                        changed |= filter_field(ui, &mut fields.path, "library path filter");
                        ui.end_row();

                        ui.label("Year");
                        ui.horizontal(|ui| {
                            changed |= range_field(ui, &mut fields.year_from, "library year from");
                            ui.label("to");
                            changed |= range_field(ui, &mut fields.year_to, "library year to");
                        });
                        ui.label("BPM");
                        ui.horizontal(|ui| {
                            changed |= range_field(ui, &mut fields.bpm_from, "library bpm from");
                            ui.label("to");
                            changed |= range_field(ui, &mut fields.bpm_to, "library bpm to");
                        });
                        ui.end_row();
                    });
                ui.label("Key");
                for letter in [Letter::A, Letter::B] {
                    ui.horizontal(|ui| {
                        for number in 1..=12 {
                            let Some(code) = Camelot::new(number, letter) else {
                                continue;
                            };
                            let chosen = fields.keys.contains(&code);
                            if ui.selectable_label(chosen, code.to_string()).clicked() {
                                if chosen {
                                    fields.keys.retain(|key| *key != code);
                                } else {
                                    fields.keys.push(code);
                                }
                                changed = true;
                            }
                        }
                    });
                }
                if ui.button("Clear keys").clicked() {
                    changed |= !fields.keys.is_empty();
                    fields.keys.clear();
                }
            });
        if changed {
            self.library.set_filters(self.filters.filters());
            self.refresh_rows();
        }
    }

    /// Takes the requests the timeline has left since the last repaint and
    /// carries them out: a move of the preview, an edited document to hand
    /// it, and the message of a correction that could not be written.
    fn take_pending_requests(&mut self) {
        if let Some(at) = self.timeline.take_seek() {
            // A click on the ruler is a person choosing where to listen
            // from, so it is where stopping comes back to as well.
            let orders = self.playback.click_ruler(at);
            self.carry_out(orders);
        }
        if self.timeline.take_document_change() {
            self.document.edited();
            self.write_the_autosave();
            self.library.set_mix(self.timeline.mix());
            self.refresh_rows();
            let mix = self.timeline.mix().clone();
            if let Some(transport) = &mut self.transport {
                transport.replace(mix);
            }
        }
        if let Some(problem) = self.timeline.take_correction_failure() {
            self.message = problem;
        }
    }

    /// Puts the document's title on the window when the title has changed,
    /// which is at the first edit after a save, at every save, and whenever
    /// another document comes under the window.
    fn show_the_title(&mut self, ctx: &egui::Context) {
        let title = self.document.title();
        if title != self.shown_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.shown_title = title;
        }
    }
}

impl eframe::App for Window {
    /// Runs before every repaint, and in place of one while the window is
    /// minimized or hidden.
    ///
    /// A request to close the window arrives here on that hidden path, where
    /// no repaint happens at all, so the check that holds the window open has
    /// to run here rather than only while drawing.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.open_what_macos_sent(ctx);
        self.consider_closing(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.collect_findings();
        self.collect_the_scan();
        self.close_a_stale_editor();

        // The playhead follows the preview, so it moves while a person is
        // listening and holds still when nothing is playing. How far the
        // render has reached follows it too, because the master BPM control
        // writes its nodes from just past that frame, where nothing has been
        // rendered yet, so the render carries on rather than starting over.
        if let Some(transport) = &self.transport {
            let status = transport.status();
            self.playback.heard(&status);
            self.timeline.set_playhead(self.playback.playhead());
            self.timeline.set_reach(status.reached);
            if matches!(
                status.state,
                TransportState::Playing | TransportState::Buffering
            ) {
                ctx.request_repaint_after(PLAYING_REPAINT);
            }
            // An edit that made the render start over says why it did, and
            // that reason goes up beside the buffering it caused. It is
            // taken up when it is not the one already showing, which is what
            // keeps a move of the playhead, which leaves the reason where it
            // was, from putting the same words up again.
            let showing = self
                .status
                .as_ref()
                .and_then(|earlier| earlier.last_restart.clone());
            if status.last_restart != showing {
                self.started_over = status
                    .last_restart
                    .clone()
                    .map(|reason| (reason, Instant::now()));
            }
            self.status = Some(status);
        } else {
            // With no preview running, nothing is rendered ahead and the
            // playhead's own frame is as far as the render has reached.
            // Saying so on every repaint keeps a reach recorded while the mix
            // was playing from standing after a stop, which would put the
            // control's nodes far ahead of the playhead.
            let at = self.playback.playhead();
            self.timeline.set_playhead(at);
            self.timeline.set_reach(at);
        }

        self.watch_the_editor(&ctx);
        self.follow_the_selection();
        self.keyboard(&ctx);

        egui::Panel::top("controls").show(ui, |ui| {
            self.menu_bar(ui);
            self.controls(ui);
            self.track_controls(ui);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            self.status_line(ui);
        });
        if !self.library_collapsed {
            // egui keeps a panel's width under the panel's own name for as
            // long as the window is open, and reads `default_size` only
            // before the first time it draws that panel, so a panel hidden
            // and shown again comes back at the width it had and a window
            // that has not drawn the panel yet opens it at
            // `LIBRARY_WIDTH_PX`. Nothing records the width between one
            // window and the next.
            egui::Panel::right("library")
                .default_size(LIBRARY_WIDTH_PX)
                .show(ui, |ui| {
                    self.library_panel(ui);
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let (ruler, clicked) = Window::ruler_strip(ui);
            // A lane nobody has resized is fitted to the panel, so the
            // scroll bar appears for a mix with more tracks than the window
            // has room for, and for lanes a person has dragged taller than
            // the panel. This area takes no wheel of its own, because
            // `Window::wheel` reads the wheel's events itself and hands this
            // area the movement that belongs to it.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_source(egui::containers::scroll_area::ScrollSource::SCROLL_BAR)
                .show(ui, |ui| {
                    self.lanes(ui);
                });
            // The ruler and the lanes share one scale: the view the lanes
            // have just set, whose width is theirs.
            self.read_the_ruler(ruler, &clicked);
            paint::ruler(&ui.painter_at(ruler), ruler, self.timeline.view());
        });

        self.restore_dialog(&ctx);
        self.question_dialog(&ctx);
        self.settings_dialog(&ctx);
        self.take_pending_requests();
        self.show_the_title(&ctx);
    }
}

/// The app icon the window shows in its title bar, the dock, and the task
/// bar, decoded from the 256 pixel PNG that `scripts/make-icon.sh` renders
/// from `packaging/icon/dermixen.svg`. The PNG is compiled into the program,
/// so the window carries its icon wherever the executable goes. A PNG that
/// cannot be decoded gives the windowing system's own default icon rather
/// than stopping the window from opening.
fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/icon/dermixen-256.png"))
        .unwrap_or_default()
}

/// Reads the mix at `path` and opens a window on it, or opens a window on an
/// untitled empty mix when there is no path.
///
/// The autosave file of the mix is read before the window is built, so that a
/// session that ended without a save is offered back before anything else the
/// person does. The settings file is read there too, and a settings file that
/// cannot be read stops the window from opening, with a message naming that
/// file and what is wrong with it, as `docs/settings.md` requires. Nothing
/// the file gives is replaced by a default in its place.
///
/// The command's `main` calls this with the one argument it was given, or
/// with nothing, and turns what comes back into an exit code: the reason in
/// the error is one sentence for the `error:` line. This returns when the
/// window closes, and it runs the event loop of the windowing system until
/// then, so it belongs on the thread the program started on and is called
/// once.
pub fn open(path: Option<&Path>) -> Result<(), String> {
    let (document, mix) = match path {
        Some(path) => (Document::at(path.to_path_buf()), read_the_mix(path)?),
        None => (Document::untitled(), Mix::new()),
    };
    let offer = offer_for(&document, &mix);
    // A user with no configuration folder has no settings file to name, so
    // that problem is reported in its own words rather than under a path.
    let settings_file = Settings::location().map_err(|problem| problem.to_string())?;
    let settings = Settings::read(&settings_file)
        .map_err(|problem| format!("{}: {problem}", settings_file.display()))?;

    // macOS sends the document a person double-clicks to the running
    // application rather than on the command line, and it sends the first
    // one while the application is finishing its launch, before the window
    // exists. The listening therefore starts here, before the event loop,
    // and the window drains the channel on each repaint. The window's
    // context is not built yet, so the repaint a path arriving asks for goes
    // through this slot, which the creator below fills.
    let repainting: Arc<Mutex<Option<egui::Context>>> = Arc::new(Mutex::new(None));
    let waking = Arc::clone(&repainting);
    let documents = macos_documents_sys::watch_opened_documents(Box::new(move || {
        if let Ok(context) = waking.lock()
            && let Some(context) = context.as_ref()
        {
            context.request_repaint();
        }
    }));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(document.title())
            .with_icon(icon())
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Dermixen",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            if let Ok(mut repainting) = repainting.lock() {
                *repainting = Some(cc.egui_ctx.clone());
            }
            let mut window = Window::new(document, mix, settings_file, settings, &cc.egui_ctx);
            window.documents = Some(documents);
            window.take_the_offer(offer);
            // The line is printed here rather than before the window is
            // built, so a run whose window the windowing system refuses says
            // only why it failed.
            say_which_file_was_opened(window.document.path());
            Ok(Box::new(window))
        }),
    )
    .map_err(|problem| format!("the window could not be opened: {problem}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_just_now_was_less_than_a_minute_ago() {
        assert_eq!(
            how_long_ago(Some(SystemTime::now())),
            "less than a minute ago"
        );
    }

    #[test]
    fn a_moment_the_file_system_did_not_report_has_no_age_to_report() {
        assert_eq!(how_long_ago(None), "at a time this machine cannot report");
    }

    #[test]
    fn an_hour_and_a_half_ago_is_reported_in_whole_hours() {
        let then = SystemTime::now() - Duration::from_secs(90 * 60);
        assert_eq!(how_long_ago(Some(then)), "1 hour ago");
        let earlier = SystemTime::now() - Duration::from_secs(3 * 60 * 60);
        assert_eq!(how_long_ago(Some(earlier)), "3 hours ago");
    }

    #[test]
    fn one_track_and_several_are_counted_alike() {
        assert_eq!(track_count(1), "1 track");
        assert_eq!(track_count(4), "4 tracks");
    }

    #[test]
    fn a_track_is_named_in_the_status_line_by_its_file_name() {
        assert_eq!(file_name(Path::new("/music/goa/lsd.mp3")), "lsd.mp3");
        assert_eq!(file_name(Path::new("/")), "/");
    }

    /// A library index in `folder` containing one record: a file of `bytes`
    /// written at `name`, hashed as the library hashes it, with a grid and
    /// anchors that nothing here reads. The record's hash and its path come
    /// back.
    fn an_index_over(folder: &Path, name: &str, bytes: &[u8]) -> (PathBuf, ContentHash, PathBuf) {
        let file = folder.join(name);
        std::fs::write(&file, bytes).expect("the audio file");
        let hash = dermixen_media::hash_file(&file).expect("the file's hash");
        let record = TrackRecord {
            hash,
            path: file.clone(),
            length: Samples(44_100),
            grid: BeatGrid {
                first_beat: Samples::ZERO,
                bpm: Bpm(140.0),
            },
            grid_confidence: 1.0,
            grid_analyzer: "test".to_owned(),
            key: None,
            extent: dermixen_analysis::Extent {
                begins: Samples::ZERO,
                ends: Samples(44_100),
            },
            anchors: dermixen_core::Anchors {
                intro: Beats(0.0),
                outro: Beats(16.0),
            },
            anchor_confidence: 1.0,
            anchor_analyzer: "test".to_owned(),
            metadata: dermixen_library::Metadata {
                artist: None,
                title: None,
                year: None,
                year_is_approximate: false,
                source: dermixen_library::MetadataSource::Filename,
            },
            release: dermixen_library::Release::default(),
            phrases: None,
            loudness: None,
        };
        let index = folder.join("library.sqlite");
        let mut open = Index::open(&index).expect("the index");
        open.upsert(&record).expect("the record");
        (index, hash, file)
    }

    /// A one-track mix whose track has `hash` and names `path`, which may be
    /// a file that is not there.
    fn a_mix_naming(hash: ContentHash, path: &Path) -> Mix {
        Mix {
            tracks: vec![dermixen_core::Track {
                path: path.to_path_buf(),
                hash,
                length: Samples(44_100),
                grid: BeatGrid {
                    first_beat: Samples::ZERO,
                    bpm: Bpm(140.0),
                },
                anchors: dermixen_core::Anchors {
                    intro: Beats(0.0),
                    outro: Beats(16.0),
                },
                keylock: true,
                gain: dermixen_core::Decibels::UNITY,
                volume: dermixen_core::Envelope::new(),
                eq: dermixen_core::EqEnvelopes::default(),
                tempo: Vec::new(),
            }],
        }
    }

    #[test]
    fn a_track_whose_file_moved_is_pointed_at_the_file_the_index_holds() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, hash, moved) = an_index_over(folder.path(), "lsd.wav", b"the track's bytes");
        let mut mix = a_mix_naming(hash, &folder.path().join("gone").join("lsd.wav"));

        let relinking = relink_from_the_index(Some(&index), &mut mix);
        assert!(relinking.relinked, "the track should have been relinked");
        assert_eq!(mix.tracks[0].path, moved.canonicalize().expect("the file"));
        assert!(relinking.no_file.is_empty());
        assert!(
            relinking.message.contains("lsd.wav"),
            "{}",
            relinking.message
        );
    }

    #[test]
    fn a_track_the_index_cannot_place_keeps_its_path_and_is_named() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, _, _) = an_index_over(folder.path(), "lsd.wav", b"the track's bytes");
        let gone = folder.path().join("mahadeva.wav");
        let mut mix = a_mix_naming(ContentHash([7; 32]), &gone);

        let relinking = relink_from_the_index(Some(&index), &mut mix);
        assert!(!relinking.relinked);
        assert_eq!(mix.tracks[0].path, gone);
        assert_eq!(
            relinking.no_file,
            HashMap::from([(ContentHash([7; 32]), NoFile::Missing)])
        );
        assert!(
            relinking.message.contains("mahadeva.wav")
                && relinking.message.contains("is missing")
                && relinking.message.contains("dermixen mix relink"),
            "{}",
            relinking.message
        );
    }

    #[test]
    fn a_file_of_other_bytes_at_the_path_is_reported_as_changed() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, _, _) = an_index_over(folder.path(), "lsd.wav", b"the track's bytes");
        let replaced = folder.path().join("mahadeva.wav");
        std::fs::write(&replaced, b"another recording entirely").expect("the file in its place");
        let mut mix = a_mix_naming(ContentHash([7; 32]), &replaced);

        let relinking = relink_from_the_index(Some(&index), &mut mix);
        assert_eq!(
            relinking.no_file,
            HashMap::from([(ContentHash([7; 32]), NoFile::Changed)])
        );
        assert!(
            relinking.message.contains("mahadeva.wav")
                && relinking.message.contains("has changed")
                && relinking.message.contains("dermixen mix relink"),
            "{}",
            relinking.message
        );
    }

    /// A file at `path` that this user cannot read, and whether the test
    /// can go on: a user who reads any file, which the superuser does, has
    /// nothing here to see.
    #[cfg(unix)]
    fn a_file_closed_to_this_user(path: &Path, bytes: &[u8]) -> bool {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, bytes).expect("the audio file");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000))
            .expect("the file closed");
        std::fs::File::open(path).is_err()
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_read_says_so_rather_than_that_it_changed() {
        use std::os::unix::fs::PermissionsExt;

        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, _, _) = an_index_over(folder.path(), "lsd.wav", b"the track's bytes");
        let closed = folder.path().join("mahadeva.wav");
        if !a_file_closed_to_this_user(&closed, b"another recording entirely") {
            return;
        }
        let mut mix = a_mix_naming(ContentHash([7; 32]), &closed);

        let relinking = relink_from_the_index(Some(&index), &mut mix);
        assert_eq!(
            relinking.no_file,
            HashMap::from([(ContentHash([7; 32]), NoFile::Unreadable)])
        );
        assert!(
            relinking.message.contains("mahadeva.wav")
                && relinking.message.contains("could not be read"),
            "{}",
            relinking.message
        );
        // Relinking is no remedy for a file that is there and closed to this
        // user, so the status line does not offer it as one.
        assert!(
            !relinking.message.contains("dermixen mix relink"),
            "{}",
            relinking.message
        );

        // The folder is removed with the test's own files in it, which needs
        // this one openable again on some file systems.
        std::fs::set_permissions(&closed, std::fs::Permissions::from_mode(0o644))
            .expect("the file opened again");
    }

    #[test]
    fn a_track_whose_file_is_where_the_document_says_is_left_alone() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, hash, file) = an_index_over(folder.path(), "lsd.wav", b"the track's bytes");
        let mut mix = a_mix_naming(hash, &file);

        let relinking = relink_from_the_index(Some(&index), &mut mix);
        assert!(!relinking.relinked);
        assert!(relinking.no_file.is_empty());
        assert_eq!(mix.tracks[0].path, file);
        assert!(relinking.message.is_empty(), "{}", relinking.message);
    }

    #[test]
    fn with_no_index_a_track_with_no_file_is_still_noted_and_the_command_named() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let gone = folder.path().join("lsd.wav");
        let mut mix = a_mix_naming(ContentHash([7; 32]), &gone);

        let relinking = relink_from_the_index(None, &mut mix);
        assert!(!relinking.relinked);
        assert_eq!(mix.tracks[0].path, gone);
        assert_eq!(
            relinking.no_file,
            HashMap::from([(ContentHash([7; 32]), NoFile::Missing)])
        );
        assert!(
            relinking.message.contains("lsd.wav")
                && relinking.message.contains("There is no library to look in")
                && relinking.message.contains("dermixen mix relink"),
            "{}",
            relinking.message
        );
    }

    #[test]
    fn a_restored_document_is_relinked_as_the_opened_one_was() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, hash, moved) = an_index_over(folder.path(), "lsd.wav", b"the track's bytes");
        let stale = folder.path().join("gone").join("lsd.wav");
        let found = moved.canonicalize().expect("the file");

        // What the window does as it opens the mix.
        let mut opened = a_mix_naming(hash, &stale);
        let opening = relink_from_the_index(Some(&index), &mut opened);
        let mut handed = HashSet::new();
        let wanted = to_be_read(&opened, &opening.no_file, &mut handed);
        assert_eq!(wanted.len(), 1, "{wanted:?}");
        assert_eq!(wanted[0].path, found);

        // The autosave file names the path the last session wrote, which is
        // the path the file has left, so the restored document is relinked
        // as the opened one was.
        let mut restored = a_mix_naming(hash, &stale);
        let restoring = relink_from_the_index(Some(&index), &mut restored);
        assert!(restoring.relinked);
        assert_eq!(restored.tracks[0].path, found);
        assert!(restoring.no_file.is_empty());
        // The reading thread has the track's audio under that hash already.
        assert!(
            to_be_read(&restored, &restoring.no_file, &mut handed).is_empty(),
            "a track already read should not be read again"
        );
    }

    #[test]
    fn a_track_the_restored_document_gives_a_file_loses_its_note_and_is_read() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let (index, _, _) = an_index_over(folder.path(), "lsd.wav", b"another track");
        let file = folder.path().join("mahadeva.wav");
        std::fs::write(&file, b"the track's bytes").expect("the audio file");
        let hash = dermixen_media::hash_file(&file).expect("the file's hash");
        std::fs::remove_file(&file).expect("the file taken away");

        // The window opens the mix while the file is nowhere to be found.
        let mut opened = a_mix_naming(hash, &file);
        let opening = relink_from_the_index(Some(&index), &mut opened);
        assert_eq!(opening.no_file.get(&hash), Some(&NoFile::Missing));
        let mut handed = HashSet::new();
        assert!(to_be_read(&opened, &opening.no_file, &mut handed).is_empty());
        assert!(handed.is_empty(), "{handed:?}");

        // The person puts the file back and restores the autosaved document.
        std::fs::write(&file, b"the track's bytes").expect("the audio file again");
        let mut restored = a_mix_naming(hash, &file);
        let restoring = relink_from_the_index(Some(&index), &mut restored);
        assert!(restoring.no_file.is_empty());
        let wanted = to_be_read(&restored, &restoring.no_file, &mut handed);
        assert_eq!(wanted.len(), 1, "{wanted:?}");
        assert_eq!(wanted[0].path, file);
    }

    #[test]
    fn a_file_at_two_positions_of_the_playlist_is_read_once() {
        let folder = tempfile::tempdir().expect("a folder to work in");
        let mut mix = a_mix_naming(ContentHash([7; 32]), &folder.path().join("lsd.wav"));
        let again = mix.tracks[0].clone();
        mix.tracks.push(again);

        let mut handed = HashSet::new();
        let wanted = to_be_read(&mix, &HashMap::new(), &mut handed);
        assert_eq!(wanted.len(), 1, "{wanted:?}");
        assert_eq!(handed, HashSet::from([ContentHash([7; 32])]));
    }

    #[test]
    fn two_messages_are_shown_as_one_and_either_alone() {
        assert_eq!(
            say_both(
                "The file of 1 track is missing.",
                "Cannot read the autosave."
            ),
            "The file of 1 track is missing. Cannot read the autosave."
        );
        assert_eq!(
            say_both("", "Cannot read the autosave."),
            "Cannot read the autosave."
        );
        assert_eq!(
            say_both("The file of 1 track is missing.", ""),
            "The file of 1 track is missing."
        );
    }

    #[test]
    fn a_reason_from_the_file_system_begins_as_a_sentence() {
        assert_eq!(
            capitalized("cannot write /music/set.dmx: no space left".to_owned()),
            "Cannot write /music/set.dmx: no space left"
        );
        assert_eq!(capitalized(String::new()), "");
    }

    #[test]
    fn an_empty_tempo_field_is_named_as_empty() {
        assert_eq!(not_a_tempo(""), "The tempo field is empty");
        assert_eq!(not_a_tempo("quick"), "quick is not a tempo");
    }
}
