//! The `mix new`, `mix add`, `mix move-anchor`, and `mix set-gain` commands,
//! which create a mix document, extend it, re-anchor it, and level it.

use std::io::Write;
use std::path::{Path, PathBuf};

use dermixen_core::files::{LARGEST_DOCUMENT, read_text, write_atomically};
use dermixen_core::{
    Anchor, Anchors, Beats, Decibels, Edit, EditError, Envelope, EqEnvelopes, MAX_BEAT, Mix,
    Preset, Settings, Track, apply, apply_edit, clear_incoming, clear_outgoing,
    fits_between_the_anchors, leveling_gain, outro_for, span_of,
};
use dermixen_library::TrackRecord;
use dermixen_media::hash_file;

use crate::analyze;
use crate::analyzers::Given;
use crate::text::{note, say};

/// The four transition presets, named as `--preset` takes them.
const PRESET_NAMES: &str = "blend, beatmix, bass-swap, and cut";

/// Reads a mix document, naming the file if it cannot be read or is not a
/// valid document.
///
/// The text comes through [`read_text`], so the command reads a mix document
/// only from a regular file of at most [`LARGEST_DOCUMENT`] bytes and never
/// from a device or a named pipe. [`Mix::from_json`] then holds every number
/// in the document to the limits `DESIGN.md` states, so the mix this returns
/// is one the layout and the render can take as it stands.
pub fn read(mix: &Path) -> Result<Mix, String> {
    let text = read_text(mix, LARGEST_DOCUMENT).map_err(|problem| problem.to_string())?;
    let document =
        Mix::from_json(&text).map_err(|problem| format!("{}: {problem}", mix.display()))?;
    check_track_paths(mix, &document)?;
    Ok(document)
}

/// Refuses a document whose track names something that is not an audio file.
///
/// A track path with nothing at it is left alone, because a file that has
/// moved or is on a volume that is not mounted is what `mix relink` is for.
/// A path that names a folder, a device, or a named pipe is different: no
/// command wrote it, reading one never gives audio, and reading a device
/// never ends, so the document is refused as soon as it is read rather than
/// at the moment some command reaches that path.
fn check_track_paths(mix: &Path, document: &Mix) -> Result<(), String> {
    for (index, track) in document.tracks.iter().enumerate() {
        let Ok(data) = std::fs::metadata(&track.path) else {
            continue;
        };
        if !data.file_type().is_file() {
            return Err(format!(
                "track {} of {} names {}, which is not a regular file. A track of a mix is an audio file.",
                index + 1,
                mix.display(),
                track.path.display()
            ));
        }
    }
    Ok(())
}

/// Replaces a mix document with the text for `document` in one step.
///
/// The text comes from [`Mix::checked_json`], so a mix that could not be
/// opened again is refused before the disk is touched. The write goes
/// through [`write_atomically`], which writes a temporary file nobody can
/// name in advance and moves it onto the document, so a write that fails
/// partway leaves the document exactly as it was and an edit keeps the
/// permissions the document had.
pub fn replace(mix: &Path, document: &Mix) -> Result<(), String> {
    let text = document.checked_json().map_err(|problem| {
        format!(
            "{} is left as it was: the change would make a document dermixen cannot open again: {problem}",
            mix.display()
        )
    })?;
    write_atomically(mix, false, text.as_bytes())
        .map_err(|problem| format!("cannot write {}: {problem}", mix.display()))
}

/// Carries out `mix new`, refusing to replace a document that is already there.
pub fn new(mix: &Path, json: bool) -> Result<(), String> {
    let text = Mix::new()
        .checked_json()
        .map_err(|problem| format!("cannot write {}: {problem}", mix.display()))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(mix)
        .map_err(|problem| {
            if problem.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "{} exists, and dermixen will not replace a mix document that is already there",
                    mix.display()
                )
            } else {
                format!("cannot write {}: {problem}", mix.display())
            }
        })?;
    file.write_all(text.as_bytes())
        .map_err(|problem| format!("cannot write {}: {problem}", mix.display()))?;

    #[derive(serde::Serialize)]
    struct Written {
        path: String,
    }
    if json {
        analyze::print_json(&Written {
            path: mix.display().to_string(),
        });
    } else {
        say!("wrote {}", mix.display());
    }
    Ok(())
}

/// Reads an anchor that a person gave, refusing one that is not a whole beat
/// and one outside the beats a mix document holds.
fn whole_beat(option: &str, beat: f64) -> Result<Beats, String> {
    let beat = Beats(beat);
    if !beat.0.is_finite() || beat.0.abs() > MAX_BEAT.0 {
        return Err(format!(
            "{option} must be a beat of the track from -{} to {}, not {}",
            MAX_BEAT.0, MAX_BEAT.0, beat.0
        ));
    }
    if !beat.is_whole() {
        return Err(format!(
            "{option} must be a whole beat of the track, not {}",
            beat.0
        ));
    }
    Ok(beat)
}

/// The anchors a track takes when the person gives neither `--intro` nor
/// `--outro`, and where the outro anchor among them came from.
struct Defaults {
    /// The anchors themselves.
    anchors: Anchors,
    /// A clause naming where the outro anchor came from, for the message
    /// that refuses a pair of anchors with no room between them.
    outro_source: &'static str,
}

/// Settles the pair of anchors for a track being added.
///
/// `--intro` and `--outro` each replace one of `defaults`, and both must be
/// whole beats. The outro anchor must be a later beat than the intro anchor,
/// because the two of them mark the span the track holds the mix for: a span
/// of no beats, or of a negative number of them, leaves the tracks on either
/// side of this one landing on top of it. A pair that does not leave that
/// span is refused whether the anchors were given on the command line or
/// came from the defaults, and the refusal names both beats and where the
/// outro anchor came from.
fn settle_anchors(
    intro: Option<f64>,
    outro: Option<f64>,
    defaults: &Defaults,
) -> Result<Anchors, String> {
    let intro = match intro {
        Some(beat) => whole_beat("--intro", beat)?,
        None => defaults.anchors.intro,
    };
    let (outro, source) = match outro {
        Some(beat) => (whole_beat("--outro", beat)?, "which is what --outro gave"),
        None => (defaults.anchors.outro, defaults.outro_source),
    };
    if !outro.0.is_finite() || outro.0 <= intro.0 {
        return Err(format!(
            "the outro anchor must be a later beat than the intro anchor: the intro anchor is beat {}, and the outro anchor is beat {}, {source}",
            intro.0, outro.0
        ));
    }
    Ok(Anchors { intro, outro })
}

/// Reads the gain a person gave, refusing one that is not a finite number
/// and one outside the gains a mix document holds. `named` is what the
/// message calls the value: `--gain` for `mix add`, where that is the flag
/// the person typed, and `the gain` for `mix set-gain`, whose value is a
/// bare number.
///
/// clap reads `nan` and `inf` as `f64` values, and the render multiplies
/// every sample of the track by the gain, so a gain that is not finite would
/// turn the whole track into NaN or infinity in the rendered file. The range
/// is the one `DESIGN.md` states under "Limits", which
/// [`Decibels::is_level`] holds a value to, so a gain the command takes is
/// one the document keeps.
fn given_gain(named: &str, db: f64) -> Result<Decibels, String> {
    if !db.is_finite() {
        return Err(format!(
            "{named} {db} is not a number of decibels. Give a finite number, like -3.5"
        ));
    }
    let gain = Decibels(db);
    if !gain.is_level() {
        return Err(format!(
            "{named} {db} is outside the gains a mix document holds, which run from {} to {} decibels",
            Decibels::LOWEST_LEVEL.0,
            Decibels::HIGHEST_LEVEL.0
        ));
    }
    Ok(gain)
}

/// The message that refuses a track number the playlist does not have,
/// naming the number asked for and how many tracks the document contains.
///
/// `mix move-anchor`, `mix set-gain`, and `render --handover` all take a
/// track by its position in the playlist and refuse a position outside it
/// with this one sentence.
pub fn no_such_track(mix: &Path, track: usize, held: usize) -> String {
    format!(
        "there is no track {track}: {} contains {held} {}",
        mix.display(),
        if held == 1 { "track" } else { "tracks" }
    )
}

/// The gain a track takes when the person gave no `--gain`: the leveling
/// gain the record's loudness gives, as `docs/project-file.md` describes
/// under "The gain".
///
/// A record with no loudness is one stored without a loudness measurement,
/// or one for a track the meter finds nothing in, which is one quieter than
/// the meter's gate, shorter than its block, silent, or measuring above
/// 0 LUFS, which no real master does.
/// Such a record gives a gain of zero. The command says so on standard error,
/// because a track sitting at its own level among leveled tracks is worth
/// knowing about, and the two cases have different remedies: a scan of the
/// file's folder gives the first case its loudness, and the second case
/// keeps a gain of zero however often it is scanned.
fn leveling_for(record: &TrackRecord) -> Decibels {
    match record.loudness {
        Some(loudness) => leveling_gain(loudness.integrated, loudness.true_peak),
        None => {
            note!(
                "warning: the library has no loudness for {}, so its gain is +0.0 dB rather than the leveling gain. Scanning its folder with dermixen library scan measures a track whose record has no loudness. A track the meter finds nothing in (a track quieter than the meter's gate, shorter than its block, silent, or measuring above 0 LUFS, which no real master does) keeps a gain of +0.0 dB.",
                record.path.display()
            );
            Decibels::UNITY
        }
    }
}

/// Refuses a transition that does not fit between the outgoing track's own
/// two anchors, naming the file as well as the length asked for, the beats
/// between the anchors, and the beat each anchor sits on.
///
/// A preset writes the outgoing track's fade out from its outro anchor and
/// the incoming track's fade in from its intro anchor. A track in the middle
/// of a mix is on both sides of that: it fades in from its intro anchor and
/// out from its outro anchor. A transition longer than the beats between
/// those two anchors would have the track fading in and fading out at the
/// same time, and the next track added would cut the fade in short where the
/// two met, so the command turns the request down instead. The rule itself
/// is [`fits_between_the_anchors`] in the core crate, which the timeline's
/// insert edit holds a track to as well; this adds the file's name, which
/// only the command knows to mention, and the way out: fewer bars for a
/// preset that has a length, and another preset or other anchors for the
/// blend, whose rise is always twenty-eight bars. `docs/cli.md` states the
/// rule under "mix add".
fn check_fit(outgoing: &Track, preset: Preset) -> Result<(), String> {
    fits_between_the_anchors(outgoing, span_of(preset)).map_err(|problem| match problem {
        EditError::TransitionDoesNotFit { span, length } => {
            let way_out = match preset {
                Preset::Blend => "The blend's rise is always twenty-eight bars, so choose another preset or move the anchors.",
                _ => "Ask for fewer bars.",
            };
            format!(
                "a transition of {} beats is longer than the {} beats between the anchors of {}, whose intro anchor is beat {} and whose outro anchor is beat {}. {way_out}",
                length.0,
                span.0,
                outgoing.path.display(),
                outgoing.anchors.intro.0,
                outgoing.anchors.outro.0
            )
        }
        other => other.to_string(),
    })
}

/// Where in the playlist a track goes, counting from zero.
///
/// A position given on the command line counts from one and may be one past
/// the last track, which is the same as leaving it out and adding at the end.
fn placement(mix: &Path, position: Option<usize>, held: usize) -> Result<usize, String> {
    let Some(position) = position else {
        return Ok(held);
    };
    if position < 1 || position > held + 1 {
        return Err(format!(
            "--position {position} is outside the playlist: {} contains {held} {}, so the position must be from 1 to {}",
            mix.display(),
            if held == 1 { "track" } else { "tracks" },
            held + 1
        ));
    }
    Ok(position - 1)
}

/// The options `mix add` takes, as the person typed them.
pub struct AddArgs<'a> {
    /// The mix document.
    pub mix: &'a Path,
    /// The audio file.
    pub file: &'a Path,
    /// Where in the playlist to put the track, counting from one.
    pub position: Option<usize>,
    /// The transition preset's name.
    pub preset: &'a str,
    /// The length of the transition in bars.
    pub bars: u32,
    /// The intro anchor, when it was given.
    pub intro: Option<f64>,
    /// The outro anchor, when it was given.
    pub outro: Option<f64>,
    /// The tempo, when it was given.
    pub given: Given,
    /// Whether the track keeps its pitch when its speed changes.
    pub keylock: bool,
    /// The gain to write, when one was given instead of the leveling gain.
    pub gain: Option<f64>,
    /// Whether to print the mix as JSON when the track has been added.
    pub json: bool,
    /// The library file, when one was named.
    pub library: Option<&'a Path>,
}

/// The record for a file, taken from the library when the library already
/// contains its bytes and analyzed and stored when it does not, so a file is
/// analyzed once however many mixes it joins.
///
/// `settings` is what [`crate::settings::read`] gave the command, which names
/// the library file when neither `--library` nor the environment does.
fn record_for(
    path: &Path,
    given: &Given,
    library: Option<&Path>,
    settings: &Settings,
) -> Result<TrackRecord, String> {
    let location = crate::index::location(library, settings)?;
    let mut index = crate::index::open(&location)?;
    let hash = hash_file(path).map_err(|problem| problem.to_string())?;
    if let Some(record) = index.get(hash).map_err(|problem| problem.to_string())? {
        return Ok(record);
    }
    let record = analyze::record_of(path, given)?;
    index
        .upsert(&record)
        .map_err(|problem| problem.to_string())?;
    Ok(record)
}

/// The preset called `name`, `bars` bars long where the preset has a length.
/// A length of no bars and a name that is not one of the four presets are
/// refused.
pub fn preset_of(name: &str, bars: u32) -> Result<Preset, String> {
    if bars == 0 {
        return Err("--bars must be at least one bar".to_owned());
    }
    Preset::from_name(name, bars)
        .ok_or_else(|| format!("{name} is not a transition preset. The presets are {PRESET_NAMES}"))
}

/// The preset `mix add` writes when the person names none. The `--preset`
/// option in `main.rs` reads its default from here, and `mix plan` predicts
/// the mix that this preset and [`dermixen_core::DEFAULT_BARS`] build.
pub const DEFAULT_PRESET: &str = "blend";

/// What shapes one track of a mix beyond its file and its record. `mix add`
/// takes each of these from the command line, and `mix plan` takes each at
/// the value `mix add` uses when the person gives no option.
pub struct TrackOptions<'a> {
    /// The length of the transition in bars, which moves the outro anchor.
    pub bars: u32,
    /// The intro anchor, when it was given.
    pub intro: Option<f64>,
    /// The outro anchor, when it was given.
    pub outro: Option<f64>,
    /// The tempo, when it was given.
    pub given: &'a Given,
    /// Whether the track keeps its pitch when its speed changes.
    pub keylock: bool,
    /// The gain to write, when one was given instead of the leveling gain.
    pub gain: Option<f64>,
}

/// Builds the track that a file and its record make, before that track joins
/// a mix.
///
/// The grid, the anchors, and the gain come from the record unless an option
/// replaces one of the three, so `mix add` and `mix plan` build the same
/// track from the same file. For a record with no loudness, [`leveling_for`]
/// writes its warning on standard error.
pub fn build_track(
    path: PathBuf,
    record: &TrackRecord,
    options: &TrackOptions<'_>,
) -> Result<Track, String> {
    // A tempo given on the command line replaces the grid analysis found,
    // and the anchors analysis placed belong to the analyzed grid, so they
    // are not used with a grid the person named.
    let (grid, defaults) = match options.given.grid()? {
        Some(grid) => {
            let last_beat = Beats(grid.beat_at_position(record.length).0.floor());
            (
                grid,
                Defaults {
                    anchors: Anchors {
                        intro: Beats::ZERO,
                        outro: last_beat,
                    },
                    outro_source: "which is the last whole beat of the track, because --outro was not given",
                },
            )
        }
        None => (
            record.grid,
            Defaults {
                anchors: Anchors {
                    intro: record.anchors.intro,
                    outro: outro_for(record.anchors, options.bars),
                },
                outro_source: "which is where analysis put it for a transition of this many bars, because --outro was not given",
            },
        ),
    };
    let anchors = settle_anchors(options.intro, options.outro, &defaults)?;
    let gain = match options.gain {
        Some(db) => given_gain("--gain", db)?,
        None => leveling_for(record),
    };
    Ok(Track {
        path,
        hash: record.hash,
        length: record.length,
        grid,
        anchors,
        keylock: options.keylock,
        gain,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    })
}

/// Puts a track into a mix at `at`, counting from zero, and joins the track
/// to its neighbors with `preset`.
///
/// A transition that will not fit is refused before any node is written, so
/// a refused join leaves the mix exactly as it was. `mix add` writes the mix
/// it gets back to the document, and `mix plan` lays the mix out and keeps
/// no document at all.
pub fn join(document: &mut Mix, at: usize, track: Track, preset: Preset) -> Result<(), String> {
    // Each transition about to be written has an outgoing track: the track
    // before this one, and this one itself when a track follows it. Both are
    // held to the rule before any node is written, so a transition that will
    // not fit leaves the playlist as it was.
    if at > 0 {
        check_fit(&document.tracks[at - 1], preset)?;
    }
    let goes_between_two_tracks = at > 0 && at < document.tracks.len();
    if at < document.tracks.len() {
        check_fit(&track, preset)?;
    }
    document.tracks.insert(at, track);

    // Appending a track writes nodes and removes none, so a node a person
    // placed on the track before survives. A track going between two others
    // replaces the transition that joined them, so the nodes of that
    // transition come off both of them before the two new ones go on.
    if at > 0 {
        let (earlier, rest) = document.tracks.split_at_mut(at);
        let outgoing = earlier
            .last_mut()
            .expect("a position past the front has a track before it");
        if goes_between_two_tracks {
            clear_outgoing(outgoing);
        }
        apply(preset, outgoing, &mut rest[0]);
    }
    if at + 1 < document.tracks.len() {
        let (through_new, later) = document.tracks.split_at_mut(at + 1);
        let outgoing = through_new
            .last_mut()
            .expect("the track just added is the last of this half");
        if goes_between_two_tracks {
            clear_incoming(&mut later[0]);
        }
        apply(preset, outgoing, &mut later[0]);
    }
    Ok(())
}

/// Carries out `mix add`.
///
/// The settings file is read first, because the settings name the library
/// file when `--library` and the environment do not, and a settings file
/// that cannot be read stops every command that opens the library.
///
/// The mix document is rewritten once, after the track has been built and
/// joined to its neighbors, so a file that cannot be read, a pair of anchors
/// or a transition length this command refuses, and a document that is not
/// valid each leave the document exactly as it was. A file analyzed along
/// the way is different: its record goes into the library as soon as it is
/// analyzed and stays there whatever becomes of this add, because the record
/// is true of the file either way.
pub fn add(args: &AddArgs<'_>) -> Result<(), String> {
    let settings = crate::settings::read()?;
    let preset = preset_of(args.preset, args.bars)?;
    let mut document = read(args.mix)?;
    let at = placement(args.mix, args.position, document.tracks.len())?;
    let path = analyze::canonical(args.file)?;
    let record = record_for(&path, &args.given, args.library, &settings)?;
    let track = build_track(
        path,
        &record,
        &TrackOptions {
            bars: args.bars,
            intro: args.intro,
            outro: args.outro,
            given: &args.given,
            keylock: args.keylock,
            gain: args.gain,
        },
    )?;
    join(&mut document, at, track, preset)?;
    replace(args.mix, &document)?;
    crate::show::print(&crate::show::layout(args.mix, &document), args.json);
    Ok(())
}

/// Carries out `mix move-anchor`.
///
/// Each anchor given is moved as the window moves a dragged anchor, with
/// [`Edit::MoveAnchor`], so the nodes of that anchor's transition go with
/// it and a node placed in the body of the track stays where it sits in the
/// music. The pair the track ends up with must leave the outro anchor after
/// the intro anchor, and the two moves are made in whichever order keeps
/// that true along the way, so `--intro` and `--outro` given together are
/// judged as a pair. Whether the transitions still fit between the new
/// anchors is not checked, as the window does not check it: a track whose
/// span becomes shorter than its transitions plays them overlapping, and
/// `mix show` prints the anchors every track has now. The document is
/// rewritten once, after both moves, so a move that is refused leaves it as
/// it was.
pub fn move_anchor(
    mix: &Path,
    track: usize,
    intro: Option<f64>,
    outro: Option<f64>,
    json: bool,
) -> Result<(), String> {
    let intro = intro.map(|beat| whole_beat("--intro", beat)).transpose()?;
    let outro = outro.map(|beat| whole_beat("--outro", beat)).transpose()?;
    let mut document = read(mix)?;
    let held = document.tracks.len();
    if track < 1 || track > held {
        return Err(no_such_track(mix, track, held));
    }
    let position = track - 1;
    let now = document.tracks[position].anchors;
    let after = Anchors {
        intro: intro.unwrap_or(now.intro),
        outro: outro.unwrap_or(now.outro),
    };
    if after.outro.0 <= after.intro.0 {
        return Err(format!(
            "the outro anchor must be a later beat than the intro anchor: track {track} would have its intro anchor on beat {} and its outro anchor on beat {}",
            after.intro.0, after.outro.0
        ));
    }
    // The outro anchor's move is refused when it lands at or before the
    // intro anchor the track has at that moment, and the intro anchor's move
    // likewise. One of the two orders always keeps the pair in order along
    // the way when the final pair is in order, and this is which.
    let outro_first = after.outro.0 > now.intro.0;
    let moves = [
        (Anchor::Outro, outro, outro_first),
        (Anchor::Intro, intro, !outro_first),
    ];
    let mut ordered: Vec<(Anchor, Beats)> = Vec::new();
    for (anchor, to, first) in moves {
        if let Some(to) = to {
            if first {
                ordered.insert(0, (anchor, to));
            } else {
                ordered.push((anchor, to));
            }
        }
    }
    for (anchor, to) in ordered {
        apply_edit(
            &mut document,
            &Edit::MoveAnchor {
                track: position,
                anchor,
                to,
            },
        )
        .map_err(|problem| problem.to_string())?;
    }
    replace(mix, &document)?;
    crate::show::print(&crate::show::layout(mix, &document), json);
    Ok(())
}

/// Carries out `mix set-gain`.
///
/// The gain replaces the one the track has, and nothing else about the track
/// changes. A gain that is not a finite number and a track number the
/// playlist does not have are both refused before the document is rewritten,
/// so a refused command leaves the document as it was. On success the command
/// prints the mix's timeline as `mix show` does.
pub fn set_gain(mix: &Path, track: usize, db: f64, json: bool) -> Result<(), String> {
    let gain = given_gain("the gain", db)?;
    let mut document = read(mix)?;
    let held = document.tracks.len();
    if track < 1 || track > held {
        return Err(no_such_track(mix, track, held));
    }
    document.tracks[track - 1].gain = gain;
    replace(mix, &document)?;
    crate::show::print(&crate::show::layout(mix, &document), json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::{BeatGrid, Bpm, ContentHash, Decibels, EnvelopeNode, Samples, TempoNode};

    /// The defaults of a track long enough to mix, used where the test does
    /// not care which anchors it starts from.
    fn defaults() -> Defaults {
        Defaults {
            anchors: Anchors {
                intro: Beats::ZERO,
                outro: Beats(64.0),
            },
            outro_source: "which is the last whole beat of the track",
        }
    }

    /// A track with the anchors of the two-track mix the tests build.
    fn a_track() -> Track {
        Track {
            path: PathBuf::from("/music/a.wav"),
            hash: ContentHash([0; 32]),
            length: Samples(882_000),
            grid: BeatGrid {
                first_beat: Samples::ZERO,
                bpm: Bpm(130.0),
            },
            anchors: Anchors {
                intro: Beats::ZERO,
                outro: Beats(64.0),
            },
            keylock: false,
            gain: dermixen_core::Decibels::UNITY,
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        }
    }

    #[test]
    fn anchors_fall_back_to_the_defaults_they_were_given() {
        let settled = settle_anchors(None, None, &defaults()).unwrap();
        assert_eq!(settled.intro, Beats::ZERO);
        assert_eq!(settled.outro, Beats(64.0));
    }

    #[test]
    fn an_anchor_that_is_not_a_whole_beat_is_refused() {
        let message = settle_anchors(Some(3.5), None, &defaults()).unwrap_err();
        assert!(
            message.contains("--intro") && message.contains("whole"),
            "{message}"
        );
        let message = settle_anchors(None, Some(7.25), &defaults()).unwrap_err();
        assert!(
            message.contains("--outro") && message.contains("whole"),
            "{message}"
        );
    }

    #[test]
    fn an_outro_anchor_that_is_not_after_the_intro_anchor_is_refused() {
        for (intro, outro) in [(100.0, 8.0), (16.0, 16.0)] {
            let message = settle_anchors(Some(intro), Some(outro), &defaults()).unwrap_err();
            assert!(message.contains(&format!("{intro}")), "{message}");
            assert!(message.contains(&format!("{outro}")), "{message}");
        }
    }

    #[test]
    fn a_default_outro_anchor_before_the_intro_anchor_is_refused() {
        // A first beat given past the end of the file puts every beat of the
        // file before beat zero, so the last whole beat is a negative one.
        let short = Defaults {
            anchors: Anchors {
                intro: Beats::ZERO,
                outro: Beats(-23.0),
            },
            outro_source: "which is the last whole beat of the track",
        };
        let message = settle_anchors(None, None, &short).unwrap_err();
        assert!(message.contains("-23"), "{message}");
    }

    #[test]
    fn anchors_from_analysis_that_leave_no_room_are_refused_too() {
        // A transition longer than the track has room for moves the outro
        // anchor analysis placed to a beat before the intro anchor, and that
        // pair is refused like any other with no span between its anchors.
        let stretched = Defaults {
            anchors: Anchors {
                intro: Beats::ZERO,
                outro: Beats(-24.0),
            },
            outro_source: "which is where analysis put it",
        };
        let message = settle_anchors(None, None, &stretched).unwrap_err();
        assert!(
            message.contains('0') && message.contains("-24"),
            "{message}"
        );
    }

    #[test]
    fn a_position_counts_from_one_and_may_be_one_past_the_end() {
        let mix = Path::new("set.dmx");
        assert_eq!(placement(mix, None, 3).unwrap(), 3);
        assert_eq!(placement(mix, Some(1), 3).unwrap(), 0);
        assert_eq!(placement(mix, Some(4), 3).unwrap(), 3);
        let message = placement(mix, Some(5), 3).unwrap_err();
        assert!(message.contains('5') && message.contains('3'), "{message}");
        assert!(placement(mix, Some(0), 3).is_err());
    }

    #[test]
    fn a_transition_spans_its_bars_and_a_cut_spans_a_quarter_beat() {
        assert_eq!(span_of(Preset::Blend), Beats(112.0));
        assert_eq!(span_of(Preset::Beatmix { bars: 8 }), Beats(32.0));
        assert_eq!(span_of(Preset::BassSwap { bars: 2 }), Beats(8.0));
        assert_eq!(span_of(Preset::Cut), Beats(0.25));
    }

    #[test]
    fn a_transition_longer_than_the_outgoing_anchor_span_is_refused() {
        let mut track = a_track();
        assert_eq!(track.anchors.span(), Beats(64.0));
        assert!(check_fit(&track, Preset::Beatmix { bars: 16 }).is_ok());
        let message = check_fit(&track, Preset::Beatmix { bars: 24 }).unwrap_err();
        assert!(
            message.contains("96") && message.contains("64") && message.contains("fewer bars"),
            "{message}"
        );
        // The blend's rise is twenty-eight bars whatever --bars says, so the
        // message points at the other presets and the anchors instead.
        let message = check_fit(&track, Preset::Blend).unwrap_err();
        assert!(
            message.contains("112") && message.contains("64") && message.contains("preset"),
            "{message}"
        );
        assert!(!message.contains("fewer bars"), "{message}");
        // A cut covers a quarter beat, so it fits wherever two anchors do.
        track.anchors.outro = Beats(1.0);
        assert!(check_fit(&track, Preset::Cut).is_ok());
        assert!(check_fit(&track, Preset::Beatmix { bars: 1 }).is_err());
    }

    #[test]
    fn clearing_one_side_of_a_track_leaves_the_other_side_alone() {
        let mut track = a_track();
        // A fade in around the intro anchor and a fade out around the outro
        // anchor, of the kind the presets write.
        for at in [0.0, 8.0, 16.0, 64.0, 80.0, 96.0] {
            track
                .volume
                .insert(EnvelopeNode {
                    at: Beats(at),
                    value: Decibels::UNITY,
                })
                .unwrap();
        }
        track.tempo = vec![
            TempoNode {
                at: Beats(16.0),
                bpm: Bpm(130.0),
            },
            TempoNode {
                at: Beats(64.0),
                bpm: Bpm(130.0),
            },
        ];

        let mut outgoing_cleared = track.clone();
        clear_outgoing(&mut outgoing_cleared);
        let left: Vec<f64> = outgoing_cleared
            .volume
            .nodes()
            .iter()
            .map(|node| node.at.0)
            .collect();
        assert_eq!(left, vec![0.0, 8.0, 16.0]);
        assert_eq!(outgoing_cleared.tempo.len(), 1);

        let mut incoming_cleared = track;
        clear_incoming(&mut incoming_cleared);
        let left: Vec<f64> = incoming_cleared
            .volume
            .nodes()
            .iter()
            .map(|node| node.at.0)
            .collect();
        assert_eq!(left, vec![64.0, 80.0, 96.0]);
        assert_eq!(incoming_cleared.tempo.len(), 1);
    }
}
