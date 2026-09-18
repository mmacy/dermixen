//! The grid audition: one track played on its own, at its own speed, with a
//! metronome that clicks on every beat of the track's grid.
//!
//! A person fixing a beat grid needs to hear whether the grid sits on the
//! music, and the way to hear that is a click on every beat played over the
//! track itself. The audition does not go through the mix render: it plays
//! the track's own frames, unstretched, because a grid is judged against
//! the track and not against the mix. The grid may be replaced while the
//! audition plays, as it is when a person drags the grid in the editor or
//! moves it with an arrow key, the metronome may be turned on and off at
//! any time, and the audition may be moved to another frame of the track.
//! Every one of those changes is heard from the next frame the device
//! pulls: the audition makes the frames it had made ahead under the old
//! setting again under the new one before the device takes them, so a
//! person hears a grid moved by one frame as soon as the device's own
//! buffer allows and never a lookahead later.
//!
//! The frames are defined exactly, so that the acceptance tests in
//! `tests/audition.rs` can compute them without the audition. At every
//! track frame a setting is in force: the grid and whether the metronome
//! is on, as they stood when the device pulled the frame. A change made
//! between two pulls is in force from the first frame of the later pull,
//! and that frame is what the change reports. Frame `t` of the audition,
//! counted from the first frame of the track, is the track's frame `t` at
//! [`TRACK_GAIN`], with a sample that is not a finite number counted as
//! zero, plus, when the metronome is on at `t`, frame `t - b` of the click
//! of every beat `b` of the grid in force at `t` with
//! `b <= t < b + CLICK_LENGTH`, the clicks summed when they overlap, and
//! the whole held within full scale: a sum above one is one and a sum
//! below minus one is minus one, so a grid fast enough for its clicks to
//! pile up cannot clip on the way to the device. A change of grid
//! therefore cuts the old grid's clicks at the frame the change takes
//! effect, and sounds the tails of the new grid's clicks that began before
//! that frame, which is what a person hears as the click jumping to the
//! new beat. The frame of beat `k` is
//! [`BeatGrid::position_of`](dermixen_core::BeatGrid::position_of) for
//! beat `k`, for every whole `k`, negative ones included, so a beat zero
//! that falls after the start of the track has beats before it, and a
//! beat before the first frame of the track may begin a click whose tail
//! falls inside the track. Beat `k` is a downbeat when `k` is a whole multiple of
//! [`BEATS_PER_BAR`](dermixen_core::BEATS_PER_BAR), counting negative
//! multiples too. The audition plays frames from the one it starts at up to
//! the track's last frame, and then ends.
//!
//! The audition keeps a device fed the way the transport does, through the
//! crate-private `Channel` in `preview.rs`.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, SAMPLE_RATE, Samples};
use dermixen_media::{Audio, Frame};

use crate::preview::{Channel, Feed, Output};
use crate::render::BLOCK_FRAMES;

/// How long a click lasts: twenty milliseconds.
pub const CLICK_LENGTH: Samples = Samples(882);

/// The pitch of the click on a beat that is not a downbeat.
pub const CLICK_HZ: f64 = 1000.0;

/// The pitch of the click on a downbeat, which is higher so that the bars
/// can be heard.
pub const DOWNBEAT_HZ: f64 = 1500.0;

/// The click's loudest possible sample.
pub const CLICK_PEAK: f32 = 0.5;

/// The level the track is played at under the click: half, so that a
/// track mastered to full scale plus one click at [`CLICK_PEAK`] stays
/// within full scale without the ceiling. Each sample is multiplied by
/// this before the clicks are added.
pub const TRACK_GAIN: f32 = 0.5;

/// How many frames the audition keeps ready ahead of the device: a tenth
/// of a second. This is how far the device may run before the thread
/// making the frames has to be scheduled again. The lookahead bounds no
/// change: the audition remakes the frames already ahead when the grid or
/// the metronome changes, and throws them away when the position changes,
/// so a person hears each change from the next frame the device pulls.
pub const AUDITION_LOOKAHEAD: Samples = Samples(4410);

/// A block of frames fits inside the lookahead, so that the thread making
/// the frames can always make a whole block into the room the device has
/// left.
const _: () = assert!(BLOCK_FRAMES as i64 <= AUDITION_LOOKAHEAD.0);

/// The click's frames: a sine at [`CLICK_HZ`], or at [`DOWNBEAT_HZ`] for a
/// downbeat, that starts at [`CLICK_PEAK`] and fades in a straight line to
/// silence over [`CLICK_LENGTH`] frames.
///
/// Frame `i` holds, in both channels, `CLICK_PEAK` times the sine of
/// `2 * pi * hz * i / SAMPLE_RATE` times `1 - i / CLICK_LENGTH`, worked out
/// in double precision and then narrowed to a sample.
pub fn click(downbeat: bool) -> Vec<Frame> {
    let hz = if downbeat { DOWNBEAT_HZ } else { CLICK_HZ };
    (0..CLICK_LENGTH.0)
        .map(|i| {
            let phase = 2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(SAMPLE_RATE);
            let fade = 1.0 - i as f64 / CLICK_LENGTH.0 as f64;
            let sample = (f64::from(CLICK_PEAK) * phase.sin() * fade) as f32;
            [sample, sample]
        })
        .collect()
}

/// What an audition is doing at a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditionState {
    /// The device is pulling the track's frames.
    Playing,
    /// The device has pulled the track's last frame, and the position is
    /// the track's length. An audition started at or past the length ends
    /// at once. So does one that [`seek`](Audition::seek) moves to or past
    /// the length: the status says the audition has ended from the move on,
    /// before the device has pulled anything more.
    Ended,
    /// The audition stopped on the message the output gave
    /// [`Feed::fail`](crate::Feed::fail) when its device stopped taking
    /// frames. The position holds, and only [`stop`](Audition::stop) ends
    /// the failed state.
    Failed(String),
}

/// Where an audition is and what it is doing, for drawing a playhead over
/// the track and the metronome control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditionStatus {
    /// What the audition is doing.
    pub state: AuditionState,
    /// The track frame the device will play next: the frame just past the
    /// last one it pulled, and the frame the audition started at until the
    /// device has pulled anything.
    pub position: Samples,
    /// The track's length in frames.
    pub length: Samples,
    /// Whether the metronome is on.
    pub click: bool,
}

/// The setting the audition's frames are made under, kept under a lock of
/// its own.
///
/// [`Audition::change`] holds this lock while it makes the frames already
/// ahead of the device again, so the setting here and the frames waiting in
/// the feed always agree. The thread making the frames takes the lock only
/// to copy the setting, never while it makes a block, so a person replacing
/// the grid or turning the metronome on or off waits for a copy rather than
/// for a block.
///
/// Whoever holds both this lock and the feed's takes this one first. The
/// thread making the frames holds neither while it takes the other, so the
/// two locks cannot wait on each other.
struct Setting {
    /// The grid the metronome clicks on.
    grid: BeatGrid,
    /// Whether the metronome is on.
    click: bool,
}

/// The two clicks the metronome plays, worked out once when the audition
/// starts: the higher one for a downbeat and the other for every other
/// beat.
///
/// Both channels of a click hold the same sample, so one number describes a
/// frame of it.
struct Clicks {
    /// The click on a beat that is not a downbeat.
    beat: Vec<Frame>,
    /// The click on a downbeat.
    downbeat: Vec<Frame>,
}

impl Clicks {
    /// Both clicks.
    fn new() -> Clicks {
        Clicks {
            beat: click(false),
            downbeat: click(true),
        }
    }

    /// Frame `at` of the click a beat begins.
    fn sample(&self, downbeat: bool, at: i64) -> f32 {
        let click = if downbeat { &self.downbeat } else { &self.beat };
        click[at as usize][0]
    }
}

/// Whether track frame `frame` is a beat of `grid`, and when it is, whether
/// that beat is a downbeat.
///
/// A frame is a beat when some whole beat index, negative ones included,
/// falls on it. The beat index nearest the frame is worked out first, and
/// that index and its two neighbours are the only ones whose frame can be
/// this one, so three tries settle it. That holds while one frame spans at
/// most about two beats, which every grid a person works with does by a
/// wide margin: a frame is one forty-four-thousandth of a second and a beat
/// is a fraction of a second. Only a tempo of hundreds of thousands of
/// beats a minute would break it, and the metronome would be a buzz rather
/// than a click long before that.
fn beat_at(grid: &BeatGrid, frame: i64) -> Option<bool> {
    let near = grid.beat_at_position(Samples(frame)).0.round() as i64;
    (near.saturating_sub(1)..=near.saturating_add(1))
        .find(|&beat| grid.position_of(Beats(beat as f64)).0 == frame)
        .map(|beat| beat.rem_euclid(i64::from(BEATS_PER_BAR)) == 0)
}

/// A track's sample as the audition plays it: the sample at
/// [`TRACK_GAIN`], or silence when the sample is not a finite number, so
/// the audition never hands a device a sample it cannot play.
fn playable(sample: f32) -> f32 {
    if sample.is_finite() {
        sample * TRACK_GAIN
    } else {
        0.0
    }
}

/// A finished sample held within full scale, which is what a device is
/// handed: a sum above one is one and a sum below minus one is minus one,
/// so a grid fast enough for its clicks to pile up on one another cannot
/// clip.
fn held(sample: f32) -> f32 {
    sample.clamp(-1.0, 1.0)
}

/// Makes the `out.len()` frames of the audition from track frame `from` on,
/// all of them under one setting: the track's frames at [`TRACK_GAIN`] plus
/// the clicks of `grid` when `metronome` is on, held within full scale.
///
/// Frame `t` holds frame `t - b` of the click of every beat `b` of `grid`
/// with `b <= t < b + CLICK_LENGTH`, so a beat before `from`, and a beat
/// before the track's first frame, sounds its tail here. A beat whose click
/// reaches these frames falls either among them or in the click's length
/// before `from`, and those are the frames searched for beats.
fn frames_under(
    audio: &Audio,
    clicks: &Clicks,
    grid: &BeatGrid,
    metronome: bool,
    from: i64,
    out: &mut [Frame],
) {
    let end = from + out.len() as i64;
    for (offset, frame) in out.iter_mut().enumerate() {
        let track = audio.frames[(from + offset as i64) as usize];
        *frame = [playable(track[0]), playable(track[1])];
    }
    if metronome {
        for beat in (from - CLICK_LENGTH.0 + 1)..end {
            let Some(downbeat) = beat_at(grid, beat) else {
                continue;
            };
            for frame in beat.max(from)..(beat + CLICK_LENGTH.0).min(end) {
                let sample = clicks.sample(downbeat, frame - beat);
                let sounding = &mut out[(frame - from) as usize];
                sounding[0] += sample;
                sounding[1] += sample;
            }
        }
    }
    // Every click that sounds in these frames has been added by now, because
    // a click begins at or before the frame it sounds in, so the frames are
    // finished and can be held within full scale.
    for frame in out.iter_mut() {
        *frame = [held(frame[0]), held(frame[1])];
    }
}

/// Makes the audition's frames block by block and hands each block to the
/// device's feed, ending when the device has pulled the track's last frame,
/// when the device has stopped taking frames, or when the audition has been
/// stopped.
///
/// Where each block belongs comes from the feed rather than from a count
/// kept here, so a move that throws the frames of the old position away is
/// followed by making the frames of the new one, and a block that is refused
/// is made again from the frame it was meant for.
///
/// Nothing made under a replaced setting reaches the device. The run number
/// comes from the feed before the setting is copied, and frames go in only
/// while the feed still stands at that run number. A change of setting makes
/// the frames already in the feed again and moves the run number on, both
/// under the feed's lock. A block made beside such a change is therefore
/// either in the feed before the change, and the change makes that block
/// again along with the rest, or it meets the new run number at the delivery
/// and is refused. One block can be both, because the feed takes a block
/// piece by piece: the change makes the pieces already in the feed again,
/// the feed refuses the rest of the block, and the next look at the feed
/// puts this loop on the first frame still wanted, which it makes again
/// under the new setting.
fn make_frames(channel: &Channel, setting: &Mutex<Setting>, audio: &Audio, clicks: &Clicks) {
    let length = audio.frames.len() as i64;
    let mut block = vec![[0.0f32; 2]; BLOCK_FRAMES];
    // The room in the feed is asked for before the block is made, and the
    // block is no larger than that room, so the frames already made are never
    // more than a lookahead past the frame the device is at.
    while let Some(room) = channel.room_for_audition() {
        // The run number in `room` is read from the feed before this copy of
        // the setting, so a change that lands between the two is either in
        // this block already or refuses it at the delivery below.
        let (grid, metronome) = {
            let setting = setting.lock().unwrap();
            (setting.grid, setting.click)
        };
        let count = room
            .frames
            .min(BLOCK_FRAMES)
            .min((length - room.from) as usize);
        let block = &mut block[..count];
        frames_under(audio, clicks, &grid, metronome, room.from, block);
        // A block the feed refuses was made under a setting a person has
        // replaced, or for a position a person has moved away from. Either
        // way the next look at the feed says where the frames now wanted
        // begin, and a device that has stopped taking frames ends the loop
        // there.
        let _ = channel.deliver(block, room.run);
    }
    // However the frames ended, no more are coming, so an output that pulls
    // until its feed is finished stops once the device has taken what is
    // left in it.
    channel.close();
}

/// One track playing on its own through an [`Output`], with a metronome on
/// its grid.
pub struct Audition {
    /// The output the audition plays through, started by
    /// [`start`](Audition::start) and stopped by [`stop`](Audition::stop) on
    /// whichever thread called them, and touched by nothing else.
    output: Box<dyn Output>,
    /// The frames in flight between the thread making them and the output,
    /// which is also where the position and what the device is doing at this
    /// moment are kept.
    channel: Arc<Channel>,
    /// The setting the frames are made under, shared with the thread that
    /// makes them.
    setting: Arc<Mutex<Setting>>,
    /// The track the audition plays, kept so that the frames already made
    /// ahead of the device can be made again when a person changes the
    /// setting.
    audio: Arc<Audio>,
    /// The clicks the metronome plays, kept for the same reason as the
    /// track and shared with the thread making the frames.
    clicks: Arc<Clicks>,
    /// The thread the frames are made on, until [`stop`](Audition::stop) or
    /// dropping the audition ends it.
    thread: Option<JoinHandle<()>>,
    /// The track's length in frames.
    length: i64,
}

impl Audition {
    /// Starts playing `audio` from the track frame `from`, clicking on the
    /// beats of `grid` when `click` is true, through `output`.
    ///
    /// A `from` before the first frame starts at the first frame. The
    /// output is started before this returns, and the audition keeps up to
    /// [`AUDITION_LOOKAHEAD`] frames ready ahead of it from then on. A grid
    /// whose tempo is not a positive finite number is refused with a
    /// message saying so, before the output is touched at all; an output
    /// that cannot start is refused with the output's own message, and is
    /// not stopped, since it never started. In neither case is anything
    /// left running.
    pub fn start(
        audio: Arc<Audio>,
        grid: BeatGrid,
        from: Samples,
        click: bool,
        mut output: Box<dyn Output>,
    ) -> Result<Audition, String> {
        if !grid.bpm.is_valid() {
            return Err(format!(
                "a grid whose tempo is {} beats per minute has no beats to click on",
                grid.bpm.0
            ));
        }
        let length = audio.frames.len() as i64;
        let at = from.0.clamp(0, length);
        // The feed holds one lookahead, and never less than one block: a
        // feed with no room for a whole block would leave the frames just
        // made nowhere to go.
        let capacity = AUDITION_LOOKAHEAD.0.max(BLOCK_FRAMES as i64) as usize;
        let channel = Arc::new(Channel::for_audition(capacity, length, at));
        output.start(Feed::new(Arc::clone(&channel)))?;

        let setting = Arc::new(Mutex::new(Setting { grid, click }));
        let clicks = Arc::new(Clicks::new());
        let thread = std::thread::spawn({
            let channel = Arc::clone(&channel);
            let setting = Arc::clone(&setting);
            let audio = Arc::clone(&audio);
            let clicks = Arc::clone(&clicks);
            move || make_frames(&channel, &setting, &audio, &clicks)
        });
        Ok(Audition {
            output,
            channel,
            setting,
            audio,
            clicks,
            thread: Some(thread),
            length,
        })
    }

    /// Where the audition is and what it is doing, read in one look so that
    /// the position and the state agree.
    pub fn status(&self) -> AuditionStatus {
        let reading = self.channel.reading();
        let state = if let Some(message) = reading.failure {
            AuditionState::Failed(message)
        } else if reading.at >= self.length {
            AuditionState::Ended
        } else {
            AuditionState::Playing
        };
        AuditionStatus {
            state,
            position: Samples(reading.at),
            length: Samples(self.length),
            click: self.setting.lock().unwrap().click,
        }
    }

    /// Replaces the grid the metronome clicks on, and returns the first
    /// track frame the device pulls with the new grid, which is the
    /// position at the moment the change takes effect. Every frame the
    /// device pulled before that frame was made with the grid it replaces,
    /// and every frame it pulls from that frame on is made with the new
    /// one: the audition makes the frames it had ready ahead again, in
    /// place, under the same lock it reads the position under, and a block
    /// the making thread built under the old grid is never delivered after
    /// that. No frame is lost or repeated and no pull comes up short
    /// because of the call. The call
    /// does not wait for the device: it returns within the time it takes
    /// to make one lookahead of frames, so the window may call it on every
    /// move of a drag and on every repeat of a held key. A grid whose
    /// tempo is not a positive finite number is refused whatever the
    /// audition's state: nothing changes and nothing is returned. Once the
    /// audition has ended or failed no more frames are made, so the grid
    /// is taken and the returned frame is the position, which after the
    /// end is the track's length.
    pub fn set_grid(&mut self, grid: BeatGrid) -> Option<Samples> {
        if !grid.bpm.is_valid() {
            return None;
        }
        Some(self.change(|setting| setting.grid = grid))
    }

    /// Turns the metronome on or off, and returns the first track frame the
    /// device pulls with the new setting, which is the position at the
    /// moment the change takes effect, on the same terms as
    /// [`set_grid`](Audition::set_grid): every frame pulled before it was
    /// made with the old setting and every frame from it on with the new
    /// one, nothing is lost or repeated, no pull comes up short, and the
    /// call returns within the time it takes to make one lookahead of
    /// frames. Once the audition has ended or failed no more frames are
    /// made, so the setting is taken and the returned frame is the
    /// position.
    pub fn set_click(&mut self, on: bool) -> Samples {
        self.change(|setting| setting.click = on)
    }

    /// Moves the audition to track frame `to`, held between the first frame
    /// and the track's length. The audition throws away the frames it made
    /// ahead of the old position, reports `to` as the position at once,
    /// and holds the device off until the frames of `to` fill the
    /// lookahead or the last frame of the track has been made, whichever
    /// comes first. A pull that lands while the device is held off gets
    /// nothing rather than the wrong frames, and a pull already under way
    /// when the move is made completes with the frames of the old position
    /// and the move follows it. The next pull that gets frames gets the
    /// frames of `to` and nothing of the old position, with the tails of
    /// clicks that began before `to` sounding as they do when an audition
    /// starts there, and every pull after that follows on without a gap.
    /// The grid and the metronome setting are kept. A move while the
    /// status says [`Playing`](AuditionState::Playing) plays from `to`,
    /// including while the last frames the audition made are still being
    /// drained by the device. A move to or past the length ends the
    /// audition at the length. A move once the status says
    /// [`Ended`](AuditionState::Ended) or [`Failed`](AuditionState::Failed)
    /// changes nothing, so the window starts a new audition to play again
    /// after the end. [`AuditionStatus`] reports no underrun count, so the
    /// pulls a move leaves empty are reported nowhere.
    pub fn seek(&mut self, to: Samples) {
        self.channel.move_to(to.0.clamp(0, self.length));
    }

    /// Makes a change to the setting and answers the first track frame the
    /// device pulls with it.
    ///
    /// The setting lock is held while the frames already made ahead of the
    /// device are made again under the new setting, and the frames are made
    /// again under the feed's lock, which is the lock a pull takes and the
    /// lock the position is read under. The change therefore reaches every
    /// frame the device has not yet taken, and the frame it reports is the
    /// frame the next pull begins at: a pull that lands before the change
    /// finishes takes frames of the setting it replaces and moves the
    /// position on before the rewrite sees it, and a pull that lands after
    /// takes frames made under the new setting.
    ///
    /// An audition that has ended or failed has no frames waiting, so
    /// nothing is made again and the frame reported is the position, which
    /// after the end is the track's length.
    fn change(&self, change: impl FnOnce(&mut Setting)) -> Samples {
        let mut setting = self.setting.lock().unwrap();
        change(&mut setting);
        let at = self.channel.remake(|from, frames| {
            frames_under(
                &self.audio,
                &self.clicks,
                &setting.grid,
                setting.click,
                from,
                frames,
            );
        });
        Samples(at)
    }

    /// Ends the thread making the frames and closes the feed, which an
    /// output that pulls until its feed is finished sees. A second call does
    /// nothing.
    fn end_making(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.channel.close();
        // The Rust runtime prints a panic on the thread making the frames
        // to standard error as it happens, so the panic a join reports has
        // already been seen and there is nothing more to do about it here.
        let _ = thread.join();
    }

    /// Ends the audition and stops the output. When this returns, the
    /// device has been given back.
    pub fn stop(mut self) {
        // The output is stopped first, and on this thread, so that no pull
        // can happen after it.
        self.output.stop();
        self.end_making();
    }
}

impl Drop for Audition {
    /// Stops the output and ends the thread making the frames of an
    /// audition dropped without being stopped, so that no thread of it and
    /// no device it was playing through is left running.
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.output.stop();
            self.end_making();
        }
    }
}
