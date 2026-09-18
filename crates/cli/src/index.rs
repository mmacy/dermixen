//! Choosing and opening the library file.

use std::path::{Path, PathBuf};

use dermixen_core::Settings;
use dermixen_library::Index;

/// The environment variable that names the library file when `--library`
/// does not. [`location`] reads it, and the message
/// [`crate::settings::data_folder`] gives names it as one way to say where
/// the library file is.
pub const VARIABLE: &str = "DERMIXEN_LIBRARY_FILE";

/// Which library file a command uses: the `--library` option, else the
/// `DERMIXEN_LIBRARY_FILE` environment variable, else the `library_file`
/// setting, else `dermixen/library.sqlite` in the user's data folder.
///
/// `settings` is what [`crate::settings::read`] gave the command, which
/// every command that opens the library calls before it does anything else,
/// so a settings file that cannot be read stops the command whichever of the
/// four names the library file. Pass the answer to [`open`], or to
/// [`open_existing`] for a command that writes nothing.
///
/// The answer is an absolute path, so that the file a command reports is the
/// same file whatever folder the command was run from. A relative
/// `--library` option and a relative value of the environment variable are
/// both resolved against the folder the command was run from, while a
/// relative `library_file` setting is resolved against the user's home
/// folder, as `docs/settings.md` states. The home folder is looked for only
/// when the setting is a relative path, so a user who has no home folder
/// still gets the default library file and a setting written as a path in
/// full.
pub fn location(option: Option<&Path>, settings: &Settings) -> Result<PathBuf, String> {
    if let Some(path) = option {
        return absolute(path);
    }
    if let Some(value) = std::env::var_os(VARIABLE)
        && !value.is_empty()
    {
        return absolute(Path::new(&value));
    }
    let data = crate::settings::data_folder()?;
    // Only a setting that is a relative path is worked out below the home
    // folder, so a user with no home folder still gets the default file and
    // a setting written as a path in full, as the window does.
    let home = match settings.library_file.as_deref() {
        Some(setting) if setting.is_relative() && !setting.as_os_str().is_empty() => {
            crate::settings::home_folder()?
        }
        _ => PathBuf::new(),
    };
    Ok(settings.library_file(&home, &data))
}

/// A path as an absolute one, resolved against the folder the command was
/// run from. The file itself need not exist yet.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let here = std::env::current_dir()
        .map_err(|problem| format!("cannot read the current folder: {problem}"))?;
    Ok(here.join(path))
}

/// Opens the library file at `path`, creating the file and the folders above
/// it when they are not there yet, so that a first run needs no setup.
///
/// A folder this command creates is readable, writable, and enterable by its
/// owner alone, because the library names every audio file a person owns and
/// nobody else on the machine has business reading that list. A folder that
/// is already there keeps the permissions it has, which is the case of a
/// library file in a folder the person chose. The library crate creates the
/// file itself with the same reach, as `docs/library.md` states.
pub fn open(path: &Path) -> Result<Index, String> {
    if let Some(folder) = path.parent()
        && !folder.as_os_str().is_empty()
        && !folder.exists()
    {
        make_private_folder(folder)
            .map_err(|problem| format!("cannot make the folder {}: {problem}", folder.display()))?;
    }
    Index::open(path).map_err(|problem| problem.to_string())
}

/// Creates `folder` and every folder above it that is missing, each of them
/// for its owner alone.
#[cfg(unix)]
fn make_private_folder(folder: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
}

/// Creates `folder` and every folder above it that is missing, on a system
/// with no permission bits to ask for.
#[cfg(not(unix))]
fn make_private_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)
}

/// Opens the library file at `path` for a command that only reads it,
/// refusing a path that names no file.
///
/// [`open`] creates a library file and the folders above it, which is what a
/// command that stores records needs. A command that writes nothing, like
/// `mix plan`, would leave an empty library file behind on a mistyped path,
/// so such a command opens the library here and the person is given the path
/// named and the command that builds a library.
pub fn open_existing(path: &Path) -> Result<Index, String> {
    if !path.exists() {
        return Err(format!(
            "there is no library file at {}. dermixen library scan FOLDER creates a library and adds the tracks under FOLDER to it",
            path.display()
        ));
    }
    open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_option_names_the_index_and_is_made_absolute() {
        let chosen = location(Some(Path::new("here.sqlite")), &Settings::default()).unwrap();
        assert!(chosen.is_absolute(), "{}", chosen.display());
        assert!(chosen.ends_with("here.sqlite"), "{}", chosen.display());
    }

    #[test]
    fn an_absolute_option_is_left_as_it_is() {
        let given = Path::new("/tmp/dermixen-test/library.sqlite");
        assert_eq!(location(Some(given), &Settings::default()).unwrap(), given);
    }

    #[test]
    fn a_setting_written_as_a_path_in_full_is_the_library_file() {
        let given = PathBuf::from("/tmp/dermixen-test/by-setting.sqlite");
        let settings = Settings {
            library_file: Some(given.clone()),
            ..Settings::default()
        };
        // The environment variable comes before the setting, so a machine
        // that has it set would answer with the variable's file instead.
        if std::env::var_os(VARIABLE).is_some_and(|value| !value.is_empty()) {
            return;
        }
        assert_eq!(location(None, &settings).unwrap(), given);
    }

    #[test]
    fn the_option_comes_before_the_setting() {
        let settings = Settings {
            library_file: Some(PathBuf::from("/tmp/dermixen-test/by-setting.sqlite")),
            ..Settings::default()
        };
        let given = Path::new("/tmp/dermixen-test/by-option.sqlite");
        assert_eq!(location(Some(given), &settings).unwrap(), given);
    }
}
