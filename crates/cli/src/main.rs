#![forbid(unsafe_code)]

//! The `dermixen` command-line interface.
//!
//! `docs/cli.md` describes every command, its flags, its output, and its
//! exit codes, and `docs/json/dermixen.schema.json` describes every JSON
//! object a command prints; the definitions here are those documents as code.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{ArgGroup, Args, Parser, Subcommand};

mod analyze;
mod analyzers;
mod decode;
mod document;
mod index;
mod library;
mod open;
mod paths;
mod plan;
mod play;
mod relink;
mod render;
mod scoreboard;
mod settings;
mod show;
mod text;

use analyzers::Given;
use text::note;

/// Dermixen authors continuous DJ mixes from analyzed tracks.
#[derive(Parser, Debug)]
#[command(name = "dermixen", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The library file to use: the `--library` option, else the
/// `DERMIXEN_LIBRARY_FILE` environment variable, else the default location.
#[derive(Args, Debug, Clone)]
struct LibraryLocation {
    /// The library file. Defaults to $DERMIXEN_LIBRARY_FILE, then to the library_file setting, then to dermixen/library.sqlite in the user's data folder.
    #[arg(long, value_name = "PATH", global = true)]
    library: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Decode an audio file, or a span of it, to a 16-bit WAV at 44.1 kHz, as the library and the render read it.
    Decode {
        /// The audio file.
        file: PathBuf,
        /// The WAV file to write.
        out: PathBuf,
        /// Where to start, counted from the start of the file, as minutes and seconds such as 7:30 or as seconds. The start of the file if omitted.
        #[arg(long, value_name = "TIME")]
        from: Option<String>,
        /// How much to write, as minutes and seconds or as seconds. To the end of the file if omitted.
        #[arg(long = "for", value_name = "LENGTH")]
        length: Option<String>,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Analyze one audio file and report everything the library would store about it.
    Analyze {
        /// The audio file.
        file: PathBuf,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
        /// Use this tempo instead of analyzing.
        #[arg(long)]
        bpm: Option<f64>,
        /// The time of the first beat in seconds, used with --bpm. Zero if omitted.
        #[arg(long, value_name = "SECONDS", requires = "bpm")]
        first_beat: Option<f64>,
    },
    /// Scan your music into the library and query it.
    Library {
        #[command(subcommand)]
        command: LibraryCommand,
        #[command(flatten)]
        library: LibraryLocation,
    },
    /// Create, extend, and inspect mix documents.
    Mix {
        #[command(subcommand)]
        command: MixCommand,
    },
    /// Show and change the settings the window and this command share. The file is $DERMIXEN_SETTINGS_FILE, then dermixen/settings.toml in the user's configuration folder.
    Settings {
        #[command(subcommand)]
        command: SettingsCommand,
    },
    /// Render a mix document, or a span of it, to a 16-bit WAV file, or an MP3 when the name ends in .mp3.
    Render {
        /// The mix document.
        mix: PathBuf,
        /// The file to write: a 16-bit WAV file, or an MP3 when the name ends in .mp3.
        out: PathBuf,
        /// Where to start, counted from the start of the mix, as minutes and seconds such as 7:30 or as seconds. The start of the mix if omitted.
        #[arg(long, value_name = "TIME")]
        from: Option<String>,
        /// How much to render, as minutes and seconds or as seconds. To the end of the mix if omitted.
        #[arg(long = "for", value_name = "LENGTH")]
        length: Option<String>,
        /// Render the clip around the handover into this track, counting from one, instead of a span given by hand.
        #[arg(long, value_name = "N", conflicts_with_all = ["from", "length"])]
        handover: Option<usize>,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Play a mix document, or a span of it, through the default audio device.
    Play {
        /// The mix document.
        mix: PathBuf,
        /// Where to start, counted from the start of the mix, as minutes and seconds such as 7:30 or as seconds. The start of the mix if omitted.
        #[arg(long, value_name = "TIME")]
        from: Option<String>,
        /// How much to play, as minutes and seconds or as seconds. To the end of the mix if omitted.
        #[arg(long = "for", value_name = "LENGTH")]
        length: Option<String>,
        /// Write the frames the device would have played to this WAV file instead of playing them.
        #[arg(long, value_name = "WAV")]
        capture: Option<PathBuf>,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Open a mix document in the window.
    Open {
        /// The mix document.
        mix: PathBuf,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Measure every built-in analyzer against a ground-truth directory and print the scoreboard.
    Scoreboard {
        /// The ground-truth directory.
        dir: PathBuf,
        /// Read the directory as a GiantSteps dataset checkout.
        #[arg(long)]
        giantsteps: bool,
        /// Print one JSON object instead of the tables.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum LibraryCommand {
    /// Find every audio file under a folder, analyze the new ones, and store them in the library.
    Scan {
        /// The folder to scan. The music_folder setting if omitted.
        root: Option<PathBuf>,
        /// A folder not to enter, relative to the root or absolute. May be repeated.
        #[arg(long, value_name = "DIR")]
        exclude: Vec<PathBuf>,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// List the tracks in the library that meet every condition given.
    Query {
        /// Only tracks in this folder or one below it.
        #[arg(long, value_name = "DIR")]
        under: Option<PathBuf>,
        /// Only tracks whose tempo is in this range, written as LOW-HIGH or as one number.
        #[arg(long, value_name = "RANGE")]
        bpm: Option<String>,
        /// Only tracks whose year is in this range, written as LOW-HIGH or as one year.
        #[arg(long, value_name = "RANGE")]
        year: Option<String>,
        /// Only tracks whose length is in this range, written as LOW-HIGH with each end as minutes and seconds, such as 7:00-9:00, or as seconds.
        #[arg(long, value_name = "RANGE")]
        length: Option<String>,
        /// Only tracks in this Camelot key, such as 8A.
        #[arg(long, value_name = "CODE")]
        key: Option<String>,
        /// Only tracks whose key mixes well with this Camelot key.
        #[arg(long, value_name = "CODE")]
        compatible_with: Option<String>,
        /// Only tracks whose artist contains this text.
        #[arg(long, value_name = "TEXT")]
        artist: Option<String>,
        /// Only tracks whose title contains this text.
        #[arg(long, value_name = "TEXT")]
        title: Option<String>,
        /// Leave out every track whose year is an estimate rather than one a source states.
        #[arg(long)]
        no_approximate_years: bool,
        /// Only tracks whose grid confidence is at least this value, from 0 to 1.
        #[arg(long, value_name = "CONFIDENCE", allow_negative_numbers = true)]
        min_grid_confidence: Option<String>,
        /// Only tracks whose anchor confidence is at least this value, from 0 to 1.
        #[arg(long, value_name = "CONFIDENCE", allow_negative_numbers = true)]
        min_anchor_confidence: Option<String>,
        /// Print a JSON list instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Rank the tracks in the library that match some text, such as a line of a tracklisting.
    Find {
        /// The text to match.
        text: String,
        /// How many matches to print at most.
        #[arg(long, default_value_t = 5)]
        limit: usize,
        /// Print a JSON list instead of text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SettingsCommand {
    /// Print every setting, its value, and whether the file sets it.
    Show {
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Set one setting and write the file. The settings are audio_buffer_frames (a whole number of frames), metronome (true or false), grid_strip_collapsed (true or false), library_collapsed (true or false), library_word_wrap (true or false), music_folder (a path in quotes), and library_file (a path in quotes).
    Set {
        /// The setting's name.
        name: String,
        /// The value to set.
        #[arg(allow_hyphen_values = true)]
        value: String,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Remove one setting from the file, so that it has its default again.
    Reset {
        /// The setting's name.
        name: String,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum MixCommand {
    /// Write an empty mix document.
    New {
        /// The mix document to create.
        mix: PathBuf,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Add a track to a mix document, joined to its neighbors by a transition preset.
    Add {
        /// The mix document.
        mix: PathBuf,
        /// The audio file.
        file: PathBuf,
        /// Where in the playlist to put the track, counting from one. The end if omitted.
        #[arg(long, value_name = "N")]
        position: Option<usize>,
        /// The transition preset: blend, beatmix, bass-swap, or cut.
        #[arg(long, default_value = document::DEFAULT_PRESET)]
        preset: String,
        /// The length of the transition in bars, for beatmix and bass-swap. It moves the analyzed outro anchor for every preset.
        #[arg(long, default_value_t = dermixen_core::DEFAULT_BARS)]
        bars: u32,
        /// The intro anchor, as a whole beat of the track. From analysis if omitted.
        #[arg(long, value_name = "BEAT")]
        intro: Option<f64>,
        /// The outro anchor, as a whole beat of the track. From analysis if omitted.
        #[arg(long, value_name = "BEAT")]
        outro: Option<f64>,
        /// Use this tempo instead of the analyzed grid.
        #[arg(long)]
        bpm: Option<f64>,
        /// The time of the first beat in seconds, used with --bpm. Zero if omitted.
        #[arg(long, value_name = "SECONDS", requires = "bpm")]
        first_beat: Option<f64>,
        /// Let the pitch move with the speed, as a record does, instead of keeping it.
        #[arg(long)]
        no_keylock: bool,
        /// The gain to write for the track, in decibels, instead of the one volume leveling gives it.
        #[arg(long, value_name = "DB", allow_hyphen_values = true)]
        gain: Option<f64>,
        /// Print the mix's timeline as one JSON object instead of text.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        library: LibraryLocation,
    },
    /// Predict the timeline mix add would build from a playlist, without writing a mix document.
    Plan {
        /// A file of audio paths, one per line, in playlist order.
        playlist: PathBuf,
        /// The most the tempo may move at one transition, in beats per minute.
        #[arg(
            long,
            value_name = "BPM",
            default_value_t = 1.0,
            allow_negative_numbers = true
        )]
        max_step: f64,
        /// Let one artist appear on more than one track without a warning.
        #[arg(long)]
        allow_repeats: bool,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        library: LibraryLocation,
    },
    /// Show the tracks of a mix document on the timeline.
    Show {
        /// The mix document.
        mix: PathBuf,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Move an anchor of a track already in a mix document, taking the nodes of its transition along.
    #[command(group = ArgGroup::new("anchor").required(true).multiple(true).args(["intro", "outro"]))]
    MoveAnchor {
        /// The mix document.
        mix: PathBuf,
        /// The track's position in the playlist, counting from one.
        #[arg(value_name = "N")]
        track: usize,
        /// The whole beat of the track to put its intro anchor on.
        #[arg(long, value_name = "BEAT")]
        intro: Option<f64>,
        /// The whole beat of the track to put its outro anchor on.
        #[arg(long, value_name = "BEAT")]
        outro: Option<f64>,
        /// Print the mix's timeline as one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Set the gain of a track already in a mix document.
    SetGain {
        /// The mix document.
        mix: PathBuf,
        /// The track's position in the playlist, counting from one.
        #[arg(value_name = "N")]
        track: usize,
        /// The gain in decibels, which may be negative.
        #[arg(value_name = "DB", allow_negative_numbers = true)]
        db: f64,
        /// Print the mix's timeline as one JSON object instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Point the tracks of a mix document at their files again after the files have moved.
    Relink {
        /// The mix document.
        mix: PathBuf,
        /// A folder to search for the files by their bytes. May be given more than once.
        #[arg(long, value_name = "DIR")]
        under: Vec<PathBuf>,
        /// Print one JSON object instead of text.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        library: LibraryLocation,
    },
}

/// The exit code a defect in dermixen itself ends the command with, which
/// `docs/cli.md` lists beside the others.
const DEFECT: u8 = 3;

/// Runs one command and reports what happened as an exit code.
///
/// A defect in dermixen itself is an exit code of its own rather than a
/// backtrace and whatever code the runtime chooses. The hook prints one line
/// beginning `error:` that says the command met a defect and names the file
/// and line it happened at, the unwind is caught here, and the command ends
/// with [`DEFECT`]. A person reading the terminal, or an agent reading the
/// exit code, can tell a defect from input dermixen refused, which ends with
/// code 1.
fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|panic| {
        let place = match panic.location() {
            Some(location) => format!("{}, line {}", location.file(), location.line()),
            None => "a place it cannot name".to_owned(),
        };
        note!(
            "error: dermixen met a defect in itself at {place} and stopped: {}. Nothing this command had not already written has been written. Please report this.",
            defect_message(panic)
        );
    }));
    let cli = Cli::parse();
    match std::panic::catch_unwind(|| run(cli.command)) {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(message)) => {
            note!("error: {message}");
            ExitCode::from(1)
        }
        // The hook above has already printed the line, so this only decides
        // the exit code.
        Err(_) => ExitCode::from(DEFECT),
    }
}

/// What a panic said, for the one line the hook prints.
fn defect_message(panic: &std::panic::PanicHookInfo<'_>) -> String {
    let said = panic.payload();
    if let Some(text) = said.downcast_ref::<&str>() {
        return (*text).to_owned();
    }
    if let Some(text) = said.downcast_ref::<String>() {
        return text.clone();
    }
    "no message".to_owned()
}

/// Carries out one command, returning the message to print on failure.
fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Analyze {
            file,
            json,
            bpm,
            first_beat,
        } => analyze::run(&file, json, &Given { bpm, first_beat }),
        Command::Decode {
            file,
            out,
            from,
            length,
            json,
        } => decode::run(&decode::Args {
            file: &file,
            out: &out,
            from: from.as_deref(),
            length: length.as_deref(),
            json,
        }),
        Command::Library { command, library } => match command {
            LibraryCommand::Scan {
                root,
                exclude,
                json,
            } => library::scan(root.as_deref(), &exclude, library.library.as_deref(), json),
            LibraryCommand::Query {
                under,
                bpm,
                year,
                length,
                key,
                compatible_with,
                artist,
                title,
                no_approximate_years,
                min_grid_confidence,
                min_anchor_confidence,
                json,
            } => library::query(
                &library::QueryArgs {
                    under,
                    bpm,
                    year,
                    length,
                    key,
                    compatible_with,
                    artist,
                    title,
                    no_approximate_years,
                    min_grid_confidence,
                    min_anchor_confidence,
                },
                library.library.as_deref(),
                json,
            ),
            LibraryCommand::Find { text, limit, json } => {
                library::find(&text, limit, library.library.as_deref(), json)
            }
        },
        Command::Mix { command } => match command {
            MixCommand::New { mix, json } => document::new(&mix, json),
            MixCommand::Add {
                mix,
                file,
                position,
                preset,
                bars,
                intro,
                outro,
                bpm,
                first_beat,
                no_keylock,
                gain,
                json,
                library,
            } => document::add(&document::AddArgs {
                mix: &mix,
                file: &file,
                position,
                preset: &preset,
                bars,
                intro,
                outro,
                given: Given { bpm, first_beat },
                keylock: !no_keylock,
                gain,
                json,
                library: library.library.as_deref(),
            }),
            MixCommand::Plan {
                playlist,
                max_step,
                allow_repeats,
                json,
                library,
            } => plan::run(&plan::PlanArgs {
                playlist: &playlist,
                max_step,
                allow_repeats,
                library: library.library.as_deref(),
                json,
            }),
            MixCommand::Show { mix, json } => show::run(&mix, json),
            MixCommand::MoveAnchor {
                mix,
                track,
                intro,
                outro,
                json,
            } => document::move_anchor(&mix, track, intro, outro, json),
            MixCommand::SetGain {
                mix,
                track,
                db,
                json,
            } => document::set_gain(&mix, track, db, json),
            MixCommand::Relink {
                mix,
                under,
                json,
                library,
            } => relink::run(&mix, &under, library.library.as_deref(), json),
        },
        Command::Render {
            mix,
            out,
            from,
            length,
            handover,
            json,
        } => render::run(&render::Args {
            mix: &mix,
            out: &out,
            from: from.as_deref(),
            length: length.as_deref(),
            handover,
            json,
        }),
        Command::Settings { command } => match command {
            SettingsCommand::Show { json } => settings::show(json),
            SettingsCommand::Set { name, value, json } => settings::set(&name, &value, json),
            SettingsCommand::Reset { name, json } => settings::reset(&name, json),
        },
        Command::Play {
            mix,
            from,
            length,
            capture,
            json,
        } => play::run(
            &mix,
            from.as_deref(),
            length.as_deref(),
            capture.as_deref(),
            json,
        ),
        Command::Open { mix, json } => open::run(&mix, json),
        Command::Scoreboard {
            dir,
            giantsteps,
            json,
        } => scoreboard::run(&dir, giantsteps, json),
    }
}
