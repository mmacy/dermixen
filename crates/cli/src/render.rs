//! The `render` command, which turns a mix document, or a span of one, into
//! one continuous audio file: a 16-bit WAV, or, when the output name ends
//! in `.mp3` in either case, a 320 kbps constant bit rate MP3.

use std::ops::Range;
use std::path::Path;

use dermixen_core::files::AtomicFile;
use dermixen_core::{Mix, Samples, Seconds, Track};
pub(crate) use dermixen_engine::mix_length;
use dermixen_engine::{Resampler, Source, TimeStretcher, render_range};
use dermixen_media::{Frame, WavDepth, WavFile, decode, hash_file};
use serde::Serialize;

use crate::analyze::print_json;
use crate::show::length_text;
use crate::text::{note, say};

/// What `render --json` prints.
#[derive(Debug, Serialize)]
struct Written {
    /// The file written.
    path: String,
    /// The length of the rendered mix.
    length_samples: i64,
    /// The same length in seconds.
    length_seconds: f64,
    /// Where the handover falls in the clip, when `--handover` asked for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    handover: Option<HandoverTimes>,
}

/// Where one handover falls in the clip written for it, as
/// `render --handover --json` prints it.
#[derive(Debug, Serialize)]
struct HandoverTimes {
    /// The track the handover goes into, counting from one.
    into_track: usize,
    /// Where the clip starts, on the clock `mix show` reports.
    from_seconds: f64,
    /// How long the clip is.
    for_seconds: f64,
    /// Where the incoming track's rise begins, counted from the start of the clip.
    rise_seconds: f64,
    /// Where the two anchors meet, counted the same way.
    anchor_seconds: f64,
    /// Where the outgoing track's last sample is heard, counted the same way.
    outgoing_end_seconds: f64,
}

/// How much of the mix the clip keeps on either side of the handover: a
/// minute of the outgoing track before the rise begins, and a minute of the
/// incoming track after the outgoing track has ended.
const MARGIN: Seconds = Seconds(60.0);

/// The clip around one handover: the span of the mix to render, and the
/// three moments within the clip a person listens for.
///
/// The span is on the clock `mix show` reports, which counts from the mix's
/// first sample. The three moments are counted from the start of the clip
/// instead. A moment that falls before the mix's first sample is reported as
/// the start of the clip. That can happen to the rise alone, when the
/// incoming track is the first track to sound, and there is nothing to hear
/// before the mix's first sample anyway.
struct Handover {
    /// The track the handover goes into, counting from one.
    into_track: usize,
    /// Where the clip starts, counted from the start of the mix.
    from: Seconds,
    /// How long the clip is.
    length: Seconds,
    /// Where the incoming track's rise begins.
    rise: Seconds,
    /// Where the two anchors meet.
    anchor: Seconds,
    /// Where the outgoing track's last sample is heard.
    outgoing_end: Seconds,
}

impl Handover {
    /// What `--json` prints for the clip.
    fn times(&self) -> HandoverTimes {
        HandoverTimes {
            into_track: self.into_track,
            from_seconds: self.from.0,
            for_seconds: self.length.0,
            rise_seconds: self.rise.0,
            anchor_seconds: self.anchor.0,
            outgoing_end_seconds: self.outgoing_end.0,
        }
    }

    /// The sentence a person reads after the line naming the file written.
    fn line(&self) -> String {
        format!(
            "the handover into track {}, counted from the start of the clip: the rise begins at {}, the anchors meet at {}, and track {} ends at {}",
            self.into_track,
            length_text(self.rise),
            length_text(self.anchor),
            self.into_track - 1,
            length_text(self.outgoing_end)
        )
    }
}

/// Works out the clip around the handover into track `into_track`, counting
/// from one.
///
/// The three moments come from the document's timeline the way `mix show`
/// works out each track's start and end. The rise begins where the incoming
/// track's volume envelope begins, at its first node: thirty-two beats before
/// the intro anchor for the blend, at the anchor for a beatmix or a bass
/// swap, and a quarter beat before it for a cut. A track whose envelope has
/// no node before its intro anchor rises at the anchor. The two anchors meet
/// at the incoming track's intro anchor, and the outgoing track ends at its
/// last sample. The clip runs from [`MARGIN`] before the rise to [`MARGIN`]
/// after the outgoing track's end, cut off at either end of the mix.
///
/// Track 1 is refused, because the handover into a track is the one that
/// leads into it from the track before, and no track comes before the first
/// one. A number the playlist does not have is refused as `mix move-anchor`
/// refuses one.
fn handover_clip(mix: &Path, document: &Mix, into_track: usize) -> Result<Handover, String> {
    let held = document.tracks.len();
    if into_track < 1 || into_track > held {
        return Err(crate::document::no_such_track(mix, into_track, held));
    }
    if into_track == 1 {
        return Err(
            "--handover 1 asks for the handover into track 1, and there is no track before track 1, so no handover leads into it. The first handover of a mix is the one into track 2"
                .to_owned(),
        );
    }
    let timeline = document
        .timeline()
        .expect("a playlist with a track before this one has a timeline");
    let opening = timeline.start();
    let mix_length = timeline.end() - opening;
    let incoming = &document.tracks[into_track - 1];
    let placed_incoming = timeline.tracks[into_track - 1];
    let placed_outgoing = timeline.tracks[into_track - 2];
    let rise_beat = incoming
        .volume
        .nodes()
        .first()
        .map(|node| node.at)
        .filter(|at| at.0 <= incoming.anchors.intro.0)
        .unwrap_or(incoming.anchors.intro);
    let rise =
        placed_incoming.mix_time_of(&timeline.curve, incoming.grid.time_of(rise_beat)) - opening;
    let anchor = placed_incoming.mix_time_of(
        &timeline.curve,
        incoming.grid.time_of(incoming.anchors.intro),
    ) - opening;
    let outgoing_end = placed_outgoing.end(&timeline.curve) - opening;

    let from = Seconds((rise.0 - MARGIN.0).max(0.0));
    let until = Seconds((outgoing_end.0 + MARGIN.0).min(mix_length.0));
    if until.0 <= from.0 {
        return Err(format!(
            "the handover into track {into_track} of {} covers none of the mix: the clip would start at {} and end at {}",
            mix.display(),
            length_text(from),
            length_text(until)
        ));
    }
    let within = |moment: Seconds| Seconds((moment.0 - from.0).max(0.0));
    Ok(Handover {
        into_track,
        from,
        length: until - from,
        rise: within(rise),
        anchor: within(anchor),
        outgoing_end: within(outgoing_end),
    })
}

/// Whether `path` names an MP3 file: its extension is `mp3`, in either case.
fn wants_mp3(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
}

/// The file a render writes its frames into: a 16-bit WAV, or, in a build
/// with the `mp3` feature, an MP3 at [`dermixen_media::MP3_BITRATE_KBPS`]
/// constant bit rate.
enum Output {
    /// A 16-bit WAV file.
    Wav(WavFile),
    /// An MP3 file, in a build with the `mp3` feature.
    #[cfg(feature = "mp3")]
    Mp3(dermixen_media::Mp3File),
}

impl Output {
    /// Starts the file that will contain the render's frames in `writing`,
    /// the temporary file that will be moved onto the command's real output,
    /// `out`. The frames go into an MP3 when `out` ends in `.mp3` in either
    /// case and into a WAV otherwise.
    ///
    /// A build without the `mp3` feature refuses an MP3 output with a
    /// message naming `out`, since that is the name the person gave, and the
    /// missing feature.
    fn create(writing: &AtomicFile, out: &Path) -> Result<Output, String> {
        let file = writing
            .file()
            .try_clone()
            .map_err(|problem| format!("cannot write {}: {problem}", out.display()))?;
        if wants_mp3(out) {
            #[cfg(feature = "mp3")]
            {
                return dermixen_media::Mp3File::from_file(file, out)
                    .map(Output::Mp3)
                    .map_err(|problem| problem.to_string());
            }
            #[cfg(not(feature = "mp3"))]
            {
                drop(file);
                return Err(format!(
                    "{} ends in .mp3, and this dermixen was built without the mp3 feature, so it cannot write an MP3",
                    out.display()
                ));
            }
        }
        WavFile::from_file(file, out, WavDepth::Int16)
            .map(Output::Wav)
            .map_err(|problem| problem.to_string())
    }

    /// Appends frames to the file.
    fn write(&mut self, frames: &[Frame]) -> Result<(), String> {
        match self {
            Output::Wav(file) => file.write(frames).map_err(|problem| problem.to_string()),
            #[cfg(feature = "mp3")]
            Output::Mp3(file) => file.write(frames).map_err(|problem| problem.to_string()),
        }
    }

    /// Completes the file and closes it.
    fn finish(self) -> Result<(), String> {
        match self {
            Output::Wav(file) => file.finish().map_err(|problem| problem.to_string()),
            #[cfg(feature = "mp3")]
            Output::Mp3(file) => file.finish().map_err(|problem| problem.to_string()),
        }
    }
}

/// A stretcher for a track whose keylock is on, which keeps the track's pitch
/// where its speed changes.
#[cfg(feature = "signalsmith")]
pub(crate) fn keylock_stretcher(_warned: &mut bool) -> Box<dyn TimeStretcher> {
    Box::new(dermixen_engine::SignalsmithStretcher::new())
}

/// A stretcher for a track whose keylock is on, in a build with no
/// pitch-preserving stretcher.
///
/// Plain resampling stands in, which moves the pitch with the speed. The
/// command prints one warning when it reaches the first keylock track of the
/// render, so the person listening knows why the mix sounds as it does.
#[cfg(not(feature = "signalsmith"))]
pub(crate) fn keylock_stretcher(warned: &mut bool) -> Box<dyn TimeStretcher> {
    if !*warned {
        *warned = true;
        note!(
            "warning: this dermixen was built without a pitch-preserving stretcher, so tracks with keylock are resampled and their pitch moves with their speed"
        );
    }
    Box::new(Resampler::new())
}

/// Starts the file a render or a capture writes, which replaces `out` in one
/// step once the whole render has finished.
///
/// The frames go into a temporary file beside `out` whose name nobody can
/// work out in advance, so a symbolic link somebody planted at a name this
/// command might have chosen is never written through, and a run that stops
/// partway leaves no file at all. `out` keeps its permissions when a file is
/// already there, as replacing an earlier render does.
pub(crate) fn writing_to(out: &Path) -> Result<AtomicFile, String> {
    AtomicFile::create(out, false)
        .map_err(|problem| format!("cannot write {}: {problem}", out.display()))
}

/// Refuses an output that is the mix document or one of its tracks.
///
/// `render` and `play --capture` both read the document and every track of
/// it, and neither writes over a file it reads, whether the output names one
/// of those files directly, through `..`, through a symbolic link, or
/// through a hard link. An output that is an earlier render is replaced,
/// which is what `render` is for.
pub(crate) fn refuse_the_mix_and_its_tracks(
    out: &Path,
    mix: &Path,
    document: &Mix,
) -> Result<(), String> {
    crate::paths::refuse_an_input(out, "the mix document", &[mix])?;
    let tracks: Vec<&Path> = document
        .tracks
        .iter()
        .map(|track| track.path.as_path())
        .collect();
    crate::paths::refuse_an_input(out, "a track of the mix", &tracks)
}

/// How many frames a render of `span` writes from a mix `total` frames long.
/// A span that runs past the end of the mix stops there, and a span that
/// starts past the end writes nothing.
pub(crate) fn span_frames(span: &Range<Samples>, total: Samples) -> Samples {
    Samples(span.end.min(total).0.saturating_sub(span.start.0).max(0))
}

/// Refuses a mix too long for the WAV file it is about to be written into.
///
/// A WAV file describes at most 4 GiB, which at sixteen bits and two
/// channels is about six hours and forty-five minutes, while a mix document
/// may lay out as much as twenty-four hours. The check runs before anything
/// is created, so a mix over the limit costs the person the message alone,
/// and the message points at the MP3 output, which has no such limit. An
/// output that already ends in `.mp3` is not checked.
pub(crate) fn refuse_a_mix_too_long_for_a_wav(out: &Path, frames: Samples) -> Result<(), String> {
    if wants_mp3(out) {
        return Ok(());
    }
    let capacity = WavFile::capacity(WavDepth::Int16);
    if frames <= capacity {
        return Ok(());
    }
    Err(format!(
        "{} would be {} long, which is {} frames, and a WAV file describes at most 4 GiB, which is {} frames at sixteen bits ({} long). Write the mix to a name ending in .mp3, which has no such limit, or render a span of it with --from and --for.",
        out.display(),
        length_text(frames.to_seconds()),
        frames.0,
        capacity.0,
        length_text(capacity.to_seconds())
    ))
}

/// Checks that every track's file still holds the bytes it held when it was
/// added to the mix.
///
/// This runs before the output file is created, so a mix naming a file that
/// has been re-encoded or replaced stops the render rather than writing a
/// file with the wrong audio in it. A file that cannot be read and a file
/// whose bytes have changed are both reported with the file's name and with
/// the `mix relink` command that points the document at the file wherever it
/// is now, as `docs/cli.md` states under "render".
pub(crate) fn check_hashes(tracks: &[Track], mix: &Path) -> Result<(), String> {
    let remedy = format!(
        "Run dermixen mix relink {} to point the mix at the file wherever it is now.",
        mix.display()
    );
    for track in tracks {
        let found = hash_file(&track.path).map_err(|problem| {
            format!(
                "cannot read {}, which {} names: {problem}. {remedy}",
                track.path.display(),
                mix.display()
            )
        })?;
        if found != track.hash {
            return Err(format!(
                "{} has changed since it was added to the mix: its hash is now {found}, and {} names the hash {}. {remedy}",
                track.path.display(),
                mix.display(),
                track.hash
            ));
        }
    }
    Ok(())
}

/// The `--from` and `--for` flags as they were given, before the length of
/// the mix is known.
///
/// The command reads the two flags on their own, so it refuses a value that
/// is not a time before it opens the mix document. It turns the two values
/// into frames only once it knows the mix's length, since that length
/// decides whether the start falls within the mix at all.
pub(crate) struct SpanRequest {
    /// Where to start, or the start of the mix when `--from` was omitted.
    from: Option<Seconds>,
    /// How much to cover, or to the end of the mix when `--for` was omitted.
    length: Option<Seconds>,
}

impl SpanRequest {
    /// Reads the two flags, refusing a value that is not a time and a length
    /// of zero with a message naming the flag concerned.
    pub(crate) fn read(from: Option<&str>, length: Option<&str>) -> Result<SpanRequest, String> {
        let from = from
            .map(|text| crate::library::time_value("--from", text, text))
            .transpose()?;
        let length = length
            .map(|text| {
                let length = crate::library::time_value("--for", text, text)?;
                if length.to_samples() <= Samples::ZERO {
                    return Err(format!(
                        "--for {text} is a length of zero. Give a length greater than zero"
                    ));
                }
                Ok(length)
            })
            .transpose()?;
        Ok(SpanRequest { from, length })
    }

    /// The span in output frames, counted from the start of a mix `total`
    /// frames long, as [`SpanRequest::resolve_within`] gives it with the mix
    /// as the subject.
    pub(crate) fn resolve(&self, total: Samples) -> Result<Range<Samples>, String> {
        self.resolve_within(total, "mix")
    }

    /// The span in output frames, counted from the start of a span `total`
    /// frames long, where `subject` names what `total` measures, like
    /// `"mix"` or `"file"`, for the message a start past the end is refused
    /// with.
    ///
    /// A start at or past the end is refused with the subject's length,
    /// since a person who asks to hear a moment the span does not reach
    /// wants to be told so rather than handed nothing. An end past the end
    /// is left to the caller, which clips it there.
    pub(crate) fn resolve_within(
        &self,
        total: Samples,
        subject: &str,
    ) -> Result<Range<Samples>, String> {
        let start = self.from.map_or(Samples::ZERO, Seconds::to_samples);
        if self.from.is_some() && start >= total {
            return Err(format!(
                "--from {} is at or past the end of the {subject}, which is {} long",
                length_text(start.to_seconds()),
                length_text(total.to_seconds())
            ));
        }
        let until = match self.length {
            Some(length) => Samples(start.0.saturating_add(length.to_samples().0)),
            None => Samples(i64::MAX),
        };
        Ok(start..until)
    }
}

/// Decodes a track's audio the first time the render needs it, saying on
/// standard error which of the mix's `total` tracks is being read, since
/// decoding a long track takes a noticeable moment.
///
/// The decoder hashes the file as it reads it, and the hash it reports must
/// be the hash the document names. [`check_hashes`] has already compared the
/// two, and comparing them again here covers a file that changed between
/// that check and this read, so the frames a render writes are always the
/// frames the document names.
pub(crate) fn decoder(
    total: usize,
) -> impl FnMut(usize, &Track) -> Result<Box<dyn Source>, String> {
    move |index: usize, track: &Track| -> Result<Box<dyn Source>, String> {
        note!(
            "decoding track {} of {}: {}",
            index + 1,
            total,
            track.path.display()
        );
        let decoded = decode(&track.path).map_err(|problem| problem.to_string())?;
        if decoded.hash != track.hash {
            return Err(format!(
                "{} changed while it was being read: its hash is now {}, and the mix names the hash {}",
                track.path.display(),
                decoded.hash,
                track.hash
            ));
        }
        Ok(Box::new(decoded.audio))
    }
}

/// Chooses each track's time stretcher: the pitch-preserving one for a track
/// with keylock, and plain resampling for a track without it. In a build
/// with no pitch-preserving stretcher the command prints one warning,
/// however many keylock tracks the mix holds.
pub(crate) fn stretchers() -> impl FnMut(&Track) -> Box<dyn TimeStretcher> {
    let mut warned = false;
    move |track: &Track| -> Box<dyn TimeStretcher> {
        if track.keylock {
            keylock_stretcher(&mut warned)
        } else {
            Box::new(Resampler::new())
        }
    }
}

/// The options `render` takes, as the person typed them.
pub struct Args<'a> {
    /// The mix document.
    pub mix: &'a Path,
    /// The file to write.
    pub out: &'a Path,
    /// Where to start, when `--from` was given.
    pub from: Option<&'a str>,
    /// How much to render, when `--for` was given.
    pub length: Option<&'a str>,
    /// The track to render the handover into, when `--handover` was given.
    pub handover: Option<usize>,
    /// Whether to print one JSON object instead of text.
    pub json: bool,
}

/// Carries out the `render` command.
///
/// The output is checked against the command's own inputs first, then every
/// track's file is hashed, and then the mix, or the span of it that `--from`
/// and `--for` select as `docs/cli.md` describes, is rendered block by block
/// into a temporary file beside the output, which is moved into place only
/// once the whole render has finished. A render that fails at any point
/// leaves no file where the output was asked for and no temporary file
/// either.
///
/// `--handover N` gives the span instead of `--from` and `--for`, and clap
/// refuses a command line that has `--handover` and either of the other two.
/// The span is then the clip around the handover into track `N` that
/// [`handover_clip`] works out, and the render goes on from there as it does
/// for a span given by hand.
pub fn run(args: &Args<'_>) -> Result<(), String> {
    let (mix, out, json) = (args.mix, args.out, args.json);
    let span = SpanRequest::read(args.from, args.length)?;
    let document = crate::document::read(mix)?;
    refuse_the_mix_and_its_tracks(out, mix, &document)?;
    let handover = args
        .handover
        .map(|into_track| handover_clip(mix, &document, into_track))
        .transpose()?;
    let span = match &handover {
        Some(clip) => SpanRequest {
            from: Some(clip.from),
            length: Some(clip.length),
        },
        None => span,
    };
    let total = mix_length(&document);
    let span = span.resolve(total)?;
    refuse_a_mix_too_long_for_a_wav(out, span_frames(&span, total))?;
    check_hashes(&document.tracks, mix)?;

    let writing = writing_to(out)?;
    let mut file = Output::create(&writing, out)?;

    let mut load = decoder(document.tracks.len());
    let mut stretch = stretchers();
    let mut sink = |block: &[Frame]| -> Result<(), String> { file.write(block) };
    // A render of a long mix takes minutes, so it says how far along it is
    // rather than looking as though it has stopped. One line per ten seconds
    // of finished audio is often enough to watch and rare enough to read.
    let mut next_report = span.start;
    let mut progress = |progress: dermixen_engine::Progress| {
        if progress.written >= next_report {
            note!(
                "rendered {} of {}",
                length_text(progress.written.to_seconds()),
                length_text(progress.total.to_seconds())
            );
            next_report = progress.written + Seconds(10.0).to_samples();
        }
    };

    let rendered = render_range(
        &document,
        span,
        &mut load,
        &mut stretch,
        &mut sink,
        &mut progress,
    );
    // The temporary file is removed when `writing` is dropped, so every way
    // out from here on leaves nothing behind unless the commit has run.
    let length = rendered.map_err(|problem| problem.to_string())?;
    file.finish()?;
    writing
        .commit()
        .map_err(|problem| format!("cannot write {}: {problem}", out.display()))?;

    let seconds = length.to_seconds();
    if json {
        print_json(&Written {
            path: out.display().to_string(),
            length_samples: length.0,
            length_seconds: seconds.0,
            handover: handover.as_ref().map(Handover::times),
        });
    } else {
        say!(
            "wrote {}: {} long, {} samples",
            out.display(),
            length_text(seconds),
            length.0
        );
        if let Some(clip) = &handover {
            say!("{}", clip.line());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mix_over_the_wav_limit_is_refused_and_an_mp3_is_not() {
        let capacity = WavFile::capacity(WavDepth::Int16);
        let over = Samples(capacity.0 + 1);
        assert!(refuse_a_mix_too_long_for_a_wav(Path::new("out.wav"), capacity).is_ok());
        let message = refuse_a_mix_too_long_for_a_wav(Path::new("out.wav"), over).unwrap_err();
        assert!(
            message.contains("4 GiB") && message.contains(".mp3"),
            "{message}"
        );
        assert!(refuse_a_mix_too_long_for_a_wav(Path::new("out.mp3"), over).is_ok());
    }

    #[test]
    fn a_span_past_the_end_of_the_mix_stops_there() {
        let total = Samples(1_000);
        assert_eq!(span_frames(&(Samples(0)..Samples(i64::MAX)), total), total);
        assert_eq!(
            span_frames(&(Samples(400)..Samples(600)), total),
            Samples(200)
        );
        assert_eq!(
            span_frames(&(Samples(2_000)..Samples(3_000)), total),
            Samples(0)
        );
    }

    #[test]
    fn wants_mp3_reads_the_extension_in_either_case() {
        assert!(wants_mp3(Path::new("out.mp3")));
        assert!(wants_mp3(Path::new("OUT.MP3")));
        assert!(!wants_mp3(Path::new("out.mp3.bak")));
        assert!(!wants_mp3(Path::new("out")));
        // ".mp3" begins with a dot and has no other dot in it, so
        // `Path::extension` reports no extension at all, and this name gets
        // a WAV, the same as any other name with no extension.
        assert!(!wants_mp3(Path::new(".mp3")));
    }
}
