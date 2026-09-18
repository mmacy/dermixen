//! The `mix relink` command, which finds a mix's files again after they
//! have moved.

use std::io::Write;
use std::path::{Path, PathBuf};

use dermixen_library::{Relink, relink};
use serde::Serialize;

use crate::analyze::{canonical, print_json};

/// What `mix relink --json` prints, which is the `mix_relink` document of
/// `docs/json/dermixen.schema.json`.
#[derive(Debug, Serialize)]
struct Report {
    /// The mix document.
    path: String,
    /// How many tracks now name a new path.
    relinked: usize,
    /// How many tracks still have no file.
    missing: usize,
    /// One entry per track, in playlist order.
    tracks: Vec<TrackLine>,
}

/// What became of one track's file.
#[derive(Debug, Serialize)]
struct TrackLine {
    /// The track's place in the playlist, counting from one.
    position: usize,
    /// The path the document names now.
    path: String,
    /// Whether the track was kept, relinked, or is missing.
    outcome: &'static str,
    /// The path the document named before, for a relinked track, and `None`
    /// otherwise.
    from: Option<String>,
    /// Where the file was looked for, for a missing track, and `None`
    /// otherwise.
    reason: Option<String>,
}

/// Replaces a mix document with new text in one step.
///
/// The text is written to a new file beside the document and then moved onto
/// it, so a write that fails partway leaves the document exactly as it was
/// rather than truncated. `mix add` rewrites a document the same way, and
/// `docs/cli.md` says under "mix relink" that `mix relink` rewrites one the
/// way `mix add` does.
fn replace(mix: &Path, text: &str) -> Result<(), String> {
    let mut temporary = mix.as_os_str().to_owned();
    temporary.push(".new");
    let temporary = PathBuf::from(temporary);
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, mix)
    };
    write().map_err(|problem| {
        let _ = std::fs::remove_file(&temporary);
        format!("cannot write {}: {problem}", mix.display())
    })
}

/// Carries out `mix relink`, as `docs/cli.md` describes it: every track of
/// the mix is checked against its file, the ones that are missing or changed
/// are looked for by hash in the library and then under the folders given,
/// the document is rewritten when any track was relinked, and one line per
/// track says what became of it.
///
/// The library crate writes a found path as the library's record names it or
/// as the scan listed it, and a folder given on the command line may be a
/// relative path, so this command makes every relinked path absolute with the
/// symbolic links resolved before it writes the document, and points the
/// library at that path too. The document is written once, after every track
/// has been dealt with, so a path that cannot be resolved and a library file
/// that cannot be written each leave the document as it was.
///
/// The settings file is read before the mix document, because the settings
/// name the library file when `--library` and the environment do not, and a
/// settings file that cannot be read stops every command that opens the
/// library.
///
/// The library file is opened only when it is already there. A person who
/// has never scanned their music, or who names a library file that has not
/// been built yet, gets a search under the folders alone, and the command
/// creates no library file and no folder above one. A library file that is
/// there and cannot be opened is an error.
pub fn run(
    mix: &Path,
    under: &[PathBuf],
    library: Option<&Path>,
    json: bool,
) -> Result<(), String> {
    let settings = crate::settings::read()?;
    let mut document = crate::document::read(mix)?;
    let location = crate::index::location(library, &settings)?;
    let mut index = match location.exists() {
        true => Some(crate::index::open(&location)?),
        false => None,
    };

    // Hashing a large library takes minutes, so the command says which file
    // it is reading rather than looking as though it has stopped.
    let mut progress = |path: &Path| eprintln!("hashing {}", path.display());
    let done = relink(&mut document, index.as_mut(), under, &mut progress)
        .map_err(|problem| problem.to_string())?;

    let mut lines = Vec::with_capacity(done.tracks.len());
    for (at, outcome) in done.tracks.iter().enumerate() {
        let position = at + 1;
        let track = &mut document.tracks[at];
        let line = match outcome {
            Relink::Kept => TrackLine {
                position,
                path: track.path.display().to_string(),
                outcome: "kept",
                from: None,
                reason: None,
            },
            Relink::Relinked { from, .. } => {
                let to = canonical(&track.path)?;
                if let Some(index) = index.as_mut() {
                    index
                        .set_path(track.hash, &to)
                        .map_err(|problem| problem.to_string())?;
                }
                track.path = to;
                TrackLine {
                    position,
                    path: track.path.display().to_string(),
                    outcome: "relinked",
                    from: Some(from.display().to_string()),
                    reason: None,
                }
            }
            Relink::Missing { path, reason } => TrackLine {
                position,
                path: path.display().to_string(),
                outcome: "missing",
                from: None,
                reason: Some(reason.clone()),
            },
        };
        lines.push(line);
    }

    let report = Report {
        path: mix.display().to_string(),
        relinked: done.relinked(),
        missing: done.missing(),
        tracks: lines,
    };
    if report.relinked > 0 {
        replace(mix, &document.to_json())?;
    }

    if json {
        print_json(&report);
    } else {
        for line in &report.tracks {
            let tail = match (&line.from, &line.reason) {
                (Some(from), _) => format!("  was {from}"),
                (_, Some(reason)) => format!("  {reason}"),
                _ => String::new(),
            };
            println!(
                "{:>3}. {:<9}{}{tail}",
                line.position, line.outcome, line.path
            );
        }
    }
    Ok(())
}
