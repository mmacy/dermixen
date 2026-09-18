//! Acceptance tests for the grid audition: one track played on its own with
//! a metronome on its grid. A coder agent makes these pass without editing them.
//!
//! The frames an audition plays are defined exactly in
//! `crates/engine/src/audition.rs`, and `expected_frames` below works them
//! out without the audition, so that every test compares what the device
//! pulled against a number the test derived on its own.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, Bpm, SAMPLE_RATE, Samples, Seconds};
use dermixen_engine::audition::{
    AUDITION_LOOKAHEAD, Audition, AuditionState, CLICK_HZ, CLICK_LENGTH, CLICK_PEAK, DOWNBEAT_HZ,
    TRACK_GAIN, click,
};
use dermixen_engine::{Feed, Output};
use dermixen_media::{Audio, Frame};
use dermixen_testkit::synth;

fn grid(first_beat: i64, bpm: f64) -> BeatGrid {
    BeatGrid {
        first_beat: Samples(first_beat),
        bpm: Bpm(bpm),
    }
}

/// Sample `i` of the click, from the definition in the audition module.
fn click_sample(downbeat: bool, i: i64) -> f32 {
    let hz = if downbeat { DOWNBEAT_HZ } else { CLICK_HZ };
    let phase = 2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(SAMPLE_RATE);
    let fade = 1.0 - i as f64 / CLICK_LENGTH.0 as f64;
    (f64::from(CLICK_PEAK) * phase.sin() * fade) as f32
}

/// The frame of beat `k` of a grid.
fn beat_frame(grid: &BeatGrid, k: i64) -> i64 {
    grid.position_of(Beats(k as f64)).0
}

/// Whether frame `b` is a beat of the grid, and if so whether that beat is
/// a downbeat.
fn beat_at(grid: &BeatGrid, b: i64) -> Option<bool> {
    let near = grid.beat_at_position(Samples(b)).0.round() as i64;
    (near - 1..=near + 1)
        .find(|&k| beat_frame(grid, k) == b)
        .map(|k| k.rem_euclid(i64::from(BEATS_PER_BAR)) == 0)
}

/// Whether two frames agree to within a rounding of the click.
fn same(a: Frame, b: Frame) -> bool {
    (a[0] - b[0]).abs() <= 1e-6 && (a[1] - b[1]).abs() <= 1e-6
}

/// The frames of the audition of `audio` from frame `from` to the end, and
/// for each whether any click sounds in it, worked out from the definition
/// in the audition module: the track at [`TRACK_GAIN`] plus the clicks,
/// held within full scale. `setting` is given a frame and answers with the
/// grid in force there and whether the metronome is on there. Frame `t`
/// holds, when the metronome is on at `t`, frame `t - b` of the click of
/// every beat `b` of the grid in force at `t` with `b <= t < b +
/// CLICK_LENGTH`, beats before the track and before `from` included.
fn expected_frames(
    audio: &[Frame],
    from: i64,
    setting: &impl Fn(i64) -> (BeatGrid, bool),
) -> (Vec<Frame>, Vec<bool>) {
    let len = audio.len() as i64;
    let mut frames: Vec<Frame> = audio[from as usize..]
        .iter()
        .map(|frame| {
            frame.map(|sample| {
                if sample.is_finite() {
                    sample * TRACK_GAIN
                } else {
                    0.0
                }
            })
        })
        .collect();
    let mut clicked = vec![false; frames.len()];
    for t in from..len {
        let (grid, on) = setting(t);
        if !on {
            continue;
        }
        // The beats whose click can reach `t` lie within a click's length
        // before it. Their indices are found from the grid, with a beat of
        // margin on each side so that rounding never drops one.
        let beats_per_click = CLICK_LENGTH.0 as f64 * grid.bpm.0 / 60.0 / f64::from(SAMPLE_RATE);
        let at = grid.beat_at_position(Samples(t)).0;
        let lo = (at - beats_per_click).floor() as i64 - 1;
        let hi = at.ceil() as i64 + 1;
        for k in lo..=hi {
            let b = beat_frame(&grid, k);
            if b <= t && t < b + CLICK_LENGTH.0 {
                let downbeat = k.rem_euclid(i64::from(BEATS_PER_BAR)) == 0;
                let c = click_sample(downbeat, t - b);
                let i = (t - from) as usize;
                frames[i] = [frames[i][0] + c, frames[i][1] + c];
                clicked[i] = true;
            }
        }
    }
    for frame in &mut frames {
        *frame = frame.map(|sample| sample.clamp(-1.0, 1.0));
    }
    (frames, clicked)
}

/// Checks that the frames pulled from `from` on are the expected frames of
/// the audition under `setting`: a frame with a click in it to within a
/// rounding, and a frame without one bit for bit.
fn frames_are_expected(
    pulls: &[Pull],
    audio: &[Frame],
    from: i64,
    setting: impl Fn(i64) -> (BeatGrid, bool),
) {
    let frames = frames_of(pulls);
    let end = from + frames.len() as i64;
    assert_eq!(
        end,
        audio.len() as i64,
        "the audition ends at the track's last frame"
    );
    let (want, clicked) = expected_frames(audio, from, &setting);
    for (offset, frame) in frames.iter().enumerate() {
        let t = from + offset as i64;
        let (grid, metronome) = setting(t);
        let agree = if clicked[offset] {
            same(*frame, want[offset])
        } else {
            *frame == want[offset]
        };
        assert!(
            agree,
            "frame {t} is {frame:?}, expected {:?} with the grid at {} and {} beats per minute, metronome {}",
            want[offset],
            grid.first_beat.0,
            grid.bpm.0,
            if metronome { "on" } else { "off" }
        );
    }
}

/// One pull the device made: where the feed said it began and what it got.
#[derive(Debug, Clone)]
struct Pull {
    position: Samples,
    frames: Vec<Frame>,
}

/// How a recorder behaves, chosen per test.
#[derive(Clone, Copy)]
enum Pace {
    /// Pulls whatever is ready, as fast as it can, so the recording is
    /// exact and never waits on the audition.
    Greedy,
    /// Pulls a device-sized buffer of 1024 frames every twenty-three
    /// milliseconds, which is real time, as an audio callback does.
    RealTime,
}

/// An output that records every pull with its position, on a thread of its
/// own as a device would.
struct Recorder {
    pace: Pace,
    /// Whether `start` should refuse.
    refuse: bool,
    /// Whether the thread should tell the feed the device failed after this
    /// many frames.
    fail_after: Option<usize>,
    pulls: Arc<Mutex<Vec<Pull>>>,
    stopped: Arc<Mutex<bool>>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    fn new(pace: Pace) -> Recorder {
        Recorder {
            pace,
            refuse: false,
            fail_after: None,
            pulls: Arc::new(Mutex::new(Vec::new())),
            stopped: Arc::new(Mutex::new(false)),
            thread: None,
        }
    }

    fn pulls(&self) -> Arc<Mutex<Vec<Pull>>> {
        Arc::clone(&self.pulls)
    }

    fn stopped(&self) -> Arc<Mutex<bool>> {
        Arc::clone(&self.stopped)
    }
}

impl Output for Recorder {
    fn start(&mut self, mut feed: Feed) -> Result<(), String> {
        if self.refuse {
            return Err("no device".to_owned());
        }
        let pulls = Arc::clone(&self.pulls);
        let stopped = Arc::clone(&self.stopped);
        let pace = self.pace;
        let fail_after = self.fail_after;
        self.thread = Some(std::thread::spawn(move || {
            let mut recorded = 0usize;
            loop {
                if *stopped.lock().unwrap() {
                    break;
                }
                if fail_after.is_some_and(|after| recorded >= after) {
                    feed.fail("the device was unplugged");
                    break;
                }
                let wanted = match pace {
                    Pace::Greedy => feed.available().max(1),
                    Pace::RealTime => 1024,
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
                    Pace::RealTime => std::thread::sleep(Duration::from_micros(23_220)),
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

/// Every frame in the pulls, in order.
fn frames_of(pulls: &[Pull]) -> Vec<Frame> {
    pulls
        .iter()
        .flat_map(|pull| pull.frames.iter().copied())
        .collect()
}

/// Checks that the pulls that got frames follow one another without a gap
/// or a repeat, from `from` on.
fn contiguous_from(pulls: &[Pull], from: i64) {
    let mut next = from;
    for pull in pulls.iter().filter(|pull| !pull.frames.is_empty()) {
        assert_eq!(
            pull.position.0, next,
            "a pull began at {} where {next} was expected",
            pull.position.0
        );
        next += pull.frames.len() as i64;
    }
}

/// Checks that no pull of a real-time recorder came up short, except the
/// first pull that got frames, which may have landed while the first
/// lookahead was still being made, and the last, which is the end of the
/// track: a short pull in between is a gap the device fills with silence.
fn no_short_pulls(pulls: &[Pull]) {
    let first = pulls.iter().position(|pull| !pull.frames.is_empty());
    let last = pulls.iter().rposition(|pull| !pull.frames.is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return;
    };
    for (i, pull) in pulls.iter().enumerate().take(last).skip(first + 1) {
        assert_eq!(
            pull.frames.len(),
            1024,
            "pull {i} at {} came up short with {} frames",
            pull.position.0,
            pull.frames.len()
        );
    }
}

/// Waits until `done` answers true, or fails after `patience`.
fn wait_until(what: &str, patience: Duration, mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(began.elapsed() < patience, "gave up waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Waits until the audition reports its end and the recorder holds every
/// frame it was to play, since the state can say ended before the
/// recorder has put its last pull away.
fn wait_for_the_end(audition: &Audition, pulls: &Arc<Mutex<Vec<Pull>>>, frames: usize) {
    wait_until("the audition to end", Duration::from_secs(20), || {
        audition.status().state == AuditionState::Ended
            && pulls
                .lock()
                .unwrap()
                .iter()
                .map(|pull| pull.frames.len())
                .sum::<usize>()
                == frames
    });
}

/// Waits until the audition reports its end and the recorder's last pull
/// with frames ends at `length`, for an audition that has moved and so
/// pulled an unknown number of frames altogether.
fn wait_for_the_end_at(audition: &Audition, pulls: &Arc<Mutex<Vec<Pull>>>, length: i64) {
    wait_until("the audition to end", Duration::from_secs(20), || {
        audition.status().state == AuditionState::Ended
            && pulls
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|pull| !pull.frames.is_empty())
                .is_some_and(|pull| pull.position.0 + pull.frames.len() as i64 == length)
    });
}

/// Waits until the audition's position has reached `frame`.
fn wait_for_position(audition: &Audition, frame: i64) {
    wait_until(
        &format!("the audition to reach frame {frame}"),
        Duration::from_secs(10),
        || audition.status().position.0 >= frame,
    );
}

fn noise(seconds: f64) -> Arc<Audio> {
    Arc::new(synth::white_noise(7, 0.3, Seconds(seconds)))
}

/// The track's frame at the audition's level plus a click sample, in both
/// channels.
fn with_click(audio: &[Frame], t: usize, c: f32) -> Frame {
    [audio[t][0] * TRACK_GAIN + c, audio[t][1] * TRACK_GAIN + c]
}

/// The track's frame at the audition's level, with no click.
fn without_click(audio: &[Frame], t: usize) -> Frame {
    [audio[t][0] * TRACK_GAIN, audio[t][1] * TRACK_GAIN]
}

#[test]
fn the_click_is_a_fading_sine_at_the_beats_pitch_and_higher_on_a_downbeat() {
    assert_eq!(CLICK_LENGTH, Samples(882), "twenty milliseconds");
    // A track at full scale under a click at its peak stays within full
    // scale, which the compiler checks.
    const { assert!(TRACK_GAIN + CLICK_PEAK <= 1.0) };
    assert_eq!(TRACK_GAIN, 0.5);
    for downbeat in [false, true] {
        let frames = click(downbeat);
        assert_eq!(frames.len(), 882);
        assert_eq!(frames[0], [0.0, 0.0], "a sine starts at zero");
        for (i, frame) in frames.iter().enumerate() {
            let want = click_sample(downbeat, i as i64);
            assert!(
                (frame[0] - want).abs() <= 1e-6 && frame[0] == frame[1],
                "sample {i} of the {} click is {frame:?}, expected {want}",
                if downbeat { "downbeat" } else { "beat" }
            );
            assert!(frame[0].abs() <= CLICK_PEAK);
        }
    }
    // One thousand hertz at 44.1 kHz is 44.1 samples a cycle, so sample 15
    // is a third of a cycle in and still high; fifteen hundred hertz is
    // 29.4 samples a cycle, so the downbeat's click is crossing zero there.
    let beat = click(false);
    let downbeat = click(true);
    assert!(beat[15][0] > 0.4 && beat[15][0] <= 0.5, "{}", beat[15][0]);
    assert!(downbeat[15][0].abs() < 0.05, "{}", downbeat[15][0]);
    assert!(
        beat[881][0].abs() < 1e-3 && downbeat[881][0].abs() < 1e-3,
        "the click fades out"
    );
}

#[test]
fn the_audition_is_the_track_plus_a_click_on_every_beat() {
    let audio = noise(2.0);
    assert_eq!(audio.frames.len(), 88_200);
    // Beats every half second from frame 1000: at 1000, 23050, 45100, and
    // 67150; the beat before beat zero falls before the track, and beat
    // four falls after it.
    let g = grid(1000, 120.0);
    assert_eq!(beat_frame(&g, 1), 23_050);
    assert_eq!(beat_frame(&g, 3), 67_150);
    assert_eq!(beat_frame(&g, -1), -21_050);
    assert_eq!(beat_frame(&g, 4), 89_200);
    assert_eq!(beat_at(&g, 1000), Some(true));
    assert_eq!(beat_at(&g, 23_050), Some(false));
    assert_eq!(beat_at(&g, 23_051), None);

    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let stopped = recorder.stopped();
    let audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_the_end(&audition, &pulls, 88_200);

    let status = audition.status();
    assert_eq!(status.state, AuditionState::Ended);
    assert_eq!(status.position, Samples(88_200));
    assert_eq!(status.length, Samples(88_200));
    assert!(status.click);

    let pulls = pulls.lock().unwrap().clone();
    contiguous_from(&pulls, 0);
    frames_are_expected(&pulls, &audio.frames, 0, |_| (g, true));
    // The downbeat at frame 1000 is the higher click, and the beat at 23050
    // the lower one: sample eleven of each tells them apart.
    let frames = frames_of(&pulls);
    assert!(same(
        frames[1011],
        with_click(&audio.frames, 1011, click_sample(true, 11))
    ));
    assert!(same(
        frames[23_061],
        with_click(&audio.frames, 23_061, click_sample(false, 11))
    ));
    assert!(!same(
        frames[23_061],
        with_click(&audio.frames, 23_061, click_sample(true, 11))
    ));
    assert_eq!(
        frames[50_000],
        without_click(&audio.frames, 50_000),
        "no click here"
    );

    audition.stop();
    assert!(
        *stopped.lock().unwrap(),
        "stopping the audition stops the output"
    );
}

#[test]
fn beats_before_beat_zero_click_too_and_the_downbeats_count_from_beat_zero() {
    let audio = noise(2.0);
    // Beat zero at frame 30000 has beat minus one at 7950, which is not a
    // downbeat; beat minus four would be at -58200, before the track.
    let late = grid(30_000, 120.0);
    assert_eq!(beat_frame(&late, -1), 7_950);
    assert_eq!(beat_at(&late, 7_950), Some(false));
    assert_eq!(beat_at(&late, 30_000), Some(true));
    // Beat zero at frame -1000 puts beat four, a downbeat, at 87200, whose
    // click ends at 88082, inside the track.
    let early = grid(-1000, 120.0);
    assert_eq!(beat_frame(&early, 4), 87_200);
    // Beat zero at frame -500 is before the track, and its click runs into
    // the track's first 382 frames.
    let tail = grid(-500, 120.0);
    assert_eq!(beat_frame(&tail, 0), -500);
    let (want, clicked) = expected_frames(&audio.frames, 0, &|_| (tail, true));
    assert!(clicked[0] && clicked[381] && !clicked[382]);
    assert!(same(
        want[0],
        with_click(&audio.frames, 0, click_sample(true, 500))
    ));

    for g in [late, early, tail] {
        let recorder = Recorder::new(Pace::Greedy);
        let pulls = recorder.pulls();
        let audition = Audition::start(
            Arc::clone(&audio),
            g,
            Samples::ZERO,
            true,
            Box::new(recorder),
        )
        .expect("the audition starts");
        wait_for_the_end(&audition, &pulls, 88_200);
        let pulls = pulls.lock().unwrap().clone();
        contiguous_from(&pulls, 0);
        frames_are_expected(&pulls, &audio.frames, 0, |_| (g, true));
        audition.stop();
    }
}

#[test]
fn starting_partway_through_plays_from_there_with_the_click_already_under_way() {
    let audio = noise(2.0);
    let g = grid(1000, 120.0);
    // Beat two is at 45100, so frame 45500 is four hundred frames into its
    // click.
    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples(45_500),
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_the_end(&audition, &pulls, 88_200 - 45_500);
    let pulls = pulls.lock().unwrap().clone();
    contiguous_from(&pulls, 45_500);
    frames_are_expected(&pulls, &audio.frames, 45_500, |_| (g, true));
    let first = frames_of(&pulls)[0];
    assert!(same(
        first,
        with_click(&audio.frames, 45_500, click_sample(false, 400))
    ));
    audition.stop();

    // A start before the track begins at its first frame.
    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples(-5),
        false,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_the_end(&audition, &pulls, 88_200);
    let pulls = pulls.lock().unwrap().clone();
    contiguous_from(&pulls, 0);
    frames_are_expected(&pulls, &audio.frames, 0, |_| (g, false));
    audition.stop();

    // A start at or past the end has nothing to play and ends at once.
    for from in [88_200, 100_000] {
        let recorder = Recorder::new(Pace::Greedy);
        let pulls = recorder.pulls();
        let audition = Audition::start(
            Arc::clone(&audio),
            g,
            Samples(from),
            true,
            Box::new(recorder),
        )
        .expect("the audition starts");
        wait_for_the_end(&audition, &pulls, 0);
        let status = audition.status();
        assert_eq!(status.position, Samples(88_200));
        assert!(frames_of(&pulls.lock().unwrap()).is_empty());
        audition.stop();
    }
}

#[test]
fn a_grid_replaced_while_it_plays_is_heard_from_the_next_frame_the_device_pulls() {
    let audio = noise(3.0);
    // The old grid has beats at 1000, 23050, 45100, 67150, and 89200; the
    // new one has beats every 20671.875 frames from 4000, so at 4000,
    // 24672, 45344, and 66016.
    let before = grid(1000, 120.0);
    let after = grid(4000, 128.0);
    assert_eq!(beat_frame(&after, 2), 45_344);
    assert_eq!(beat_frame(&after, 3), 66_016);
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let mut audition = Audition::start(
        Arc::clone(&audio),
        before,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");

    // The change is asked for one lookahead before the old grid's beat at
    // 45100, so that the frames of that beat's click, which runs to 45981,
    // have already been made under the old grid and must be made again.
    wait_for_position(&audition, 45_100 - AUDITION_LOOKAHEAD.0);
    let position_before = audition.status().position;
    let boundary = audition.set_grid(after).expect("a valid grid is taken");
    let position_after = audition.status().position;
    assert!(
        boundary >= position_before,
        "the new grid begins at {} before the position {} at the call",
        boundary.0,
        position_before.0
    );
    assert!(
        boundary <= position_after,
        "the new grid begins at {}, past the position {} after the call",
        boundary.0,
        position_after.0
    );

    wait_for_the_end(&audition, &pulls, 132_300);
    let pulls = pulls.lock().unwrap().clone();
    contiguous_from(&pulls, 0);
    no_short_pulls(&pulls);
    let setting = |t: i64| {
        if t < boundary.0 {
            (before, true)
        } else {
            (after, true)
        }
    };
    frames_are_expected(&pulls, &audio.frames, 0, setting);
    let frames = frames_of(&pulls);
    // The boundary falls before the old grid's beat at 45100, so that
    // beat never clicks: its frames were made again under the new grid,
    // whose nearest beats are 24672 and 45344.
    assert!(boundary.0 < 45_100, "{}", boundary.0);
    assert_eq!(frames[45_100], without_click(&audio.frames, 45_100));
    let t = 45_344 + 11;
    assert!(same(
        frames[t],
        with_click(&audio.frames, t, click_sample(false, 11))
    ));
    // The new grid's beat at 66016 clicks, and the old grid's beat at 67150
    // does not.
    let t = 66_016 + 11;
    assert!(same(
        frames[t],
        with_click(&audio.frames, t, click_sample(false, 11))
    ));
    assert_eq!(frames[67_161], without_click(&audio.frames, 67_161));
    audition.stop();
}

#[test]
fn the_metronome_turns_off_and_on_from_the_next_frame_the_device_pulls() {
    let audio = noise(3.0);
    let g = grid(1000, 120.0);
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let mut audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    assert!(audition.status().click);

    // The metronome goes off one lookahead before the beat at 23050, whose
    // click had already been made, so that beat is never heard.
    wait_for_position(&audition, 23_050 - AUDITION_LOOKAHEAD.0);
    let p = audition.status().position;
    let off = audition.set_click(false);
    assert!(off >= p && off <= audition.status().position);
    assert!(!audition.status().click);

    // It comes back on one lookahead before the beat at 67150, whose
    // frames had been made without a click and are made again with one.
    wait_for_position(&audition, 67_150 - AUDITION_LOOKAHEAD.0);
    let p = audition.status().position;
    let on = audition.set_click(true);
    assert!(on >= p && on <= audition.status().position);
    assert!(audition.status().click);
    assert!(on > off);

    wait_for_the_end(&audition, &pulls, 132_300);
    let pulls = pulls.lock().unwrap().clone();
    contiguous_from(&pulls, 0);
    no_short_pulls(&pulls);
    frames_are_expected(&pulls, &audio.frames, 0, |t| (g, t < off.0 || t >= on.0));
    let frames = frames_of(&pulls);
    assert!(off.0 < 23_050 && on.0 < 67_150, "{} {}", off.0, on.0);
    assert_eq!(frames[23_061], without_click(&audio.frames, 23_061));
    assert!(same(
        frames[67_161],
        with_click(&audio.frames, 67_161, click_sample(false, 11))
    ));
    assert!(same(
        frames[89_211],
        with_click(&audio.frames, 89_211, click_sample(true, 11))
    ));
    audition.stop();
}

#[test]
fn a_move_plays_the_new_frame_from_the_next_pull_and_a_move_to_the_end_ends() {
    let audio = noise(3.0);
    let g = grid(1000, 120.0);
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let mut audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");

    // A move from around frame 20000 to frame 80000. The beat at 67150 has
    // a click that ends at 68032, before 80000, so nothing of it is heard;
    // the beat at 89200 is.
    wait_for_position(&audition, 20_000);
    let position_before = audition.status().position;
    audition.seek(Samples(80_000));
    let status = audition.status();
    assert_eq!(status.state, AuditionState::Playing);
    assert!(
        status.position.0 >= 80_000 && status.position.0 <= 80_000 + 1024,
        "the status says the new position at once, not {}",
        status.position.0
    );
    assert!(status.click);

    wait_for_the_end_at(&audition, &pulls, 132_300);
    let pulls = pulls.lock().unwrap().clone();
    let split = pulls
        .iter()
        .position(|pull| pull.position.0 >= 80_000)
        .expect("a pull at the new position");
    let (first, second) = pulls.split_at(split);
    contiguous_from(first, 0);
    let first_frames = frames_of(first);
    assert!(
        first_frames.len() as i64 <= position_before.0 + 8 * 1024,
        "nothing of the old position is heard past the pulls around the move, but {} frames were",
        first_frames.len()
    );
    frames_are_expected(first, &audio.frames[..first_frames.len()], 0, |_| (g, true));
    contiguous_from(second, 80_000);
    no_short_pulls(second);
    frames_are_expected(second, &audio.frames, 80_000, |_| (g, true));
    let frames = frames_of(second);
    let t = 89_211 - 80_000;
    assert!(same(
        frames[t],
        with_click(&audio.frames, 89_211, click_sample(true, 11))
    ));

    // A move once the audition has ended changes nothing.
    audition.seek(Samples(1000));
    let status = audition.status();
    assert_eq!(status.state, AuditionState::Ended);
    assert_eq!(status.position, Samples(132_300));
    audition.stop();

    // A move before the first frame plays from the first frame, and a move
    // to within a lookahead of the end plays those last frames out and
    // ends.
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let mut audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples(50_000),
        false,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_position(&audition, 51_000);
    audition.seek(Samples(-5));
    wait_for_position(&audition, 100);
    assert!(audition.status().position.0 < 50_000);
    audition.seek(Samples(130_000));
    wait_until("the audition to end", Duration::from_secs(5), || {
        audition.status().state == AuditionState::Ended
    });
    assert_eq!(audition.status().position, Samples(132_300));
    let pulls = pulls.lock().unwrap().clone();
    let from_start = pulls
        .iter()
        .position(|pull| pull.position.0 < 50_000 && !pull.frames.is_empty())
        .expect("a pull from the first frame");
    assert_eq!(pulls[from_start].position, Samples::ZERO);
    let last = pulls
        .iter()
        .rev()
        .find(|pull| !pull.frames.is_empty())
        .expect("a last pull with frames");
    assert_eq!(last.position.0 + last.frames.len() as i64, 132_300);
    frames_are_expected(
        &pulls[pulls
            .iter()
            .position(|pull| pull.position.0 >= 130_000)
            .expect("a pull at the new position")..],
        &audio.frames,
        130_000,
        |_| (g, false),
    );
    audition.stop();

    // A move past the length ends the audition at the length, and nothing
    // more of the track is heard.
    let recorder = Recorder::new(Pace::RealTime);
    let pulls = recorder.pulls();
    let mut audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples(50_000),
        false,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_position(&audition, 51_000);
    let position_before = audition.status().position;
    audition.seek(Samples(200_000));
    wait_until("the audition to end", Duration::from_secs(5), || {
        audition.status().state == AuditionState::Ended
    });
    assert_eq!(audition.status().position, Samples(132_300));
    let pulls = pulls.lock().unwrap().clone();
    let heard = frames_of(&pulls).len() as i64;
    assert!(
        heard <= position_before.0 - 50_000 + 8 * 1024,
        "{heard} frames were heard after a move past the end"
    );
    audition.stop();
}

#[test]
fn a_grid_without_a_tempo_is_refused() {
    let audio = noise(1.0);
    for bad in [0.0, -120.0, f64::NAN, f64::INFINITY] {
        let recorder = Recorder::new(Pace::Greedy);
        let stopped = recorder.stopped();
        let started = Audition::start(
            Arc::clone(&audio),
            grid(0, bad),
            Samples::ZERO,
            true,
            Box::new(recorder),
        );
        assert!(started.is_err(), "a tempo of {bad} was accepted");
        assert!(
            !*stopped.lock().unwrap(),
            "an output the audition never started must not be stopped"
        );
    }

    let g = grid(1000, 120.0);
    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let mut audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    assert_eq!(audition.set_grid(grid(500, f64::NAN)), None);
    assert_eq!(audition.set_grid(grid(500, 0.0)), None);
    wait_for_the_end(&audition, &pulls, 44_100);
    let pulls = pulls.lock().unwrap().clone();
    frames_are_expected(&pulls, &audio.frames, 0, |_| (g, true));
    // Once the audition has ended a change is taken, and the frame
    // returned is the position, which is the track's length.
    assert_eq!(audition.set_grid(grid(500, 130.0)), Some(Samples(44_100)));
    assert_eq!(audition.set_click(false), Samples(44_100));
    assert!(!audition.status().click);
    audition.stop();
}

#[test]
fn an_output_that_cannot_start_is_an_error_and_a_device_that_stops_is_a_failure() {
    let audio = noise(1.0);
    let g = grid(0, 120.0);
    let mut recorder = Recorder::new(Pace::Greedy);
    recorder.refuse = true;
    let started = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    );
    assert_eq!(started.err().as_deref(), Some("no device"));

    let mut recorder = Recorder::new(Pace::RealTime);
    recorder.fail_after = Some(4096);
    let mut audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_until("the device to fail", Duration::from_secs(10), || {
        matches!(audition.status().state, AuditionState::Failed(_))
    });
    let status = audition.status();
    assert_eq!(
        status.state,
        AuditionState::Failed("the device was unplugged".to_owned())
    );
    assert!(status.position.0 >= 4096 && status.position.0 < 44_100);
    let held = status.position;
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        audition.status().position,
        held,
        "the position holds after a failure"
    );
    assert_eq!(
        audition.set_click(false),
        held,
        "no more frames are made after a failure"
    );
    assert_eq!(audition.set_grid(grid(0, 130.0)), Some(held));
    audition.stop();
}

#[test]
fn a_sample_that_is_not_a_number_is_played_as_silence() {
    let mut audio = synth::white_noise(7, 0.3, Seconds(1.0));
    // Frame 500 lies under no click; frame 1100 lies a hundred frames into
    // the click of beat zero.
    audio.frames[500] = [f32::NAN, 0.1];
    audio.frames[1100] = [f32::INFINITY, audio.frames[1100][1]];
    let audio = Arc::new(audio);
    let g = grid(1000, 120.0);
    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_the_end(&audition, &pulls, 44_100);
    let frames = frames_of(&pulls.lock().unwrap());
    assert_eq!(
        frames[500],
        [0.0, 0.05],
        "half of 0.1, and silence for the sample that is not a number"
    );
    let c = click_sample(true, 100);
    assert!(same(
        frames[1100],
        [c, audio.frames[1100][1] * TRACK_GAIN + c]
    ));
    frames_are_expected(&pulls.lock().unwrap(), &audio.frames, 0, |_| (g, true));
    audition.stop();
}

#[test]
fn a_track_over_full_scale_under_a_click_is_held_within_full_scale() {
    // A float file can hold samples over full scale. A track held at 1.5
    // sits at 0.75 under the clicks, so wherever a click passes a quarter of
    // full scale in either direction the sum passes one and is held there.
    // No tempo a grid may have makes two clicks overlap: a click lasts 882
    // frames, and a beat at 999 beats per minute lasts 2,649.
    let audio = Arc::new(Audio {
        frames: vec![[1.5, -1.5]; 44_100],
    });
    let g = grid(0, 120.0);
    assert_eq!(beat_frame(&g, 1), 22_050);
    assert_eq!(beat_frame(&g, 2), 44_100);
    let (want, _) = expected_frames(&audio.frames, 0, &|_| (g, true));
    assert!(
        want.iter().any(|frame| frame[0] == 1.0),
        "the ceiling is reached somewhere"
    );
    assert!(want.iter().any(|frame| frame[1] == -1.0), "and the floor");
    assert!(want.iter().all(|frame| frame[0] <= 1.0 && frame[1] >= -1.0));

    let recorder = Recorder::new(Pace::Greedy);
    let pulls = recorder.pulls();
    let audition = Audition::start(
        Arc::clone(&audio),
        g,
        Samples::ZERO,
        true,
        Box::new(recorder),
    )
    .expect("the audition starts");
    wait_for_the_end(&audition, &pulls, 44_100);
    let pulls = pulls.lock().unwrap().clone();
    contiguous_from(&pulls, 0);
    frames_are_expected(&pulls, &audio.frames, 0, |_| (g, true));
    audition.stop();
}
