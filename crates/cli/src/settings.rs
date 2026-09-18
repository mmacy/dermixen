//! The `settings` command: shows and changes the settings file that the
//! window and this command share, as `docs/settings.md` describes it.
//!
//! `show` prints the file's path and then one line per setting, as
//! `name = value`, with ` (default)` after a setting the file does not
//! set. `set` and `reset` read the file, change one setting, write the
//! file, and print that setting's line. A file that cannot be read is
//! reported and left as it is. With `--json`, each prints the `settings`
//! document `docs/json/dermixen.schema.json` defines.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use dermixen_core::{
    DEFAULT_GRID_STRIP_COLLAPSED, DEFAULT_LIBRARY_COLLAPSED, DEFAULT_LIBRARY_WORD_WRAP,
    DEFAULT_METRONOME, Settings,
};
use serde::Serialize;

use crate::analyze::print_json;

/// The settings there are, in the order `docs/settings.md` lists them. A
/// person at a terminal reads this list when a command names a setting
/// dermixen does not have.
const NAMES: [&str; 7] = [
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

/// How a buffer the file does not set is shown, since the number of frames
/// is then the device's own choice rather than a number dermixen holds.
const UNSET_BUFFER: &str = "the device's own size";

/// The two folders the path settings are worked out against: the user's
/// home folder, which a relative path in a setting is below, and the user's
/// data folder, which holds the library file when nothing names another
/// one.
#[derive(Debug, Clone)]
struct Folders {
    /// The user's home folder.
    home: PathBuf,
    /// The user's data folder.
    data: PathBuf,
}

impl Folders {
    /// The two folders this user has, or why one of the two is missing.
    fn find() -> Result<Folders, String> {
        Ok(Folders {
            home: home_folder()?,
            data: data_folder()?,
        })
    }
}

/// The user's home folder, which a relative path in the `music_folder`
/// setting or in the `library_file` setting is below.
///
/// Call this before `Settings::music_folder` or `Settings::library_file`,
/// which both take the home folder and cannot work a relative setting out
/// without it. A user who has no home folder at all gets an error whose text
/// is ready to print, telling the person to write the two path settings as
/// paths in full.
pub fn home_folder() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| {
        "there is no home folder for this user, so a path below the home folder cannot be worked \
         out. Write music_folder and library_file as paths in full."
            .to_owned()
    })
}

/// The user's data folder, which holds the library file when neither
/// `--library`, nor the environment, nor the `library_file` setting names
/// another one.
///
/// Call this together with [`home_folder`] to work out the library file, as
/// [`crate::index::location`] does. A user who has no data folder gets an
/// error whose text is ready to print, telling the person how to name a
/// library file instead.
pub fn data_folder() -> Result<PathBuf, String> {
    dirs::data_dir().ok_or_else(|| {
        format!(
            "there is no data folder for this user to keep the library file in, so name one with \
             --library, with {}, or with the library_file setting",
            crate::index::VARIABLE
        )
    })
}

/// One setting, named on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    /// `audio_buffer_frames`.
    Buffer,
    /// `metronome`.
    Metronome,
    /// `grid_strip_collapsed`.
    GridStripCollapsed,
    /// `library_collapsed`.
    LibraryCollapsed,
    /// `library_word_wrap`.
    LibraryWordWrap,
    /// `music_folder`.
    MusicFolder,
    /// `library_file`.
    LibraryFile,
}

/// Every setting, in the order `show` prints them.
const SETTINGS: [Setting; 7] = [
    Setting::Buffer,
    Setting::Metronome,
    Setting::GridStripCollapsed,
    Setting::LibraryCollapsed,
    Setting::LibraryWordWrap,
    Setting::MusicFolder,
    Setting::LibraryFile,
];

impl Setting {
    /// The setting a name stands for, or a message that names what was
    /// asked for and lists the settings there are.
    fn from_name(name: &str) -> Result<Setting, String> {
        match name {
            "audio_buffer_frames" => Ok(Setting::Buffer),
            "metronome" => Ok(Setting::Metronome),
            "grid_strip_collapsed" => Ok(Setting::GridStripCollapsed),
            "library_collapsed" => Ok(Setting::LibraryCollapsed),
            "library_word_wrap" => Ok(Setting::LibraryWordWrap),
            "music_folder" => Ok(Setting::MusicFolder),
            "library_file" => Ok(Setting::LibraryFile),
            other => Err(format!(
                "{other} is not a setting dermixen knows. The settings are {}.",
                listed(&NAMES)
            )),
        }
    }

    /// The line the commands print for this setting, which is `name = value`
    /// with ` (default)` after a value the file does not set.
    ///
    /// A path setting is shown as the path in use, so a relative path in the
    /// file is shown below the user's home folder, where the window and the
    /// commands look for it.
    fn line(self, settings: &Settings, folders: &Folders) -> String {
        match self {
            Setting::Buffer => match settings.audio_buffer_frames {
                Some(frames) => format!("audio_buffer_frames = {frames}"),
                None => format!("audio_buffer_frames = {UNSET_BUFFER} (default)"),
            },
            Setting::Metronome => match settings.metronome {
                Some(on) => format!("metronome = {on}"),
                None => format!("metronome = {DEFAULT_METRONOME} (default)"),
            },
            Setting::GridStripCollapsed => match settings.grid_strip_collapsed {
                Some(collapsed) => format!("grid_strip_collapsed = {collapsed}"),
                None => format!("grid_strip_collapsed = {DEFAULT_GRID_STRIP_COLLAPSED} (default)"),
            },
            Setting::LibraryCollapsed => match settings.library_collapsed {
                Some(collapsed) => format!("library_collapsed = {collapsed}"),
                None => format!("library_collapsed = {DEFAULT_LIBRARY_COLLAPSED} (default)"),
            },
            Setting::LibraryWordWrap => match settings.library_word_wrap {
                Some(wrap) => format!("library_word_wrap = {wrap}"),
                None => format!("library_word_wrap = {DEFAULT_LIBRARY_WORD_WRAP} (default)"),
            },
            Setting::MusicFolder => {
                let folder = settings.music_folder(&folders.home).display().to_string();
                match holds_a_path(settings.music_folder.as_deref()) {
                    true => format!("music_folder = {folder}"),
                    false => format!("music_folder = {folder} (default)"),
                }
            }
            Setting::LibraryFile => {
                let file = settings
                    .library_file(&folders.home, &folders.data)
                    .display()
                    .to_string();
                match holds_a_path(settings.library_file.as_deref()) {
                    true => format!("library_file = {file}"),
                    false => format!("library_file = {file} (default)"),
                }
            }
        }
    }

    /// Puts the value `text` holds into `settings`, or says what the
    /// setting takes and quotes the text that was given instead.
    ///
    /// A path setting takes whatever text it is given, since any text is a
    /// path and the folder or the file it names need not be there yet. The
    /// settings file keeps that text as it was typed, and the accessors in
    /// the core crate work out where a relative path is. Empty text means
    /// the default, so a path setting given empty text leaves the file
    /// rather than sitting in it as a line that means nothing.
    fn set(self, settings: &mut Settings, text: &str) -> Result<(), String> {
        match self {
            Setting::Buffer => {
                let frames = text.parse::<NonZeroU32>().map_err(|_| {
                    format!("audio_buffer_frames takes {BUFFER_TAKES}, not \"{text}\".")
                })?;
                settings.audio_buffer_frames = Some(frames);
            }
            Setting::Metronome => {
                let on = match text {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "metronome takes {METRONOME_TAKES}, not \"{text}\"."
                        ));
                    }
                };
                settings.metronome = Some(on);
            }
            Setting::GridStripCollapsed => {
                let collapsed = match text {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "grid_strip_collapsed takes {METRONOME_TAKES}, not \"{text}\"."
                        ));
                    }
                };
                settings.grid_strip_collapsed = Some(collapsed);
            }
            Setting::LibraryCollapsed => {
                let collapsed = match text {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "library_collapsed takes {METRONOME_TAKES}, not \"{text}\"."
                        ));
                    }
                };
                settings.library_collapsed = Some(collapsed);
            }
            Setting::LibraryWordWrap => {
                let wrap = match text {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "library_word_wrap takes {METRONOME_TAKES}, not \"{text}\"."
                        ));
                    }
                };
                settings.library_word_wrap = Some(wrap);
            }
            Setting::MusicFolder | Setting::LibraryFile if text.is_empty() => {
                self.clear(settings);
            }
            Setting::MusicFolder => settings.music_folder = Some(PathBuf::from(text)),
            Setting::LibraryFile => settings.library_file = Some(PathBuf::from(text)),
        }
        Ok(())
    }

    /// Takes this setting out of `settings`, so that it has its default
    /// again and the file no longer mentions it, and says whether the
    /// setting was there to take out.
    fn clear(self, settings: &mut Settings) -> bool {
        match self {
            Setting::Buffer => settings.audio_buffer_frames.take().is_some(),
            Setting::Metronome => settings.metronome.take().is_some(),
            Setting::GridStripCollapsed => settings.grid_strip_collapsed.take().is_some(),
            Setting::LibraryCollapsed => settings.library_collapsed.take().is_some(),
            Setting::LibraryWordWrap => settings.library_word_wrap.take().is_some(),
            Setting::MusicFolder => settings.music_folder.take().is_some(),
            Setting::LibraryFile => settings.library_file.take().is_some(),
        }
    }
}

/// Whether a path setting holds a path: the file sets it, and the text it
/// sets is not empty, which the accessors in the core crate read as unset.
fn holds_a_path(path: Option<&Path>) -> bool {
    path.is_some_and(|path| !path.as_os_str().is_empty())
}

/// One setting in the JSON output: its value, and whether the file sets it.
#[derive(Debug, Serialize)]
struct Reported<T> {
    /// The value the app uses, which is the default when the file sets
    /// nothing.
    value: T,
    /// Whether the file sets this setting.
    set: bool,
}

/// What `settings show --json`, `settings set --json`, and
/// `settings reset --json` print, which is the `settings` document
/// `docs/json/dermixen.schema.json` defines.
#[derive(Debug, Serialize)]
struct Report {
    /// The settings file, whether or not it exists.
    file: String,
    /// How many frames the audio device takes per pull, null when the
    /// device's own size is used.
    audio_buffer_frames: Reported<Option<u32>>,
    /// Whether the grid editor's metronome is on.
    metronome: Reported<bool>,
    /// Whether the grid editor's strip is collapsed.
    grid_strip_collapsed: Reported<bool>,
    /// Whether the library panel is collapsed.
    library_collapsed: Reported<bool>,
    /// Whether the text in a library panel cell wraps onto further lines.
    library_word_wrap: Reported<bool>,
    /// The folder the person's music is in.
    music_folder: Reported<String>,
    /// The library file the window and the `library` commands open.
    library_file: Reported<String>,
}

/// The JSON document for the settings as they stand.
///
/// Each of the two paths is the path in use, which is the default when the
/// file sets nothing, so neither path is ever null.
fn report(path: &Path, settings: &Settings, folders: &Folders) -> Report {
    Report {
        file: path.display().to_string(),
        audio_buffer_frames: Reported {
            value: settings.audio_buffer_frames.map(NonZeroU32::get),
            set: settings.audio_buffer_frames.is_some(),
        },
        metronome: Reported {
            value: settings.metronome(),
            set: settings.metronome.is_some(),
        },
        grid_strip_collapsed: Reported {
            value: settings.grid_strip_collapsed(),
            set: settings.grid_strip_collapsed.is_some(),
        },
        library_collapsed: Reported {
            value: settings.library_collapsed(),
            set: settings.library_collapsed.is_some(),
        },
        library_word_wrap: Reported {
            value: settings.library_word_wrap(),
            set: settings.library_word_wrap.is_some(),
        },
        music_folder: Reported {
            value: settings.music_folder(&folders.home).display().to_string(),
            set: holds_a_path(settings.music_folder.as_deref()),
        },
        library_file: Reported {
            value: settings
                .library_file(&folders.home, &folders.data)
                .display()
                .to_string(),
            set: holds_a_path(settings.library_file.as_deref()),
        },
    }
}

/// Where the settings file is, or why this user has none.
fn location() -> Result<PathBuf, String> {
    Settings::location().map_err(|problem| problem.to_string())
}

/// Reads the settings file, putting the file's path in front of anything
/// that went wrong, since the errors the core crate reports do not name it.
fn read_from(path: &Path) -> Result<Settings, String> {
    Settings::read(path).map_err(|problem| format!("{}: {problem}", path.display()))
}

/// The settings file and what it holds, which is every default when the
/// file is not there.
///
/// All three of the commands start here, so a file that cannot be read
/// stops the command before it changes or prints anything.
fn open() -> Result<(PathBuf, Settings), String> {
    let path = location()?;
    let settings = read_from(&path)?;
    Ok((path, settings))
}

/// Reads the settings file for a command that uses a setting rather than
/// showing one, like `play` opening the audio device or `library query`
/// opening the library file.
///
/// Call this once at the start of such a command, before it opens anything,
/// so that a file that cannot be read stops the command the way it stops
/// `settings show`. The error names the file and says what is wrong with it,
/// ready to print. A settings file that is not there is every default rather
/// than an error, so a person who has set nothing is never stopped.
pub fn read() -> Result<Settings, String> {
    let path = location()?;
    read_from(&path)
}

/// Writes the whole file, naming the file in anything that went wrong.
fn write(path: &Path, settings: &Settings) -> Result<(), String> {
    settings
        .write(path)
        .map_err(|problem| format!("{}: {problem}", path.display()))
}

/// Prints one setting, as its line or as the whole JSON document, since the
/// JSON output is every setting whichever command printed it.
fn print_one(path: &Path, settings: &Settings, folders: &Folders, setting: Setting, json: bool) {
    if json {
        print_json(&report(path, settings, folders));
    } else {
        println!("{}", setting.line(settings, folders));
    }
}

/// Prints every setting.
pub fn show(json: bool) -> Result<(), String> {
    let (path, settings) = open()?;
    let folders = Folders::find()?;
    if json {
        print_json(&report(&path, &settings, &folders));
    } else {
        println!("{}", path.display());
        for setting in SETTINGS {
            println!("{}", setting.line(&settings, &folders));
        }
    }
    Ok(())
}

/// Sets one setting from its name and the text of its value, writes the
/// file, and prints the setting.
pub fn set(name: &str, value: &str, json: bool) -> Result<(), String> {
    let (path, mut settings) = open()?;
    let setting = Setting::from_name(name)?;
    let folders = Folders::find()?;
    setting.set(&mut settings, value)?;
    write(&path, &settings)?;
    print_one(&path, &settings, &folders, setting, json);
    Ok(())
}

/// Removes one setting from the file, writes the file, and prints the
/// setting with its default.
///
/// A setting the file does not set is already at its default, so nothing is
/// written and the file keeps whatever comments a person put in it.
pub fn reset(name: &str, json: bool) -> Result<(), String> {
    let (path, mut settings) = open()?;
    let setting = Setting::from_name(name)?;
    let folders = Folders::find()?;
    if setting.clear(&mut settings) {
        write(&path, &settings)?;
    }
    print_one(&path, &settings, &folders, setting, json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two folders to work the path settings out against, so that a test
    /// reads the same on every machine.
    fn folders() -> Folders {
        Folders {
            home: PathBuf::from("/home/dermixenuser"),
            data: PathBuf::from("/home/dermixenuser/.local/share"),
        }
    }

    #[test]
    fn a_name_that_is_not_a_setting_lists_the_settings_there_are() {
        let said = Setting::from_name("metronom").unwrap_err();
        assert!(said.contains("metronom"), "{said}");
        assert!(
            said.contains(
                "audio_buffer_frames, metronome, grid_strip_collapsed, library_collapsed, \
                 library_word_wrap, music_folder, and library_file"
            ),
            "{said}"
        );
    }

    #[test]
    fn a_setting_the_file_does_not_set_is_shown_as_a_default() {
        let settings = Settings::default();
        let folders = folders();
        assert_eq!(
            Setting::Buffer.line(&settings, &folders),
            "audio_buffer_frames = the device's own size (default)"
        );
        assert_eq!(
            Setting::Metronome.line(&settings, &folders),
            "metronome = false (default)"
        );
        assert_eq!(
            Setting::MusicFolder.line(&settings, &folders),
            "music_folder = /home/dermixenuser/Music/Undefunktis (default)"
        );
        assert_eq!(
            Setting::LibraryFile.line(&settings, &folders),
            "library_file = /home/dermixenuser/.local/share/dermixen/library.sqlite (default)"
        );
    }

    #[test]
    fn a_setting_the_file_sets_is_shown_without_the_word_default() {
        let mut settings = Settings::default();
        let folders = folders();
        Setting::Buffer.set(&mut settings, "256").unwrap();
        Setting::Metronome.set(&mut settings, "false").unwrap();
        assert_eq!(
            Setting::Buffer.line(&settings, &folders),
            "audio_buffer_frames = 256"
        );
        assert_eq!(
            Setting::Metronome.line(&settings, &folders),
            "metronome = false"
        );
    }

    #[test]
    fn a_path_setting_is_shown_where_the_app_looks_for_it() {
        let mut settings = Settings::default();
        let folders = folders();
        Setting::MusicFolder
            .set(&mut settings, "/media/goa")
            .unwrap();
        assert_eq!(
            Setting::MusicFolder.line(&settings, &folders),
            "music_folder = /media/goa",
            "a path in full is shown as it was written"
        );
        Setting::LibraryFile
            .set(&mut settings, "goa/library.sqlite")
            .unwrap();
        assert_eq!(
            Setting::LibraryFile.line(&settings, &folders),
            "library_file = /home/dermixenuser/goa/library.sqlite",
            "a relative path is shown below the home folder"
        );
        assert_eq!(
            settings.library_file,
            Some(PathBuf::from("goa/library.sqlite")),
            "the file keeps the path as it was typed"
        );
    }

    #[test]
    fn an_empty_path_leaves_the_file_and_shows_as_the_default() {
        let mut settings = Settings::default();
        let folders = folders();
        Setting::MusicFolder
            .set(&mut settings, "/media/goa")
            .unwrap();
        Setting::MusicFolder.set(&mut settings, "").unwrap();
        assert_eq!(
            settings.music_folder, None,
            "empty text takes the setting out of the file"
        );
        assert_eq!(
            Setting::MusicFolder.line(&settings, &folders),
            "music_folder = /home/dermixenuser/Music/Undefunktis (default)"
        );

        let typed_by_hand = Settings {
            library_file: Some(PathBuf::new()),
            ..Settings::default()
        };
        assert_eq!(
            Setting::LibraryFile.line(&typed_by_hand, &folders),
            "library_file = /home/dermixenuser/.local/share/dermixen/library.sqlite (default)",
            "empty text a person put in the file by hand means the default"
        );
        assert!(!holds_a_path(typed_by_hand.library_file.as_deref()));
    }

    #[test]
    fn clearing_says_whether_the_setting_was_set() {
        let mut settings = Settings::default();
        assert!(!Setting::Metronome.clear(&mut settings));
        Setting::Metronome.set(&mut settings, "true").unwrap();
        assert!(Setting::Metronome.clear(&mut settings));
        assert_eq!(settings.metronome, None);
    }

    #[test]
    fn a_value_a_setting_does_not_take_says_what_it_takes() {
        let mut settings = Settings::default();
        for text in ["0", "-5", "1.5", "many", "4294967296"] {
            let said = Setting::Buffer.set(&mut settings, text).unwrap_err();
            assert!(said.contains(text), "{text}: {said}");
            assert!(said.contains(BUFFER_TAKES), "{text}: {said}");
        }
        let said = Setting::Metronome.set(&mut settings, "yes").unwrap_err();
        assert!(
            said.contains("yes") && said.contains(METRONOME_TAKES),
            "{said}"
        );
        assert_eq!(
            settings,
            Settings::default(),
            "a refused value changes nothing"
        );
    }
}
