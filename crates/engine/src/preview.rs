//! Real-time preview: a span of the mix rendered ahead of an audio device
//! that pulls it at its own pace.
//!
//! A preview is [`render_range`](crate::render_range) with its sink pointed
//! at a [`Feed`] instead of a file. The render runs on the calling thread
//! and keeps the feed filled up to a lookahead; the device, on its own
//! thread, pulls frames from the feed whenever it needs them. Because the
//! frames are the render's own, a preview never plays anything a render of
//! the same document would not write. That promise is what the acceptance
//! tests in `tests/preview.rs` hold, by pulling a preview through a
//! capturing output and comparing it with the render. The tests compare a
//! track without keylock frame for frame. They compare a keylocked track by
//! where the beats land, how loud the music is, and at what pitch each tone
//! comes through, since a run-in cannot bring the pitch-preserving stretcher
//! to the state a render of the whole mix leaves that stretcher in.
//!
//! A [`Transport`](crate::Transport) plays through the same feed, from a
//! thread of its own, and moves the frames in it to another position or
//! another document while the device goes on pulling. Where a part of a
//! feed serves only a transport, its description below says so.

use std::collections::VecDeque;
#[cfg(feature = "playback")]
use std::num::NonZeroU32;
use std::ops::Range;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use dermixen_core::{Mix, Samples, Track};
use dermixen_media::Frame;

use crate::render::{BLOCK_FRAMES, Loader, Progress, RenderError, clip, mix_length, render_range};
use crate::stretch::TimeStretcher;

/// The frames a feed holds and what has become of the ones already pulled.
///
/// One mutex guards all of it, so a pull takes its frames, moves the
/// position on, and counts itself in one step, and can never see a
/// half-finished change made by the render or by the person driving the
/// transport.
struct Queue {
    /// The frames waiting to be pulled, the earliest first.
    frames: VecDeque<Frame>,
    /// The output frame the earliest waiting frame sits at, which is the
    /// frame the next pull begins at. Every pull moves it on by the number
    /// of frames it took, and a transport moves it to wherever a person put
    /// the playhead.
    at: i64,
    /// How many frames of the span the render has put in.
    filled: i64,
    /// How many frames the output has pulled altogether.
    pulled: i64,
    /// How many pulls got fewer frames than they asked for while the render
    /// was still going.
    underruns: u64,
    /// Whether the render has put in the last frame it has to give at the
    /// position the feed stands at, so that a pull getting less than it
    /// asked for has reached the end of what there was rather than a gap.
    delivered: bool,
    /// Whether no more frames will ever be delivered, which is what an
    /// output that pulls until the feed is finished waits for.
    ended: bool,
    /// Whether the feed has been closed for good by
    /// [`close`](Channel::close), which is what tells the thread making an
    /// [`Audition`](crate::Audition)'s frames that the audition has been
    /// stopped. For an audition, [`take`](Channel::take) sets `ended` when
    /// the frame at the track's length goes in, and
    /// [`move_to`](Channel::move_to) clears `ended` again when a move takes
    /// the audition back into the track, so `ended` alone does not say that
    /// no more frames are wanted.
    closed: bool,
    /// Whether the device is held off while the render brings the position
    /// up and fills the lookahead. Only a transport holds a device off this
    /// way; a fixed span fills its feed before its output is started at all.
    holding: bool,
    /// Whether a person has paused the transport, which holds the device off
    /// and, once the lookahead is full, the render with it.
    paused: bool,
    /// Whether the device has pulled a frame since the render last began at
    /// a new position, which is how a transport tells playing apart from
    /// waiting for the first frames after a move.
    heard: bool,
    /// Which run of the render the frames belong to. A transport counts one
    /// run for every position it sets the render going at, so that a block
    /// offered by a run it has abandoned is refused rather than played.
    run: u64,
    /// What the device said when it stopped taking frames for good, if it
    /// has said anything.
    failure: Option<String>,
}

/// What a [`Transport`](crate::Transport) reads about its feed in one look
/// under the lock, so that the position and what is happening at it agree.
pub(crate) struct Reading {
    /// The output frame the next pull begins at.
    pub(crate) at: i64,
    /// The output frame just past the last one the render has put in, so
    /// that every frame from the position up to it is waiting to be pulled.
    /// It is the position itself when the feed is empty, and a run of the
    /// render that has just been set going begins at the position.
    pub(crate) reached: i64,
    /// How many pulls got fewer frames than they asked for while the device
    /// was taking frames of the mix.
    pub(crate) underruns: u64,
    /// Whether a person is holding the transport paused.
    pub(crate) paused: bool,
    /// Whether the device has taken a frame of the mix since the render was
    /// last set going at a position, rather than still waiting for the first
    /// frames of that position. A feed that has run dry under a device that
    /// was playing stays playing, since the mix is what the device is in the
    /// middle of.
    pub(crate) playing: bool,
    /// What stopped the preview for good, if anything has: the words of the
    /// device that stopped taking frames, or of the render that could not go
    /// on.
    pub(crate) failure: Option<String>,
}

/// Where an [`Audition`](crate::Audition)'s next block of frames belongs,
/// read in one look under the lock.
pub(crate) struct Room {
    /// The number of the run the block belongs to, which for an audition is
    /// the number of the setting the block is made under.
    /// [`deliver`](Channel::deliver) refuses the block under any other
    /// number, so a block made under a setting a person has since replaced
    /// never reaches the device.
    pub(crate) run: u64,
    /// The track frame the block begins at, which is the frame just past the
    /// last one waiting to be pulled.
    pub(crate) from: i64,
    /// How many frames there is room for in the feed.
    pub(crate) frames: usize,
}

/// What [`Channel::deliver`] says when the transport has moved on from the
/// run whose block it was offered. The transport tells this apart from a
/// real failure by the run number rather than by these words, and nobody is
/// ever shown them.
const ABANDONED: &str = "the transport has moved on from this run of the render";

/// The one thing the render and the device share: the frames in flight
/// between them.
///
/// The device's end of it is a [`Feed`], which only ever takes frames out.
/// The render's end puts frames in and waits when there is no room, which is
/// what holds a preview to real time. The device waits for nothing, so a
/// render that falls behind costs a gap in the sound rather than a stalled
/// audio callback.
pub(crate) struct Channel {
    /// The frames and the counts, under one lock.
    queue: Mutex<Queue>,
    /// Signalled by every pull, and by every order a transport gives, so that
    /// a render waiting for room to put more frames in, and a preview waiting
    /// for the feed to drain, wake as soon as there is something for them to
    /// do.
    room: Condvar,
    /// How many frames the feed holds when it is full, which is the lookahead
    /// the preview was asked for.
    capacity: usize,
    /// How many frames the span holds, so that the last of them going in is
    /// what marks the feed ended. A transport plays no fixed span: its feed
    /// delivers frames until the transport is stopped, so this is then as
    /// large as a count of frames can be and nothing the render puts in ever
    /// reaches it. An audition plays one track, so this is that track's
    /// length and the feed ends when the frame at that length has gone in.
    length: i64,
    /// Whether the frames in this feed are one track's own, made by an
    /// [`Audition`](crate::Audition) rather than rendered from the mix.
    ///
    /// An audition's positions are frames of its track, and a move takes the
    /// audition to any frame of that track, so how many frames have gone in
    /// says nothing about how many are left. The end of an audition's feed
    /// is the frame at the track's length going in, which is a position
    /// rather than a count.
    audition: bool,
    /// Whether the frames in this feed belong to a
    /// [`Transport`](crate::Transport) rather than to one fixed span of
    /// [`play`].
    ///
    /// A transport's device pulls on through pauses, moves, and the end of
    /// the mix, when the feed has nothing for it because a person asked for
    /// silence, so a pull that finds it empty is not counted as an underrun.
    /// The device of a fixed span finds the feed empty only when the render
    /// has fallen behind, which is a gap in the sound and is counted.
    transport: bool,
}

impl Channel {
    /// A channel with room for `capacity` frames, for the span of `length`
    /// frames that begins at output frame `at`.
    fn new(capacity: usize, length: i64, at: i64) -> Channel {
        Channel {
            queue: Mutex::new(Queue {
                frames: VecDeque::with_capacity(capacity),
                at,
                filled: 0,
                pulled: 0,
                underruns: 0,
                delivered: false,
                ended: false,
                closed: false,
                holding: false,
                paused: false,
                heard: false,
                run: 0,
                failure: None,
            }),
            room: Condvar::new(),
            capacity,
            length,
            transport: false,
            audition: false,
        }
    }

    /// A channel with room for `capacity` frames for a
    /// [`Transport`](crate::Transport) that begins at output frame `at`.
    ///
    /// The device is held off until the first run of the render has filled
    /// the lookahead, and nothing but [`close`](Channel::close) ever marks
    /// the feed ended, so an output that pulls until the feed is finished
    /// goes on pulling through pauses, moves, and the end of the mix.
    pub(crate) fn for_transport(capacity: usize, at: i64) -> Channel {
        let mut channel = Channel::new(capacity, i64::MAX, at);
        channel.transport = true;
        channel.queue.get_mut().unwrap().holding = true;
        channel
    }

    /// A channel with room for `capacity` frames, for one track of `length`
    /// frames that an [`Audition`](crate::Audition) plays from that track's
    /// frame `at`.
    ///
    /// The frames are one track's own rather than the mix's, so the position
    /// a pull reports is a position within the track. Nothing holds the
    /// device off at the start, and an audition starts its output before the
    /// thread that makes the frames, so the first pull may find the feed
    /// empty and get nothing. Every pull after that gets the frames as fast
    /// as the audition makes them. The feed ends as the frame at `length`
    /// goes in, and a move back into the track with
    /// [`move_to`](Channel::move_to) unmarks that ending and holds the
    /// device off until the frames of the new position are ready.
    pub(crate) fn for_audition(capacity: usize, length: i64, at: i64) -> Channel {
        let mut channel = Channel::new(capacity, length, at);
        channel.audition = true;
        channel
    }

    /// Puts as many of `block`'s frames in as there is room for and returns
    /// how many that was, which is none when the feed is already full. This
    /// never waits.
    fn fill(&self, block: &[Frame]) -> usize {
        self.take(self.queue.lock().unwrap(), block)
    }

    /// Waits until the feed has room, then puts as many of `block`'s frames in
    /// as fit and returns how many that was.
    ///
    /// When the device has stopped taking frames, no frames go in and its
    /// message comes back instead, which is how a device that fails part way
    /// through a preview stops the render rather than leaving it waiting for
    /// room that will never come.
    fn put(&self, block: &[Frame]) -> Result<usize, String> {
        let mut queue = self.queue.lock().unwrap();
        while queue.frames.len() >= self.capacity && queue.failure.is_none() {
            queue = self.room.wait(queue).unwrap();
        }
        if let Some(message) = &queue.failure {
            return Err(message.clone());
        }
        Ok(self.take(queue, block))
    }

    /// Waits until the feed has room, then puts the whole of `block` in on
    /// behalf of run `run`, waiting again as many times as that takes.
    ///
    /// This is what a [`Transport`](crate::Transport) delivers through. It
    /// gives up on the rest of the block, with an error, when the device has
    /// stopped taking frames and when the transport has abandoned the run,
    /// which is how a move, a replacement that starts the render over, and a
    /// stop take effect without waiting for the render to reach the end of
    /// the mix.
    ///
    /// A person holding the transport paused stops the render here as well,
    /// once the lookahead has been filled: the device is taking nothing, so
    /// there is nothing for the render to keep ahead of it, and a paused
    /// transport that renders no further is a transport whose whole status
    /// stands still. A pause taken while the lookahead is still being filled
    /// lets that finish first, so that resuming plays without a wait.
    pub(crate) fn deliver(&self, block: &[Frame], run: u64) -> Result<(), String> {
        let mut rest = block;
        while !rest.is_empty() {
            let mut queue = self.queue.lock().unwrap();
            while (queue.frames.len() >= self.capacity || (queue.paused && !queue.holding))
                && queue.failure.is_none()
                && queue.run == run
            {
                queue = self.room.wait(queue).unwrap();
            }
            if let Some(message) = &queue.failure {
                return Err(message.clone());
            }
            if queue.run != run {
                return Err(ABANDONED.to_owned());
            }
            let count = self.take(queue, rest);
            rest = &rest[count..];
        }
        Ok(())
    }

    /// Waits until an [`Audition`](crate::Audition) has frames to make and
    /// answers where the next block of them belongs, or nothing at all when
    /// no more frames are wanted: the device has stopped taking them, the
    /// audition has been stopped, or the device has pulled the frame at the
    /// track's length.
    ///
    /// The audition makes no more frames than the answer holds, so the
    /// frames it has made are never more than one lookahead past the frame
    /// the device is at. Where every frame up to the track's end is already
    /// in the feed, the call waits rather than answering, because a move can
    /// take the audition back into the track and the frames of the new
    /// position are made when it does.
    pub(crate) fn room_for_audition(&self) -> Option<Room> {
        let mut queue = self.queue.lock().unwrap();
        loop {
            if queue.failure.is_some() || queue.closed || queue.at >= self.length {
                return None;
            }
            let made = queue.at + queue.frames.len() as i64;
            if queue.frames.len() < self.capacity && made < self.length {
                return Some(Room {
                    run: queue.run,
                    from: made,
                    frames: self.capacity - queue.frames.len(),
                });
            }
            queue = self.room.wait(queue).unwrap();
        }
    }

    /// Makes the frames waiting to be pulled again, in place, under a
    /// setting an [`Audition`](crate::Audition) has just replaced, and
    /// answers the frame the earliest of them sits at, which is the frame
    /// the next pull begins at.
    ///
    /// This method gives `rewrite` that frame and the frames themselves to
    /// overwrite. `rewrite` runs under the lock, so the frames it writes are
    /// the frames the next pull takes and no pull can land in the middle of
    /// the rewriting. A pull already under way holds the lock and finishes
    /// with the frames it took, and the rewriting follows that pull.
    ///
    /// The run number moves on, so a block the thread making the frames
    /// built under the setting this call replaces is refused by
    /// [`deliver`](Channel::deliver) instead of landing on top of the
    /// rewritten frames. That thread then makes the block again under the
    /// new setting.
    pub(crate) fn remake(&self, rewrite: impl FnOnce(i64, &mut [Frame])) -> i64 {
        let mut queue = self.queue.lock().unwrap();
        let at = queue.at;
        rewrite(at, queue.frames.make_contiguous());
        queue.run += 1;
        drop(queue);
        self.room.notify_all();
        at
    }

    /// Moves an [`Audition`](crate::Audition) to track frame `to`, throwing
    /// away the frames made ahead of the old position and holding the device
    /// off until the frames of `to` fill the feed or the frame at the
    /// track's length goes in.
    ///
    /// The move is refused, and nothing changes, once the device has pulled
    /// the frame at the track's length or has stopped taking frames. A move
    /// to the track's length ends the audition there. The run number moves
    /// on, so no block made for the old position is delivered after the
    /// move, and the thread making the frames wakes to make the frames of
    /// `to`.
    pub(crate) fn move_to(&self, to: i64) {
        let mut queue = self.queue.lock().unwrap();
        if queue.failure.is_some() || queue.at >= self.length {
            return;
        }
        let ends = to >= self.length;
        queue.frames.clear();
        queue.at = to;
        queue.delivered = ends;
        queue.ended = ends;
        queue.holding = !ends;
        queue.run += 1;
        drop(queue);
        self.room.notify_all();
    }

    /// Moves frames from `block` into a queue that is already locked, stopping
    /// when the feed is full or when `block` runs out.
    fn take(&self, mut queue: MutexGuard<'_, Queue>, block: &[Frame]) -> usize {
        let count = self
            .capacity
            .saturating_sub(queue.frames.len())
            .min(block.len());
        queue.frames.extend(block[..count].iter().copied());
        queue.filled += count as i64;
        // The feed ends as the last frame of the span goes in rather than when
        // the render hands back. A pull that lands between those two moments
        // got everything there was to get, and counting it as an underrun
        // would report a gap nobody heard. An audition plays one track and a
        // person moves it to any frame of that track, so the last frame of an
        // audition is the frame at the track's length rather than the last of
        // a fixed count.
        let last_frame_in = if self.audition {
            queue.at + queue.frames.len() as i64 >= self.length
        } else {
            queue.filled >= self.length
        };
        if last_frame_in {
            queue.delivered = true;
            queue.ended = true;
            // A move to within a lookahead of the track's end leaves fewer
            // frames to make than the feed holds when it is full, so the last
            // frame going in is what lets the device take them.
            queue.holding = false;
        }
        // A transport holds its device off until the render has filled the
        // lookahead, so that the frames a person hears after a move have as
        // much again behind them as any other moment of the mix.
        if queue.frames.len() >= self.capacity {
            queue.holding = false;
        }
        count
    }

    /// Begins a new run of the render, throwing away the frames the
    /// abandoned run had put in and holding the device off until the new run
    /// has filled the lookahead.
    ///
    /// `where_to` is given the output frame the device has reached and
    /// answers with the frame the new run begins at and whether a person is
    /// holding the transport there. It is asked while the lock is held, so
    /// a pull that lands while the transport is making up its mind is part
    /// of what it decides from rather than something the decision undoes.
    /// The new run's number and the frame it begins at come back, and a
    /// render waiting for room wakes to find the run it was filling gone.
    pub(crate) fn restart(&self, where_to: impl FnOnce(i64) -> (i64, bool)) -> (u64, i64) {
        let mut queue = self.queue.lock().unwrap();
        let (at, paused) = where_to(queue.at);
        queue.frames.clear();
        queue.at = at;
        queue.paused = paused;
        queue.holding = true;
        queue.heard = false;
        queue.delivered = false;
        queue.run += 1;
        let run = queue.run;
        drop(queue);
        self.room.notify_all();
        (run, at)
    }

    /// Marks run `run` as having put in the last frame it has to give, and
    /// lets the device take what is there however little that is, since no
    /// more is coming. A run the transport has already abandoned changes
    /// nothing.
    pub(crate) fn delivered(&self, run: u64) {
        let mut queue = self.queue.lock().unwrap();
        if queue.run == run {
            queue.delivered = true;
            queue.holding = false;
        }
    }

    /// Holds the device off, or lets it take frames again, for a person
    /// pausing and resuming a transport.
    ///
    /// A render waiting out a pause in [`deliver`](Channel::deliver) is woken
    /// by the resumption, so it takes up where it left off.
    pub(crate) fn set_paused(&self, paused: bool) {
        self.queue.lock().unwrap().paused = paused;
        self.room.notify_all();
    }

    /// Everything a transport reports about itself, read in one look under
    /// the lock.
    pub(crate) fn reading(&self) -> Reading {
        let queue = self.queue.lock().unwrap();
        Reading {
            at: queue.at,
            reached: queue.at + queue.frames.len() as i64,
            underruns: queue.underruns,
            paused: queue.paused,
            playing: queue.heard && !queue.holding,
            failure: queue.failure.clone(),
        }
    }

    /// Which run of the render the feed is taking frames from.
    pub(crate) fn run(&self) -> u64 {
        self.queue.lock().unwrap().run
    }

    /// Ends the feed for good: nothing more will be delivered, the render
    /// filling it wakes to find its run abandoned, the thread making an
    /// audition's frames wakes to find the feed closed, and an output that
    /// pulls until the feed is finished stops pulling.
    pub(crate) fn close(&self) {
        let mut queue = self.queue.lock().unwrap();
        queue.run += 1;
        queue.delivered = true;
        queue.ended = true;
        queue.closed = true;
        drop(queue);
        self.room.notify_all();
    }

    /// How many frames are waiting to be pulled.
    fn waiting(&self) -> usize {
        self.queue.lock().unwrap().frames.len()
    }

    /// Marks the render's last frame as delivered, so that an output waiting
    /// for more frames sees the end of the span instead of waiting for more.
    ///
    /// A span played all the way through was already marked ended as its last
    /// frame went in, so this is what marks a span the render gave up on.
    fn end(&self) {
        let mut queue = self.queue.lock().unwrap();
        queue.delivered = true;
        queue.ended = true;
    }

    /// Records what the device said when it stopped taking frames and throws
    /// away the frames it will never take, then wakes whoever waits on the
    /// feed.
    pub(crate) fn fail(&self, message: &str) {
        let mut queue = self.queue.lock().unwrap();
        if queue.failure.is_none() {
            queue.failure = Some(message.to_owned());
        }
        queue.frames.clear();
        drop(queue);
        self.room.notify_all();
    }

    /// What stopped the preview for good, if anything has.
    pub(crate) fn failure(&self) -> Option<String> {
        self.queue.lock().unwrap().failure.clone()
    }

    /// Whether the output has pulled every frame the render delivered and
    /// none are coming, so what the device is still holding is the end of
    /// what there was to play.
    ///
    /// A preview whose device stopped taking frames has not, since the
    /// frames it was given have been thrown away, and neither has one
    /// stopped with frames still waiting in its feed.
    #[cfg(feature = "playback")]
    fn played_out(&self) -> bool {
        let queue = self.queue.lock().unwrap();
        queue.failure.is_none() && queue.delivered && queue.frames.is_empty()
    }

    /// Waits for the output to pull past `since` frames, then returns how many
    /// it has pulled altogether and whether it has now pulled every frame the
    /// render put in.
    ///
    /// Ask with a negative count, which the output can never have pulled, to
    /// read both values straight away without waiting. They come from one look
    /// under the lock, so a preview is never told the feed has drained
    /// alongside a stale count of what was played out of it.
    fn pulled_since(&self, since: i64) -> (i64, bool) {
        let mut queue = self.queue.lock().unwrap();
        while queue.pulled == since && !queue.frames.is_empty() {
            queue = self.room.wait(queue).unwrap();
        }
        (queue.pulled, queue.frames.is_empty())
    }

    /// How many frames the output has pulled so far.
    fn pulled(&self) -> i64 {
        self.queue.lock().unwrap().pulled
    }

    /// How many pulls got fewer frames than they asked for.
    fn underruns(&self) -> u64 {
        self.queue.lock().unwrap().underruns
    }
}

/// The frames a preview has rendered ahead of the device, from which the
/// device pulls.
///
/// A feed is the device's end of the preview. It owns everything it needs
/// and is `Send`, so the device moves it to its own thread. The render fills
/// the feed from the other end, and the render may wait for the device to
/// make room, but the device never waits for the render: when it asks for
/// more frames than the feed has ready, [`pull`](Feed::pull) gives what is
/// there and counts the call as an underrun if the silence was a gap in the
/// sound rather than silence a person asked for, and the device is expected
/// to play silence for the rest, so a stall in the render is heard as a gap
/// and never as the wrong audio. The feed holds at most the preview's
/// lookahead, and exactly that many frames when it is full.
pub struct Feed {
    /// The frames in flight between the render and this device.
    channel: Arc<Channel>,
}

impl Feed {
    /// The device's end of a channel.
    pub(crate) fn new(channel: Arc<Channel>) -> Feed {
        Feed { channel }
    }

    /// How many frames are ready to be pulled at this moment. A transport
    /// that is buffering or paused has none ready however many its render
    /// has put in, since the device is meant to hear silence then.
    pub fn available(&self) -> usize {
        let queue = self.channel.queue.lock().unwrap();
        if queue.holding || queue.paused {
            0
        } else {
            queue.frames.len()
        }
    }

    /// Whether no more frames will be delivered into the feed, because the
    /// render has delivered the last frame of the span or has stopped early
    /// on an error. Frames may still be waiting to be pulled. The feed of a
    /// [`Transport`](crate::Transport) says no until the transport is
    /// stopped, whatever the transport is doing, because a move or a
    /// replacement sets the render going again. For the feed of an
    /// [`Audition`](crate::Audition), this method returns true once the
    /// frame at the track's length has gone in, and false again after a move
    /// takes the audition back into the track.
    pub fn ended(&self) -> bool {
        self.channel.queue.lock().unwrap().ended
    }

    /// Whether the feed has [`ended`](Feed::ended) and every frame has been
    /// pulled, so the device has nothing more to play.
    pub fn finished(&self) -> bool {
        let queue = self.channel.queue.lock().unwrap();
        queue.ended && queue.frames.is_empty()
    }

    /// Copies the frames that are ready into `out`, up to its length, and
    /// returns how many were copied.
    ///
    /// The frames copied are the ones ready at the moment of the call; the
    /// call never waits for more, and frames the render delivers while the
    /// call is under way are left for the next call. A call that copies fewer
    /// frames than `out` holds while the render still has frames to deliver
    /// is an underrun, counted once per such call; a short copy after the
    /// render has delivered its last frame is the end of what there was and
    /// is not. The feed of a [`Transport`](crate::Transport) counts none
    /// while the transport is buffering or paused, since a person asked for
    /// the silence the device hears then, and none for a call that copies
    /// nothing at all, since an output that asks for whatever the feed
    /// happens to hold empties it on every turn of its loop.
    pub fn pull(&mut self, out: &mut [Frame]) -> usize {
        self.pull_from(out).1
    }

    /// As [`pull`](Feed::pull), and also says which output frame the first
    /// frame copied sits at, on the clock `mix show` and the rendered file
    /// use, or, when nothing was copied, the frame the next pull would
    /// begin at. The position and the frames come from one look under the
    /// lock, so a move made by a [`Transport`](crate::Transport) between two
    /// pulls is never seen as the old position with the new frames or the
    /// other way round. A capturing output records this position with each
    /// pull to prove that every pull is the render of its own position. A
    /// short copy counts as an underrun on the same terms as `pull`, and
    /// those terms are the feed's: the feed of a
    /// [`Transport`](crate::Transport) counts none while the transport is
    /// buffering or paused.
    pub fn pull_from(&mut self, out: &mut [Frame]) -> (Samples, usize) {
        let mut queue = self.channel.queue.lock().unwrap();
        let at = Samples(queue.at);
        if queue.holding || queue.paused {
            // A transport means the device to hear silence here, so it gets
            // none of the frames its render may already have put in.
            return (at, 0);
        }
        // Whether this pull was in the middle of the mix and found some of
        // it, which is what a gap in the sound is made of. A pull that finds
        // the feed empty is not counted, because an output that asks for
        // whatever the feed happens to hold, as the acceptance tests' fast
        // recorder does, drains it on every turn of its loop and would
        // otherwise report a gap on every turn as well.
        let playing = queue.heard && !queue.frames.is_empty();
        let count = queue.frames.len().min(out.len());
        for (slot, frame) in out.iter_mut().zip(queue.frames.drain(..count)) {
            *slot = frame;
        }
        queue.pulled += count as i64;
        queue.at += count as i64;
        if count > 0 {
            queue.heard = true;
        }
        if count < out.len() && !queue.delivered && (playing || !self.channel.transport) {
            queue.underruns += 1;
        }
        drop(queue);
        // The render may be waiting for the room this pull has just made.
        self.channel.room.notify_all();
        (at, count)
    }

    /// Tells the render that the device has stopped pulling for good, with
    /// the text a person should read about why, such as the device's own
    /// error message.
    ///
    /// The render stops waiting for room, discards the frames still in the
    /// feed, and [`play`] returns [`RenderError::Output`] holding the
    /// message, after stopping the output as it does on every error. An
    /// output whose device reports an error after starting calls this from
    /// the device's error path instead of pulling on, so a device that
    /// disappears in the middle of a preview ends the preview rather than
    /// leaving it waiting forever.
    pub fn fail(&self, message: &str) {
        self.channel.fail(message);
    }
}

/// A sample as an audio device is handed it: silence for a sample that is
/// not a finite number, and otherwise the sample held within full scale, so
/// that no device receives a value it cannot play. A rendered file is held
/// within full scale the same way when it is written.
pub fn device_sample(sample: f32) -> f32 {
    sample
}

/// An audio device, or anything standing in for one, that plays a
/// [`Feed`] on its own thread.
pub trait Output {
    /// Starts playing the feed. From this call until [`stop`](Output::stop),
    /// the output pulls frames from the feed on a thread of its own whenever
    /// it needs them, playing silence for any it asked for and did not get,
    /// and pulls no more once the feed is [`finished`](Feed::finished). The
    /// error text is what a person should read when the device cannot be
    /// opened.
    fn start(&mut self, feed: Feed) -> Result<(), String>;

    /// Stops the output. When this returns, the output pulls no more frames
    /// and its thread has ended.
    fn stop(&mut self);
}

/// What a preview did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayReport {
    /// How many frames the device pulled. When the preview ran to the end
    /// this is the length of the span after clipping to the mix.
    pub played: Samples,
    /// How many times the device asked for frames the feed did not have.
    /// Zero means the preview was heard without a gap.
    pub underruns: u64,
}

/// Plays the span of a mix between two output frames through an output,
/// returning when the output has pulled the last frame.
///
/// The frames the output pulls are exactly the frames
/// [`render_range`](crate::render_range) delivers for the same span, with
/// the same clipping, and they are rendered on the calling thread as the
/// output plays, never all at once beforehand. Before the output is
/// started, the feed is filled with `lookahead` frames, or with the whole
/// span when it is shorter, so that the output begins with that much in
/// hand; from then on the render keeps at most `lookahead` frames ahead of
/// the output, waiting for the output when the feed is full, so the feed
/// never holds more than `lookahead` frames. The tracks already being heard
/// when the span starts are brought up from a run-in before any of this, as
/// [`render_range`](crate::render_range) describes, so the delay before the
/// output starts is the time that takes plus the time it takes to render one
/// lookahead, which is a small fraction of the lookahead itself wherever in
/// the mix the span begins.
///
/// `progress` is called after each block rendered, again while the feed
/// drains after the render has ended, and once more when the output has
/// pulled the last frame, so the last report's `written` is the span's end.
/// A report's `written` is the output frame just past the last one the
/// output has pulled so far, so it is the position a person is hearing
/// rather than the position being rendered, and the first report, made
/// before the output has started, says the span's start; `total` is the
/// mix's length. A span that is empty after clipping returns a report of
/// zero frames without starting the output. An output whose
/// [`start`](Output::start) fails ends the preview with
/// [`RenderError::Output`], and is not stopped, since it never started; an
/// output whose device stops taking frames after starting says so through
/// [`Feed::fail`], and the preview ends with the same error holding the
/// device's message; any other error is as for `render_range`. When the preview ends early
/// because of an error, the render marks the feed ended before anything
/// else, so an output that is waiting for frames stops waiting instead of
/// stalling. Whenever the output was started, it has been stopped by the
/// time this returns, on success and on error alike.
pub fn play(
    mix: &Mix,
    span: Range<Samples>,
    load: &mut Loader<'_>,
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
    output: &mut dyn Output,
    lookahead: Samples,
    progress: &mut dyn FnMut(Progress),
) -> Result<PlayReport, RenderError> {
    let total = mix_length(mix).0;
    let span = clip(&span, total);
    if span.is_empty() {
        return Ok(PlayReport {
            played: Samples::ZERO,
            underruns: 0,
        });
    }
    // The feed holds the lookahead the preview asked for, but never less than
    // one block: a feed with no room for the block the render has just made
    // would leave the render nowhere to put it, and a lookahead below nothing
    // is nonsense rather than a request for an enormous feed.
    let capacity = usize::try_from(lookahead.0.max(BLOCK_FRAMES as i64)).unwrap_or(usize::MAX);
    let length = span.end - span.start;
    let channel = Arc::new(Channel::new(capacity, length, span.start));
    let from = span.start;

    let mut running = false;
    // The output's own words for why it would not start. Failing the sink is
    // how the render is stopped, and a sink that fails reaches the caller as a
    // write failure, so the message is kept here and reported below as the
    // output failure it really is.
    let mut refusal: Option<String> = None;

    let rendered = render_range(
        mix,
        Samples(span.start)..Samples(span.end),
        load,
        stretchers,
        &mut |block| {
            let mut rest = block;
            while !rest.is_empty() {
                if running {
                    rest = &rest[channel.put(rest)?..];
                } else {
                    // Nothing takes frames out until the output is running, so
                    // the feed is filled to the lookahead first and the output
                    // is started the moment it is full.
                    rest = &rest[channel.fill(rest)..];
                    if channel.waiting() >= capacity {
                        output
                            .start(Feed::new(Arc::clone(&channel)))
                            .inspect_err(|message| refusal = Some(message.clone()))?;
                        running = true;
                    }
                }
            }
            progress(Progress {
                written: Samples(from + channel.pulled()),
                total: Samples(total),
            });
            Ok(())
        },
        &mut |_| {},
    );

    // The render has finished putting frames in, however it ended, so the feed
    // ends here: an output waiting for frames sees the end instead of waiting
    // for more, which is what lets stopping it below join its thread. A span
    // that ran all the way through was marked ended as its last frame went in.
    channel.end();

    if let Err(error) = rendered {
        // An output that started is stopped, which joins its thread now that
        // the feed has ended; one that refused to start has no thread and is
        // left alone.
        if running {
            output.stop();
        }
        return Err(match channel.failure().or(refusal) {
            Some(message) => RenderError::Output(message),
            None => error,
        });
    }

    if !running {
        // The span was shorter than the lookahead, so the feed never filled up
        // and the output has not been asked to start yet.
        if let Err(message) = output.start(Feed::new(Arc::clone(&channel))) {
            return Err(RenderError::Output(message));
        }
    }

    // What is left in the feed plays out at the device's pace, and progress
    // follows the device rather than the render, which has finished.
    let mut pulled = -1;
    loop {
        let (played, drained) = channel.pulled_since(pulled);
        pulled = played;
        progress(Progress {
            written: Samples(from + pulled),
            total: Samples(total),
        });
        if drained {
            break;
        }
    }
    output.stop();
    // A device that gave up while the last frames were playing out ends the
    // preview too, once it has been stopped like any other output.
    if let Some(message) = channel.failure() {
        return Err(RenderError::Output(message));
    }
    Ok(PlayReport {
        played: Samples(pulled),
        underruns: channel.underruns(),
    })
}

/// The number of channels a preview stream plays.
#[cfg(feature = "playback")]
const CHANNELS: cpal::ChannelCount = 2;

/// The rate a preview stream runs at, which is the engine's own rate.
#[cfg(feature = "playback")]
const STREAM_RATE: cpal::SampleRate = dermixen_core::SAMPLE_RATE;

/// How long the stream stays open after the last frame has been handed to the
/// device.
///
/// A device holds a buffer of frames it has been given but not yet played, so
/// closing the stream the instant the feed drains would cut the end of the
/// mix off. An empty feed hands the callback nothing and the callback writes
/// silence for the whole buffer, so waiting this long costs nothing but the
/// time.
#[cfg(feature = "playback")]
const TAIL: std::time::Duration = std::time::Duration::from_millis(200);

/// The default audio output device of the machine, opened through `cpal`.
///
/// The stream runs at the internal sample rate of 44.1 kHz in stereo; a
/// device that cannot take that format is refused with a message naming
/// the device, since the engine does not resample on the way out.
#[cfg(feature = "playback")]
pub struct CpalOutput {
    /// The device the stream is built on.
    device: cpal::Device,
    /// What the device is called, for the messages a person reads.
    name: String,
    /// The stream, once it is playing. Dropping it stops the device.
    stream: Option<cpal::Stream>,
    /// The feed's own end of the channel, kept while the stream plays so that
    /// stopping can tell whether the span was played all the way out.
    channel: Option<Arc<Channel>>,
    /// How many frames the device takes per pull, when the person set it.
    /// `None` leaves the size to the device.
    buffer_frames: Option<NonZeroU32>,
}

#[cfg(feature = "playback")]
impl CpalOutput {
    /// Opens the default output device. The error text names what went
    /// wrong in words a person at a terminal can act on: no device, or a
    /// device that cannot take stereo 44.1 kHz.
    ///
    /// `buffer_frames` is how many frames the device takes per pull, or
    /// `None` for the device's own size. The size is asked of the device
    /// when the stream starts, and a size the device refuses is reported by
    /// [`start`](Output::start) with the number of frames in the message.
    pub fn open(buffer_frames: Option<NonZeroU32>) -> Result<CpalOutput, String> {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "this machine has no audio output device".to_owned())?;
        let name = device.to_string();
        let configs = device
            .supported_output_configs()
            .map_err(|error| format!("{name} could not be asked what it plays: {error}"))?;
        let plays_our_format = configs.into_iter().any(|range| {
            range.channels() == CHANNELS
                && range.sample_format() == cpal::SampleFormat::F32
                && range.contains_rate(STREAM_RATE)
        });
        if !plays_our_format {
            return Err(format!(
                "{name} cannot play 44.1 kHz stereo audio, and dermixen does not convert \
                 the mix to another rate on its way to the device"
            ));
        }
        Ok(CpalOutput {
            device,
            name,
            stream: None,
            channel: None,
            buffer_frames,
        })
    }
}

#[cfg(feature = "playback")]
impl Output for CpalOutput {
    fn start(&mut self, mut feed: Feed) -> Result<(), String> {
        use cpal::traits::{DeviceTrait, StreamTrait};

        let config = cpal::StreamConfig {
            channels: CHANNELS,
            sample_rate: STREAM_RATE,
            buffer_size: match self.buffer_frames {
                Some(frames) => cpal::BufferSize::Fixed(frames.get()),
                None => cpal::BufferSize::Default,
            },
        };
        // The frames one call of the device's callback pulls. It is kept
        // across calls so that a callback grows it at most once, on the first
        // call, rather than allocating while the device is waiting for audio.
        let mut ready: Vec<Frame> = Vec::new();
        // Two more handles on the same channel the feed pulls from: one this
        // output keeps, so that stopping knows whether the span was played all
        // the way out, and one for the device's error path, which tells the
        // preview that this device is not going to play the rest.
        let channel = Arc::clone(&feed.channel);
        let failing = Arc::clone(&feed.channel);
        let reporting = self.name.clone();
        let stream = self
            .device
            .build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let channels = usize::from(CHANNELS);
                    let wanted = data.len() / channels;
                    if ready.len() < wanted {
                        ready.resize(wanted, [0.0, 0.0]);
                    }
                    // The pull never waits, so a render that has fallen behind
                    // leaves the rest of this buffer silent instead of holding
                    // the device up.
                    let got = feed.pull(&mut ready[..wanted]);
                    for (frame, out) in ready[..got].iter().zip(data.chunks_exact_mut(channels)) {
                        out[0] = frame[0];
                        out[1] = frame[1];
                    }
                    for sample in &mut data[got * channels..] {
                        *sample = 0.0;
                    }
                },
                move |error| {
                    eprintln!("warning: {reporting} reported an error: {error}");
                    // Three kinds say in their own documentation that the
                    // stream plays on: a route that moved, a real-time
                    // scheduling request that was turned down, and a glitch.
                    // Every other kind means this device is not going to play
                    // the rest of the mix, so the preview is told and ends
                    // rather than waiting for a device that has gone.
                    if !matches!(
                        error.kind(),
                        cpal::ErrorKind::DeviceChanged
                            | cpal::ErrorKind::RealtimeDenied
                            | cpal::ErrorKind::Xrun
                    ) {
                        failing.fail(&format!("{reporting}: {error}"));
                    }
                },
                None,
            )
            .map_err(|error| match self.buffer_frames {
                Some(frames) => format!(
                    "{} could not be opened with a buffer of {frames} frames: {error}",
                    self.name
                ),
                None => format!("{} could not be opened: {error}", self.name),
            })?;
        stream
            .play()
            .map_err(|error| format!("{} would not start playing: {error}", self.name))?;
        self.stream = Some(stream);
        self.channel = Some(channel);
        Ok(())
    }

    fn stop(&mut self) {
        // A tail is left in the device only when it pulled every frame the
        // render delivered and no more are coming: the whole of a fixed
        // span, all a render that stopped early managed to deliver, or a
        // transport whose render reached the end of the mix. A device that
        // stopped taking frames, and a preview stopped with frames still
        // waiting in its feed, are closed at once.
        let played_out = self
            .channel
            .take()
            .is_some_and(|channel| channel.played_out());
        if let Some(stream) = self.stream.take() {
            if played_out {
                std::thread::sleep(TAIL);
            }
            drop(stream);
        }
    }
}
