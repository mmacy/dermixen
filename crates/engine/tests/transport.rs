//! Acceptance tests for transport control. A coder agent makes these pass without
//! editing them.
//!
//! The promise under test is the one `DESIGN.md` makes for the preview,
//! held while a person pauses, moves, and edits: every frame the device
//! pulls is the frame the offline render delivers for the document the
//! transport holds at that moment, at that output position. It is held
//! here by an output that records every pull with the position the feed
//! reports for it, and by comparing each pull with a render of the same
//! document at that position. Every mix is synthetic, so each expectation
//! follows from the document rather than from a recording.

use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Envelope, EnvelopeNode, EqEnvelopes, Mix,
    SAMPLE_RATE, Samples, Seconds, Track, beatmix,
};
use dermixen_engine::{
    Feed, Output, RUN_IN, RenderError, Resampler, SendLoader, SendStretchers, Source,
    TimeStretcher, Transport, TransportState, TransportStatus, mix_length, render,
};
use dermixen_media::{Audio, Frame};
use dermixen_testkit::synth;

/// The lookahead every transport here runs with: thirty-two blocks of the
/// render, about three quarters of a second, so that a render thread held
/// off the processor while the suite runs in parallel does not show as an
/// underrun.
const LOOKAHEAD: Samples = Samples(32 * 1024);

/// How long a test waits for the transport to reach a state or a position
/// before giving up.
const PATIENCE: Duration = Duration::from_secs(20);

/// A track whose grid starts at its first sample.
fn track(audio: &Audio, bpm: f64, intro: f64, outro: f64) -> Track {
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
        keylock: false,
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
fn two_kicks() -> (Mix, Vec<Audio>) {
    let a = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(30.0));
    let b = synth::kicks(Bpm(140.0), Seconds::ZERO, Seconds(30.0));
    let mut track_a = track(&a, 130.0, 0.0, 48.0);
    let mut track_b = track(&b, 140.0, 8.0, 68.0);
    beatmix(&mut track_a, &mut track_b, 4);
    (
        Mix {
            tracks: vec![track_a, track_b],
        },
        vec![a, b],
    )
}

/// The same two tracks with the first one held six decibels down until
/// beat forty, eight beats before its fade begins, which is the kind of
/// edit a person makes while the mix plays: every kick before beat forty
/// is six decibels quieter than in the original, and the frames between
/// kicks are silent in both.
fn two_kicks_quieter() -> Mix {
    let (mut mix, _) = two_kicks();
    for at in [-1000.0, 40.0] {
        mix.tracks[0]
            .volume
            .insert(EnvelopeNode {
                at: Beats(at),
                value: Decibels(-6.0),
            })
            .unwrap();
    }
    mix
}

fn resamplers() -> SendStretchers {
    Box::new(|_| Box::new(Resampler::new()) as Box<dyn TimeStretcher>)
}

/// A loader over the synthetic sources that counts how often each track was
/// asked for.
fn loader(sources: Vec<Audio>, asked: Arc<Mutex<Vec<usize>>>) -> SendLoader {
    Box::new(move |index, _| {
        asked.lock().unwrap().push(index);
        Ok(Box::new(sources[index].clone()) as Box<dyn Source>)
    })
}

/// A loader that refuses the second track.
fn refusing_loader(sources: Vec<Audio>) -> SendLoader {
    Box::new(move |index, _| {
        if index == 1 {
            Err("the file is gone".to_owned())
        } else {
            Ok(Box::new(sources[index].clone()) as Box<dyn Source>)
        }
    })
}

/// The whole offline render of `mix`, which every pull is compared against.
fn whole(mix: &Mix, sources: &[Audio]) -> Vec<Frame> {
    render(mix, sources, &mut |_| {
        Box::new(Resampler::new()) as Box<dyn TimeStretcher>
    })
    .unwrap()
    .frames
}

/// One pull the recorder made: where the feed said the frames sit, and the
/// frames. A pull that got nothing is recorded too, with no frames.
#[derive(Clone)]
struct Pull {
    position: Samples,
    frames: Vec<Frame>,
}

/// How a recorder behaves, chosen per test.
#[derive(Clone, Copy)]
enum Pace {
    /// Pulls whatever is ready, as fast as it can, so the recording is
    /// exact and never waits on the render.
    Greedy,
    /// Pulls at real time, as an audio callback does, so that the silence of
    /// buffering and pausing is seen as pulls that got nothing. Each pull is
    /// sized from the clock rather than fixed at a device-sized 1024 frames,
    /// because a device takes frames at the sample rate however busy the
    /// machine is: a pulling thread woken late owes it every frame it slept
    /// through. A fixed block per sleep falls behind real time on a loaded
    /// machine and never catches up, which is what a continuous integration
    /// runner does to it.
    RealTime,
}

/// An output that records every pull with its position, on a thread of its
/// own as a device would.
struct Recorder {
    pace: Pace,
    /// Whether `start` should refuse.
    refuse: bool,
    /// Whether the thread should stop pulling altogether after this many
    /// frames, saying nothing, as a device that has gone quiet does.
    quiet_after: Option<usize>,
    /// Whether the thread should tell the feed the device failed after this
    /// many frames.
    fail_after: Option<usize>,
    pulls: Arc<Mutex<Vec<Pull>>>,
    stopped: Arc<Mutex<bool>>,
    thread: Option<JoinHandle<()>>,
    started: bool,
}

impl Recorder {
    fn new(pace: Pace) -> Recorder {
        Recorder {
            pace,
            refuse: false,
            quiet_after: None,
            fail_after: None,
            pulls: Arc::new(Mutex::new(Vec::new())),
            stopped: Arc::new(Mutex::new(false)),
            thread: None,
            started: false,
        }
    }

    /// The pulls so far, and a handle to read them while the transport runs.
    fn pulls(&self) -> Arc<Mutex<Vec<Pull>>> {
        Arc::clone(&self.pulls)
    }

    fn stopped(&self) -> Arc<Mutex<bool>> {
        Arc::clone(&self.stopped)
    }
}

/// Every frame in the pulls, in order.
fn frames_of(pulls: &[Pull]) -> Vec<Frame> {
    pulls
        .iter()
        .flat_map(|pull| pull.frames.iter().copied())
        .collect()
}

/// Checks that every pull that got frames is the render at the position the
/// feed reported for it, given the whole render of the document the
/// transport held at the time.
fn each_pull_is_the_render(pulls: &[Pull], whole: &[Frame]) {
    for (index, pull) in pulls.iter().enumerate() {
        if pull.frames.is_empty() {
            continue;
        }
        let from = pull.position.0 as usize;
        let until = from + pull.frames.len();
        assert!(
            until <= whole.len(),
            "pull {index} at {from} holding {} frames runs past the render's {} frames",
            pull.frames.len(),
            whole.len()
        );
        assert!(
            pull.frames == whole[from..until],
            "pull {index} at {from} holding {} frames is not the render there",
            pull.frames.len()
        );
    }
}

/// Checks that the pulls that got frames follow one another without a gap
/// or a repeat, from `from` on: each starts where the previous one ended.
fn contiguous_from(pulls: &[Pull], from: Samples) -> Samples {
    let mut next = from;
    for pull in pulls.iter().filter(|pull| !pull.frames.is_empty()) {
        assert_eq!(
            pull.position, next,
            "a pull began at {} where {} was expected",
            pull.position.0, next.0
        );
        next = Samples(next.0 + pull.frames.len() as i64);
    }
    next
}

impl Output for Recorder {
    fn start(&mut self, mut feed: Feed) -> Result<(), String> {
        if self.refuse {
            return Err("no device".to_owned());
        }
        self.started = true;
        let pulls = Arc::clone(&self.pulls);
        let stopped = Arc::clone(&self.stopped);
        let pace = self.pace;
        let quiet_after = self.quiet_after;
        let fail_after = self.fail_after;
        self.thread = Some(std::thread::spawn(move || {
            let mut recorded = 0usize;
            // Where the real-time clock started, and how many frames have gone
            // out against it. Both restart whenever a pull comes back empty,
            // so time spent paused or waiting on the render does not build up
            // a debt that arrives as one burst when frames flow again.
            let mut since = Instant::now();
            let mut against_clock = 0i64;
            loop {
                if *stopped.lock().unwrap() {
                    break;
                }
                if quiet_after.is_some_and(|after| recorded >= after) {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                if fail_after.is_some_and(|after| recorded >= after) {
                    feed.fail("the device was unplugged");
                    break;
                }
                let wanted = match pace {
                    Pace::Greedy => feed.available().max(1),
                    Pace::RealTime => {
                        let due = (since.elapsed().as_secs_f64() * f64::from(SAMPLE_RATE)) as i64;
                        // At most a third of a second in one pull, well inside
                        // the lookahead, so catching up never asks the feed for
                        // more than it has had time to render.
                        (due - against_clock).clamp(1, 16 * 1024) as usize
                    }
                };
                let mut out = vec![[0.0f32; 2]; wanted];
                let (position, got) = feed.pull_from(&mut out);
                out.truncate(got);
                recorded += got;
                pulls.lock().unwrap().push(Pull {
                    position,
                    frames: out,
                });
                match pace {
                    Pace::Greedy => {
                        if got == 0 {
                            std::thread::sleep(Duration::from_micros(200));
                        }
                    }
                    Pace::RealTime => {
                        if got == 0 {
                            since = Instant::now();
                            against_clock = 0;
                        } else {
                            against_clock += got as i64;
                        }
                        std::thread::sleep(Duration::from_micros(23_220));
                    }
                }
            }
        }));
        Ok(())
    }

    fn stop(&mut self) {
        *self.stopped.lock().unwrap() = true;
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

/// A transport under test, with the handles the test reads while it runs.
struct Started {
    transport: Transport,
    /// The recorder's pulls.
    pulls: Arc<Mutex<Vec<Pull>>>,
    /// The tracks the loader was asked for, in order.
    asked: Arc<Mutex<Vec<usize>>>,
    /// Whether the recorder has been stopped.
    stopped: Arc<Mutex<bool>>,
}

/// Starts a transport over `mix` from `from` with a recorder.
fn start(mix: &Mix, sources: &[Audio], from: Samples, recorder: Recorder) -> Started {
    let pulls = recorder.pulls();
    let stopped = recorder.stopped();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let transport = Transport::start(
        mix.clone(),
        from,
        loader(sources.to_vec(), Arc::clone(&asked)),
        resamplers(),
        Box::new(recorder),
        LOOKAHEAD,
    )
    .unwrap();
    Started {
        transport,
        pulls,
        asked,
        stopped,
    }
}

/// Waits until the status satisfies `done`, failing after the patience runs out.
fn wait_until(transport: &Transport, what: &str, done: impl Fn(&TransportStatus) -> bool) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let status = transport.status();
        if done(&status) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "waited {} seconds for {what}; the status is {status:?}",
            PATIENCE.as_secs()
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn playing_from_a_position_is_the_render_frame_for_frame() {
    let (mix, sources) = two_kicks();
    let length = mix_length(&mix);
    let from = Samples(15 * 44_100 + 321);
    let Started {
        transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::Greedy));

    let first = transport.status();
    assert_eq!(
        first.position, from,
        "the position is the start before anything is pulled"
    );
    assert_eq!(first.length, length);
    assert!(
        matches!(
            first.state,
            TransportState::Buffering | TransportState::Playing
        ),
        "{first:?}"
    );

    wait_until(&transport, "the end", |status| {
        status.state == TransportState::Ended
    });
    let last = transport.stop();
    assert_eq!(last.state, TransportState::Ended);
    assert_eq!(last.position, length);
    assert_eq!(last.underruns, 0);

    let pulls = pulls.lock().unwrap().clone();
    let expected = whole(&mix, &sources);
    each_pull_is_the_render(&pulls, &expected);
    assert_eq!(contiguous_from(&pulls, from), length);
    assert_eq!(
        frames_of(&pulls),
        expected[from.0 as usize..],
        "the frames heard are the render from the start position to the end"
    );
    // Each track is loaded once, when the render first reaches it. That a
    // transport renders alongside the device rather than all at once is
    // held by the replacement test below, whose loader log shows the second
    // track was never reached in the first second of play.
    assert_eq!(*asked.lock().unwrap(), vec![0, 1]);
}

#[test]
fn the_position_follows_the_device_and_never_goes_backwards() {
    let (mix, sources) = two_kicks();
    let from = Samples(5 * 44_100);
    let Started {
        transport, pulls, ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));

    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    let mut seen = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1200) {
        seen.push(transport.status().position);
        std::thread::sleep(Duration::from_millis(10));
    }
    let last = transport.stop();
    for pair in seen.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "the position went back from {:?} to {:?}",
            pair[0],
            pair[1]
        );
    }
    let pulled: i64 = pulls
        .lock()
        .unwrap()
        .iter()
        .map(|pull| pull.frames.len() as i64)
        .sum();
    assert_eq!(
        last.position,
        Samples(from.0 + pulled),
        "the position is the frame just past the last one pulled"
    );
    let advanced = seen.last().unwrap().0 - seen.first().unwrap().0;
    assert!(
        (30_000..=80_000).contains(&advanced),
        "in about 1.2 seconds of real time the position moved {advanced} frames"
    );
    assert_eq!(
        last.underruns, 0,
        "a device at real time on a synthetic mix never runs dry"
    );
}

#[test]
fn pausing_holds_the_position_and_resuming_loses_no_frame() {
    let (mix, sources) = two_kicks();
    let from = Samples(10 * 44_100);
    let Started {
        mut transport,
        pulls,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));

    wait_until(&transport, "half a second of play", |status| {
        status.position.0 >= from.0 + 22_050
    });
    transport.pause();
    let paused = transport.status();
    assert_eq!(paused.state, TransportState::Paused);
    // The position moves as a pull takes its frames, and the recorder lists
    // the pull only afterwards, so the snapshot waits for the recorder to
    // settle before the half second of silence is measured.
    std::thread::sleep(Duration::from_millis(50));
    let pulled_at_pause = frames_of(&pulls.lock().unwrap()).len();
    std::thread::sleep(Duration::from_millis(500));
    let still = transport.status();
    assert_eq!(still, paused, "nothing changes while paused");
    let pulls_while_paused = pulls.lock().unwrap().len();
    assert_eq!(
        frames_of(&pulls.lock().unwrap()).len(),
        pulled_at_pause,
        "no frame is pulled while paused"
    );

    transport.resume();
    wait_until(&transport, "a second more of play", |status| {
        status.position.0 >= paused.position.0 + 44_100
    });
    let last = transport.stop();
    assert_eq!(
        last.underruns, 0,
        "the silence of a pause is not an underrun"
    );
    let pulls = pulls.lock().unwrap().clone();
    assert!(
        pulls.len() > pulls_while_paused,
        "the device kept asking while paused and got nothing"
    );
    each_pull_is_the_render(&pulls, &whole(&mix, &sources));
    assert_eq!(
        contiguous_from(&pulls, from),
        last.position,
        "no frame is lost or repeated across the pause"
    );
}

#[test]
fn a_move_while_paused_plays_the_render_from_the_new_position() {
    let (mix, sources) = two_kicks();
    let from = Samples(2 * 44_100);
    let to = Samples(30 * 44_100 + 7);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::Greedy));

    wait_until(&transport, "a second of play", |status| {
        status.position.0 >= from.0 + 44_100
    });
    transport.pause();
    // A pull that took its frames just before the pause may record them just
    // after it, so the snapshot waits for the recorder to settle.
    std::thread::sleep(Duration::from_millis(50));
    let before = pulls.lock().unwrap().clone();
    let heard_before = frames_of(&before).len();
    transport.seek(to);
    let moved = transport.status();
    assert_eq!(
        moved.state,
        TransportState::Paused,
        "a move while paused stays paused"
    );
    assert_eq!(
        moved.position, to,
        "the status says the new position at once"
    );
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        frames_of(&pulls.lock().unwrap()).len(),
        heard_before,
        "nothing is heard until the transport resumes"
    );

    transport.resume();
    wait_until(&transport, "the end", |status| {
        status.state == TransportState::Ended
    });
    let last = transport.stop();
    assert_eq!(last.position, mix_length(&mix));
    let pulls = pulls.lock().unwrap().clone();
    each_pull_is_the_render(&pulls, &whole(&mix, &sources));
    let (first, second) = pulls.split_at(before.len());
    assert_eq!(
        contiguous_from(first, from),
        Samples(from.0 + heard_before as i64)
    );
    assert_eq!(contiguous_from(second, to), mix_length(&mix));
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0, 1],
        "only the track heard at the new position is loaded, and it is loaded once"
    );
}

#[test]
fn a_move_while_playing_switches_at_the_next_pull() {
    let (mix, sources) = two_kicks();
    let length = mix_length(&mix);
    let from = Samples::ZERO;
    let to = Samples(40 * 44_100);
    let Started {
        mut transport,
        pulls,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));

    wait_until(&transport, "a second of play", |status| {
        status.position.0 >= 44_100
    });
    transport.seek(to);
    let moved = transport.status();
    assert_eq!(moved.position, to);
    assert!(
        matches!(
            moved.state,
            TransportState::Buffering | TransportState::Playing
        ),
        "{moved:?}"
    );
    wait_until(&transport, "the end", |status| {
        status.state == TransportState::Ended
    });
    let last = transport.stop();
    assert_eq!(
        last.underruns, 0,
        "the silence while the render catches up is not an underrun"
    );

    let pulls = pulls.lock().unwrap().clone();
    each_pull_is_the_render(&pulls, &whole(&mix, &sources));
    let seam = pulls
        .iter()
        .position(|pull| pull.position == to && !pull.frames.is_empty())
        .expect("a pull that begins exactly at the position moved to");
    let (first, second) = pulls.split_at(seam);
    let reached = contiguous_from(first, from);
    assert!(
        reached.0 >= 44_100 && reached.0 < to.0,
        "before the move the device heard the start of the mix up to {reached:?}"
    );
    assert_eq!(contiguous_from(second, to), length);
}

#[test]
fn a_replaced_document_is_heard_from_the_next_pull() {
    let (mix, sources) = two_kicks();
    let quieter = two_kicks_quieter();
    let from = Samples(3 * 44_100);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));

    wait_until(&transport, "a second of play", |status| {
        status.position.0 >= from.0 + 44_100
    });
    // A pull can land between any two of these three calls, so the pulls
    // before the first position are the old document's, the pulls from the
    // second position on are the new document's, and a pull between the two
    // is the render of one or the other.
    let not_before = transport.status().position;
    transport.replace(quieter.clone());
    let replaced = transport.status();
    let switched_at = replaced.position;
    assert!(
        matches!(
            replaced.state,
            TransportState::Buffering | TransportState::Playing
        ),
        "{replaced:?}"
    );
    wait_until(&transport, "two more seconds of play", |status| {
        status.position.0 >= switched_at.0 + 2 * 44_100
    });
    let last = transport.stop();
    assert_eq!(last.underruns, 0);

    let pulls = pulls.lock().unwrap().clone();
    let old = whole(&mix, &sources);
    let new = whole(&quieter, &sources);
    let before: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position < not_before)
        .cloned()
        .collect();
    let after: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= switched_at)
        .cloned()
        .collect();
    let between: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= not_before && pull.position < switched_at)
        .cloned()
        .collect();
    assert!(!before.is_empty() && !after.is_empty());
    each_pull_is_the_render(&before, &old);
    each_pull_is_the_render(&after, &new);
    for pull in &between {
        let from = pull.position.0 as usize;
        let until = from + pull.frames.len();
        assert!(
            pull.frames == old[from..until] || pull.frames == new[from..until],
            "the pull at {from} during the replacement is neither document's render"
        );
    }
    assert_eq!(
        contiguous_from(&pulls, from),
        last.position,
        "no frame is lost across the replacement"
    );
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0, 0],
        "the track heard at the position is loaded again for the new document"
    );
}

#[test]
fn repeated_replacements_while_playing_keep_the_sound_going() {
    // The window replaces the document each time the person releases
    // the control. Twenty replacements over two seconds, each a different
    // document, must leave the sound going: at least a second of the mix is
    // heard over those two seconds, and what is heard after the last
    // replacement is that document's render.
    let (mix, sources) = two_kicks();
    let from = Samples(3 * 44_100);
    let Started {
        mut transport,
        pulls,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    let started_at = transport.status().position;
    let mut latest = mix.clone();
    for step in 1..=20 {
        let mut edited = mix.clone();
        edited.tracks[0]
            .volume
            .insert(EnvelopeNode {
                at: Beats(-1000.0),
                value: Decibels(-f64::from(step)),
            })
            .unwrap();
        edited.tracks[0]
            .volume
            .insert(EnvelopeNode {
                at: Beats(40.0),
                value: Decibels(-f64::from(step)),
            })
            .unwrap();
        transport.replace(edited.clone());
        latest = edited;
        std::thread::sleep(Duration::from_millis(100));
    }
    let after_last = transport.status().position;
    let heard = after_last.0 - started_at.0;
    assert!(
        heard >= 44_100,
        "only {heard} frames were heard across two seconds of replacements"
    );
    wait_until(&transport, "half a second more", |status| {
        status.position.0 >= after_last.0 + 22_050
    });
    let last = transport.stop();
    let pulls = pulls.lock().unwrap().clone();
    let tail: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= after_last)
        .cloned()
        .collect();
    assert!(!tail.is_empty());
    each_pull_is_the_render(&tail, &whole(&latest, &sources));
    assert_eq!(contiguous_from(&tail, tail[0].position), last.position);
}

#[test]
fn a_silent_track_entering_or_leaving_the_rendered_stretch_does_not_restart() {
    // The second track's audio begins at mix beat 40, which is 18.46
    // seconds, and its fade in starts at its intro anchor, mix beat 48,
    // which is 22.15 seconds, so between the two it is silent. Moving the
    // first track's outro anchor a bar earlier brings that silent entry
    // before the frames already rendered; moving it a bar later takes the
    // entry out of them. Neither changes a rendered frame, so the render
    // continues both times.
    let (mix, sources) = two_kicks();
    let from = Samples(17 * 44_100);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    let reached = transport.status().reached;
    assert!(
        reached < Samples((18.46 * 44_100.0) as i64),
        "the render at {reached:?} has not reached the second track's entry"
    );

    let mut earlier = mix.clone();
    dermixen_core::apply_edit(
        &mut earlier,
        &dermixen_core::Edit::MoveAnchor {
            track: 0,
            anchor: dermixen_core::Anchor::Outro,
            to: Beats(44.0),
        },
    )
    .unwrap();
    transport.replace(earlier.clone());
    let switched_at = transport.status().position;
    let mut seen = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        seen.push(transport.status().state);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        seen.iter().all(|state| *state == TransportState::Playing),
        "the transport went through {:?} when a silent track entered the rendered stretch",
        seen.iter().find(|state| **state != TransportState::Playing)
    );
    assert_eq!(transport.status().restarts, 0);
    // The second track now fades in from mix beat 44, which is 20.3 seconds,
    // so by 25 seconds it has been audible for several seconds, and every
    // frame of that fade must be the new document's own render, which is
    // what proves the track was brought up warm from its silent entry.
    wait_until(&transport, "the fade in", |status| {
        status.position.0 >= 25 * 44_100
    });
    let last = transport.stop();
    assert_eq!(last.restarts, 0);
    assert_eq!(last.underruns, 0);
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0, 1],
        "the second track was loaded when it entered the rendered stretch"
    );
    let entered: Vec<Pull> = pulls
        .lock()
        .unwrap()
        .iter()
        .filter(|pull| pull.position >= switched_at && pull.position < last.position)
        .cloned()
        .collect();
    assert!(!entered.is_empty());
    each_pull_is_the_render(&entered, &whole(&earlier, &sources));

    // The other way round, on a transport of its own: playing the moved
    // document from 19 seconds, the second track's audio has begun at 16.6
    // seconds and is silent until its fade in at 20.3. Moving the outro
    // anchor a bar later than it first was, to beat 56, puts the second
    // track's entry at mix beat 48, which is 22.15 seconds, past everything
    // rendered, so it leaves the rendered stretch, where it was silent.
    let from = Samples(19 * 44_100);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&earlier, &sources, from, Recorder::new(Pace::RealTime));
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    let reached = transport.status().reached;
    assert!(
        reached < Samples((20.3 * 44_100.0) as i64),
        "the render at {reached:?} has not reached the fade in"
    );
    let mut later = earlier.clone();
    dermixen_core::apply_edit(
        &mut later,
        &dermixen_core::Edit::MoveAnchor {
            track: 0,
            anchor: dermixen_core::Anchor::Outro,
            to: Beats(56.0),
        },
    )
    .unwrap();
    transport.replace(later.clone());
    let switched_again = transport.status().position;
    let mut seen = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        seen.push(transport.status().state);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        seen.iter().all(|state| *state == TransportState::Playing),
        "the transport went through {:?} when a silent track left the rendered stretch",
        seen.iter().find(|state| **state != TransportState::Playing)
    );
    assert_eq!(transport.status().restarts, 0);
    // The second track now fades in from mix beat 56, which is 25.8 seconds.
    wait_until(&transport, "the later fade", |status| {
        status.position.0 >= 29 * 44_100
    });
    let last = transport.stop();
    assert_eq!(last.restarts, 0);
    assert_eq!(last.underruns, 0);
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0, 1, 1],
        "the second track was loaded at the start and again when it re-entered"
    );
    let left: Vec<Pull> = pulls
        .lock()
        .unwrap()
        .iter()
        .filter(|pull| pull.position >= switched_again && pull.position < last.position)
        .cloned()
        .collect();
    assert!(!left.is_empty());
    each_pull_is_the_render(&left, &whole(&later, &sources));
}

#[test]
fn a_replacement_that_leaves_the_rendered_frames_alone_continues_without_restarting() {
    let (mix, sources) = two_kicks();
    let from = Samples(3 * 44_100);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    let status = transport.status();
    assert_eq!(status.restarts, 0, "the start is not a restart");
    assert!(
        status.reached >= status.position && status.reached.0 <= status.position.0 + LOOKAHEAD.0,
        "the render is between the position and one lookahead past it: {status:?}"
    );

    // A volume node far past everything rendered leaves every level before
    // it as it was, since the fade's first node still sets the level before
    // the fade, so the render continues.
    let mut later_node = mix.clone();
    later_node.tracks[0]
        .volume
        .insert(EnvelopeNode {
            at: Beats(1000.0),
            value: Decibels(-6.0),
        })
        .unwrap();
    transport.replace(later_node.clone());
    let switched_at = transport.status().position;
    let mut seen = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        seen.push(transport.status().state);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        seen.iter().all(|state| *state == TransportState::Playing),
        "the transport went through {:?} after a replacement that changed nothing rendered",
        seen.iter().find(|state| **state != TransportState::Playing)
    );
    assert_eq!(transport.status().restarts, 0);

    // The master BPM control's change, placed past what the render has
    // reached: a pin holding the tempo the curve has there, which is 130
    // everywhere before the transition, and the new tempo one beat later.
    // The nodes lie after every rendered frame, so the render continues.
    let reached = transport.status().reached;
    let pin_beat = ((reached.0 as f64 / 44_100.0 + 0.25) * 130.0 / 60.0).ceil();
    let mut excursion = later_node.clone();
    excursion.tracks[0].tempo.push(dermixen_core::TempoNode {
        at: Beats(pin_beat),
        bpm: Bpm(130.0),
    });
    excursion.tracks[0].tempo.push(dermixen_core::TempoNode {
        at: Beats(pin_beat + 1.0),
        bpm: Bpm(125.0),
    });
    transport.replace(excursion.clone());
    let mut seen = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        seen.push(transport.status().state);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        seen.iter().all(|state| *state == TransportState::Playing),
        "the transport went through {:?} after the tempo excursion",
        seen.iter().find(|state| **state != TransportState::Playing)
    );
    assert_eq!(transport.status().restarts, 0);
    assert_eq!(*asked.lock().unwrap(), vec![0], "nothing was loaded again");

    // A level changed before everything rendered is heard only from a
    // render that starts over.
    let mut earlier_node = excursion.clone();
    earlier_node.tracks[0]
        .volume
        .insert(EnvelopeNode {
            at: Beats(0.0),
            value: Decibels(-6.0),
        })
        .unwrap();
    transport.replace(earlier_node.clone());
    let restarted_at = transport.status().position;
    wait_until(&transport, "playing again", |status| {
        status.state == TransportState::Playing && status.restarts == 1
    });
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0, 0],
        "the track was loaded again"
    );
    wait_until(&transport, "half a second of the new render", |status| {
        status.position.0 >= restarted_at.0 + 22_050
    });
    transport.seek(Samples(20 * 44_100));
    wait_until(&transport, "playing after the move", |status| {
        status.state == TransportState::Playing && status.restarts == 2
    });
    assert_eq!(*asked.lock().unwrap(), vec![0, 0, 0, 1]);
    let last = transport.stop();
    assert_eq!(last.restarts, 2);
    assert_eq!(last.underruns, 0, "no pull came up short");

    // Every pull between the first replacement and the restart is the render
    // of the excursion document, which renders the same frames as the two
    // documents before it up to where the render had reached; every pull
    // from the restart to the move is the render of the last document.
    let pulls = pulls.lock().unwrap().clone();
    let continued: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= switched_at && pull.position < restarted_at)
        .cloned()
        .collect();
    let restarted: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= restarted_at && pull.position < Samples(20 * 44_100))
        .cloned()
        .collect();
    assert!(!continued.is_empty() && !restarted.is_empty());
    each_pull_is_the_render(&continued, &whole(&excursion, &sources));
    each_pull_is_the_render(&restarted, &whole(&earlier_node, &sources));
}

#[test]
fn a_gain_change_on_a_heard_track_starts_the_render_over_and_names_the_gain() {
    let (mix, sources) = two_kicks();
    let from = Samples(3 * 44_100);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::RealTime));
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });

    // A gain is a level over the whole track, so changing it on a track
    // that is sounding changes frames already rendered: the render starts
    // over, and the reason names the gain and both values.
    let mut leveled = mix.clone();
    leveled.tracks[0].gain = Decibels(-3.0);
    transport.replace(leveled.clone());
    let restarted_at = transport.status().position;
    wait_until(&transport, "playing again", |status| {
        status.state == TransportState::Playing && status.restarts == 1
    });
    let reason = transport
        .status()
        .last_restart
        .clone()
        .expect("a restart has a reason");
    assert!(
        reason.contains("gain") && reason.contains("0.0") && reason.contains("-3.0"),
        "{reason}"
    );
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0, 0],
        "the track was loaded again"
    );

    // The second track is not heard for another seven seconds, so a gain
    // on it changes nothing rendered and the render continues.
    let mut later = leveled.clone();
    later.tracks[1].gain = Decibels(-3.0);
    transport.replace(later.clone());
    let mut seen = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(300) {
        seen.push(transport.status().state);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        seen.iter().all(|state| *state == TransportState::Playing),
        "the transport went through {:?} after a gain change on a track not yet heard",
        seen.iter().find(|state| **state != TransportState::Playing)
    );
    assert_eq!(transport.status().restarts, 1);

    wait_until(&transport, "half a second of the new render", |status| {
        status.position.0 >= restarted_at.0 + 22_050
    });
    let last = transport.stop();
    assert_eq!(last.restarts, 1);

    // Every pull from the restart on is the render of the leveled document,
    // which the later document renders identically this early in the mix.
    let pulls = pulls.lock().unwrap().clone();
    let restarted: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= restarted_at)
        .cloned()
        .collect();
    assert!(!restarted.is_empty());
    each_pull_is_the_render(&restarted, &whole(&later, &sources));
}

#[test]
fn a_replacement_while_paused_waits_for_resumption() {
    let (mix, sources) = two_kicks();
    let quieter = two_kicks_quieter();
    let from = Samples(3 * 44_100);
    let Started {
        mut transport,
        pulls,
        ..
    } = start(&mix, &sources, from, Recorder::new(Pace::Greedy));

    wait_until(&transport, "a second of play", |status| {
        status.position.0 >= from.0 + 44_100
    });
    transport.pause();
    std::thread::sleep(Duration::from_millis(50));
    let paused_at = transport.status().position;
    let before = pulls.lock().unwrap().clone();
    transport.replace(quieter.clone());
    let replaced = transport.status();
    assert_eq!(replaced.state, TransportState::Paused);
    assert_eq!(replaced.position, paused_at);
    transport.resume();
    wait_until(&transport, "the end", |status| {
        status.state == TransportState::Ended
    });
    transport.stop();

    let pulls = pulls.lock().unwrap().clone();
    let (first, second) = pulls.split_at(before.len());
    each_pull_is_the_render(first, &whole(&mix, &sources));
    each_pull_is_the_render(second, &whole(&quieter, &sources));
    assert_eq!(contiguous_from(first, from), paused_at);
    assert_eq!(contiguous_from(second, paused_at), mix_length(&quieter));
}

#[test]
fn a_replacement_shorter_than_the_position_ends_the_transport() {
    let (mix, sources) = two_kicks();
    let from = Samples(40 * 44_100);
    let Started { mut transport, .. } = start(&mix, &sources, from, Recorder::new(Pace::Greedy));
    wait_until(&transport, "a second of play", |status| {
        status.position.0 >= from.0 + 44_100
    });
    let shorter = Mix {
        tracks: vec![mix.tracks[0].clone()],
    };
    let length = mix_length(&shorter);
    assert!(
        length < from,
        "the one-track document ends before the position"
    );
    transport.replace(shorter);
    let status = transport.status();
    assert_eq!(status.state, TransportState::Ended);
    assert_eq!(status.position, length);
    assert_eq!(status.length, length);
    transport.stop();
}

#[test]
fn moving_past_the_end_ends_and_moving_back_leaves_it_paused() {
    let (mix, sources) = two_kicks();
    let length = mix_length(&mix);
    let Started {
        mut transport,
        pulls,
        ..
    } = start(&mix, &sources, Samples::ZERO, Recorder::new(Pace::Greedy));
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    transport.seek(Samples(length.0 + 44_100));
    let ended = transport.status();
    assert_eq!(ended.state, TransportState::Ended);
    assert_eq!(ended.position, length, "a move is clipped to the length");

    let back = Samples(length.0 - 2 * 44_100);
    transport.seek(back);
    let moved = transport.status();
    assert_eq!(
        moved.state,
        TransportState::Paused,
        "a move after the end does not start playback"
    );
    assert_eq!(moved.position, back);
    // The recorder lists a pull a moment after taking it, so the snapshot
    // waits for the recorder to settle before the silence is measured.
    std::thread::sleep(Duration::from_millis(50));
    let heard_before_resume = frames_of(&pulls.lock().unwrap()).len();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(frames_of(&pulls.lock().unwrap()).len(), heard_before_resume);

    transport.resume();
    wait_until(&transport, "the end", |status| {
        status.state == TransportState::Ended
    });
    transport.stop();
    let pulls = pulls.lock().unwrap().clone();
    each_pull_is_the_render(&pulls, &whole(&mix, &sources));
    let tail: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= back && !pull.frames.is_empty())
        .cloned()
        .collect();
    assert_eq!(
        contiguous_from(&tail, back),
        length,
        "the last two seconds are heard whole"
    );
}

#[test]
fn starting_at_or_past_the_end_begins_ended_with_the_output_running() {
    let (mix, sources) = two_kicks();
    let length = mix_length(&mix);
    let Started {
        mut transport,
        pulls,
        asked,
        ..
    } = start(&mix, &sources, length, Recorder::new(Pace::Greedy));
    let status = transport.status();
    assert_eq!(status.state, TransportState::Ended);
    assert_eq!(status.position, length);
    assert!(
        asked.lock().unwrap().is_empty(),
        "nothing is rendered for an ended transport"
    );
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        !pulls.lock().unwrap().is_empty(),
        "the output was started and is pulling, so a later move can be heard"
    );
    transport.seek(Samples(length.0 - 44_100));
    transport.resume();
    wait_until(&transport, "the end", |status| {
        status.state == TransportState::Ended
    });
    let last = transport.stop();
    assert_eq!(last.position, length);
    assert!(!frames_of(&pulls.lock().unwrap()).is_empty());

    let empty = Mix { tracks: Vec::new() };
    let transport = Transport::start(
        empty,
        Samples::ZERO,
        loader(Vec::new(), Arc::new(Mutex::new(Vec::new()))),
        resamplers(),
        Box::new(Recorder::new(Pace::Greedy)),
        LOOKAHEAD,
    )
    .unwrap();
    assert_eq!(
        transport.status(),
        TransportStatus {
            state: TransportState::Ended,
            position: Samples::ZERO,
            length: Samples::ZERO,
            underruns: 0,
            reached: Samples::ZERO,
            restarts: 0,
            last_restart: None,
        }
    );
    transport.stop();
}

#[test]
fn a_track_that_cannot_be_loaded_fails_the_transport_with_the_loaders_message() {
    let (mix, sources) = two_kicks();
    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let mut transport = Transport::start(
        mix.clone(),
        Samples(15 * 44_100),
        refusing_loader(sources.clone()),
        resamplers(),
        Box::new(recorder),
        LOOKAHEAD,
    )
    .unwrap();
    wait_until(&transport, "the failure", |status| {
        matches!(status.state, TransportState::Failed(_))
    });
    let failed = transport.status();
    match &failed.state {
        TransportState::Failed(message) => {
            assert!(message.contains("the file is gone"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    // The controls are inert once the transport has failed.
    transport.resume();
    transport.seek(Samples::ZERO);
    transport.replace(mix.clone());
    assert_eq!(transport.status(), failed);
    let last = transport.stop();
    assert_eq!(last.state, failed.state);
    let pulls = pulls.lock().unwrap().clone();
    each_pull_is_the_render(&pulls, &whole(&mix, &sources));
    let heard = contiguous_from(&pulls, Samples(15 * 44_100));
    // The second track enters about eighteen and a half seconds in, so the
    // frames heard before the failure stop before it does.
    assert!(
        heard.0 <= 19 * 44_100,
        "frames were heard up to {heard:?}, past the point the second track was refused"
    );
}

#[test]
fn a_device_that_stops_taking_frames_fails_the_transport_with_its_message() {
    let (mix, sources) = two_kicks();
    let mut recorder = Recorder::new(Pace::Greedy);
    recorder.fail_after = Some(3 * 44_100);
    let Started { transport, .. } = start(&mix, &sources, Samples::ZERO, recorder);
    wait_until(&transport, "the failure", |status| {
        matches!(status.state, TransportState::Failed(_))
    });
    match transport.stop().state {
        TransportState::Failed(message) => {
            assert!(message.contains("the device was unplugged"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn stopping_returns_promptly_against_a_device_that_has_gone_quiet() {
    let (mix, sources) = two_kicks();
    let mut recorder = Recorder::new(Pace::Greedy);
    recorder.quiet_after = Some(44_100);
    let Started {
        transport, stopped, ..
    } = start(&mix, &sources, Samples::ZERO, recorder);
    wait_until(&transport, "a second of play", |status| {
        status.position.0 >= 44_100
    });
    // The device now asks for nothing more, and says nothing. The render
    // fills the lookahead and waits for room that never comes; the position
    // stops moving.
    std::thread::sleep(Duration::from_millis(300));
    let stalled = transport.status();
    assert_eq!(stalled.state, TransportState::Playing);
    let started = Instant::now();
    let last = transport.stop();
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(2),
        "stop took {took:?} against a quiet device"
    );
    assert_eq!(last.position, stalled.position);
    assert!(*stopped.lock().unwrap(), "the output was stopped");
}

#[test]
fn an_output_that_cannot_start_is_the_error_and_leaves_nothing_running() {
    let (mix, sources) = two_kicks();
    let mut recorder = Recorder::new(Pace::Greedy);
    recorder.refuse = true;
    let asked = Arc::new(Mutex::new(Vec::new()));
    let result = Transport::start(
        mix,
        Samples::ZERO,
        loader(sources, Arc::clone(&asked)),
        resamplers(),
        Box::new(recorder),
        LOOKAHEAD,
    );
    match result {
        Err(RenderError::Output(message)) => assert_eq!(message, "no device"),
        Ok(_) => panic!("the transport started without an output"),
        Err(other) => panic!("{other:?}"),
    }
}

/// A resampler that counts the frames of a track it is fed, so a test can
/// tell how much of the track a run of the render played through it.
struct Counting {
    inner: Resampler,
    fed: Arc<AtomicI64>,
}

impl TimeStretcher for Counting {
    fn process(&mut self, input: &[Frame], output: &mut [Frame]) {
        self.fed.fetch_add(input.len() as i64, Ordering::SeqCst);
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

#[test]
fn a_move_deep_into_a_track_costs_a_run_in_not_the_whole_track() {
    // Sixty seconds of kicks as a mix of one track, which plays at its own
    // tempo, so a frame of the mix is a frame of the track. A move forty
    // seconds in starts the render over there, and the stretcher that run
    // makes is fed a run-in of the track before the first frame it
    // delivers, rather than the forty seconds before the position. By the
    // time the device is playing at the new position the run may also have
    // rendered the lookahead ahead of it, and the device goes on pulling
    // between its first pull there and this test's look, so the bound
    // allows two seconds of that.
    let audio = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(60.0));
    let mix = Mix {
        tracks: vec![track(&audio, 130.0, 0.0, 48.0)],
    };
    let whole = whole(&mix, std::slice::from_ref(&audio));
    let counters: Arc<Mutex<Vec<Arc<AtomicI64>>>> = Arc::new(Mutex::new(Vec::new()));
    let made = Arc::clone(&counters);
    let stretchers: SendStretchers = Box::new(move |_| {
        let fed = Arc::new(AtomicI64::new(0));
        made.lock().unwrap().push(Arc::clone(&fed));
        Box::new(Counting {
            inner: Resampler::new(),
            fed,
        }) as Box<dyn TimeStretcher>
    });
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut transport = Transport::start(
        mix.clone(),
        Samples::ZERO,
        loader(vec![audio.clone()], asked),
        stretchers,
        Box::new(recorder),
        LOOKAHEAD,
    )
    .unwrap();
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });

    let to = Samples(40 * 44_100 + 123);
    transport.seek(to);
    wait_until(&transport, "playing from the new position", |status| {
        status.state == TransportState::Playing && status.position > to
    });
    let fed = {
        let counters = counters.lock().unwrap();
        assert_eq!(
            counters.len(),
            2,
            "one stretcher for each of the two runs of the render"
        );
        counters[1].load(Ordering::SeqCst)
    };
    let most = RUN_IN.0 + LOOKAHEAD.0 + 2 * 44_100;
    assert!(
        fed <= most,
        "the run started by the move had fed its stretcher {fed} frames of the track by the \
         time the device was playing at the new position, more than the {most} a run-in, the \
         lookahead, and two seconds of playing come to"
    );

    let status = transport.stop();
    assert_eq!(status.restarts, 1);
    let pulls = pulls.lock().unwrap();
    each_pull_is_the_render(&pulls, &whole);
    let after: Vec<Pull> = pulls
        .iter()
        .filter(|pull| pull.position >= to)
        .cloned()
        .collect();
    assert!(
        after.iter().any(|pull| !pull.frames.is_empty()),
        "nothing was pulled at the new position"
    );
    contiguous_from(&after, to);
}

#[test]
fn a_track_brought_up_at_a_handover_is_brought_up_from_a_run_in() {
    // Two minutes of kicks at 130 beats per minute into two minutes of
    // kicks at 140, joined by a four-bar beatmix from the first track's
    // beat 200, which is 92.3 seconds in. The second track's intro anchor
    // is its beat 100, so its audio begins at about 46 seconds of the mix,
    // and playing from 40 seconds the render has not reached it. Moving
    // that intro anchor to beat 180 pulls the second track's audio back to
    // about nine seconds, still silent until the beatmix, so it is sounding
    // at the handover frame without changing a rendered frame: the render
    // continues, and brings the track up there. Brought up from its first
    // frame, the track costs its stretcher about thirty seconds of audio.
    // Brought up from a run-in, it costs half a second, plus what the run
    // goes on to render ahead of the device.
    let a = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(120.0));
    let b = synth::kicks(Bpm(140.0), Seconds::ZERO, Seconds(120.0));
    let mut track_a = track(&a, 130.0, 0.0, 200.0);
    let mut track_b = track(&b, 140.0, 100.0, 260.0);
    beatmix(&mut track_a, &mut track_b, 4);
    let mix = Mix {
        tracks: vec![track_a, track_b],
    };
    let sources = vec![a, b];
    let counters: Arc<Mutex<Vec<Arc<AtomicI64>>>> = Arc::new(Mutex::new(Vec::new()));
    let made = Arc::clone(&counters);
    let stretchers: SendStretchers = Box::new(move |_| {
        let fed = Arc::new(AtomicI64::new(0));
        made.lock().unwrap().push(Arc::clone(&fed));
        Box::new(Counting {
            inner: Resampler::new(),
            fed,
        }) as Box<dyn TimeStretcher>
    });
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let from = Samples(40 * 44_100);
    let mut transport = Transport::start(
        mix.clone(),
        from,
        loader(sources.clone(), Arc::clone(&asked)),
        stretchers,
        Box::new(recorder),
        LOOKAHEAD,
    )
    .unwrap();
    wait_until(&transport, "playing", |status| {
        status.state == TransportState::Playing
    });
    assert_eq!(
        *asked.lock().unwrap(),
        vec![0],
        "only the first track sounds at forty seconds"
    );

    let mut pulled_back = mix.clone();
    dermixen_core::apply_edit(
        &mut pulled_back,
        &dermixen_core::Edit::MoveAnchor {
            track: 1,
            anchor: dermixen_core::Anchor::Intro,
            to: Beats(180.0),
        },
    )
    .unwrap();
    transport.replace(pulled_back.clone());
    let switched_at = transport.status().position;
    wait_until(&transport, "half a second past the handover", |status| {
        status.position.0 >= switched_at.0 + 44_100 / 2
    });
    assert_eq!(
        transport.status().restarts,
        0,
        "the replacement was to continue the render, not start it over: {:?}",
        transport.status().last_restart
    );
    let fed = {
        let counters = counters.lock().unwrap();
        assert_eq!(
            counters.len(),
            2,
            "one stretcher for the first track and one made at the handover for the second"
        );
        counters[1].load(Ordering::SeqCst)
    };
    let most = RUN_IN.0 + LOOKAHEAD.0 + 2 * 44_100;
    assert!(
        fed >= RUN_IN.0 - 2 && fed <= most,
        "the second track's stretcher had been fed {fed} frames of the track half a second \
         after the handover, where a run-in of {} frames, and at most the {most} that a \
         run-in, the lookahead, and two seconds of playing come to, was expected",
        RUN_IN.0
    );
    let last = transport.stop();
    assert_eq!(last.restarts, 0);
    assert_eq!(*asked.lock().unwrap(), vec![0, 1]);
    let after: Vec<Pull> = pulls
        .lock()
        .unwrap()
        .iter()
        .filter(|pull| pull.position >= switched_at)
        .cloned()
        .collect();
    assert!(after.iter().any(|pull| !pull.frames.is_empty()));
    each_pull_is_the_render(&after, &whole(&pulled_back, &sources));
}

/// What the loader below panics with, which is the text the transport's
/// status goes on to report.
const THE_DEFECT: &str = "a defect in the loader";

#[test]
fn a_render_thread_that_panics_fails_the_transport_rather_than_going_quiet() {
    // The panic this test causes is deliberate, so the panic message that
    // appears on standard error while it runs is what the test is asking
    // for rather than a sign of a failure.
    let (mix, _) = two_kicks();
    let recorder = Recorder::new(Pace::Greedy);
    let stopped = recorder.stopped();
    let transport = Transport::start(
        mix,
        Samples::ZERO,
        Box::new(|_, _| panic!("{THE_DEFECT}")),
        resamplers(),
        Box::new(recorder),
        LOOKAHEAD,
    )
    .expect("the transport starts");
    wait_until(
        &transport,
        "the transport to report the defect",
        |status| matches!(&status.state, TransportState::Failed(message) if message.contains(THE_DEFECT)),
    );
    let last = transport.stop();
    match last.state {
        TransportState::Failed(message) => assert!(message.contains(THE_DEFECT), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(*stopped.lock().unwrap(), "the output was stopped");
}
