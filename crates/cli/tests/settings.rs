//! Acceptance tests for the `settings` commands and for the settings file's
//! reach into `play`. A coder agent makes these pass without editing them.
//!
//! The commands share the settings file with the window, so the rules
//! under test are the ones `docs/settings.md` states: a missing file is
//! every default, a set value is read back, a value that cannot be read is
//! reported and never replaced, and every command that uses a setting reads
//! the file before it does anything else.

mod common;

use std::path::Path;

use common::{dermixen, fails, json, ok, stdout, two_track_mix};

/// The settings file every test points the command at, in its own folder,
/// so that no test reads the person's own file.
fn settings_file(dir: &Path) -> String {
    dir.join("settings.toml").to_str().unwrap().to_owned()
}

/// Runs `dermixen` with the settings file in `dir`.
fn with_settings(dir: &Path, args: &[&str]) -> std::process::Output {
    dermixen(
        dir,
        &[("DERMIXEN_SETTINGS_FILE", &settings_file(dir))],
        args,
    )
}

/// The line of `show`'s output for one setting.
fn line_for<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find(|line| line.starts_with(&format!("{name} = ")))
        .unwrap_or_else(|| panic!("no line for {name}:\n{text}"))
}

/// Checks a value against one definition in the schema file, failing with
/// every violation the validator found.
fn fits(value: &serde_json::Value, definition: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let whole: serde_json::Value = serde_json::from_str(&text).unwrap();
    let schema = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": format!("#/$defs/{definition}"),
        "$defs": whole["$defs"],
    });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let problems: Vec<String> = validator
        .iter_errors(value)
        .map(|problem| format!("{} at {}", problem, problem.instance_path()))
        .collect();
    assert!(
        problems.is_empty(),
        "does not fit {definition}:\n{}\n{}",
        problems.join("\n"),
        serde_json::to_string_pretty(value).unwrap()
    );
}

#[test]
fn show_prints_every_default_when_there_is_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    let text = stdout(&out);
    assert!(
        text.contains(&settings_file(dir.path())),
        "show names the file:\n{text}"
    );
    let buffer = line_for(&text, "audio_buffer_frames");
    assert_eq!(
        buffer,
        "audio_buffer_frames = the device's own size (default)"
    );
    let metronome = line_for(&text, "metronome");
    assert!(metronome.starts_with("metronome = false"), "{metronome}");
    assert!(metronome.ends_with("(default)"), "{metronome}");
    assert!(
        !dir.path().join("settings.toml").exists(),
        "show makes no file"
    );
}

#[test]
fn set_writes_the_file_and_show_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");

    let out = with_settings(dir.path(), &["settings", "set", "metronome", "true"]);
    ok(&out);
    assert_eq!(stdout(&out).trim(), "metronome = true");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "metronome = true"
    );

    let out = with_settings(
        dir.path(),
        &["settings", "set", "audio_buffer_frames", "256"],
    );
    ok(&out);
    assert_eq!(stdout(&out).trim(), "audio_buffer_frames = 256");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("audio_buffer_frames = 256"), "{written}");
    assert!(written.contains("metronome = true"), "{written}");

    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    let text = stdout(&out);
    assert_eq!(
        line_for(&text, "audio_buffer_frames"),
        "audio_buffer_frames = 256"
    );
    assert_eq!(line_for(&text, "metronome"), "metronome = true");

    let out = with_settings(dir.path(), &["settings", "set", "metronome", "false"]);
    ok(&out);
    assert_eq!(stdout(&out).trim(), "metronome = false");
    let out = with_settings(dir.path(), &["settings", "show"]);
    assert_eq!(
        line_for(&stdout(&out), "metronome"),
        "metronome = false",
        "false set by the file is not a default"
    );
}

#[test]
fn reset_removes_the_setting_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    ok(&with_settings(
        dir.path(),
        &["settings", "set", "metronome", "true"],
    ));
    ok(&with_settings(
        dir.path(),
        &["settings", "set", "audio_buffer_frames", "128"],
    ));

    let out = with_settings(dir.path(), &["settings", "reset", "metronome"]);
    ok(&out);
    let line = stdout(&out);
    assert!(line.trim().starts_with("metronome = false"), "{line}");
    assert!(line.trim().ends_with("(default)"), "{line}");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(!written.contains("metronome"), "{written}");
    assert!(written.contains("audio_buffer_frames = 128"), "{written}");

    let out = with_settings(dir.path(), &["settings", "reset", "audio_buffer_frames"]);
    ok(&out);
    assert_eq!(
        stdout(&out).trim(),
        "audio_buffer_frames = the device's own size (default)"
    );
    assert!(std::fs::read_to_string(&file).unwrap().trim().is_empty());

    let out = with_settings(dir.path(), &["settings", "reset", "metronome"]);
    ok(&out);
    assert!(
        stdout(&out).trim().ends_with("(default)"),
        "resetting a setting that is not set is not a failure"
    );
}

#[test]
fn the_json_output_fits_the_schema_and_says_what_is_set() {
    let dir = tempfile::tempdir().unwrap();
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["file"], settings_file(dir.path()));
    assert_eq!(
        value["audio_buffer_frames"]["value"],
        serde_json::Value::Null
    );
    assert_eq!(value["audio_buffer_frames"]["set"], false);
    assert_eq!(value["metronome"]["value"], false);
    assert_eq!(value["metronome"]["set"], false);

    let value = json(&with_settings(
        dir.path(),
        &["settings", "set", "metronome", "true", "--json"],
    ));
    fits(&value, "settings");
    assert_eq!(value["metronome"]["value"], true);
    assert_eq!(value["metronome"]["set"], true);
    assert_eq!(value["audio_buffer_frames"]["set"], false);

    let value = json(&with_settings(
        dir.path(),
        &["settings", "set", "audio_buffer_frames", "64", "--json"],
    ));
    fits(&value, "settings");
    assert_eq!(value["audio_buffer_frames"]["value"], 64);
    assert_eq!(value["audio_buffer_frames"]["set"], true);

    let value = json(&with_settings(
        dir.path(),
        &["settings", "reset", "metronome", "--json"],
    ));
    fits(&value, "settings");
    assert_eq!(value["metronome"]["value"], false);
    assert_eq!(value["metronome"]["set"], false);
    assert_eq!(value["audio_buffer_frames"]["value"], 64);
}

#[test]
fn a_name_or_a_value_the_command_does_not_know_is_refused_and_nothing_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "metronom", "true"],
    ));
    assert!(said.contains("metronom"), "{said}");
    assert!(
        said.contains("audio_buffer_frames") && said.contains("metronome"),
        "the message lists the settings there are: {said}"
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "metronome", "yes"],
    ));
    assert!(said.contains("metronome") && said.contains("yes"), "{said}");
    assert!(
        said.contains("true or false"),
        "the message says what the setting takes: {said}"
    );

    for value in ["0", "-5", "1.5", "many", "4294967296"] {
        let said = fails(&with_settings(
            dir.path(),
            &["settings", "set", "audio_buffer_frames", value],
        ));
        assert!(
            said.contains("audio_buffer_frames") && said.contains(value),
            "{value}: {said}"
        );
        assert!(
            said.contains("a whole number of frames from 1 to 4294967295"),
            "{value}: the message says what the setting takes: {said}"
        );
    }

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "reset", "nothing"],
    ));
    assert!(said.contains("nothing"), "{said}");

    assert!(!file.exists(), "a refused command writes no file");
}

#[test]
fn a_file_that_cannot_be_read_is_reported_and_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    let text = "metronome = \"yes\"\n";
    std::fs::write(&file, text).unwrap();

    let said = fails(&with_settings(dir.path(), &["settings", "show"]));
    assert!(said.contains(&settings_file(dir.path())), "{said}");
    assert!(said.contains("line 1"), "{said}");
    assert!(said.contains("yes"), "{said}");

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "audio_buffer_frames", "256"],
    ));
    assert!(said.contains("line 1"), "{said}");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        text,
        "the file is left as it was"
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "reset", "metronome"],
    ));
    assert!(said.contains("line 1"), "{said}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), text);

    std::fs::write(&file, "metronome = true\nclick_level = 0.5\n").unwrap();
    let said = fails(&with_settings(dir.path(), &["settings", "show"]));
    assert!(
        said.contains("line 2") && said.contains("click_level"),
        "{said}"
    );
}

#[test]
fn play_reads_the_settings_file_before_it_plays() {
    let dir = tempfile::tempdir().unwrap();
    two_track_mix(dir.path());
    let file = dir.path().join("settings.toml");
    let play = [
        "play",
        "set.dmx",
        "--capture",
        "out.wav",
        "--from",
        "17",
        "--for",
        "1",
    ];

    std::fs::write(&file, "audio_buffer_frames = \"big\"\n").unwrap();
    let said = fails(&with_settings(dir.path(), &play));
    assert!(said.contains(&settings_file(dir.path())), "{said}");
    assert!(said.contains("line 1"), "{said}");
    assert!(!dir.path().join("out.wav").exists(), "nothing was captured");

    std::fs::write(&file, "audio_buffer_frames = 256\n").unwrap();
    ok(&with_settings(dir.path(), &play));
    assert!(dir.path().join("out.wav").exists());
}

#[test]
fn the_help_names_the_file_and_the_settings() {
    let dir = tempfile::tempdir().unwrap();
    let out = with_settings(dir.path(), &["settings", "--help"]);
    ok(&out);
    let text = stdout(&out);
    assert!(text.contains("DERMIXEN_SETTINGS_FILE"), "{text}");
    for word in ["show", "set", "reset"] {
        assert!(text.contains(word), "{word} is not in:\n{text}");
    }
    let out = with_settings(dir.path(), &["settings", "set", "--help"]);
    ok(&out);
    let text = stdout(&out);
    assert!(
        text.contains("audio_buffer_frames") && text.contains("metronome"),
        "{text}"
    );
}

#[test]
fn the_grid_strip_setting_is_shown_set_reset_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");

    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    assert_eq!(
        line_for(&stdout(&out), "grid_strip_collapsed"),
        "grid_strip_collapsed = false (default)"
    );

    let out = with_settings(
        dir.path(),
        &["settings", "set", "grid_strip_collapsed", "true"],
    );
    ok(&out);
    assert_eq!(stdout(&out).trim(), "grid_strip_collapsed = true");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "grid_strip_collapsed = true"
    );
    let out = with_settings(dir.path(), &["settings", "show"]);
    assert_eq!(
        line_for(&stdout(&out), "grid_strip_collapsed"),
        "grid_strip_collapsed = true"
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "grid_strip_collapsed", "yes"],
    ));
    assert!(
        said.contains("grid_strip_collapsed") && said.contains("yes"),
        "{said}"
    );
    assert!(
        said.contains("true or false"),
        "the message says what the setting takes: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "grid_strip_collapsed = true",
        "a refused value leaves the file alone"
    );

    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["grid_strip_collapsed"]["value"], true);
    assert_eq!(value["grid_strip_collapsed"]["set"], true);

    let out = with_settings(dir.path(), &["settings", "reset", "grid_strip_collapsed"]);
    ok(&out);
    assert_eq!(
        stdout(&out).trim(),
        "grid_strip_collapsed = false (default)"
    );
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("grid_strip_collapsed"),
        "the reset took the setting out of the file"
    );
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["grid_strip_collapsed"]["value"], false);
    assert_eq!(value["grid_strip_collapsed"]["set"], false);

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let required = schema["$defs"]["settings"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"grid_strip_collapsed"),
        "the settings document requires the setting: {required:?}"
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "metronom", "true"],
    ));
    assert!(
        said.trim_end().ends_with(
            "The settings are audio_buffer_frames, metronome, grid_strip_collapsed, \
             library_collapsed, library_word_wrap, music_folder, and library_file."
        ),
        "the message lists every setting there is: {said}"
    );

    let out = with_settings(dir.path(), &["settings", "set", "--help"]);
    ok(&out);
    assert!(
        stdout(&out).contains("grid_strip_collapsed"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn the_library_setting_is_shown_set_reset_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");

    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    assert_eq!(
        line_for(&stdout(&out), "library_collapsed"),
        "library_collapsed = false (default)"
    );
    let shown = stdout(&out);
    let lines: Vec<&str> = shown
        .lines()
        .filter(|line| line.contains(" = "))
        .map(|line| line.split(" = ").next().unwrap())
        .collect();
    assert_eq!(
        lines,
        vec![
            "audio_buffer_frames",
            "metronome",
            "grid_strip_collapsed",
            "library_collapsed",
            "library_word_wrap",
            "music_folder",
            "library_file"
        ],
        "show prints the settings in the order docs/settings.md lists them"
    );

    let out = with_settings(
        dir.path(),
        &["settings", "set", "library_collapsed", "true"],
    );
    ok(&out);
    assert_eq!(stdout(&out).trim(), "library_collapsed = true");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "library_collapsed = true"
    );
    let out = with_settings(dir.path(), &["settings", "show"]);
    assert_eq!(
        line_for(&stdout(&out), "library_collapsed"),
        "library_collapsed = true"
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "library_collapsed", "hidden"],
    ));
    assert!(
        said.contains("library_collapsed") && said.contains("hidden"),
        "{said}"
    );
    assert!(
        said.contains("true or false"),
        "the message says what the setting takes: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "library_collapsed = true",
        "a refused value leaves the file alone"
    );

    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["library_collapsed"]["value"], true);
    assert_eq!(value["library_collapsed"]["set"], true);

    let out = with_settings(dir.path(), &["settings", "set", "metronome", "true"]);
    ok(&out);
    let out = with_settings(dir.path(), &["settings", "reset", "library_collapsed"]);
    ok(&out);
    assert_eq!(stdout(&out).trim(), "library_collapsed = false (default)");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        !written.contains("library_collapsed"),
        "the reset took the setting out of the file:\n{written}"
    );
    assert!(
        written.contains("metronome = true"),
        "the reset left the other settings alone:\n{written}"
    );
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["library_collapsed"]["value"], false);
    assert_eq!(value["library_collapsed"]["set"], false);

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let required = schema["$defs"]["settings"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"library_collapsed"),
        "the settings document requires the setting: {required:?}"
    );

    let out = with_settings(dir.path(), &["settings", "set", "--help"]);
    ok(&out);
    assert!(
        stdout(&out).contains("library_collapsed"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn the_word_wrap_setting_is_shown_set_reset_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");

    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    assert_eq!(
        line_for(&stdout(&out), "library_word_wrap"),
        "library_word_wrap = false (default)"
    );
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["library_word_wrap"]["value"], false);
    assert_eq!(value["library_word_wrap"]["set"], false);

    let out = with_settings(
        dir.path(),
        &["settings", "set", "library_word_wrap", "true"],
    );
    ok(&out);
    assert_eq!(stdout(&out).trim(), "library_word_wrap = true");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "library_word_wrap = true"
    );
    let out = with_settings(dir.path(), &["settings", "show"]);
    assert_eq!(
        line_for(&stdout(&out), "library_word_wrap"),
        "library_word_wrap = true"
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "library_word_wrap", "yes"],
    ));
    assert!(
        said.contains("library_word_wrap") && said.contains("yes"),
        "{said}"
    );
    assert!(
        said.contains("true or false"),
        "the message says what the setting takes: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "library_word_wrap = true",
        "a refused value leaves the file alone"
    );

    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["library_word_wrap"]["value"], true);
    assert_eq!(value["library_word_wrap"]["set"], true);

    let out = with_settings(dir.path(), &["settings", "set", "metronome", "true"]);
    ok(&out);
    let out = with_settings(dir.path(), &["settings", "reset", "library_word_wrap"]);
    ok(&out);
    assert_eq!(stdout(&out).trim(), "library_word_wrap = false (default)");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        !written.contains("library_word_wrap"),
        "the reset took the setting out of the file:\n{written}"
    );
    assert!(
        written.contains("metronome = true"),
        "the reset left the other settings alone:\n{written}"
    );
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["library_word_wrap"]["value"], false);
    assert_eq!(value["library_word_wrap"]["set"], false);

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let required = schema["$defs"]["settings"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"library_word_wrap"),
        "the settings document requires the setting: {required:?}"
    );

    let out = with_settings(dir.path(), &["settings", "set", "--help"]);
    ok(&out);
    assert!(
        stdout(&out).contains("library_word_wrap"),
        "{}",
        stdout(&out)
    );

    let said = fails(&with_settings(
        dir.path(),
        &["settings", "set", "wrap", "true"],
    ));
    assert!(
        said.contains("library_word_wrap"),
        "a name the command does not know lists the settings there are: {said}"
    );
}

/// The music folder and the library file the command reports when the
/// settings file does not set them, which are the defaults below the home
/// folder and the data folder of whoever runs the test.
fn default_paths() -> (String, String) {
    let home = dirs::home_dir().expect("the test runs as a user with a home folder");
    let data = dirs::data_dir().expect("the test runs as a user with a data folder");
    (
        home.join("Music/Undefunktis").display().to_string(),
        data.join("dermixen/library.sqlite").display().to_string(),
    )
}

#[test]
fn the_music_folder_setting_is_shown_set_reset_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    let (default_folder, _) = default_paths();

    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    assert_eq!(
        line_for(&stdout(&out), "music_folder"),
        format!("music_folder = {default_folder} (default)")
    );

    let out = with_settings(
        dir.path(),
        &["settings", "set", "music_folder", "/media/goa/comp"],
    );
    ok(&out);
    assert_eq!(stdout(&out).trim(), "music_folder = /media/goa/comp");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "music_folder = \"/media/goa/comp\"",
        "the path is written in quotes, as TOML text"
    );
    let out = with_settings(dir.path(), &["settings", "show"]);
    assert_eq!(
        line_for(&stdout(&out), "music_folder"),
        "music_folder = /media/goa/comp"
    );

    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(value["music_folder"]["value"], "/media/goa/comp");
    assert_eq!(value["music_folder"]["set"], true);

    let out = with_settings(dir.path(), &["settings", "reset", "music_folder"]);
    ok(&out);
    assert_eq!(
        stdout(&out).trim(),
        format!("music_folder = {default_folder} (default)")
    );
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("music_folder"),
        "the reset took the setting out of the file"
    );
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(
        value["music_folder"]["value"], default_folder,
        "the JSON gives the folder in use, which is the default"
    );
    assert_eq!(value["music_folder"]["set"], false);

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/json/dermixen.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let required = schema["$defs"]["settings"]["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(
        required.contains(&"music_folder") && required.contains(&"library_file"),
        "the settings document requires both path settings: {required:?}"
    );

    let out = with_settings(dir.path(), &["settings", "set", "--help"]);
    ok(&out);
    assert!(
        stdout(&out).contains("music_folder") && stdout(&out).contains("library_file"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn the_library_file_setting_is_shown_set_reset_and_a_relative_path_is_below_home() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("settings.toml");
    let (_, default_file) = default_paths();
    let home = dirs::home_dir().unwrap();

    let out = with_settings(dir.path(), &["settings", "show"]);
    ok(&out);
    assert_eq!(
        line_for(&stdout(&out), "library_file"),
        format!("library_file = {default_file} (default)")
    );

    let out = with_settings(
        dir.path(),
        &["settings", "set", "library_file", "goa/library.sqlite"],
    );
    ok(&out);
    assert_eq!(
        stdout(&out).trim(),
        format!(
            "library_file = {}",
            home.join("goa/library.sqlite").display()
        ),
        "a relative path is shown where it will be looked for, below the home folder"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap().trim(),
        "library_file = \"goa/library.sqlite\"",
        "the file keeps the path as it was given"
    );
    let value = json(&with_settings(dir.path(), &["settings", "show", "--json"]));
    fits(&value, "settings");
    assert_eq!(
        value["library_file"]["value"],
        home.join("goa/library.sqlite").display().to_string()
    );
    assert_eq!(value["library_file"]["set"], true);

    let out = with_settings(dir.path(), &["settings", "reset", "library_file"]);
    ok(&out);
    assert_eq!(
        stdout(&out).trim(),
        format!("library_file = {default_file} (default)")
    );
    assert!(
        std::fs::read_to_string(&file).unwrap().trim().is_empty(),
        "the reset took the setting out of the file"
    );

    // A file that gives a path setting something other than text is refused
    // by every command that reads the file, with the line, the setting, and
    // what it takes, and nothing is written.
    std::fs::write(&file, "library_file = 7\n").unwrap();
    let said = fails(&with_settings(dir.path(), &["settings", "show"]));
    assert!(said.contains("line 1"), "{said}");
    assert!(said.contains("library_file"), "{said}");
    assert!(
        said.contains("a path in quotes"),
        "the message says what the setting takes: {said}"
    );
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "library_file = 7\n"
    );
}

#[test]
fn the_library_commands_open_the_file_the_setting_names_after_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    let by_setting = dir.path().join("by-setting.sqlite");
    let by_env = dir.path().join("by-env.sqlite");
    let by_option = dir.path().join("by-option.sqlite");
    ok(&with_settings(
        dir.path(),
        &[
            "settings",
            "set",
            "library_file",
            by_setting.to_str().unwrap(),
        ],
    ));

    // With nothing else naming a library file, the setting does. The test
    // runner sets DERMIXEN_LIBRARY_FILE unless the test does, and an empty
    // value counts as unset.
    let out = dermixen(
        dir.path(),
        &[
            ("DERMIXEN_SETTINGS_FILE", &settings_file(dir.path())),
            ("DERMIXEN_LIBRARY_FILE", ""),
        ],
        &["library", "query", "--json"],
    );
    assert_eq!(json(&out), serde_json::json!([]));
    assert!(
        by_setting.exists(),
        "the query opened the file the setting names, creating it empty"
    );

    // The environment variable comes before the setting, and the option
    // before both.
    let out = dermixen(
        dir.path(),
        &[
            ("DERMIXEN_SETTINGS_FILE", &settings_file(dir.path())),
            ("DERMIXEN_LIBRARY_FILE", by_env.to_str().unwrap()),
        ],
        &["library", "query", "--json"],
    );
    assert_eq!(json(&out), serde_json::json!([]));
    assert!(by_env.exists());
    let out = dermixen(
        dir.path(),
        &[
            ("DERMIXEN_SETTINGS_FILE", &settings_file(dir.path())),
            ("DERMIXEN_LIBRARY_FILE", by_env.to_str().unwrap()),
        ],
        &[
            "library",
            "query",
            "--json",
            "--library",
            by_option.to_str().unwrap(),
        ],
    );
    assert_eq!(json(&out), serde_json::json!([]));
    assert!(by_option.exists());

    // A settings file that cannot be read stops a library command as it
    // stops `settings show`, since the command would not know which file
    // to open.
    std::fs::write(settings_file(dir.path()), "library_file = 7\n").unwrap();
    let said = fails(&dermixen(
        dir.path(),
        &[
            ("DERMIXEN_SETTINGS_FILE", &settings_file(dir.path())),
            ("DERMIXEN_LIBRARY_FILE", ""),
        ],
        &["library", "query", "--json"],
    ));
    assert!(
        said.contains("library_file") && said.contains("line 1"),
        "{said}"
    );
}
