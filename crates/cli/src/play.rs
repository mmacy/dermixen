//! The `play` command, which plays a mix document, or a span of one, through
//! the default audio device by the same path `render` writes a file.

use std::num::NonZeroU32;
use std::path::Path;
use std::thread::JoinHandle;
use std::time::Duration;

use dermixen_core::Seconds;
use dermixen_core::files::AtomicFile;
use dermixen_engine::{Feed, Output, Progress, play};
use dermixen_media::{Frame, WavDepth, WavFile};
use serde::Serialize;

use crate::analyze::print_json;
use crate::render::{
    SpanRequest, check_hashes, decoder, mix_length, refuse_a_mix_too_long_for_a_wav,
    refuse_the_mix_and_its_tracks, span_frames, stretchers, writing_to,
};
use crate::show::length_text;
use crate::text::{note, say};

/// How much of the mix the engine keeps rendered ahead of the device.
///
/// Two seconds is long enough that an ordinary machine never runs the device
/// dry while the render decodes the next track. Filling it before sound
/// starts costs a small fraction of that on a machine that renders well
/// above real time; the wait a person notices is the decoding and warm-up
/// of the tracks sounding at the start of the span, which comes first.
const LOOKAHEAD: Seconds = Seconds(2.0);

/// How long the capturing output waits before looking again when the render
/// has delivered nothing yet.
///
/// A capture runs as fast as the render can go, so the feed is nearly always
/// holding frames and this wait is rarely reached; when it is, a fifth of a
/// millisecond costs nothing and keeps the thread off the processor.
const PAUSE: Duration = Duration::from_micros(200);

/// What `play --json` prints.
#[derive(Debug, Serialize)]
struct Played {
    /// The mix document played.
    mix: String,
    /// The WAV file the frames were written to, or null when a device played them.
    capture: Option<String>,
    /// Where playing began, counted from the start of the mix.
    from_seconds: f64,
    /// How many frames were played.
    length_samples: i64,
    /// The same length in seconds.
    length_seconds: f64,
    /// The length of the whole mix, as `mix show` reports it.
    mix_length_seconds: f64,
    /// How many times the device needed frames that were not ready.
    underruns: u64,
}

/// An output that writes the frames a device would have played to a WAV file.
///
/// It pulls from the feed on a thread of its own, exactly as a device does,
/// so `--capture` runs the same preview path a device runs rather than a
/// quieter shortcut around it. The thread takes only the frames that are
/// ready, so it never asks for frames the render has not delivered and a
/// capture therefore never counts an underrun. When the file cannot be
/// written, this output gives up the way a device that stops taking frames
/// does, so the command reports why the capture failed rather than passing
/// the failure over.
struct CaptureOutput {
    /// The file to write, until the pulling thread takes it.
    file: Option<WavFile>,
    /// The pulling thread, which hands the file back when it stops, or says
    /// what went wrong writing it.
    thread: Option<JoinHandle<Result<WavFile, String>>>,
    /// What the pulling thread said, once it has stopped.
    outcome: Option<Result<WavFile, String>>,
}

impl CaptureOutput {
    /// An output that writes to a file already opened.
    fn new(file: WavFile) -> CaptureOutput {
        CaptureOutput {
            file: Some(file),
            thread: None,
            outcome: None,
        }
    }

    /// Completes the captured file and moves `writing` onto `out`, so a
    /// capture that failed partway leaves nothing where the person asked for
    /// the file.
    fn finish(self, writing: AtomicFile, out: &Path) -> Result<(), String> {
        let file = match (self.outcome, self.file) {
            // The pulling thread ran and handed the file back.
            (Some(Ok(file)), _) => file,
            // It ran and could not write, so there is nothing to keep.
            (Some(Err(problem)), _) => return Err(problem),
            // The span held no frames, so the thread was never started and
            // the file is still here, empty.
            (None, Some(file)) => file,
            (None, None) => {
                return Err("the capture was started and never stopped".to_owned());
            }
        };
        file.finish().map_err(|problem| problem.to_string())?;
        writing
            .commit()
            .map_err(|problem| format!("cannot write {}: {problem}", out.display()))
    }
}

/// Takes the frames that are ready from the feed, waiting for the render
/// when none are yet, and returns how many were put into `block`, which is
/// grown to hold them. A count of none means the feed has finished and no
/// more frames are coming.
fn take(feed: &mut Feed, block: &mut Vec<Frame>) -> usize {
    loop {
        if feed.finished() {
            return 0;
        }
        let ready = feed.available();
        if ready > 0 {
            if block.len() < ready {
                block.resize(ready, [0.0, 0.0]);
            }
            return feed.pull(&mut block[..ready]);
        }
        std::thread::sleep(PAUSE);
    }
}

impl Output for CaptureOutput {
    fn start(&mut self, mut feed: Feed) -> Result<(), String> {
        let Some(mut file) = self.file.take() else {
            return Err("the capture file has already been started".to_owned());
        };
        self.thread = Some(std::thread::spawn(move || {
            let mut block = Vec::new();
            let mut trouble = None;
            loop {
                let got = take(&mut feed, &mut block);
                if got == 0 {
                    break;
                }
                if let Err(problem) = file.write(&block[..got]) {
                    let message = problem.to_string();
                    // A file that cannot be written is this output giving up,
                    // exactly as a device that stops taking frames is. Telling
                    // the feed so ends the preview with the message a person
                    // needs, rather than leaving the render waiting for room
                    // in a feed nobody is draining.
                    feed.fail(&message);
                    trouble = Some(message);
                    break;
                }
            }
            match trouble {
                Some(problem) => Err(problem),
                None => Ok(file),
            }
        }));
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.outcome = Some(match thread.join() {
                Ok(outcome) => outcome,
                Err(_) => Err("the capture stopped unexpectedly".to_owned()),
            });
        }
    }
}

/// Where a preview's frames go: a WAV file, or the machine's audio device.
enum Destination {
    /// A file, written by a thread that pulls the feed as a device does.
    /// The output sits behind a box because it holds the whole open WAV
    /// file, which is far larger than the handle a device needs.
    File(Box<CaptureOutput>),
    /// The machine's default audio output device.
    #[cfg(feature = "playback")]
    Device(dermixen_engine::CpalOutput),
}

impl Output for Destination {
    fn start(&mut self, feed: Feed) -> Result<(), String> {
        match self {
            Destination::File(capture) => capture.start(feed),
            #[cfg(feature = "playback")]
            Destination::Device(device) => device.start(feed),
        }
    }

    fn stop(&mut self) {
        match self {
            Destination::File(capture) => capture.stop(),
            #[cfg(feature = "playback")]
            Destination::Device(device) => device.stop(),
        }
    }
}

/// Opens the machine's default audio output device, which takes
/// `buffer_frames` frames each time it pulls, or a size of its own when the
/// settings file sets no size.
#[cfg(feature = "playback")]
fn device(buffer_frames: Option<NonZeroU32>) -> Result<Destination, String> {
    Ok(Destination::Device(dermixen_engine::CpalOutput::open(
        buffer_frames,
    )?))
}

/// Refuses to play through a device, in a build made without the `playback`
/// feature and so without any way to reach one.
#[cfg(not(feature = "playback"))]
fn device(_buffer_frames: Option<NonZeroU32>) -> Result<Destination, String> {
    Err(
        "this dermixen was built without the playback feature, so it cannot open an audio \
         device; give --capture WAV to write the frames it would have played to a file"
            .to_owned(),
    )
}

/// How the success line names the underruns there were.
fn underrun_text(underruns: u64) -> String {
    match underruns {
        0 => "no underruns".to_owned(),
        1 => "1 underrun".to_owned(),
        many => format!("{many} underruns"),
    }
}

/// Carries out the `play` command as `docs/cli.md` describes it.
///
/// A `capture` file that is the mix document or one of its tracks is
/// refused, and so is a capture of a span too long for a WAV file, both
/// before anything is created and in the words `render` refuses them. Every
/// track's file is then hashed, as `render` does. The span `from`
/// and `length` select is then played through the default audio device,
/// or, with `capture`, written to that WAV file instead, and one line on
/// standard output, or the `play` JSON document, reports what was played
/// and how many underruns there were.
pub fn run(
    mix: &Path,
    from: Option<&str>,
    length: Option<&str>,
    capture: Option<&Path>,
    json: bool,
) -> Result<(), String> {
    // The settings file is read before anything else, so a file that cannot
    // be read stops the command before it hashes a track, writes a capture
    // file, or opens a device.
    let settings = crate::settings::read()?;
    let span = SpanRequest::read(from, length)?;
    let document = crate::document::read(mix)?;
    if let Some(out) = capture {
        refuse_the_mix_and_its_tracks(out, mix, &document)?;
    }
    let total = mix_length(&document);
    let span = span.resolve(total)?;
    if let Some(out) = capture {
        refuse_a_mix_too_long_for_a_wav(out, span_frames(&span, total))?;
    }
    // The span is checked and the files hashed before any device is opened,
    // so a mistaken span costs nothing but the message that says so.
    check_hashes(&document.tracks, mix)?;

    let writing = capture.map(writing_to).transpose()?;
    let mut destination = match &writing {
        Some(writing) => {
            let out = capture.expect("a capture file is open only when one was asked for");
            let file = writing
                .file()
                .try_clone()
                .map_err(|problem| format!("cannot write {}: {problem}", out.display()))?;
            let file = WavFile::from_file(file, out, WavDepth::Int16)
                .map_err(|problem| problem.to_string())?;
            Destination::File(Box::new(CaptureOutput::new(file)))
        }
        None => device(settings.audio_buffer_frames)?,
    };

    let mut load = decoder(document.tracks.len());
    let mut stretch = stretchers();
    // A preview of a long span runs for as long as it takes to hear, so it
    // says where the sound has reached rather than looking as though it has
    // stopped. One line per ten seconds heard matches what `render` reports
    // per ten seconds written.
    let mut next_report = span.start;
    let mut progress = |progress: Progress| {
        if progress.written >= next_report {
            note!(
                "played {} of {}",
                length_text(progress.written.to_seconds()),
                length_text(progress.total.to_seconds())
            );
            next_report = progress.written + Seconds(10.0).to_samples();
        }
    };

    let from_samples = span.start;
    let played = play(
        &document,
        span,
        &mut load,
        &mut stretch,
        &mut destination,
        LOOKAHEAD.to_samples(),
        &mut progress,
    );
    // The temporary capture file is removed when `writing` is dropped, so a
    // preview that fails leaves nothing where the person asked for the file.
    let report = played.map_err(|problem| problem.to_string())?;
    if let (Destination::File(written), Some(writing), Some(out)) = (destination, writing, capture)
    {
        written.finish(writing, out)?;
    }

    let seconds = report.played.to_seconds();
    if json {
        print_json(&Played {
            mix: mix.display().to_string(),
            capture: capture.map(|path| path.display().to_string()),
            from_seconds: from_samples.to_seconds().0,
            length_samples: report.played.0,
            length_seconds: seconds.0,
            mix_length_seconds: total.to_seconds().0,
            underruns: report.underruns,
        });
    } else {
        let file = match capture {
            Some(path) => format!(", wrote {}", path.display()),
            None => String::new(),
        };
        say!(
            "played {} from {} of {}, {}{}",
            length_text(seconds),
            length_text(from_samples.to_seconds()),
            length_text(total.to_seconds()),
            underrun_text(report.underruns),
            file
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_underrun_count_reads_as_a_sentence() {
        assert_eq!(underrun_text(0), "no underruns");
        assert_eq!(underrun_text(1), "1 underrun");
        assert_eq!(underrun_text(7), "7 underruns");
    }
}
