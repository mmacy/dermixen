//! The `open` command, which hands a mix built on the command line to the
//! window: `DESIGN.md` names it under "The CLI is a product surface" as the
//! handoff from an agent's work to a person's.
//!
//! The command reads the mix first, so a file that is missing or is not a
//! mix document is refused with the reader's message before anything is
//! launched. It then finds the window's executable and starts it on the
//! mix, detached: the window runs on after the command has returned, with
//! no terminal of its own, and the command neither waits for it nor
//! reports what becomes of it.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Serialize;

use crate::analyze::{canonical, print_json};

/// The environment variable that names the window's executable, which
/// takes precedence over the one beside this command's own executable. An
/// empty value is treated the same as the variable being unset.
pub const APP_VARIABLE: &str = "DERMIXEN_APP";

/// The file name of the window's executable, looked for beside this
/// command's own executable when [`APP_VARIABLE`] is not set.
pub const APP_EXECUTABLE: &str = "dermixen-app";

/// Where the window's executable is, before it is checked to exist.
///
/// [`APP_VARIABLE`] names it directly when it is set to a non-empty value;
/// otherwise it is [`APP_EXECUTABLE`] in the folder holding this command's
/// own executable.
fn candidate_app() -> Result<PathBuf, String> {
    if let Ok(named) = std::env::var(APP_VARIABLE)
        && !named.is_empty()
    {
        return Ok(PathBuf::from(named));
    }
    let running = std::env::current_exe()
        .map_err(|problem| format!("cannot find this command's own executable: {problem}"))?;
    let folder = running
        .parent()
        .ok_or_else(|| format!("cannot find the folder containing {}", running.display()))?;
    Ok(folder.join(APP_EXECUTABLE))
}

/// Finds the window's executable, refusing one that is not there and
/// naming [`APP_VARIABLE`] as the way to point at another one.
fn locate_app() -> Result<PathBuf, String> {
    let app = candidate_app()?;
    if !app.exists() {
        return Err(format!(
            "cannot find the window's executable at {}; set {APP_VARIABLE} to point at another one",
            app.display()
        ));
    }
    Ok(app)
}

/// The `open` JSON document: the mix, the executable, and the process ID of
/// the window that was started on the mix.
#[derive(Debug, Serialize)]
struct Opened {
    /// The mix document, as an absolute path with symbolic links resolved.
    mix: String,
    /// The window's executable that was started.
    app: String,
    /// The process ID of the window.
    pid: u32,
}

/// Runs `open`.
///
/// The mix is read as `mix show` reads it, and a mix that cannot be read
/// is the failure, with the reader's message. The window's executable is
/// the path in [`APP_VARIABLE`] when that variable is set to a non-empty
/// value, or otherwise [`APP_EXECUTABLE`] in the folder holding the running
/// executable; a path that does not exist is a failure naming the path
/// looked at and the variable that can point elsewhere, one that exists but
/// cannot be started is a failure naming the path and giving the operating
/// system's reason, and a system that cannot say where the running
/// executable is is a failure with that reason. The executable is started
/// with one argument, the mix's path made absolute with every symbolic link
/// resolved, as `std::fs::canonicalize` gives it, in a process group of its
/// own so that it keeps running after the terminal that started it closes,
/// with its standard input, output, and error connected to the null
/// device, and is not waited for. On success the command prints one line
/// naming the mix, the executable, and the process ID, or with `json` the
/// `open` document of `docs/json/dermixen.schema.json`.
pub fn run(mix: &Path, json: bool) -> Result<(), String> {
    crate::document::read(mix)?;
    let app = locate_app()?;
    let absolute = canonical(mix)?;

    let child = Command::new(&app)
        .arg(&absolute)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|problem| format!("cannot start {}: {problem}", app.display()))?;
    let pid = child.id();

    if json {
        print_json(&Opened {
            mix: absolute.display().to_string(),
            app: app.display().to_string(),
            pid,
        });
    } else {
        println!(
            "opened {} in {} (pid {pid})",
            absolute.display(),
            app.display()
        );
    }
    Ok(())
}
