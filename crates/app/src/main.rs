#![forbid(unsafe_code)]

//! The Dermixen window.
//!
//! `dermixen-app [MIX]` opens one mix document in a window that shows the
//! playlist as the timeline, with the waveforms, the tempo curve, the
//! anchors, the volume and EQ curves, and the phrase starts and section
//! changes analysis found. With no mix named it opens on an untitled empty
//! mix. The window is a thin shell: it paints the view-models of the
//! `dermixen_app` library and hands them the mouse and the keyboard, and
//! everything it plays comes from the engine's transport.

mod paint;
mod window;

use std::path::PathBuf;
use std::process::ExitCode;

/// How the command is used, printed when it is used any other way.
const USAGE: &str = "usage: dermixen-app [MIX]";

/// Reads the one argument, if there is one, and opens the window.
///
/// The exit codes are the ones `docs/cli.md` gives for the `dermixen`
/// command: zero when the window opened and closed, one when the mix could
/// not be opened, and two when the command line could not be read, which is
/// more than one argument.
fn main() -> ExitCode {
    let mut arguments = std::env::args_os().skip(1);
    let first = arguments.next();
    if let Some(first) = &first
        && (first == "--help" || first == "-h")
    {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if arguments.next().is_some() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    let mix = first.map(PathBuf::from);
    match window::open(mix.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("error: {problem}");
            ExitCode::FAILURE
        }
    }
}
