//! Acceptance tests for rendering a span of a mix and for real-time preview.
//! A coder agent makes these pass without editing them.
//!
//! The promise under test is the one `DESIGN.md` makes: the app never
//! renders something different from what was heard in preview. It is held
//! here by pulling a preview through a capturing output, on a thread of its
//! own as a device would, and comparing the frames with the offline render
//! frame for frame, and for a span of keylocked tracks, whose stretchers
//! cannot be brought to the same state from a run-in, by measuring where
//! the beats land and how loud the music is against the same render. Every
//! mix is synthetic, so each expectation follows from the document rather
//! than from a recording.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Envelope, EqEnvelopes, Mix, Samples, Seconds,
    Track, beatmix,
};
use dermixen_engine::{
    BLOCK_FRAMES, Feed, Output, PlayReport, Progress, RUN_IN, RenderError, Resampler, Source,
    TimeStretcher, play, render, render_range,
};
use dermixen_media::{Audio, Frame};
use dermixen_testkit::synth;

/// The lookahead every preview here runs with: eight blocks of the render.
const LOOKAHEAD: Samples = Samples(8 * 1024);

/// A track whose grid starts at its first sample.
fn track(audio: &Audio, bpm: f64, intro: f64, outro: f64, keylock: bool) -> Track {
    Track {
        path: PathBuf::from("synthetic"),
        hash: ContentHash([0; 32]),
        length: audio.len(),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(bpm),
        },
        anchors: Anchors {
            intro: Beats(intro),
            outro: Beats(outro),
        },
        keylock,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    }
}

/// Thirty seconds of kicks at 130 beats per minute followed by thirty
/// seconds of kicks at 140, joined by a four-bar beatmix: the outgoing outro
/// anchor is beat 48, about twenty-two seconds in, and the incoming intro
/// anchor is beat 8, so the second track enters about eighteen and a half
/// seconds into the mix and the whole mix runs about forty-nine seconds.
fn two_kicks(keylock: bool) -> (Mix, Vec<Audio>) {
    let a = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(30.0));
    let b = synth::kicks(Bpm(140.0), Seconds::ZERO, Seconds(30.0));
    let mut track_a = track(&a, 130.0, 0.0, 48.0, keylock);
    let mut track_b = track(&b, 140.0, 8.0, 68.0, keylock);
    beatmix(&mut track_a, &mut track_b, 4);
    (
        Mix {
            tracks: vec![track_a, track_b],
        },
        vec![a, b],
    )
}

fn resamplers() -> impl FnMut(&Track) -> Box<dyn TimeStretcher> {
    |_| Box::new(Resampler::new())
}

/// What one call of `render_range` delivered: the frames, the progress
/// reports, the tracks the loader was asked for, and the count returned.
struct Span {
    frames: Vec<Frame>,
    progress: Vec<Progress>,
    loaded: Vec<usize>,
    returned: Result<Samples, RenderError>,
}

fn span(mix: &Mix, sources: &[Audio], from: Samples, until: Samples) -> Span {
    let mut frames = Vec::new();
    let mut progress = Vec::new();
    let mut loaded = Vec::new();
    let returned = render_range(
        mix,
        from..until,
        &mut |index, _| {
            loaded.push(index);
            Ok(Box::new(sources[index].clone()) as Box<dyn Source>)
        },
        &mut resamplers(),
        &mut |block| {
            frames.extend_from_slice(block);
            Ok(())
        },
        &mut |report| progress.push(report),
    );
    Span {
        frames,
        progress,
        loaded,
        returned,
    }
}

fn slice(frames: &[Frame], from: Samples, until: Samples) -> &[Frame] {
    &frames[from.0 as usize..until.0 as usize]
}

/// Checks that progress ran from within the span to its end and never went
/// backwards, against the whole mix's length.
fn check_progress(progress: &[Progress], from: Samples, until: Samples, total: Samples) {
    assert!(!progress.is_empty(), "no progress was reported");
    for pair in progress.windows(2) {
        assert!(
            pair[1].written >= pair[0].written,
            "progress went backwards from {:?} to {:?}",
            pair[0],
            pair[1]
        );
    }
    for report in progress {
        assert_eq!(report.total, total);
        assert!(
            report.written >= from && report.written <= until,
            "progress {report:?} is outside the span {from:?} to {until:?}"
        );
    }
    assert_eq!(progress.last().unwrap().written, until);
}

#[test]
fn a_span_is_the_same_frames_as_the_whole_render() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let from = Samples(7 * 44_100 + 123);
    let until = Samples(from.0 + 3 * 44_100 + 77);

    let got = span(&mix, &sources, from, until);
    assert_eq!(got.returned, Ok(until - from));
    assert_eq!(got.frames.len() as i64, (until - from).0);
    assert_eq!(got.frames, slice(&whole.frames, from, until));
    assert_eq!(got.loaded, vec![0]);
    check_progress(&got.progress, from, until, whole.len());
    assert!(
        got.progress[0].written > from,
        "the first report comes after the first block"
    );
}

#[test]
fn a_span_is_clipped_to_the_mix_and_an_empty_span_delivers_nothing() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let length = whole.len();

    let tail = span(&mix, &sources, length - Samples(1000), Samples(i64::MAX));
    assert_eq!(tail.returned, Ok(Samples(1000)));
    assert_eq!(
        tail.frames,
        slice(&whole.frames, length - Samples(1000), length)
    );
    assert_eq!(tail.progress.last().unwrap().written, length);

    let head = span(&mix, &sources, Samples(-500), Samples(1000));
    assert_eq!(head.returned, Ok(Samples(1000)));
    assert_eq!(
        head.frames,
        slice(&whole.frames, Samples::ZERO, Samples(1000))
    );

    let past = span(&mix, &sources, length, length + Samples(5));
    assert_eq!(past.returned, Ok(Samples::ZERO));
    assert!(past.frames.is_empty());
    assert!(
        past.loaded.is_empty(),
        "nothing is loaded for an empty span"
    );
    assert!(past.progress.is_empty());

    let backwards = span(&mix, &sources, Samples(2000), Samples(1000));
    assert_eq!(backwards.returned, Ok(Samples::ZERO));
    assert!(backwards.frames.is_empty());
    assert!(backwards.loaded.is_empty());
}

#[test]
fn a_track_that_has_finished_before_the_span_is_never_loaded() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let from = Samples(35 * 44_100);
    let until = Samples(36 * 44_100);

    let got = span(&mix, &sources, from, until);
    assert_eq!(got.loaded, vec![1], "only the second track is heard here");
    assert_eq!(got.frames, slice(&whole.frames, from, until));
}

/// A resampler that counts the frames of a track it is fed, and keeps the
/// left sample of the first few, so a test can tell how much of the track
/// the render played through it before the first frame it delivered, and
/// where in the track that began.
struct Counting {
    inner: Resampler,
    fed: Arc<AtomicI64>,
    first: Arc<Mutex<Vec<f32>>>,
}

/// How many of the first frames fed a [`Counting`] resampler keeps.
const FIRST_KEPT: usize = 8;

impl TimeStretcher for Counting {
    fn process(&mut self, input: &[Frame], output: &mut [Frame]) {
        self.fed.fetch_add(input.len() as i64, Ordering::SeqCst);
        let mut first = self.first.lock().unwrap();
        for frame in input {
            if first.len() >= FIRST_KEPT {
                break;
            }
            first.push(frame[0]);
        }
        self.inner.process(input, output);
    }

    fn input_latency(&self) -> Samples {
        self.inner.input_latency()
    }

    fn output_latency(&self) -> Samples {
        self.inner.output_latency()
    }

    fn reset(&mut self) {
        self.inner.reset();
    }
}

/// Sixty seconds of white noise as a mix of one track at 130 beats per
/// minute, which plays at its own tempo throughout, so a frame of the mix
/// is a frame of the track. Noise rather than kicks, so that no two frames
/// of the track are alike and the first frames fed to a stretcher say
/// where in the track the feeding began.
fn one_long_track() -> (Mix, Vec<Audio>) {
    let audio = synth::white_noise(7, 0.5, Seconds(60.0));
    let mix = Mix {
        tracks: vec![track(&audio, 130.0, 0.0, 48.0, false)],
    };
    (mix, vec![audio])
}

/// Sixty seconds of kicks at 130 beats per minute followed by sixty seconds
/// of kicks at 140, joined by a four-bar beatmix late in the first track:
/// the outgoing outro anchor is beat 100, about 46 seconds in, and the
/// incoming intro anchor is beat 60, so the second track's audio begins
/// about 18.5 seconds into the mix and holds silence until the beatmix,
/// stretched to the first track's tempo all the while.
fn long_pair() -> (Mix, Vec<Audio>) {
    let a = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(60.0));
    let b = synth::kicks(Bpm(140.0), Seconds::ZERO, Seconds(60.0));
    let mut track_a = track(&a, 130.0, 0.0, 100.0, false);
    let mut track_b = track(&b, 140.0, 60.0, 130.0, false);
    beatmix(&mut track_a, &mut track_b, 4);
    (
        Mix {
            tracks: vec![track_a, track_b],
        },
        vec![a, b],
    )
}

/// What one counting resampler saw: how many frames of its track it had
/// been fed when the first block of the span was delivered, and the left
/// sample of the first [`FIRST_KEPT`] frames it was fed.
struct Fed {
    count: i64,
    first: Vec<f32>,
}

/// Renders a span of a mix through counting resamplers: the frames
/// delivered, and what each stretcher the render made saw, in the order
/// the render made them.
fn span_counting(
    mix: &Mix,
    sources: &[Audio],
    from: Samples,
    until: Samples,
) -> (Vec<Frame>, Vec<Fed>) {
    type Seen = (Arc<AtomicI64>, Arc<Mutex<Vec<f32>>>);
    let counters: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
    let at_first: Arc<Mutex<Option<Vec<i64>>>> = Arc::new(Mutex::new(None));
    let mut frames = Vec::new();
    let made = Arc::clone(&counters);
    let mut stretchers = |_: &Track| {
        let fed = Arc::new(AtomicI64::new(0));
        let first = Arc::new(Mutex::new(Vec::new()));
        made.lock()
            .unwrap()
            .push((Arc::clone(&fed), Arc::clone(&first)));
        Box::new(Counting {
            inner: Resampler::new(),
            fed,
            first,
        }) as Box<dyn TimeStretcher>
    };
    let seen = Arc::clone(&counters);
    let first = Arc::clone(&at_first);
    let mut sink = |block: &[Frame]| {
        let mut first = first.lock().unwrap();
        if first.is_none() {
            *first = Some(
                seen.lock()
                    .unwrap()
                    .iter()
                    .map(|(fed, _)| fed.load(Ordering::SeqCst))
                    .collect(),
            );
        }
        frames.extend_from_slice(block);
        Ok(())
    };
    let returned = render_range(
        mix,
        from..until,
        &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
        &mut stretchers,
        &mut sink,
        &mut |_| {},
    );
    assert_eq!(returned, Ok(until - from));
    let at_first = at_first.lock().unwrap().clone().unwrap_or_default();
    let seen = counters.lock().unwrap();
    let fed = at_first
        .into_iter()
        .zip(seen.iter())
        .map(|(count, (_, first))| Fed {
            count,
            first: first.lock().unwrap().clone(),
        })
        .collect();
    (frames, fed)
}

/// Where the run-in begins, in the frames of a track that plays at its own
/// tempo from output frame zero, for a span starting at `from`, by the rule
/// `render_range` documents: the track's block boundary at or before the
/// frame one `RUN_IN` before the span, or the track's first frame when that
/// is earlier.
fn run_in_start(from: Samples) -> i64 {
    let block = BLOCK_FRAMES as i64;
    ((from.0 - RUN_IN.0).max(0) / block) * block
}

/// How many frames of such a track the render feeds its counting resampler
/// before the first block of a span starting at `from` is delivered, by the
/// same rule: the resampler's priming takes one frame, and the first
/// delivered block makes the track produce its own blocks out to the end of
/// the one that contains the end of the delivered block.
fn expected_fed(from: Samples) -> i64 {
    let block = BLOCK_FRAMES as i64;
    let produced_to = (from.0 + block + block - 1) / block * block;
    1 + produced_to - run_in_start(from)
}

/// Checks what a counting resampler saw against the rule: the count within
/// one block over the count the rule gives, since a render may finish the
/// block it is in, and the first frames fed being the frames of the track
/// from the frame at which the run-in begins. The priming consumes the
/// frames just before the run-in's first block, and a block's frames begin
/// one lead past the block's own frame, so the priming's frames begin at
/// the run-in's frame: one frame for the resampler, whose lead is one, and
/// the first block's frames follow.
fn check_fed(fed: &Fed, audio: &Audio, from: Samples, what: &str) {
    let expected = expected_fed(from);
    assert!(
        fed.count >= expected && fed.count <= expected + BLOCK_FRAMES as i64,
        "{what}: the stretcher had been fed {} frames of the track when the first block \
         was delivered, where the rule gives {expected}",
        fed.count
    );
    let primed_from = run_in_start(from) as usize;
    let expected_first: Vec<f32> = audio.frames[primed_from..primed_from + FIRST_KEPT]
        .iter()
        .map(|frame| frame[0])
        .collect();
    assert_eq!(
        fed.first, expected_first,
        "{what}: the first frames fed to the stretcher are not the track's frames from \
         {primed_from}, where the run-in begins"
    );
}

#[test]
fn a_span_deep_in_a_track_is_brought_up_from_a_run_in() {
    let (mix, sources) = one_long_track();
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();

    // Forty seconds in. Bringing the track up from its first frame would
    // feed the stretcher forty seconds of it before the first block was
    // delivered; the run-in feeds it half a second, from the track's own
    // block boundary at or before that, plus the block being delivered.
    let from = Samples(40 * 44_100 + 123);
    let until = Samples(43 * 44_100);
    let (frames, fed) = span_counting(&mix, &sources, from, until);
    assert_eq!(frames, slice(&whole.frames, from, until));
    assert_eq!(fed.len(), 1);
    check_fed(&fed[0], &sources[0], from, "forty seconds in");

    // The same with keylock on. The stretcher is still the counting
    // resampler, since the render does not choose the stretcher, so the
    // frames are still exact, and the run-in is the same: the rule is one
    // rule for every track, not one for tracks without keylock.
    let mut keylocked = mix.clone();
    keylocked.tracks[0].keylock = true;
    let (frames, fed) = span_counting(&keylocked, &sources, from, until);
    assert_eq!(frames, slice(&whole.frames, from, until));
    assert_eq!(fed.len(), 1);
    check_fed(
        &fed[0],
        &sources[0],
        from,
        "forty seconds in with keylock on",
    );

    // A tenth of a second in, which is less than a run-in, so the track is
    // brought up from its first frame, and nothing beyond the block being
    // delivered is fed.
    let from = Samples(4_410);
    let until = Samples(from.0 + 44_100);
    let (frames, fed) = span_counting(&mix, &sources, from, until);
    assert_eq!(frames, slice(&whole.frames, from, until));
    assert_eq!(fed.len(), 1);
    check_fed(&fed[0], &sources[0], from, "a tenth of a second in");
}

#[test]
fn each_track_sounding_at_the_span_has_a_run_in_of_its_own() {
    // Forty seconds into the pair, the first track plays at its own tempo
    // and the second has been sounding, silently and stretched, for over
    // twenty seconds. Each is brought up from its own run-in, and the two
    // run-ins begin at different output frames, so neither track is fed
    // before its own run-in begins. The first track's count is the rule's
    // exactly. The second track's count is a run-in of its own frames plus
    // what rounding to its own blocks adds. The run-in begins at a block
    // boundary of the track, the first delivery ends at a block boundary
    // of the track, and each block of the track is about 950 of its frames
    // at this tempo, so the count runs to at most three blocks over the
    // run-in.
    let (mix, sources) = long_pair();
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let from = Samples(40 * 44_100 + 123);
    let until = Samples(43 * 44_100);
    let (frames, fed) = span_counting(&mix, &sources, from, until);
    assert_eq!(frames, slice(&whole.frames, from, until));
    assert_eq!(fed.len(), 2, "one stretcher per track sounding at the span");
    check_fed(&fed[0], &sources[0], from, "the first track");
    assert!(
        fed[1].count >= RUN_IN.0 - 2 && fed[1].count <= RUN_IN.0 + 3 * BLOCK_FRAMES as i64,
        "the second track's stretcher had been fed {} frames of the track when the first block \
         was delivered, where a run-in of {} frames and at most three blocks was expected",
        fed[1].count,
        RUN_IN.0
    );
}

/// Two tracks of steady tones, 440 hertz for thirty seconds and 660 hertz
/// for thirty seconds, laid out exactly as [`two_kicks`] lays out its
/// kicks, so the same four-bar beatmix stretches both.
#[cfg(feature = "signalsmith")]
fn two_tones(keylock: bool) -> (Mix, Vec<Audio>) {
    let a = synth::sine(440.0, 0.5, Seconds(30.0));
    let b = synth::sine(660.0, 0.5, Seconds(30.0));
    let mut track_a = track(&a, 130.0, 0.0, 48.0, keylock);
    let mut track_b = track(&b, 140.0, 8.0, 68.0, keylock);
    beatmix(&mut track_a, &mut track_b, 4);
    (
        Mix {
            tracks: vec![track_a, track_b],
        },
        vec![a, b],
    )
}

/// The level of the left channel of `frames` in decibels relative to full
/// scale, and minus two hundred for silence.
#[cfg(feature = "signalsmith")]
fn level_db(frames: &[Frame]) -> f64 {
    let power = frames
        .iter()
        .map(|frame| f64::from(frame[0]) * f64::from(frame[0]))
        .sum::<f64>()
        / frames.len() as f64;
    10.0 * (power + 1e-20).log10()
}

/// The loudest frame of the left channel within `span` of `frames`, and
/// its level in decibels relative to full scale.
#[cfg(feature = "signalsmith")]
fn loudest_within(frames: &[Frame], span: std::ops::Range<usize>) -> (usize, f64) {
    let span = span.start.min(frames.len())..span.end.min(frames.len());
    let at = span
        .clone()
        .max_by(|a, b| frames[*a][0].abs().total_cmp(&frames[*b][0].abs()))
        .expect("a span with frames in it");
    let level = f64::from(frames[at][0].abs());
    (at, 20.0 * (level + 1e-10).log10())
}

/// Where the kicks land in `frames` that no other kick comes near: every
/// frame louder than a tenth of full scale that is the loudest within fifty
/// milliseconds either side and that nothing outside ten milliseconds
/// either side comes within three decibels of.
#[cfg(feature = "signalsmith")]
fn lone_kicks(frames: &[Frame]) -> Vec<usize> {
    let reach = 2_205;
    let own = 441;
    let mut peaks = Vec::new();
    let mut at = reach;
    while at + reach < frames.len() {
        let level = frames[at][0].abs();
        if level <= 0.1 {
            at += 1;
            continue;
        }
        let window = at - reach..at + reach + 1;
        let (loudest, _) = loudest_within(frames, window.clone());
        if loudest != at {
            at += 1;
            continue;
        }
        let lone = window
            .filter(|j| j.abs_diff(at) > own)
            .all(|j| frames[j][0].abs() < 0.7 * level);
        if lone {
            peaks.push(at);
        }
        at += reach;
    }
    peaks
}

/// The frequency in hertz of the strongest bin of the left channel's
/// spectrum between `low_hz` and `high_hz`.
#[cfg(feature = "signalsmith")]
fn strongest_hz(frames: &[Frame], low_hz: f64, high_hz: f64) -> f64 {
    use dermixen_testkit::spectrum::{bin_width, power_spectrum};

    let power = power_spectrum(frames, 0).expect("at least one window of frames");
    let first = (low_hz / bin_width()).ceil() as usize;
    let last = ((high_hz / bin_width()).floor() as usize).min(power.len() - 1);
    let strongest = (first..=last)
        .max_by(|a, b| power[*a].total_cmp(&power[*b]))
        .expect("a band with bins in it");
    strongest as f64 * bin_width()
}

/// Renders a span of `mix` through pitch-preserving stretchers.
#[cfg(feature = "signalsmith")]
fn span_keylocked(mix: &Mix, sources: &[Audio], from: Samples, until: Samples) -> Vec<Frame> {
    use dermixen_engine::SignalsmithStretcher;

    let mut frames = Vec::new();
    let returned = render_range(
        mix,
        from..until,
        &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
        &mut |_: &Track| Box::new(SignalsmithStretcher::new()) as Box<dyn TimeStretcher>,
        &mut |block| {
            frames.extend_from_slice(block);
            Ok(())
        },
        &mut |_| {},
    );
    assert_eq!(returned, Ok(until - from));
    frames
}

#[cfg(feature = "signalsmith")]
#[test]
fn a_span_of_keylocked_tracks_is_the_same_music() {
    use dermixen_engine::SignalsmithStretcher;
    use dermixen_testkit::spectrum::band_level_db;

    // The pitch-preserving stretcher accumulates phase from everything it
    // has been fed, so a span brought up from a run-in is not the whole
    // render sample for sample. It is the same music: every kick lands
    // within a millisecond of where the whole render puts it, at the same
    // level, and a steady tone comes through at the same pitch and level
    // from the first quarter second on.
    let mut stretchers =
        |_: &Track| -> Box<dyn TimeStretcher> { Box::new(SignalsmithStretcher::new()) };
    let from = Samples(20 * 44_100 + 123);
    let until = Samples(24 * 44_100);

    // Kicks: the first track's outro anchor is at 22.15 seconds and the
    // second track has been sounding since 18.46, so this span holds the
    // start of the beatmix. The second track is stretched throughout the
    // span, to the first track's tempo and then along the ramp, and the
    // first track from 22.15 seconds on.
    let (mix, sources) = two_kicks(true);
    let whole = render(&mix, &sources, &mut stretchers).unwrap();
    let expected = slice(&whole.frames, from, until);
    let got = span_keylocked(&mix, &sources, from, until);
    assert_eq!(got.len(), expected.len());
    let kicks = lone_kicks(expected);
    assert!(
        kicks.len() >= 6,
        "the whole render has {} kicks standing alone in the span, too few to judge by",
        kicks.len()
    );
    for at in kicks {
        let (_, level) = loudest_within(expected, at..at + 1);
        let (found, found_level) = loudest_within(&got, at.saturating_sub(2_205)..at + 2_206);
        assert!(
            found.abs_diff(at) <= 44,
            "the kick at frame {at} of the whole render lands at frame {found} of the span, \
             {} frames away, where a millisecond is 44",
            found.abs_diff(at)
        );
        assert!(
            (found_level - level).abs() <= 1.0,
            "the kick at frame {at} peaks at {level:.2} dB in the whole render and \
             {found_level:.2} dB in the span"
        );
    }
    for (index, (theirs, ours)) in expected.chunks(4_410).zip(got.chunks(4_410)).enumerate() {
        let (theirs, ours) = (level_db(theirs), level_db(ours));
        if theirs > -30.0 {
            assert!(
                (theirs - ours).abs() <= 2.0,
                "tenth of a second {index}: {theirs:.2} dB in the whole render and {ours:.2} dB \
                 in the span"
            );
        }
    }

    // Tones: the 660 hertz track holds silence until the beatmix at 22.15
    // seconds and then fades in, stretched along the ramp, while the 440
    // hertz track fades out, stretched along the ramp from the same moment.
    // Both keep their pitch, so in every quarter second of the span, the
    // first included, the level overall and the level of the band around
    // each tone are the whole render's within half a decibel, and over the
    // whole span the strongest frequency in each band is the whole
    // render's within ten hertz. The bands are sixty hertz either side of
    // the tone, wide enough that the stretcher's leakage into the bins
    // beside the tone stays inside them. A tone that moved with the speed,
    // as it would through a resampler, would keep its band level and move
    // its strongest frequency by nearly forty hertz.
    let (mix, sources) = two_tones(true);
    let whole = render(&mix, &sources, &mut stretchers).unwrap();
    let expected = slice(&whole.frames, from, until);
    let got = span_keylocked(&mix, &sources, from, until);
    assert_eq!(got.len(), expected.len());
    let bands = [("440", 380.0, 500.0), ("660", 600.0, 720.0)];
    for (index, (theirs, ours)) in expected
        .as_chunks::<11_025>()
        .0
        .iter()
        .zip(got.as_chunks::<11_025>().0)
        .enumerate()
    {
        for (name, low, high) in bands {
            let theirs_band = band_level_db(theirs, 0, low, high);
            let ours_band = band_level_db(ours, 0, low, high);
            assert!(
                (theirs_band - ours_band).abs() <= 0.5,
                "quarter second {index}, the band around {name} hertz: {theirs_band:.2} dB in the \
                 whole render and {ours_band:.2} dB in the span"
            );
        }
        let (theirs, ours) = (level_db(theirs), level_db(ours));
        assert!(
            (theirs - ours).abs() <= 0.5,
            "quarter second {index}: {theirs:.2} dB in the whole render and {ours:.2} dB in the \
             span"
        );
    }
    for (name, low, high) in bands {
        let theirs = strongest_hz(expected, low, high);
        let ours = strongest_hz(&got, low, high);
        assert!(
            (theirs - ours).abs() <= 10.0,
            "the band around {name} hertz is strongest at {theirs:.1} hertz in the whole render \
             and {ours:.1} hertz in the span"
        );
    }
}

/// An output that stands in for a device: it pulls from the feed on a
/// thread of its own, in awkward sizes, waiting for frames as a device
/// would by only asking when the feed has them, and keeps everything it
/// pulled.
struct Capture {
    /// Every frame pulled, in order, without the silence a device would add.
    frames: Arc<Mutex<Vec<Frame>>>,
    /// How many frames the feed held at the thread's first look.
    first_available: Arc<Mutex<Option<usize>>>,
    /// The first pull that got fewer frames than it asked for, as the count
    /// asked for and the count got.
    short_pull: Arc<Mutex<Option<(usize, usize)>>>,
    /// When set, once this many frames have been pulled the thread waits
    /// for the feed to be full and then asks for more than it can hold,
    /// once, as a device that fell behind would.
    hurry_after: Option<usize>,
    /// When set, `start` refuses as a missing device would.
    refuse: bool,
    /// When set, once this many frames have been pulled the thread tells
    /// the feed the device has failed and pulls no more, as a device that
    /// was unplugged would.
    fail_after: Option<usize>,
    started: bool,
    stopped: bool,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    fn new(hurry_after: Option<usize>) -> Self {
        Capture {
            frames: Arc::new(Mutex::new(Vec::new())),
            first_available: Arc::new(Mutex::new(None)),
            short_pull: Arc::new(Mutex::new(None)),
            hurry_after,
            refuse: false,
            fail_after: None,
            started: false,
            stopped: false,
            thread: None,
        }
    }

    fn frames(&self) -> Vec<Frame> {
        self.frames.lock().unwrap().clone()
    }
}

/// Waits until the feed holds at least `wanted` frames or has ended. A
/// feed that does neither within ten seconds is a stalled preview, and the
/// wait panics rather than hanging the test run.
fn wait_for(feed: &Feed, wanted: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while feed.available() < wanted && !feed.ended() {
        assert!(
            Instant::now() < deadline,
            "waited ten seconds for {wanted} frames; the feed holds {} and has not ended",
            feed.available()
        );
        std::thread::sleep(Duration::from_micros(100));
    }
}

impl Output for Capture {
    fn start(&mut self, mut feed: Feed) -> Result<(), String> {
        if self.refuse {
            return Err("no device".to_owned());
        }
        self.started = true;
        let frames = Arc::clone(&self.frames);
        let first_available = Arc::clone(&self.first_available);
        let short_pull = Arc::clone(&self.short_pull);
        let hurry_after = self.hurry_after;
        let fail_after = self.fail_after;
        self.thread = Some(std::thread::spawn(move || {
            *first_available.lock().unwrap() = Some(feed.available());
            let sizes = [700usize, 1000, 333, 1024, 1];
            let mut turn = 0;
            let mut hurried = false;
            loop {
                if feed.finished() {
                    break;
                }
                let pulled = frames.lock().unwrap().len();
                if fail_after.is_some_and(|after| pulled >= after) {
                    feed.fail("the device was unplugged");
                    break;
                }
                let wanted = if !hurried && hurry_after.is_some_and(|after| pulled >= after) {
                    hurried = true;
                    wait_for(&feed, LOOKAHEAD.0 as usize);
                    LOOKAHEAD.0 as usize + 500
                } else {
                    let wanted = sizes[turn % sizes.len()];
                    turn += 1;
                    wait_for(&feed, wanted);
                    wanted
                };
                let mut out = vec![[0.0f32; 2]; wanted];
                let got = feed.pull(&mut out);
                if got < wanted {
                    short_pull.lock().unwrap().get_or_insert((wanted, got));
                }
                frames.lock().unwrap().extend_from_slice(&out[..got]);
            }
        }));
        Ok(())
    }

    fn stop(&mut self) {
        self.stopped = true;
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

/// What one preview did: the report, the progress reports, and how many
/// frames the output had pulled at the moment each track was loaded.
struct Played {
    report: Result<PlayReport, RenderError>,
    progress: Vec<Progress>,
    pulled_at_load: Vec<usize>,
}

/// Plays a span of the mix through a capture.
fn preview(
    mix: &Mix,
    sources: &[Audio],
    from: Samples,
    until: Samples,
    capture: &mut Capture,
) -> Played {
    let mut progress = Vec::new();
    let mut pulled_at_load = Vec::new();
    let pulled = Arc::clone(&capture.frames);
    let report = play(
        mix,
        from..until,
        &mut |index, _| {
            pulled_at_load.push(pulled.lock().unwrap().len());
            Ok(Box::new(sources[index].clone()) as Box<dyn Source>)
        },
        &mut resamplers(),
        capture,
        LOOKAHEAD,
        &mut |report| progress.push(report),
    );
    Played {
        report,
        progress,
        pulled_at_load,
    }
}

#[test]
fn the_preview_is_the_render_frame_for_frame() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let from = Samples(15 * 44_100 + 321);
    let until = Samples(from.0 + 12 * 44_100 + 5);

    let mut capture = Capture::new(None);
    let played = preview(&mix, &sources, from, until, &mut capture);
    assert_eq!(
        played.report,
        Ok(PlayReport {
            played: until - from,
            underruns: 0
        })
    );
    assert!(capture.started);
    assert!(capture.stopped, "the output is stopped before play returns");
    assert_eq!(capture.frames(), slice(&whole.frames, from, until));
    assert_eq!(
        *capture.first_available.lock().unwrap(),
        Some(LOOKAHEAD.0 as usize),
        "the feed holds one lookahead when the output starts"
    );
    check_progress(&played.progress, from, until, whole.len());
    assert_eq!(
        played.progress[0].written, from,
        "the first report comes before the output has pulled anything, so it says the span's start"
    );
    // The second track enters about three and a half seconds into this span,
    // far past one lookahead, so a preview that renders alongside the output
    // loads it only after the output has pulled a good deal of the first
    // track; a preview that rendered the whole span before starting the
    // output would load it while nothing had been pulled at all.
    assert_eq!(played.pulled_at_load.len(), 2);
    assert_eq!(played.pulled_at_load[0], 0);
    assert!(
        played.pulled_at_load[1] > 2 * 44_100,
        "the second track was loaded after only {} frames had been pulled",
        played.pulled_at_load[1]
    );
}

#[test]
fn a_late_device_hears_silence_and_the_underrun_is_counted() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let from = Samples(10 * 44_100);
    let until = Samples(14 * 44_100);

    let mut capture = Capture::new(Some(44_100));
    let played = preview(&mix, &sources, from, until, &mut capture);
    assert_eq!(
        played.report,
        Ok(PlayReport {
            played: until - from,
            underruns: 1
        })
    );
    assert_eq!(
        *capture.short_pull.lock().unwrap(),
        Some((LOOKAHEAD.0 as usize + 500, LOOKAHEAD.0 as usize)),
        "the hurried pull got what the feed held and no more"
    );
    assert_eq!(
        capture.frames(),
        slice(&whole.frames, from, until),
        "no frame is lost across an underrun"
    );
}

#[test]
fn a_whole_mix_previews_from_its_first_frame_to_its_last() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();

    let mut capture = Capture::new(None);
    let played = preview(
        &mix,
        &sources,
        Samples::ZERO,
        Samples(i64::MAX),
        &mut capture,
    );
    assert_eq!(
        played.report,
        Ok(PlayReport {
            played: whole.len(),
            underruns: 0
        })
    );
    assert_eq!(capture.frames(), whole.frames);
    check_progress(&played.progress, Samples::ZERO, whole.len(), whole.len());
    assert_eq!(played.progress[0].written, Samples::ZERO);
}

#[test]
fn a_span_past_the_end_plays_nothing_and_never_starts_the_output() {
    let (mix, sources) = two_kicks(false);
    let length = render(&mix, &sources, &mut resamplers()).unwrap().len();

    let mut capture = Capture::new(None);
    let played = preview(
        &mix,
        &sources,
        length + Samples(10),
        length + Samples(20),
        &mut capture,
    );
    assert_eq!(
        played.report,
        Ok(PlayReport {
            played: Samples::ZERO,
            underruns: 0
        })
    );
    assert!(!capture.started);
    assert!(played.progress.is_empty());
    assert!(
        played.pulled_at_load.is_empty(),
        "nothing is loaded for an empty span"
    );
}

#[test]
fn a_track_that_cannot_be_loaded_stops_the_preview_and_the_output() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();

    let mut capture = Capture::new(None);
    let mut progress = Vec::new();
    let report = play(
        &mix,
        Samples::ZERO..Samples(i64::MAX),
        &mut |index, _| {
            if index == 1 {
                Err("gone".to_owned())
            } else {
                Ok(Box::new(sources[index].clone()) as Box<dyn Source>)
            }
        },
        &mut resamplers(),
        &mut capture,
        LOOKAHEAD,
        &mut |report| progress.push(report),
    );
    assert_eq!(
        report,
        Err(RenderError::Load {
            index: 1,
            message: "gone".to_owned()
        })
    );
    assert!(capture.stopped, "the output is stopped after an error");
    let heard = capture.frames();
    assert!(
        !heard.is_empty(),
        "the first track was heard before the second failed to load"
    );
    assert!(
        whole.frames.starts_with(&heard),
        "what was heard before the error is the start of the render"
    );
}

#[test]
fn a_device_that_stops_taking_frames_ends_the_preview_with_its_message() {
    let (mix, sources) = two_kicks(false);
    let whole = render(&mix, &sources, &mut resamplers()).unwrap();
    let mut capture = Capture::new(None);
    capture.fail_after = Some(44_100);
    let heard = Arc::clone(&capture.frames);

    // The preview runs on its own thread here so that a play that never
    // returns fails the test instead of hanging it.
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let played = preview(
            &mix,
            &sources,
            Samples::ZERO,
            Samples(i64::MAX),
            &mut capture,
        );
        done.send((played.report, capture.stopped)).unwrap();
    });
    let (report, stopped) = finished
        .recv_timeout(Duration::from_secs(10))
        .expect("play returns within ten seconds of the device failing");
    assert_eq!(
        report,
        Err(RenderError::Output("the device was unplugged".to_owned()))
    );
    assert!(stopped, "the output is stopped after the device fails");
    let heard = heard.lock().unwrap();
    assert!(heard.len() >= 44_100);
    assert!(whole.frames.starts_with(&heard));
}

#[test]
fn an_output_that_cannot_start_ends_the_preview_with_its_message() {
    let (mix, sources) = two_kicks(false);
    let mut capture = Capture::new(None);
    capture.refuse = true;
    let played = preview(&mix, &sources, Samples::ZERO, Samples(44_100), &mut capture);
    assert_eq!(
        played.report,
        Err(RenderError::Output("no device".to_owned()))
    );
    assert!(!capture.started);
}

#[test]
#[ignore = "engine-guards"]
fn a_device_is_handed_only_samples_it_can_play() {
    use dermixen_engine::device_sample;
    for (given, handed) in [
        (0.0, 0.0),
        (0.5, 0.5),
        (-1.0, -1.0),
        (1.0, 1.0),
        (1.5, 1.0),
        (-8.0, -1.0),
        (1e30, 1.0),
        (f32::INFINITY, 0.0),
        (f32::NEG_INFINITY, 0.0),
        (f32::NAN, 0.0),
    ] {
        assert_eq!(device_sample(given), handed, "{given}");
    }
}
