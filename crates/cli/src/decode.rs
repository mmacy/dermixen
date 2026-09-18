//! The `decode` command, which writes the audio the library and the render
//! read from a file, or a span of it, as a WAV.
//!
//! Every analysis in the library and every render reads a file through
//! `dermixen_media::decode`, and so does this command, so a program that
//! reads the WAV this command writes reads the same samples the library
//! measured its beat grids against.

use std::path::Path;

use dermixen_core::Samples;
use dermixen_media::{WavDepth, WavFile, decode};
use serde::Serialize;

use crate::analyze::print_json;
use crate::paths::refuse_an_input;
use crate::render::{SpanRequest, writing_to};
use crate::show::length_text;
use crate::text::say;

/// The options `decode` takes, as the person typed them.
pub struct Args<'a> {
    /// The audio file to decode.
    pub file: &'a Path,
    /// The WAV file to write.
    pub out: &'a Path,
    /// Where in the file to start, as text, or `None` for the start.
    pub from: Option<&'a str>,
    /// How much to write, as text, or `None` for the rest of the file.
    pub length: Option<&'a str>,
    /// Whether to print one JSON document instead of text.
    pub json: bool,
}

/// What `decode --json` prints, matching the `decode` definition in
/// `docs/json/dermixen.schema.json`.
#[derive(Debug, Serialize)]
struct Written {
    /// The audio file decoded, as its canonical path.
    file: String,
    /// The WAV file written.
    path: String,
    /// Where in the file the WAV begins.
    from_seconds: f64,
    /// How many frames the WAV contains.
    length_samples: i64,
    /// The same length in seconds.
    length_seconds: f64,
}

/// Carries out `decode`.
///
/// `args.file` is decoded once with `dermixen_media::decode`, the call the
/// library and the render both make, so the frames written are the frames
/// either would have read. The span `--from` and `--for` ask for is resolved
/// against the length of that decode, in the words `render` reads them: a
/// value that is not a time and a length of zero are refused with the flag
/// named, a start at or past the end of the file is refused, and a length
/// that runs past the end is cut there.
///
/// The output is refused when it is the file being decoded, however the two
/// names are written, since a decode that replaced its own source would lose
/// the audio it was made from. The frames are then written to a temporary
/// file beside `args.out` whose name nobody can work out in advance, and
/// that file is moved into place only once the write has finished, so a
/// decode that fails partway leaves neither the output nor a temporary file
/// behind.
pub fn run(args: &Args<'_>) -> Result<(), String> {
    let span = SpanRequest::read(args.from, args.length)?;
    refuse_an_input(args.out, "the file being decoded", &[args.file])?;
    let decoded = decode(args.file).map_err(|problem| problem.to_string())?;
    let total = decoded.audio.len();
    let range = span.resolve_within(total, "file")?;
    let until = range.end.min(total);
    let start = range.start.0 as usize;
    let end = until.0 as usize;
    let frames = decoded.audio.frames[start..end].to_vec();
    let length = Samples((end - start) as i64);

    let canonical = args
        .file
        .canonicalize()
        .unwrap_or_else(|_| args.file.to_path_buf());

    let writing = writing_to(args.out)?;
    let file = writing
        .file()
        .try_clone()
        .map_err(|problem| format!("cannot write {}: {problem}", args.out.display()))?;
    let mut wav = WavFile::from_file(file, args.out, WavDepth::Int16)
        .map_err(|problem| problem.to_string())?;
    wav.write(&frames).map_err(|problem| problem.to_string())?;
    wav.finish().map_err(|problem| problem.to_string())?;
    writing
        .commit()
        .map_err(|problem| format!("cannot write {}: {problem}", args.out.display()))?;

    let from_seconds = range.start.to_seconds();
    let length_seconds = length.to_seconds();
    if args.json {
        print_json(&Written {
            file: canonical.display().to_string(),
            path: args.out.display().to_string(),
            from_seconds: from_seconds.0,
            length_samples: length.0,
            length_seconds: length_seconds.0,
        });
    } else {
        say!(
            "wrote {}: {} long, {} samples",
            args.out.display(),
            length_text(length_seconds),
            length.0
        );
    }
    Ok(())
}
