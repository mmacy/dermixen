//! The settings file: the choices a person makes once, which every run of
//! the window and the `dermixen` command then reads.
//!
//! The file is `settings.toml` in `dermixen` below the user's configuration
//! folder, or the file the `DERMIXEN_SETTINGS_FILE` environment variable
//! names. It is plain text a person edits by hand. Every setting is
//! optional: a setting the file does not mention has its default, a missing
//! file means every default, and an empty file reads the same as a missing
//! one. A value the file gives that cannot be read, and a name the app does
//! not know, are errors that name the line, and nothing is replaced by a
//! default in their place. `docs/settings.md` lists every setting, its
//! default, and what it changes.

use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::files::ReadError;

/// The environment variable that names the settings file.
pub const SETTINGS_VARIABLE: &str = "DERMIXEN_SETTINGS_FILE";

/// The folder below the user's configuration folder that the settings file
/// is in.
pub const SETTINGS_FOLDER: &str = "dermixen";

/// The settings file's name below that folder.
pub const SETTINGS_FILE: &str = "settings.toml";

/// Whether the grid editor's metronome is on when nothing sets it.
pub const DEFAULT_METRONOME: bool = false;

/// Whether the grid editor's strip is collapsed when nothing sets it.
pub const DEFAULT_GRID_STRIP_COLLAPSED: bool = false;

/// Whether the library panel is hidden when nothing sets it.
pub const DEFAULT_LIBRARY_COLLAPSED: bool = false;

/// Whether the text in the library panel's cells wraps when nothing sets
/// it. Wrapping is off, so a cell is one line and a value too wide for its
/// column is cut off at the column's edge. A person turns wrapping on in
/// the settings file, with `dermixen settings set library_word_wrap true`,
/// or with the settings dialog's **Wrap library cells** checkbox, and
/// [`Settings::library_word_wrap()`] then gives that choice in place of
/// this default.
pub const DEFAULT_LIBRARY_WORD_WRAP: bool = false;

/// The music folder below the user's home folder when nothing sets it. It
/// is where the starter tracks go, so that the first library a person
/// builds has something in it.
pub const DEFAULT_MUSIC_FOLDER: &str = "Music/Undefunktis";

/// The library file below the user's data folder when nothing sets it.
pub const DEFAULT_LIBRARY_FILE: &str = "dermixen/library.sqlite";

/// Every setting, each `None` when the file does not set it.
///
/// A file written from this type contains only the settings that are set,
/// so a person's file never fills up with defaults, and a file from a
/// version of the app with fewer settings still reads.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// How many frames the audio device takes each time it pulls from the
    /// preview or the audition. `None` leaves the size to the device. A
    /// change is heard from the next frame the device pulls, so this is the
    /// delay between a change and the ear. A smaller size shortens that
    /// delay and risks dropouts on a slow machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_buffer_frames: Option<NonZeroU32>,
    /// Whether the grid editor's metronome clicks. Turning the metronome on
    /// or off in the window changes this setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metronome: Option<bool>,
    /// Whether the grid editor's strip, the waveform with the grid drawn
    /// over it, is collapsed, so that the editor shows its row of controls
    /// alone. Collapsing or expanding the strip in the window changes this
    /// setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_strip_collapsed: Option<bool>,
    /// Whether the library panel beside the timeline is hidden, so that the
    /// timeline has the whole width of the window. Hiding or showing the
    /// panel in the window changes this setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_collapsed: Option<bool>,
    /// Whether the text in the library panel's cells wraps onto further
    /// lines when the text is wider than its column. With the setting off,
    /// every row is one line tall and a value too wide for its column is cut
    /// off at the column's edge. Turning word wrap on or off in the settings
    /// dialog changes this setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_word_wrap: Option<bool>,
    /// The folder the person's music is in, which is what a scan started
    /// from the window or by `dermixen library scan` with no folder reads.
    /// A relative path is below the user's home folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music_folder: Option<PathBuf>,
    /// The library file the window and the `library` commands open when
    /// neither `--library` nor `DERMIXEN_LIBRARY_FILE` names one. A relative
    /// path is below the user's home folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_file: Option<PathBuf>,
}

/// Why a settings file could not be read, written, or found.
///
/// The text of each error says what went wrong without naming the file.
/// Whoever reports the error puts the file's path in front.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SettingsError {
    /// The file exists but could not be read.
    #[error("cannot be read: {0}")]
    Read(String),
    /// The file, or the folder it goes in, could not be written.
    #[error("cannot be written: {0}")]
    Write(String),
    /// The text does not read as settings: a value the setting does not
    /// take, a name the app does not know, or text that is not TOML.
    /// `line` counts from one and is `None` only when the problem has no
    /// place in the text. For a value the setting does not take, `problem`
    /// names the setting, quotes the value, and says what the setting
    /// takes, in the words `docs/settings.md` uses: `true or false` for
    /// the metronome, the grid strip, the library panel, and word wrap,
    /// `a whole number of frames from 1 to 4294967295` for the buffer, and
    /// `a path in quotes` for the music folder and the library file. For a
    /// name the app does not know, `problem` names it and lists the
    /// settings there are.
    #[error("{}{problem}", line.map(|line| format!("line {line}: ")).unwrap_or_default())]
    Text {
        /// The line of the file the problem is on, counted from one.
        line: Option<usize>,
        /// What is wrong, in words a person at a terminal reads, with no
        /// type name from the code in them.
        problem: String,
    },
    /// The user has no configuration folder, so no default location exists.
    #[error(
        "there is no configuration folder for this user to keep the settings file in, so name a settings file with {SETTINGS_VARIABLE}"
    )]
    NoFolder,
}

impl Settings {
    /// Whether the metronome is on: the setting, else [`DEFAULT_METRONOME`].
    pub fn metronome(&self) -> bool {
        self.metronome.unwrap_or(DEFAULT_METRONOME)
    }

    /// Whether the grid editor's strip is collapsed: the setting, else
    /// [`DEFAULT_GRID_STRIP_COLLAPSED`].
    pub fn grid_strip_collapsed(&self) -> bool {
        self.grid_strip_collapsed
            .unwrap_or(DEFAULT_GRID_STRIP_COLLAPSED)
    }

    /// Whether the library panel is hidden: the setting, else
    /// [`DEFAULT_LIBRARY_COLLAPSED`].
    pub fn library_collapsed(&self) -> bool {
        self.library_collapsed.unwrap_or(DEFAULT_LIBRARY_COLLAPSED)
    }

    /// Whether the library panel's cells wrap their text: the setting,
    /// else [`DEFAULT_LIBRARY_WORD_WRAP`]. With this false, every row of
    /// the library table is one line tall and a value too wide for its
    /// column is cut off at the column's edge. The window reads this value
    /// when it opens and again whenever the settings dialog's **Wrap
    /// library cells** checkbox changes the setting. `dermixen settings`
    /// shows and sets the same setting under the name `library_word_wrap`.
    pub fn library_word_wrap(&self) -> bool {
        self.library_word_wrap.unwrap_or(DEFAULT_LIBRARY_WORD_WRAP)
    }

    /// The music folder: the setting, made absolute below `home` when it is
    /// relative, else [`DEFAULT_MUSIC_FOLDER`] below `home`. A setting that
    /// is empty text counts as unset. The folder need not exist.
    pub fn music_folder(&self, home: &Path) -> PathBuf {
        below(self.music_folder.as_deref(), home).unwrap_or_else(|| home.join(DEFAULT_MUSIC_FOLDER))
    }

    /// The library file: the setting, made absolute below `home` when it is
    /// relative, else [`DEFAULT_LIBRARY_FILE`] below `data_folder`, which is
    /// `~/Library/Application Support` on macOS and `~/.local/share` on
    /// Linux. A setting that is empty text counts as unset. The file need
    /// not exist.
    ///
    /// This accessor is the third of the four ways a library file is chosen,
    /// not the first. The `dermixen` command reads the `--library` option
    /// and then the `DERMIXEN_LIBRARY_FILE` environment variable before it
    /// calls this accessor, and the window reads the same variable, so a
    /// caller that opens a library file reads those two first and comes here
    /// when neither names a file.
    pub fn library_file(&self, home: &Path, data_folder: &Path) -> PathBuf {
        below(self.library_file.as_deref(), home)
            .unwrap_or_else(|| data_folder.join(DEFAULT_LIBRARY_FILE))
    }

    /// Reads settings from the text of a file. Empty text is every default.
    pub fn from_toml(text: &str) -> Result<Settings, SettingsError> {
        toml::from_str(text).map_err(|error| text_error(text, &error))
    }

    /// The text of a file that reads back as these settings, one setting
    /// per line as `name = value`, containing only the settings that are
    /// set, and empty when none is.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a Settings value always has text")
    }

    /// Where the settings file is: the file [`SETTINGS_VARIABLE`] names,
    /// else `dermixen/settings.toml` in the user's configuration folder,
    /// which is `~/Library/Application Support` on macOS and `~/.config` on
    /// Linux. The file need not exist.
    pub fn location() -> Result<PathBuf, SettingsError> {
        Settings::location_from(
            std::env::var_os(SETTINGS_VARIABLE).as_deref(),
            dirs::config_dir(),
        )
    }

    /// [`location`](Settings::location) with its two inputs given: the
    /// value of the environment variable, and the user's configuration
    /// folder. An empty variable counts as unset.
    pub fn location_from(
        named: Option<&OsStr>,
        config_folder: Option<PathBuf>,
    ) -> Result<PathBuf, SettingsError> {
        if let Some(named) = named
            && !named.is_empty()
        {
            return Ok(PathBuf::from(named));
        }
        config_folder
            .map(|folder| folder.join(SETTINGS_FOLDER).join(SETTINGS_FILE))
            .ok_or(SettingsError::NoFolder)
    }

    /// Reads the settings file at `path`. A file that is not there is every
    /// default. A file that is there and cannot be read, or whose text does
    /// not read as settings, is an error.
    ///
    /// The file is read as [`crate::files::read_text`] reads one, so it must
    /// be a regular file of at most [`crate::files::LARGEST_SETTINGS`] bytes.
    /// A path that names a device or a named pipe is an error at once rather
    /// than a read that never ends.
    pub fn read(path: &Path) -> Result<Settings, SettingsError> {
        match crate::files::read_text(path, crate::files::LARGEST_SETTINGS) {
            Ok(text) => Settings::from_toml(&text),
            Err(ReadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok(Settings::default())
            }
            Err(ReadError::Io { source, .. }) => Err(SettingsError::Read(source.to_string())),
            Err(ReadError::NotARegularFile { .. }) => {
                Err(SettingsError::Read("it is not a regular file".to_owned()))
            }
            Err(ReadError::TooLarge { limit, .. }) => Err(SettingsError::Read(format!(
                "it is larger than the {limit} bytes a settings file may be"
            ))),
        }
    }

    /// Writes these settings to the file at `path`, making the folder it
    /// goes in when the folder is not there, and replacing the file when it
    /// is.
    ///
    /// The write goes through [`crate::files::write_atomically`], so a run
    /// that stops part way leaves the settings file as it was, and a file
    /// already at `path` keeps its permissions.
    pub fn write(&self, path: &Path) -> Result<(), SettingsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| SettingsError::Write(error.to_string()))?;
        }
        crate::files::write_atomically(path, false, self.to_toml().as_bytes())
            .map_err(|error| SettingsError::Write(error.to_string()))
    }
}

/// The path a setting gives, made absolute below `home` when the setting
/// gives a relative path, and `None` when the setting is unset or its path
/// is empty, since an unset setting and an empty path both mean the
/// default.
fn below(path: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let path = path?;
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    Some(home.join(path))
}

/// The settings there are, in the order `docs/settings.md` lists them. A
/// person at a terminal reads this list when the text names a setting this
/// version of the app does not know.
const SETTING_NAMES: [&str; 7] = [
    "audio_buffer_frames",
    "metronome",
    "grid_strip_collapsed",
    "library_collapsed",
    "library_word_wrap",
    "music_folder",
    "library_file",
];

/// `names` written out as a person reads a list: joined by commas, with
/// "and" before the last name.
fn listed(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// What the metronome setting, the grid strip setting, the library panel
/// setting, and the word wrap setting take, in the words `docs/settings.md`
/// uses.
const METRONOME_TAKES: &str = "true or false";

/// What the buffer setting takes, in the words `docs/settings.md` uses.
const BUFFER_TAKES: &str = "a whole number of frames from 1 to 4294967295";

/// What the music folder setting and the library file setting take, in the
/// words `docs/settings.md` uses.
const PATH_TAKES: &str = "a path in quotes";

/// Turns a `toml` parse error into the [`SettingsError::Text`] a person at a
/// terminal reads.
///
/// The line comes from the `toml` error's span, counted from one by
/// counting the newlines in `text` before the span starts. A problem with
/// no span has no line. Three problems get their own words, with no serde
/// or Rust vocabulary in them: a setting set twice, a value a setting does
/// not take, and a name the app does not know. For the two value problems,
/// the setting named is the key at the start of the line the span is on,
/// since the settings file contains one setting per line as `name = value`.
/// Every other parse problem is reported in the `toml` parser's own words,
/// with its line.
fn text_error(text: &str, error: &toml::de::Error) -> SettingsError {
    let message = error.message();
    let span = error.span();
    let line = span.as_ref().map(|span| line_number(text, span.start));
    let value = || {
        span.clone()
            .and_then(|span| text.get(span))
            .unwrap_or("")
            .trim()
            .to_owned()
    };
    let key_on_line = || {
        line.and_then(|line| text.lines().nth(line - 1))
            .and_then(|line_text| line_text.split('=').next())
            .unwrap_or("")
            .trim()
            .to_owned()
    };

    let problem = if message == "duplicate key" {
        format!("{} is set twice.", value())
    }
    // A later version of `toml` or `serde` can reword `unknown field`,
    // `expected a boolean`, `expected a nonzero u32`, or `expected path
    // string`. The four checks below match those exact words, and the tests
    // in `crates/core/tests/settings.rs` that check this function's wording
    // fail the day one of the four messages changes.
    else if message.starts_with("unknown field") {
        format!(
            "{} is not a setting dermixen knows. The settings are {}.",
            value(),
            listed(&SETTING_NAMES),
        )
    } else if message.contains("expected a boolean") {
        format!(
            "{} takes {METRONOME_TAKES}, not {}.",
            key_on_line(),
            value()
        )
    } else if message.contains("expected a nonzero u32") {
        format!("{} takes {BUFFER_TAKES}, not {}.", key_on_line(), value())
    } else if message.contains("expected path string") {
        format!("{} takes {PATH_TAKES}, not {}.", key_on_line(), value())
    } else {
        message.to_owned()
    };

    SettingsError::Text { line, problem }
}

/// The one-based line `offset` (a byte index into `text`) falls on, counted
/// by the newlines before it.
fn line_number(text: &str, offset: usize) -> usize {
    text.get(..offset).unwrap_or(text).matches('\n').count() + 1
}
