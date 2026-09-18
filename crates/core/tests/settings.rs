//! Acceptance tests for the settings file: what a text reads as, what is
//! written back, where the file is, and what is refused. A coder agent makes
//! these pass without editing them.
//!
//! The rules under test are the ones `docs/settings.md` states: a missing
//! file and an empty file are every default, a setting the file does not
//! mention has its default, and a value that cannot be read or a name the
//! app does not know is an error naming its line rather than a default put
//! in its place.

use std::ffi::OsStr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use dermixen_core::{
    DEFAULT_GRID_STRIP_COLLAPSED, DEFAULT_LIBRARY_COLLAPSED, DEFAULT_LIBRARY_FILE,
    DEFAULT_LIBRARY_WORD_WRAP, DEFAULT_METRONOME, DEFAULT_MUSIC_FOLDER, SETTINGS_FILE,
    SETTINGS_FOLDER, SETTINGS_VARIABLE, Settings, SettingsError,
};

/// What the buffer setting takes, as `docs/settings.md` states it and as
/// every refusal of a value says it.
const BUFFER_TAKES: &str = "a whole number of frames from 1 to 4294967295";

/// What the metronome setting takes, in the same words.
const METRONOME_TAKES: &str = "true or false";

/// What the music folder setting and the library file setting take, in the
/// same words.
const PATH_TAKES: &str = "a path in quotes";

/// Every setting there is, as the refusal of a name the app does not know
/// lists them.
const EVERY_SETTING: &str = "The settings are audio_buffer_frames, metronome, grid_strip_collapsed, \
     library_collapsed, library_word_wrap, music_folder, and library_file.";

fn frames(count: u32) -> Option<NonZeroU32> {
    Some(NonZeroU32::new(count).unwrap())
}

/// The line and the problem of a text error, failing on any other error.
fn text_error(result: Result<Settings, SettingsError>) -> (Option<usize>, String) {
    match result {
        Err(SettingsError::Text { line, problem }) => (line, problem),
        other => panic!("expected a text error, got {other:?}"),
    }
}

#[test]
fn empty_text_reads_as_every_default() {
    let settings = Settings::from_toml("").unwrap();
    assert_eq!(settings, Settings::default());
    assert_eq!(settings.audio_buffer_frames, None);
    assert_eq!(settings.metronome, None);
    assert_eq!(settings.metronome(), DEFAULT_METRONOME);
    assert!(!Settings::default().metronome(), "the metronome starts off");
    assert_eq!(
        Settings::from_toml("\n# nothing set\n\n").unwrap(),
        Settings::default(),
        "a comment and blank lines are the same as empty text"
    );
}

#[test]
fn every_setting_round_trips_through_the_text() {
    let settings = Settings {
        audio_buffer_frames: frames(256),
        metronome: Some(true),
        grid_strip_collapsed: Some(true),
        library_collapsed: Some(true),
        library_word_wrap: Some(true),
        music_folder: Some(PathBuf::from("/music/goa")),
        library_file: Some(PathBuf::from("/music/goa.sqlite")),
    };
    let text = settings.to_toml();
    assert_eq!(Settings::from_toml(&text).unwrap(), settings);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(
        lines,
        vec![
            "audio_buffer_frames = 256",
            "metronome = true",
            "grid_strip_collapsed = true",
            "library_collapsed = true",
            "library_word_wrap = true",
            "music_folder = \"/music/goa\"",
            "library_file = \"/music/goa.sqlite\""
        ],
        "one setting per line, as name = value:\n{text}"
    );
    assert!(settings.metronome());
    assert_eq!(settings.audio_buffer_frames.unwrap().get(), 256);
}

#[test]
fn a_setting_the_text_does_not_mention_has_its_default() {
    let settings = Settings::from_toml("metronome = true\n").unwrap();
    assert_eq!(settings.metronome, Some(true));
    assert!(settings.metronome());
    assert_eq!(settings.audio_buffer_frames, None);

    let settings = Settings::from_toml("audio_buffer_frames = 128\n").unwrap();
    assert_eq!(settings.audio_buffer_frames, frames(128));
    assert_eq!(settings.metronome, None);
    assert_eq!(settings.metronome(), DEFAULT_METRONOME);

    let off = Settings::from_toml("metronome = false\n").unwrap();
    assert_eq!(
        off.metronome,
        Some(false),
        "false set is not the same as unset"
    );
    assert!(!off.metronome());
}

#[test]
fn only_the_settings_that_are_set_are_written() {
    let one = Settings {
        metronome: Some(true),
        ..Settings::default()
    };
    let text = one.to_toml();
    assert!(!text.contains("audio_buffer_frames"), "{text}");
    assert!(text.contains("metronome = true"), "{text}");
    assert_eq!(Settings::from_toml(&text).unwrap(), one);

    let none = Settings::default().to_toml();
    assert!(
        none.trim().is_empty(),
        "no setting set writes no line:\n{none}"
    );
    assert_eq!(Settings::from_toml(&none).unwrap(), Settings::default());
}

#[test]
fn a_value_of_the_wrong_kind_is_refused_with_its_line() {
    let (line, problem) = text_error(Settings::from_toml("metronome = \"yes\"\n"));
    assert_eq!(line, Some(1));
    assert!(
        problem.contains("yes"),
        "the problem names the value: {problem}"
    );
    assert!(
        problem.contains("metronome"),
        "the problem names the setting: {problem}"
    );
    assert!(
        problem.contains(METRONOME_TAKES),
        "the problem says what the setting takes: {problem}"
    );
    assert!(
        !problem.contains("invalid type"),
        "no serde words: {problem}"
    );

    let (line, problem) = text_error(Settings::from_toml(
        "metronome = true\naudio_buffer_frames = \"big\"\n",
    ));
    assert_eq!(line, Some(2));
    assert!(
        problem.contains("big"),
        "the problem names the value: {problem}"
    );
    assert!(problem.contains("audio_buffer_frames"), "{problem}");
    assert!(problem.contains(BUFFER_TAKES), "{problem}");
    assert!(
        !problem.contains("u32"),
        "no type names from the code: {problem}"
    );

    let (line, problem) = text_error(Settings::from_toml(
        "metronome = true\n\naudio_buffer_frames = 1.5\n",
    ));
    assert_eq!(line, Some(3));
    assert!(
        problem.contains("1.5"),
        "the problem names the value: {problem}"
    );
    assert!(problem.contains(BUFFER_TAKES), "{problem}");

    let error = SettingsError::Text {
        line: Some(2),
        problem: "expected a boolean".to_owned(),
    };
    assert_eq!(error.to_string(), "line 2: expected a boolean");
}

#[test]
fn a_buffer_of_zero_or_below_or_past_the_top_is_refused() {
    for value in ["0", "-64", "4294967296"] {
        let (line, problem) = text_error(Settings::from_toml(&format!(
            "audio_buffer_frames = {value}\n"
        )));
        assert_eq!(line, Some(1), "{value}");
        assert!(problem.contains(value), "{value}: {problem}");
        assert!(
            problem.contains("audio_buffer_frames"),
            "{value}: {problem}"
        );
        assert!(problem.contains(BUFFER_TAKES), "{value}: {problem}");
        assert!(!problem.contains("u32"), "{value}: {problem}");
    }
}

#[test]
fn a_name_the_app_does_not_know_is_refused_by_name() {
    let (line, problem) = text_error(Settings::from_toml("metronom = true\n"));
    assert_eq!(line, Some(1));
    assert!(
        problem.contains("metronom"),
        "the problem names the name: {problem}"
    );
    assert!(
        problem.contains("audio_buffer_frames") && problem.contains("metronome"),
        "the problem lists the settings there are: {problem}"
    );
    assert!(!problem.contains("field"), "no serde words: {problem}");

    let (line, problem) = text_error(Settings::from_toml("metronome = true\nclick_level = 0.5\n"));
    assert_eq!(line, Some(2));
    assert!(problem.contains("click_level"), "{problem}");
}

#[test]
fn text_that_is_not_toml_is_refused_with_its_line() {
    let (line, _) = text_error(Settings::from_toml("metronome = true\nmetronome =\n"));
    assert_eq!(line, Some(2));
}

#[test]
fn a_missing_file_reads_as_every_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-there").join(SETTINGS_FILE);
    assert_eq!(Settings::read(&path).unwrap(), Settings::default());
    assert!(!path.exists(), "reading makes no file");
}

#[test]
fn a_written_file_reads_back_and_its_folder_is_made() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_FOLDER).join(SETTINGS_FILE);
    let settings = Settings {
        audio_buffer_frames: frames(64),
        metronome: Some(false),
        ..Settings::default()
    };
    settings.write(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), settings.to_toml());
    assert_eq!(Settings::read(&path).unwrap(), settings);

    let fewer = Settings {
        metronome: Some(true),
        ..Settings::default()
    };
    fewer.write(&path).unwrap();
    assert_eq!(
        Settings::read(&path).unwrap(),
        fewer,
        "a write replaces the file"
    );
}

#[test]
fn a_file_that_cannot_be_read_is_an_error_not_a_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SETTINGS_FILE);
    std::fs::write(&path, "metronome = true\naudio_buffer_frames = \"big\"\n").unwrap();
    let (line, problem) = text_error(Settings::read(&path));
    assert_eq!(line, Some(2));
    assert!(problem.contains("big"), "{problem}");

    let folder = dir.path().join("a-folder");
    std::fs::create_dir(&folder).unwrap();
    assert!(
        matches!(Settings::read(&folder), Err(SettingsError::Read(_))),
        "a folder in the file's place is a read error"
    );
}

#[test]
fn a_write_that_fails_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let blocking = dir.path().join("a-file");
    std::fs::write(&blocking, "").unwrap();
    let path = blocking.join(SETTINGS_FILE);
    assert!(
        matches!(
            Settings::default().write(&path),
            Err(SettingsError::Write(_))
        ),
        "a file where the folder should be is a write error"
    );
}

#[test]
fn the_location_is_the_variable_else_the_configuration_folder() {
    let folder = Some(PathBuf::from("/config"));
    assert_eq!(
        Settings::location_from(Some(OsStr::new("/elsewhere/mine.toml")), folder.clone()).unwrap(),
        PathBuf::from("/elsewhere/mine.toml")
    );
    assert_eq!(
        Settings::location_from(None, folder.clone()).unwrap(),
        PathBuf::from("/config")
            .join(SETTINGS_FOLDER)
            .join(SETTINGS_FILE)
    );
    assert_eq!(
        Settings::location_from(Some(OsStr::new("")), folder).unwrap(),
        PathBuf::from("/config")
            .join(SETTINGS_FOLDER)
            .join(SETTINGS_FILE),
        "an empty variable is the same as none"
    );
    assert_eq!(
        Settings::location_from(None, None),
        Err(SettingsError::NoFolder)
    );
    assert_eq!(
        Settings::location_from(Some(OsStr::new("/elsewhere/mine.toml")), None).unwrap(),
        PathBuf::from("/elsewhere/mine.toml"),
        "a named file needs no configuration folder"
    );
    assert_eq!(SETTINGS_FOLDER, "dermixen");
    assert_eq!(SETTINGS_FILE, "settings.toml");
    assert_eq!(SETTINGS_VARIABLE, "DERMIXEN_SETTINGS_FILE");
}

#[test]
fn the_grid_strip_setting_reads_writes_and_is_listed_by_name() {
    let settings = Settings::from_toml("").unwrap();
    assert_eq!(settings.grid_strip_collapsed, None);
    assert_eq!(
        settings.grid_strip_collapsed(),
        DEFAULT_GRID_STRIP_COLLAPSED
    );
    assert!(
        !Settings::default().grid_strip_collapsed(),
        "the strip starts expanded"
    );

    let collapsed = Settings::from_toml("grid_strip_collapsed = true\n").unwrap();
    assert_eq!(collapsed.grid_strip_collapsed, Some(true));
    assert!(collapsed.grid_strip_collapsed());
    assert_eq!(collapsed.to_toml().trim(), "grid_strip_collapsed = true");
    assert_eq!(
        Settings::from_toml("metronome = true\n")
            .unwrap()
            .grid_strip_collapsed(),
        DEFAULT_GRID_STRIP_COLLAPSED,
        "a file that does not mention the strip leaves it expanded"
    );

    let (line, problem) = text_error(Settings::from_toml(
        "metronome = true\ngrid_strip_collapsed = 1\n",
    ));
    assert_eq!(line, Some(2));
    assert!(
        problem.contains("grid_strip_collapsed"),
        "the problem names the setting: {problem}"
    );
    assert!(
        problem.contains(METRONOME_TAKES),
        "the problem says what the setting takes: {problem}"
    );
    assert!(
        !problem.contains("invalid type"),
        "no serde words: {problem}"
    );

    let (line, problem) = text_error(Settings::from_toml("grid_strip = true\n"));
    assert_eq!(line, Some(1));
    assert!(
        problem.ends_with(EVERY_SETTING),
        "the problem lists every setting there is: {problem}"
    );
}

#[test]
fn the_library_setting_reads_writes_and_is_listed_by_name() {
    let settings = Settings::from_toml("").unwrap();
    assert_eq!(settings.library_collapsed, None);
    assert_eq!(settings.library_collapsed(), DEFAULT_LIBRARY_COLLAPSED);
    assert!(
        !Settings::default().library_collapsed(),
        "the panel starts shown"
    );

    let hidden = Settings::from_toml("library_collapsed = true\n").unwrap();
    assert_eq!(hidden.library_collapsed, Some(true));
    assert!(hidden.library_collapsed());
    assert_eq!(hidden.to_toml().trim(), "library_collapsed = true");
    assert_eq!(
        Settings::from_toml("grid_strip_collapsed = true\n")
            .unwrap()
            .library_collapsed(),
        DEFAULT_LIBRARY_COLLAPSED,
        "a file that does not mention the panel leaves it shown"
    );

    let (line, problem) = text_error(Settings::from_toml(
        "metronome = true\nlibrary_collapsed = \"hidden\"\n",
    ));
    assert_eq!(line, Some(2));
    assert!(
        problem.contains("library_collapsed") && problem.contains("hidden"),
        "the problem names the setting and quotes the value: {problem}"
    );
    assert!(
        problem.contains(METRONOME_TAKES),
        "the problem says what the setting takes: {problem}"
    );
    assert!(
        !problem.contains("invalid type"),
        "no serde words: {problem}"
    );

    let (line, problem) = text_error(Settings::from_toml("library_hidden = true\n"));
    assert_eq!(line, Some(1));
    assert!(
        problem.ends_with(EVERY_SETTING),
        "the problem lists every setting there is: {problem}"
    );
}

#[test]
fn the_word_wrap_setting_reads_writes_and_is_listed_by_name() {
    let settings = Settings::from_toml("").unwrap();
    assert_eq!(settings.library_word_wrap, None);
    assert_eq!(settings.library_word_wrap(), DEFAULT_LIBRARY_WORD_WRAP);
    assert!(
        !Settings::default().library_word_wrap(),
        "a cell starts as one line"
    );

    let wrapped = Settings::from_toml("library_word_wrap = true\n").unwrap();
    assert_eq!(wrapped.library_word_wrap, Some(true));
    assert!(wrapped.library_word_wrap());
    assert_eq!(wrapped.to_toml().trim(), "library_word_wrap = true");
    assert_eq!(
        Settings::from_toml("library_collapsed = true\n")
            .unwrap()
            .library_word_wrap(),
        DEFAULT_LIBRARY_WORD_WRAP,
        "a file that does not mention word wrap leaves it off"
    );

    let (line, problem) = text_error(Settings::from_toml(
        "metronome = true\nlibrary_word_wrap = \"yes\"\n",
    ));
    assert_eq!(line, Some(2));
    assert!(
        problem.contains("library_word_wrap") && problem.contains("yes"),
        "the problem names the setting and quotes the value: {problem}"
    );
    assert!(
        problem.contains(METRONOME_TAKES),
        "the problem says what the setting takes: {problem}"
    );
    assert!(
        !problem.contains("invalid type"),
        "no serde words: {problem}"
    );

    let (line, problem) = text_error(Settings::from_toml("library_wrap = true\n"));
    assert_eq!(line, Some(1));
    assert!(
        problem.ends_with(EVERY_SETTING),
        "the problem lists every setting there is: {problem}"
    );
}

#[test]
fn the_music_folder_is_below_the_home_folder_unless_the_setting_says_where() {
    let home = Path::new("/Users/dermixenuser");
    assert_eq!(
        Settings::default().music_folder(home),
        PathBuf::from("/Users/dermixenuser/Music/Undefunktis"),
        "the default is {DEFAULT_MUSIC_FOLDER} below the home folder"
    );
    assert_eq!(
        Settings::from_toml("music_folder = \"/media/goa\"\n")
            .unwrap()
            .music_folder(home),
        PathBuf::from("/media/goa"),
        "an absolute path is used as written"
    );
    assert_eq!(
        Settings::from_toml("music_folder = \"audio/goa\"\n")
            .unwrap()
            .music_folder(home),
        PathBuf::from("/Users/dermixenuser/audio/goa"),
        "a relative path is below the home folder"
    );
}

#[test]
fn the_library_file_is_below_the_data_folder_unless_the_setting_says_where() {
    let home = Path::new("/Users/dermixenuser");
    let data = Path::new("/Users/dermixenuser/Library/Application Support");
    assert_eq!(
        Settings::default().library_file(home, data),
        PathBuf::from("/Users/dermixenuser/Library/Application Support/dermixen/library.sqlite"),
        "the default is {DEFAULT_LIBRARY_FILE} below the data folder"
    );
    assert_eq!(
        Settings::from_toml("library_file = \"/media/goa/library.sqlite\"\n")
            .unwrap()
            .library_file(home, data),
        PathBuf::from("/media/goa/library.sqlite"),
        "an absolute path is used as written"
    );
    assert_eq!(
        Settings::from_toml("library_file = \"goa.sqlite\"\n")
            .unwrap()
            .library_file(home, data),
        PathBuf::from("/Users/dermixenuser/goa.sqlite"),
        "a relative path is below the home folder, not the data folder"
    );
}

#[test]
fn a_path_setting_that_is_not_text_is_refused_with_its_line() {
    let (line, problem) = text_error(Settings::from_toml("metronome = true\nmusic_folder = 7\n"));
    assert_eq!(line, Some(2));
    assert!(
        problem.contains("music_folder"),
        "the problem names the setting: {problem}"
    );
    assert!(
        problem.contains('7'),
        "the problem names the value: {problem}"
    );
    assert!(
        problem.contains(PATH_TAKES),
        "the problem says what the setting takes: {problem}"
    );
    assert!(
        !problem.contains("invalid type") && !problem.contains("string"),
        "no serde words and no type names from the code: {problem}"
    );

    let (line, problem) = text_error(Settings::from_toml("library_file = true\n"));
    assert_eq!(line, Some(1));
    assert!(problem.contains("library_file"), "{problem}");
    assert!(problem.contains("true"), "{problem}");
    assert!(problem.contains(PATH_TAKES), "{problem}");

    assert!(
        Settings::from_toml("music_folder = \"\"\n").is_ok(),
        "empty text is a path the file may give, and the accessor decides what it means"
    );
}

#[test]
fn an_empty_path_setting_means_the_default() {
    let home = Path::new("/home/dermixenuser");
    let data = Path::new("/home/dermixenuser/.local/share");
    let settings = Settings::from_toml("music_folder = \"\"\nlibrary_file = \"\"\n").unwrap();
    assert_eq!(
        settings.music_folder(home),
        PathBuf::from("/home/dermixenuser/Music/Undefunktis")
    );
    assert_eq!(
        settings.library_file(home, data),
        PathBuf::from("/home/dermixenuser/.local/share/dermixen/library.sqlite")
    );
}
