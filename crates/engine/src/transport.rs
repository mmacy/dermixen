//! Transport control: a preview that runs until it is told to stop, and that
//! can be paused, resumed, moved to another position, and given an edited
//! document while it runs.
//!
//! [`play`](crate::play) plays a fixed span of a fixed document and returns
//! when the span ends, which is what a command needs. A play button needs
//! more: the position a person is hearing, finely enough to draw a playhead
//! on every repaint of the window; pause and resume; a move to wherever the
//! person clicked; and a document that changes under the preview, because
//! the master BPM control writes nodes onto the curve during playback and
//! a person dragging an anchor expects to hear the result. A [`Transport`]
//! is that preview. It renders on a thread of its own and hands frames to an
//! [`Output`] through a [`Feed`], as `play` does, and the promise `play`
//! keeps holds here in a stronger form: every frame the output pulls is the
//! frame [`render_range`](crate::render_range) delivers for the document
//! the transport holds at that moment, at that output position. A pause, a
//! move, or a replacement that starts the render over never plays a frame
//! of the old position or the old document; what the device hears while the
//! render catches up is silence, and that silence is not counted as an
//! underrun. A replacement the render continues through plays the frames
//! it had already rendered, which is what makes an edit cost no silence,
//! and the transport hands a document over only when those frames are the
//! new document's own render as well.
//!
//! The acceptance tests in `tests/transport.rs` hold all of this by
//! recording every pull with its position and comparing each with the
//! render of the same document at the same position.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use dermixen_core::{
    Beats, Envelope, Mix, PlacedTrack, SAMPLE_RATE, Samples, Seconds, TempoCurve, Timeline, Track,
};

use crate::preview::{Channel, Feed, Output};
use crate::render::{
    BLOCK_FRAMES, Handover, RenderError, Source, mix_length, render_carrying, track_spans,
};
use crate::stretch::TimeStretcher;

/// What a transport is doing at a moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportState {
    /// The transport is about to play from its position but the render has
    /// not reached it yet: the tracks heard there are being decoded and
    /// brought up to that point, and the lookahead is being filled. The
    /// device plays silence meanwhile, and that silence is not an underrun.
    Buffering,
    /// The device is pulling the mix's frames. A device that stops pulling
    /// without reporting an error leaves the state here with the position
    /// standing still, and only [`stop`](Transport::stop) ends that.
    Playing,
    /// The position holds and the device plays silence until
    /// [`resume`](Transport::resume).
    Paused,
    /// There is nothing more to play, and the position is the mix's length:
    /// the device has pulled the last frame, or the transport was started,
    /// moved, or given a document at or past the length.
    Ended,
    /// The preview stopped on an error, with the text a person should read:
    /// what the loader said about a track it could not provide, or the
    /// message the output gave [`Feed::fail`](crate::Feed::fail) when its
    /// device stopped taking frames. The frames rendered ahead
    /// of the failure are thrown away, the device plays silence from the
    /// next pull on, the position holds, the other controls change nothing,
    /// and only [`stop`](Transport::stop) ends it.
    Failed(String),
}

/// Where a transport is and what it is doing, for drawing a playhead and
/// the transport buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportStatus {
    /// What the transport is doing.
    pub state: TransportState,
    /// The output frame the device will hear next: the frame just past the
    /// last one it pulled, on the clock `mix show` and the rendered file
    /// use. After a move it is the position moved to, whether or not the
    /// render has reached it yet. It never goes backwards except by a move,
    /// and it holds while paused.
    pub position: Samples,
    /// The length of the document being played, as
    /// [`mix_length`](crate::mix_length) gives it.
    pub length: Samples,
    /// How many pulls took some of the mix but fewer frames than they asked
    /// for while the state was [`Playing`](TransportState::Playing), which
    /// is a gap in the middle of what a person was hearing. The silence of
    /// buffering and of a pause is never counted, and neither is a pull that
    /// found nothing at all: an output that asks for whatever the feed
    /// happens to hold, rather than for a fixed buffer of its own, empties
    /// the feed on every turn of its loop and would otherwise report a gap
    /// nobody heard every time. A render that falls behind a device with a
    /// buffer of its own is therefore counted once as the feed runs dry
    /// rather than once for every buffer of silence that follows.
    pub underruns: u64,
    /// The output frame the render has rendered up to: every frame from the
    /// position to that frame is rendered ahead, and is heard as rendered
    /// unless the render starts over before the device reaches it. It
    /// is never more than one lookahead past the position, and it is the
    /// position itself after a move, after a replacement that started the
    /// render over, and before the render has delivered anything. Once the
    /// lookahead is full it stands still while the transport is paused,
    /// since a paused transport asks the render for nothing more.
    pub reached: Samples,
    /// How many times the render has started over at a position since the
    /// transport began, not counting the start the transport began with:
    /// once for every move, and once for every replacement that the check
    /// described on [`replace`](Transport::replace) turns down. A
    /// replacement that passes the check continues the render with the
    /// state it has, is not counted, and costs no buffering.
    pub restarts: u64,
    /// Why the render last started over on a replacement, in a sentence a
    /// person can read, naming the part of the check that turned the
    /// replacement down and the numbers behind it, as in `the mix tempo at
    /// 6:32.1 changed from 135.90 to 135.87`. A replacement that the render
    /// continues through clears it, so it stands only while the last
    /// replacement is the one that cost a person the buffering. It is `None`
    /// before any replacement has been made, and a move by
    /// [`seek`](Transport::seek) neither sets nor clears it, since a person
    /// who moves the playhead has asked for the buffering that follows.
    ///
    /// Tracks are named by their file name without the extension, and
    /// positions as minutes and seconds on the clock the rendered file and
    /// `mix show` use.
    pub last_restart: Option<String>,
}

/// Provides a track's decoded audio when the render first needs it, on the
/// transport's own thread. Within one run of the render it is called once
/// per track, as `render_range` calls its loader; it is called again for
/// every track heard at a position the transport moves to, and for every
/// track heard at the position when a replacement starts the render over,
/// so a loader that decodes on every call decodes the track again on every
/// move and every such replacement. A render that continues through a
/// replacement calls it for no track it is already playing. It does call it
/// for a track the new document has sounding at the handover frame that the
/// old one did not, which is how a track whose audio has begun but whose
/// volume holds silence comes to be loaded when an edit brings it into the
/// frames already rendered.
pub type SendLoader = Box<dyn FnMut(usize, &Track) -> Result<Box<dyn Source>, String> + Send>;

/// Makes the time-stretcher for a track, on the transport's own thread.
pub type SendStretchers = Box<dyn FnMut(&Track) -> Box<dyn TimeStretcher> + Send>;

/// A running preview that the caller controls.
///
/// The transport owns the output and a render thread. The output is started
/// on the thread that calls [`start`](Transport::start) and stopped on the
/// thread that calls [`stop`](Transport::stop), so an output whose device
/// handle must stay on one thread, as the `cpal` stream must, is at home
/// here; nothing else touches the output. The render thread fills the feed
/// from the position onward, at most `lookahead` frames ahead of the device,
/// exactly as [`play`](crate::play) does, and every control below takes
/// effect at the next pull: a frame the device has already pulled is heard,
/// and no frame rendered but not yet pulled survives a pause's resumption,
/// a move, or a replacement in a form the new position or document would
/// not have produced.
pub struct Transport {
    /// The output the transport plays through, started by
    /// [`start`](Transport::start) and stopped by [`stop`](Transport::stop)
    /// on whichever thread called them, and touched by nothing else.
    output: Box<dyn Output>,
    /// The frames in flight between the render thread and the output, which
    /// is also where the position, the underrun count, and what the device
    /// is doing at this moment are kept.
    channel: Arc<Channel>,
    /// What the render thread is waiting to be told, and the signal that
    /// wakes it to read it.
    orders: Arc<(Mutex<Orders>, Condvar)>,
    /// The thread the render runs on, until [`stop`](Transport::stop) or
    /// dropping the transport ends it.
    thread: Option<JoinHandle<()>>,
    /// The document being played. A run of the render reads the copy it was
    /// handed, either with the run itself or at the block boundary where the
    /// transport handed it over, so replacing the document here never changes
    /// what a run already under way is rendering at this moment.
    mix: Arc<Mix>,
    /// How many output frames the document being played has.
    length: i64,
    /// How many times the render has been started over at a position since
    /// the transport began, as [`TransportStatus::restarts`] reports it.
    restarts: AtomicU64,
    /// Why the last replacement started the render over, as
    /// [`TransportStatus::last_restart`] reports it.
    last_restart: Mutex<Option<String>>,
}

/// One run of the render: the document, the output frame to begin at, the
/// output frame to end at, and the number the feed knows the run by.
///
/// A transport counts one run for every position it sets the render going
/// at. The feed refuses the blocks of a run whose number is no longer the
/// current one, which is how a move, a replacement that starts the render
/// over, and a stop take effect without waiting for the render to reach the
/// end of the mix. A replacement the render continues through stays within
/// one run: the document and the frame the run ends at are handed to it
/// while it goes on rendering.
/// The frame at which a run of the render will next look for a document to
/// go on with, and the run that will look.
#[derive(Copy, Clone)]
struct Boundary {
    /// The number of the run that published this frame. The transport hands
    /// a document over only when that is still the run it is waiting on, so
    /// a frame left behind by a run that has stopped is never taken for a
    /// place to hand one over at.
    run: u64,
    /// The output frame the run will look at, which is where a document
    /// handed over now takes effect. That is why the transport compares the
    /// two documents up to this frame.
    at: i64,
}

struct Run {
    /// The document to render.
    mix: Arc<Mix>,
    /// The output frame the run begins at.
    from: i64,
    /// The output frame the run ends at, which is the document's length.
    until: i64,
    /// The number the feed knows this run by.
    run: u64,
}

/// What the render thread is waiting to be told, and what the run under way
/// is waiting to be handed.
struct Orders {
    /// The run to start next, if the transport has set one going since the
    /// thread last looked. A second order given before the thread looks
    /// replaces the first, whose run the feed has already abandoned.
    next: Option<Run>,
    /// Whether the transport has stopped, which ends the thread.
    stopped: bool,
    /// The number of the run the transport has set going most recently. A
    /// run whose number is no longer this one has been abandoned and is
    /// handed nothing.
    run: u64,
    /// Where the run under way will next look here for a document to go on
    /// with, or `None` when no run can be handed one: the thread is between
    /// runs, the run has not reached its first block, the block it is
    /// building is its last, or the run has stopped.
    boundary: Option<Boundary>,
    /// The document the run under way is to go on with when it reaches that
    /// frame. A second replacement before the run gets there replaces the
    /// document the first one left here, since the earlier replacement and
    /// the later one would take effect at the same frame.
    handover: Option<Handover>,
}

/// Renders run after run until the transport stops.
///
/// Each run renders from its position to the end of the document it is
/// working from and hands every block to the feed, which refuses the blocks
/// of a run the transport has abandoned; the thread then takes whatever run
/// the transport set going in the meantime. Between two blocks the run looks
/// for a document the transport has handed it and goes on with that one from
/// there. A replacement that reaches a run that way costs no silence. As a
/// run stops, however it stops, it looks once more and takes away the frame
/// it had published, so that the transport never hands a document to a run
/// that has ended. A run that reaches the end of its document leaves the
/// thread waiting for the next order, which is what a person moving the
/// playhead after the mix has ended gives it.
fn render_runs(
    channel: &Channel,
    orders: &(Mutex<Orders>, Condvar),
    mut load: SendLoader,
    mut stretchers: SendStretchers,
) {
    let (waiting, waking) = orders;
    loop {
        let run = {
            let mut orders = waiting.lock().unwrap();
            loop {
                if orders.stopped {
                    return;
                }
                if let Some(run) = orders.next.take() {
                    break run;
                }
                orders = waking.wait(orders).unwrap();
            }
        };
        let rendered = render_carrying(
            &run.mix,
            Samples(run.from)..Samples(run.until),
            &mut *load,
            &mut *stretchers,
            &mut |block| channel.deliver(block, run.run),
            &mut |_| {},
            &mut |asking_again| {
                let mut orders = waiting.lock().unwrap();
                // A run the transport has abandoned is about to end, so it
                // takes no document, and the frame it would publish belongs
                // to no run the transport is waiting on.
                if orders.run != run.run {
                    return None;
                }
                orders.boundary = asking_again.map(|at| Boundary { run: run.run, at });
                orders.handover.take()
            },
        );
        match rendered {
            Ok(_) => channel.delivered(run.run),
            Err(error) => {
                // A run the transport abandoned ends with an error from the
                // feed, and a device that stopped taking frames has already
                // said why in words of its own. Anything else is what a
                // person needs to read about why the preview stopped, and
                // recording it in the feed throws away the frames rendered
                // ahead of it as well.
                if channel.run() == run.run && channel.failure().is_none() {
                    channel.fail(&error.to_string());
                }
            }
        }
    }
}

impl Transport {
    /// Starts playing `mix` from output frame `from` through `output`.
    ///
    /// The output is started at once and hears silence until the render has
    /// brought the tracks heard at `from` up to that point and filled the
    /// lookahead, the same fill `play` makes before it starts its output;
    /// the status says [`Buffering`](TransportState::Buffering) meanwhile
    /// and [`Playing`](TransportState::Playing) from the first pull that
    /// gets frames. `lookahead` is how many frames the render keeps ahead of
    /// the device, and the feed never holds more. The feed handed to the
    /// output is not [`finished`](crate::Feed::finished) until
    /// [`stop`](Transport::stop), whatever the state, so an output that
    /// pulls until the feed finishes, as [`Output`] allows, keeps pulling
    /// through pauses, moves, and the end of the mix, and hears a later
    /// move. A `from` at or past the length of the mix, and a mix with no
    /// tracks, start [`Ended`](TransportState::Ended) at the mix's length
    /// without rendering anything; the output is started all the same. An
    /// output that cannot start ends the call with [`RenderError::Output`]
    /// holding its message, and no thread is left running.
    pub fn start(
        mix: Mix,
        from: Samples,
        load: SendLoader,
        stretchers: SendStretchers,
        mut output: Box<dyn Output>,
        lookahead: Samples,
    ) -> Result<Transport, RenderError> {
        let length = mix_length(&mix).0;
        let at = from.0.clamp(0, length);
        // The feed holds the lookahead the transport was asked for, but never
        // less than one block, for the reason `play` gives: a feed with no
        // room for the block the render has just made would leave the render
        // nowhere to put it.
        let capacity = usize::try_from(lookahead.0.max(BLOCK_FRAMES as i64)).unwrap_or(usize::MAX);
        let channel = Arc::new(Channel::for_transport(capacity, at));
        // The device is held off until the render has filled the lookahead,
        // so the output can be started before there is anything to hear.
        // Starting it here, whatever the state, is what lets a person move a
        // transport that began at the end of the mix and hear the move.
        output
            .start(Feed::new(Arc::clone(&channel)))
            .map_err(RenderError::Output)?;

        let orders = Arc::new((
            Mutex::new(Orders {
                next: None,
                stopped: false,
                run: 0,
                boundary: None,
                handover: None,
            }),
            Condvar::new(),
        ));
        let thread = std::thread::spawn({
            let channel = Arc::clone(&channel);
            let orders = Arc::clone(&orders);
            move || render_runs(&channel, &orders, load, stretchers)
        });

        let transport = Transport {
            output,
            channel,
            orders,
            thread: Some(thread),
            mix: Arc::new(mix),
            length,
            restarts: AtomicU64::new(0),
            last_restart: Mutex::new(None),
        };
        // The run the transport begins with is not a restart, so this is the
        // one place the render is set going without counting it.
        transport.set_going(|_| (at, false));
        Ok(transport)
    }

    /// Where the transport is and what it is doing, read from one look at
    /// the shared state so that the position and the state agree.
    pub fn status(&self) -> TransportStatus {
        let reading = self.channel.reading();
        let state = if let Some(message) = reading.failure {
            TransportState::Failed(message)
        } else if reading.at >= self.length {
            TransportState::Ended
        } else if reading.paused {
            TransportState::Paused
        } else if reading.playing {
            TransportState::Playing
        } else {
            TransportState::Buffering
        };
        TransportStatus {
            state,
            position: Samples(reading.at),
            length: Samples(self.length),
            underruns: reading.underruns,
            reached: Samples(reading.reached),
            restarts: self.restarts.load(Ordering::Relaxed),
            last_restart: self.last_restart.lock().unwrap().clone(),
        }
    }

    /// Starts the render over at another position, throwing away whatever the
    /// run before it had rendered ahead, and counts it in
    /// [`TransportStatus::restarts`].
    fn start_over(&self, where_to: impl FnOnce(i64) -> (i64, bool)) {
        self.restarts.fetch_add(1, Ordering::Relaxed);
        self.set_going(where_to);
    }

    /// Sets the render going, throwing away whatever the run before it had
    /// rendered ahead.
    ///
    /// `where_to` is given the output frame the device has reached and
    /// answers with the frame to begin at and whether a person is holding
    /// the transport there; the feed asks it while it holds its lock, so a
    /// pull that lands at that moment is counted rather than undone. A
    /// position at or past the end of the document sets nothing going, since
    /// there is nothing left to render, and the order any earlier run was
    /// waiting on is taken away, so no loader is called for a position
    /// nobody is at any more. The position moves either way, which is what
    /// ends the transport when it is at the end. Any document handed to the
    /// run before this one goes with it: the frames it was to be rendered
    /// from have been thrown away.
    fn set_going(&self, where_to: impl FnOnce(i64) -> (i64, bool)) {
        let (run, at) = self.channel.restart(where_to);
        let (waiting, waking) = &*self.orders;
        let mut orders = waiting.lock().unwrap();
        orders.run = run;
        orders.boundary = None;
        orders.handover = None;
        orders.next = if at >= self.length {
            None
        } else {
            Some(Run {
                mix: Arc::clone(&self.mix),
                from: at,
                until: self.length,
                run,
            })
        };
        drop(orders);
        waking.notify_all();
    }

    /// Offers `mix` to the run under way, to be rendered from the block
    /// boundary that run is waiting at.
    ///
    /// The check that decides it is the one [`replace`](Transport::replace)
    /// describes, made between the document being played and `mix` over the
    /// output frames from `position` to that boundary. `length` is how many
    /// output frames `mix` has. What comes back says which frame the run
    /// would have taken the document at and, when it would not take it, why
    /// the render has to start over instead.
    fn hand_over(&self, mix: &Arc<Mix>, length: i64, position: i64) -> Offer {
        let (waiting, _) = &*self.orders;
        let mut orders = waiting.lock().unwrap();
        // Only the run the transport is waiting on renders anything, so a
        // frame published by any other run is not a place to hand a document
        // over at.
        let Some(boundary) = orders
            .boundary
            .filter(|boundary| boundary.run == orders.run)
        else {
            return Offer {
                boundary: None,
                refused: Some("the render has nothing rendered ahead to go on from".to_owned()),
            };
        };
        let boundary = boundary.at;
        if let Err(reason) = renders_the_same_frames(&self.mix, mix, position, boundary) {
            return Offer {
                boundary: Some(boundary),
                refused: Some(reason),
            };
        }
        orders.handover = Some(Handover {
            mix: Arc::clone(mix),
            until: length,
        });
        Offer {
            boundary: Some(boundary),
            refused: None,
        }
    }

    /// Ends the render thread and closes the feed, which an output that
    /// pulls until the feed is finished sees. A second call does nothing.
    fn end_render(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.channel.close();
        let (waiting, waking) = &*self.orders;
        let mut orders = waiting.lock().unwrap();
        orders.stopped = true;
        orders.next = None;
        drop(orders);
        waking.notify_all();
        // A render thread that panicked has already reported itself, and
        // there is nothing more to do about it here.
        let _ = thread.join();
    }

    /// Holds the position. From the next pull on, the device gets no
    /// frames and plays silence, no underrun is counted, and the status
    /// says [`Paused`](TransportState::Paused). The frames rendered ahead
    /// are kept, so resuming costs no wait, and the render fills the
    /// lookahead if it has not already and then stands still until the
    /// transport resumes, so the whole status holds where it is. Pausing
    /// while [`Buffering`](TransportState::Buffering) pauses at the position
    /// being buffered; pausing when [`Ended`](TransportState::Ended) or
    /// [`Failed`](TransportState::Failed) changes nothing.
    pub fn pause(&mut self) {
        if matches!(
            self.status().state,
            TransportState::Ended | TransportState::Failed(_)
        ) {
            return;
        }
        self.channel.set_paused(true);
    }

    /// Continues from the position, with the frame that would have been
    /// pulled next had the transport not been paused: no frame is lost or
    /// repeated across a pause. Resuming when
    /// [`Ended`](TransportState::Ended), [`Failed`](TransportState::Failed),
    /// or already playing changes nothing.
    pub fn resume(&mut self) {
        if matches!(
            self.status().state,
            TransportState::Ended | TransportState::Failed(_)
        ) {
            return;
        }
        self.channel.set_paused(false);
    }

    /// Moves to output frame `to`, clipped to the mix's length.
    ///
    /// The frames rendered ahead of the old position are thrown away and
    /// the render starts again at `to`, so from the next pull on the device
    /// gets the frames of the new position and nothing of the old one; the
    /// status says `to` at once and
    /// [`Buffering`](TransportState::Buffering) until the render has caught
    /// up. A move while paused stays [`Paused`](TransportState::Paused) at
    /// `to`, and the render starts at `to` at once so that resumption plays
    /// without a wait. A move to or past the length ends the transport. A
    /// move from [`Ended`](TransportState::Ended) to a position inside the
    /// mix leaves the transport [`Paused`](TransportState::Paused) there, so
    /// that a click on the timeline after the mix has ended moves the
    /// playhead without starting playback. A move from
    /// [`Failed`](TransportState::Failed) changes nothing.
    pub fn seek(&mut self, to: Samples) {
        let status = self.status();
        if matches!(status.state, TransportState::Failed(_)) {
            return;
        }
        // A move while paused stays paused, and a move after the mix has
        // ended puts the playhead where a person clicked without starting
        // playback there.
        let paused = matches!(status.state, TransportState::Paused | TransportState::Ended);
        let to = to.0.clamp(0, self.length);
        self.start_over(|_| (to, paused));
    }

    /// Replaces the document, keeping the position.
    ///
    /// Every frame the device gets from [`reached`](TransportStatus::reached)
    /// on is the render of `mix` at its output position. When the render
    /// has to start over, which the transport decides by the check described
    /// below, the frames rendered ahead from the old document are thrown
    /// away, so that holds from the next pull, and the status says
    /// [`Buffering`](TransportState::Buffering) while the render starts
    /// again at the position, or stays [`Paused`](TransportState::Paused) if
    /// it was, with the render starting again all the same. The position is
    /// read as the frames are thrown away, so a frame the device pulls while
    /// the document is being handed over is heard and counted rather than
    /// played a second time. The length in the status is the new document's,
    /// and a position at or past it ends the transport. A replacement when
    /// the transport has [`Ended`](TransportState::Ended) leaves it
    /// [`Paused`](TransportState::Paused) at the position when the new
    /// document reaches past it, and ended when it does not, so that editing
    /// a mix that has played out never starts it playing again. The loader
    /// is called again for the tracks heard at the position when the render
    /// starts over. A replacement when [`Failed`](TransportState::Failed)
    /// changes nothing.
    ///
    /// The render starts over at the position only when it has to. When
    /// the new document renders the same frames as the old one up to the
    /// handover frame described below, which is
    /// [`reached`](TransportStatus::reached) or a little past it, the render
    /// continues with the state it has: the frames already rendered ahead,
    /// from the position up to the handover frame, are heard as rendered;
    /// the frames from there on are the new document's render, because the
    /// state the render holds is the state that render would hold there; the
    /// status does not fall back to
    /// [`Buffering`](TransportState::Buffering); the loader is called only
    /// for a track the new document has sounding there that the old one did
    /// not, which the render brings up from a run-in of that track's own
    /// audio. [`TransportStatus::restarts`] does not move. Otherwise the
    /// frames rendered ahead are thrown away, the render starts over at the
    /// position, and `restarts` counts it.
    ///
    /// Whichever way it goes,
    /// [`TransportStatus::last_restart`] is the word on it afterwards: the
    /// reason the check turned this replacement down, or `None` when the
    /// render went on through it. Setting the environment variable
    /// `DERMIXEN_TRACE` to anything also prints one line to standard error
    /// on every replacement, holding the position, the frame the render had
    /// reached, the frame the document was offered at, whether the render
    /// went on or started over, and the reason when it started over, which
    /// is how a person finds out why an edit made the window buffer.
    ///
    /// The handover frame is where the run under way will next look for a
    /// document, which is the end of the block of the mix it is building.
    /// The render works a block at a time and each block is one document's
    /// throughout, so the handover frame is `reached` when the render is
    /// between two blocks and up to one block of the mix (about
    /// twenty-three milliseconds) past it when the render is in the middle
    /// of one. The transport makes the check below at that frame rather than
    /// at `reached`, so the frames rendered from the old document past
    /// `reached` are covered by it as well.
    ///
    /// The transport decides whether to start over from the two documents
    /// alone, with a check that is finite. The check passes when all of
    /// these hold. The set of tracks heard at any frame from the position to
    /// the handover frame is the same in both documents, where a track
    /// counts as heard at a frame only when its volume envelope is above
    /// silence there: a track whose audio has begun but whose volume holds
    /// silence, as every track does before its fade in, contributes nothing
    /// to the frames, so it may enter or leave the rendered stretch without
    /// the frames changing, and the render brings such a track up from a
    /// run-in to the handover frame before it continues. For the mix tempo
    /// curve, the nodes that lie before the mix beat at that frame, and
    /// every node at that beat, are the same in both documents, and so is
    /// the tempo at that beat. For every track in that set, the track's
    /// path, hash, length, grid, keylock, placement on the timeline, and gain
    /// are the same, and for each of its four curves the nodes before the
    /// track beat at that frame, and any node at it, are the same, and so is
    /// the level at that beat. A new document that ends before the handover
    /// frame, or that has no tracks at all, is turned down as well, since
    /// the frames rendered ahead would run past its end. Two tempos or two
    /// levels count as the same when they differ by less than one part in a
    /// million, since the pin the master BPM control writes reproduces the
    /// curve's tempo only to rounding; when the documents agree bit for bit
    /// the frames from the handover frame on are the new document's render
    /// exactly, and when they agree only within that tolerance the frames
    /// differ from it by no more than that rounding. The check is what makes
    /// the frames the same: between two nodes a curve is a straight line, so
    /// the last node before a beat and the value at the beat fix everything
    /// before it. A tempo node added after the handover frame without a pin
    /// before it bends the ramp that arrives there, so the transport turns
    /// such a replacement down and starts the render over, which is right,
    /// since the frames before it would have differed.
    pub fn replace(&mut self, mix: Mix) {
        let status = self.status();
        if matches!(status.state, TransportState::Failed(_)) {
            return;
        }
        let held = status.state == TransportState::Paused;
        // The document being replaced ends here, so a position that has
        // reached it is a transport that has ended, whatever the new
        // document's length turns out to be.
        let ended_at = self.length;
        let mix = Arc::new(mix);
        let length = mix_length(&mix).0;
        // The position is read before the run is asked for its boundary, so
        // it may already have moved on; a position from a moment ago only
        // widens the stretch of mix the check covers.
        let offer = self.hand_over(&mix, length, status.position.0);
        trace_replacement(&status, &offer);
        self.mix = mix;
        self.length = length;
        // The reason stands only while it is the last word on a replacement,
        // so one the render goes on through takes it away again.
        *self.last_restart.lock().unwrap() = offer.refused.clone();
        if offer.refused.is_none() {
            return;
        }
        self.start_over(|at| (at.min(length), held || at >= ended_at));
    }

    /// Stops the output, then the render thread, and returns the status as
    /// it stood when the output stopped, so its position is the frame just
    /// past the last one the output pulled.
    ///
    /// This returns within two seconds whatever the device is doing,
    /// including a device that has stopped pulling without reporting an
    /// error, and however far the render has got, including a render waiting
    /// for the device to make room, for as long as the loader answers
    /// promptly: a render that is inside a call to the loader is joined only
    /// when that call returns, so a loader that blocks on a slow disk holds
    /// this up for as long as it blocks. When it returns, the output has
    /// been stopped and no thread of the transport is running.
    pub fn stop(mut self) -> TransportStatus {
        // The output is stopped first, and on this thread, so that no pull
        // can happen after it and the status read below is the last word on
        // where the device got to.
        self.output.stop();
        let status = self.status();
        self.end_render();
        status
    }
}

impl Drop for Transport {
    /// Stops the output and ends the render thread of a transport dropped
    /// without being stopped, so that no thread of it and no device it was
    /// playing through is left running.
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.output.stop();
            self.end_render();
        }
    }
}

/// How far apart two tempos, or two levels, may be and still count as the
/// same: one part in a million of the larger of the two, or of one beat per
/// minute or one decibel when both of them are smaller than that, so that a
/// level at or near nothing does not have to agree to the last bit.
const TOLERANCE: f64 = 1e-6;

/// What came of offering a document to the run of the render under way.
struct Offer {
    /// The output frame that run would take the document at, when there is a
    /// run waiting to be handed one at all.
    boundary: Option<i64>,
    /// Why the render has to start over instead, or `None` when the run took
    /// the document.
    refused: Option<String>,
}

/// The name of the environment variable that turns the line below on.
const TRACE: &str = "DERMIXEN_TRACE";

/// Prints what a replacement did, when [`TRACE`] is set in the environment.
///
/// One line per replacement, on standard error: where the device is, how far
/// the render had got, which frame the document was offered at, whether the
/// render went on or started over, and the reason when it started over.
fn trace_replacement(status: &TransportStatus, offer: &Offer) {
    if std::env::var_os(TRACE).is_none() {
        return;
    }
    let handover = match offer.boundary {
        Some(frame) => format!("{} ({frame})", moment_text(frame)),
        None => "none".to_owned(),
    };
    let outcome = match &offer.refused {
        Some(reason) => format!("started over, because {reason}"),
        None => "continued".to_owned(),
    };
    eprintln!(
        "dermixen: replacement at {} ({}), reached {} ({}), handover {handover}: {outcome}",
        moment_text(status.position.0),
        status.position.0,
        moment_text(status.reached.0),
        status.reached.0,
    );
}

/// How a curve of one document differs from the same curve of another, where
/// the replacement check looks at them.
///
/// A curve is fixed everywhere before a beat by the nodes behind that beat
/// and the value at it, so those are the two ways it can differ, and each
/// wants a different sentence from the check.
#[derive(Debug, PartialEq)]
enum Difference {
    /// The two hold the same nodes and reach the same value.
    None,
    /// A node before the beat, or at it, is not in both curves, or holds
    /// another position or another value.
    Nodes,
    /// The nodes agree, and the value the curves reach at the beat does not.
    Value {
        /// What the document being played reaches there.
        was: f64,
        /// What the document replacing it reaches there.
        is: f64,
    },
}

/// An output frame as minutes and seconds, on the clock the rendered file and
/// `mix show` use, which is how every position in a reason a person reads is
/// written.
fn moment_text(frame: i64) -> String {
    let total = (frame as f64 / f64::from(SAMPLE_RATE)).max(0.0);
    let minutes = (total / 60.0).floor();
    let seconds = total - minutes * 60.0;
    format!("{minutes}:{seconds:04.1}")
}

/// A track named for a person: its place in the playlist and its file name
/// without the extension, as in `track 2 (Space Tribe - The Great Spirit)`.
///
/// A path with no file name at all, which a document built in memory may
/// hold, gives the place in the playlist alone.
fn track_text(index: usize, track: &Track) -> String {
    match track.path.file_stem().and_then(|name| name.to_str()) {
        Some(name) => format!("track {index} ({name})"),
        None => format!("track {index}"),
    }
}

/// Whether two tempos, or two levels, are the same within [`TOLERANCE`].
///
/// Two values that are equal are the same however extreme they are, and that
/// is answered first, because subtracting one infinity from another gives
/// something that is not a number and no comparison would call that small.
/// How far apart the two are has to be a real distance for the tolerance to
/// mean anything, so values that are infinitely far apart, and values that
/// are not numbers at all, are not the same. A document holds neither: the
/// envelope and the tempo curve both refuse a level or a tempo that is not a
/// finite number. This says what it means for them all the same, since the
/// answer decides whether a person hears an edit or hears the mix start
/// again.
fn nearly(one: f64, other: f64) -> bool {
    if one == other {
        return true;
    }
    let apart = (one - other).abs();
    apart.is_finite() && apart <= TOLERANCE * one.abs().max(other.abs()).max(1.0)
}

/// A mix laid out on the timeline, together with the mix time of output frame
/// zero and how many output frames the mix has. A mix with no tracks has no
/// timeline and gives `None`.
fn laid_out(mix: &Mix) -> Option<(Timeline, Seconds, i64)> {
    let timeline = mix.timeline()?;
    let opening = timeline.start();
    let length = (timeline.end() - opening).to_samples().0.max(0);
    Some((timeline, opening, length))
}

/// Whether two tempo curves hold the same nodes up to and including `beat`
/// and reach the same tempo there.
///
/// That is what makes them one curve everywhere before that beat: a curve is
/// a straight line in time between two nodes, so the nodes behind a beat and
/// the tempo at it fix the whole of the curve up to it, whatever either curve
/// does after it.
fn same_tempo_curve(before: &TempoCurve, after: &TempoCurve, beat: Beats) -> Difference {
    let upto = |curve: &TempoCurve| {
        curve
            .nodes()
            .iter()
            .take_while(|node| node.at.0 <= beat.0)
            .count()
    };
    let (one, other) = (
        &before.nodes()[..upto(before)],
        &after.nodes()[..upto(after)],
    );
    let same_nodes = one.len() == other.len()
        && one
            .iter()
            .zip(other)
            .all(|(node, same)| node.at.0 == same.at.0 && nearly(node.bpm.0, same.bpm.0));
    if !same_nodes {
        return Difference::Nodes;
    }
    let (was, is) = (before.bpm_at_beat(beat).0, after.bpm_at_beat(beat).0);
    if nearly(was, is) {
        Difference::None
    } else {
        Difference::Value { was, is }
    }
}

/// Whether two envelopes hold the same nodes up to and including `beat` and
/// reach the same level there, which fixes both of them everywhere before
/// that beat for the reason [`same_tempo_curve`] gives.
fn same_envelope(before: &Envelope, after: &Envelope, beat: Beats) -> Difference {
    let upto = |envelope: &Envelope| {
        envelope
            .nodes()
            .iter()
            .take_while(|node| node.at.0 <= beat.0)
            .count()
    };
    let (one, other) = (
        &before.nodes()[..upto(before)],
        &after.nodes()[..upto(after)],
    );
    let same_nodes = one.len() == other.len()
        && one
            .iter()
            .zip(other)
            .all(|(node, same)| node.at.0 == same.at.0 && nearly(node.value.0, same.value.0));
    if !same_nodes {
        return Difference::Nodes;
    }
    let (was, is) = (before.value_at(beat).0, after.value_at(beat).0);
    if nearly(was, is) {
        Difference::None
    } else {
        Difference::Value { was, is }
    }
}

/// Whether a track's volume rises above silence anywhere in the output frames
/// from `from` to `until`, which is what makes it heard there.
///
/// At or below the silence floor the render multiplies the track by exactly
/// nothing, so a track that holds silence across every one of those frames
/// adds nothing to any of them. Between two nodes a volume envelope is a
/// straight line in decibels, so the loudest level it reaches over a stretch
/// of beats is at one end of the stretch or at a node within it, and this
/// function reads the envelope at exactly those places. The stretch is
/// widened by one block at each end because the render reads one level for a
/// whole block of a track, at the middle of that block: a block that lays any
/// frame inside the stretch has its middle inside the widened stretch, so no
/// level the render will actually use is missed.
///
/// The envelope alone decides whether a track is heard, and the track's gain
/// has no part in that decision. A gain far enough below zero puts the
/// envelope's value plus the gain under the floor, so the render writes exact
/// silence for a track this function counts as heard. That is the safe
/// direction. A track counted as heard while it renders silent costs at most
/// a restart the frames did not need, and a track missed while it does sound
/// would let the render go on through a change to it, leaving frames already
/// rendered that differ from the ones the new document makes.
fn above_silence(
    track: &Track,
    placed: &PlacedTrack,
    curve: &TempoCurve,
    opening: Seconds,
    from: i64,
    until: i64,
) -> bool {
    let beat_at = |frame: i64| {
        curve.beat_at(Seconds(opening.0 + frame as f64 / f64::from(SAMPLE_RATE))) - placed.origin
    };
    let block = BLOCK_FRAMES as i64;
    let (low, high) = (beat_at(from - block), beat_at(until + block));
    let heard = |beat: Beats| track.volume.value_at(beat).to_linear() > 0.0;
    heard(low)
        || heard(high)
        || track
            .volume
            .nodes()
            .iter()
            .any(|node| node.at.0 > low.0 && node.at.0 < high.0 && node.value.to_linear() > 0.0)
}

/// Whether a render of `current` that has reached output frame `until` may go
/// on with `next` from that frame, because the two documents render the same
/// frames from `position` to it.
///
/// [`Transport::replace`] states the check in full, and why each part of it is
/// what makes the frames the same.
fn renders_the_same_frames(
    current: &Mix,
    next: &Mix,
    position: i64,
    until: i64,
) -> Result<(), String> {
    let (Some((before, opening_before, length_before)), Some((after, opening_after, length_after))) =
        (laid_out(current), laid_out(next))
    else {
        return Err("the mix has no tracks left to play".to_owned());
    };
    // Output frame zero is the moment the earliest track begins, so a
    // document whose earliest track begins at another moment of the mix is
    // heard differently at every frame; and the frames rendered ahead run
    // past the end of a document that is shorter than they are.
    if opening_before != opening_after {
        return Err("the mix begins at a different moment".to_owned());
    }
    if length_after < until {
        return Err(format!(
            "the mix now ends at {}, inside the frames already rendered",
            moment_text(length_after)
        ));
    }

    // Which tracks are heard at any frame between the position and the frame
    // the render has reached, asked of each document with the same arithmetic
    // the render itself uses. A track whose audio runs through those frames
    // but whose volume holds silence throughout them is not among them: the
    // render multiplies it by nothing, so it stands in the mix for silence
    // and may enter or leave without a frame changing.
    let spans_before = track_spans(&before, opening_before, length_before);
    let spans_after = track_spans(&after, opening_after, length_after);
    let sounding = |mix: &Mix, timeline: &Timeline, opening: Seconds, spans: &[(i64, i64)]| {
        spans
            .iter()
            .enumerate()
            .filter(|(index, (entry, exit))| {
                *entry < until
                    && *exit > position
                    && above_silence(
                        &mix.tracks[*index],
                        &timeline.tracks[*index],
                        &timeline.curve,
                        opening,
                        (*entry).max(position),
                        (*exit).min(until),
                    )
            })
            .map(|(index, _)| index)
            .collect::<Vec<usize>>()
    };
    let heard = sounding(current, &before, opening_before, &spans_before);
    let heard_after = sounding(next, &after, opening_after, &spans_after);
    if heard != heard_after {
        // The first track the two documents disagree about is the one to
        // name, and the side that holds it says whether it begins or stops.
        let widest = current.tracks.len().max(next.tracks.len());
        for index in 0..widest {
            let (was, is) = (heard.contains(&index), heard_after.contains(&index));
            if was == is {
                continue;
            }
            return Err(if is {
                format!(
                    "{} begins to sound inside the frames already rendered",
                    track_text(index, &next.tracks[index])
                )
            } else {
                format!(
                    "{} stops sounding inside the frames already rendered",
                    track_text(index, &current.tracks[index])
                )
            });
        }
        return Err("another set of tracks is heard in the frames already rendered".to_owned());
    }

    // The mix beat the frames rendered ahead reach, which is where the curves
    // of both documents are compared.
    let moment = Seconds(opening_before.0 + until as f64 / f64::from(SAMPLE_RATE));
    let beat = before.curve.beat_at(moment);
    match same_tempo_curve(&before.curve, &after.curve, beat) {
        Difference::None => {}
        Difference::Nodes => {
            return Err(format!(
                "the mix tempo curve changed before {}",
                moment_text(until)
            ));
        }
        Difference::Value { was, is } => {
            return Err(format!(
                "the mix tempo at {} changed from {was:.2} to {is:.2}",
                moment_text(until)
            ));
        }
    }

    for index in heard {
        let (one, other) = (&current.tracks[index], &next.tracks[index]);
        let name = track_text(index, other);
        if one.path != other.path || one.hash != other.hash {
            return Err(format!("{name} is a different file"));
        }
        if one.length != other.length {
            return Err(format!("{name} has a different length"));
        }
        if one.grid != other.grid {
            return Err(format!("{name} has a different beat grid"));
        }
        if one.keylock != other.keylock {
            let now = if other.keylock { "on" } else { "off" };
            return Err(format!("{name} has keylock switched {now}"));
        }
        if before.tracks[index] != after.tracks[index] {
            return Err(format!("{name} is placed at a different point"));
        }
        // The gain is one level over the whole track rather than a curve
        // through it, so a track that sounds anywhere in the rendered frames
        // sounds at a different level in all of them once the gain changes,
        // and the reason names no moment.
        if !nearly(one.gain.0, other.gain.0) {
            return Err(format!(
                "{name}'s gain changed from {:.1} to {:.1} decibels",
                one.gain.0, other.gain.0
            ));
        }
        // Envelope nodes sit at beats of the track, so the mix beat is moved
        // back by the beat at which the track was placed.
        let track_beat = beat - before.tracks[index].origin;
        for (curve, was, is) in [
            ("volume", &one.volume, &other.volume),
            ("low EQ", &one.eq.low, &other.eq.low),
            ("mid EQ", &one.eq.mid, &other.eq.mid),
            ("high EQ", &one.eq.high, &other.eq.high),
        ] {
            match same_envelope(was, is, track_beat) {
                Difference::None => {}
                Difference::Nodes => {
                    return Err(format!(
                        "{name}'s {curve} curve changed before {}",
                        moment_text(until)
                    ));
                }
                Difference::Value { was, is } => {
                    return Err(format!(
                        "{name}'s {curve} at {} changed from {was:.1} to {is:.1} decibels",
                        moment_text(until)
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use dermixen_core::{
        Anchor, Anchors, BeatGrid, Bpm, ContentHash, Decibels, Edit, EnvelopeNode, EqEnvelopes,
        apply_edit, beatmix,
    };

    /// The output frame the device is at, and the frame the render has
    /// reached, in the mix below.
    ///
    /// The numbers are the ones a session in the window produces: the
    /// playhead at six minutes ten, and two seconds of mix rendered ahead of
    /// it, which is the lookahead the window runs with.
    const POSITION: i64 = 370 * 44_100;
    const REACH: i64 = POSITION + 2 * 44_100;

    /// An eight-minute track, which is what a real one is.
    fn track(name: &str, bpm: f64, intro: f64, outro: f64) -> Track {
        Track {
            path: PathBuf::from(format!("/audio/goa/{name}.mp3")),
            hash: ContentHash([0; 32]),
            length: Samples(8 * 60 * 44_100),
            grid: BeatGrid {
                first_beat: Samples::ZERO,
                bpm: Bpm(bpm),
            },
            anchors: Anchors {
                intro: Beats(intro),
                outro: Beats(outro),
            },
            keylock: true,
            gain: dermixen_core::Decibels::UNITY,
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        }
    }

    /// Two eight-minute tracks joined by a four-bar crossfade at the outgoing
    /// track's beat 880, which at 130 beats per minute is six minutes and
    /// forty-six seconds into the mix.
    ///
    /// The incoming track's intro anchor is its own beat 128, which is
    /// fifty-five seconds into its audio, so that audio begins at mix beat
    /// 752, five minutes and forty-seven seconds in, and its fade in holds
    /// silence over the whole of that opening. That is the mix the tests
    /// below drag an anchor in.
    fn two_tracks() -> Mix {
        let mut outgoing = track("Etnica - Trip Tonight", 130.0, 0.0, 880.0);
        let mut incoming = track("Space Tribe - The Great Spirit", 140.0, 128.0, 1000.0);
        beatmix(&mut outgoing, &mut incoming, 4);
        Mix {
            tracks: vec![outgoing, incoming],
        }
    }

    /// Why the check turned a replacement down, or a panic naming what it let
    /// through instead.
    fn refused(current: &Mix, next: &Mix) -> String {
        match renders_the_same_frames(current, next, POSITION, REACH) {
            Err(reason) => reason,
            Ok(()) => panic!("the check let this replacement through"),
        }
    }

    /// The document with the outgoing track's outro anchor moved by `beats`,
    /// which is the drag that moves the incoming track along the timeline.
    fn dragged(mix: &Mix, beats: f64) -> Mix {
        let mut next = mix.clone();
        let to = mix.tracks[0].anchors.outro + Beats(beats);
        apply_edit(
            &mut next,
            &Edit::MoveAnchor {
                track: 0,
                anchor: Anchor::Outro,
                to,
            },
        )
        .unwrap();
        next
    }

    #[test]
    fn a_position_is_written_as_minutes_and_seconds() {
        assert_eq!(moment_text(0), "0:00.0");
        assert_eq!(moment_text(44_100 * 5), "0:05.0");
        assert_eq!(moment_text(44_100 * 392 + 4_410), "6:32.1");
        assert_eq!(moment_text(-1), "0:00.0");
    }

    #[test]
    fn a_track_is_named_by_its_file_name_without_the_extension() {
        let mix = two_tracks();
        assert_eq!(
            track_text(1, &mix.tracks[1]),
            "track 1 (Space Tribe - The Great Spirit)"
        );
        let mut bare = mix.tracks[0].clone();
        bare.path = PathBuf::new();
        assert_eq!(track_text(0, &bare), "track 0");
    }

    #[test]
    fn a_level_that_changed_is_named_with_the_track_the_moment_and_both_levels() {
        let current = two_tracks();
        let mut next = current.clone();
        // The node at the outgoing track's outro anchor lies past the frames
        // already rendered, so no node before them moves; what changes is the
        // level the curve reaches at the frame the render got to.
        next.tracks[0]
            .volume
            .insert(EnvelopeNode {
                at: Beats(880.0),
                value: Decibels(-3.0),
            })
            .unwrap();
        assert_eq!(
            refused(&current, &next),
            "track 0 (Etnica - Trip Tonight)'s volume at 6:12.0 changed from 0.0 to -3.0 decibels"
        );
    }

    #[test]
    fn a_tempo_that_changed_is_named_with_the_moment_and_both_tempos() {
        let current = two_tracks();
        let mut next = current.clone();
        // The outgoing track's own tempo node sits at its outro anchor, past
        // the frames already rendered; moving its tempo bends the ramp that
        // arrives at them.
        next.tracks[0].tempo[0].bpm = Bpm(132.0);
        assert_eq!(
            refused(&current, &next),
            "the mix tempo at 6:12.0 changed from 130.00 to 131.83"
        );
    }

    #[test]
    fn a_track_that_becomes_audible_inside_the_rendered_frames_is_named() {
        let current = two_tracks();
        // A drag of a bar, with the incoming track's silence taken away, so
        // that it is heard in the rendered frames rather than standing in
        // them for silence.
        let mut next = dragged(&current, -4.0);
        next.tracks[1]
            .volume
            .insert(EnvelopeNode {
                at: Beats(-1000.0),
                value: Decibels::UNITY,
            })
            .unwrap();
        assert_eq!(
            refused(&current, &next),
            "track 1 (Space Tribe - The Great Spirit) begins to sound inside the frames \
             already rendered"
        );
    }

    #[test]
    fn dragging_the_outro_anchor_through_a_silent_opening_gives_no_reason() {
        // The playhead at six minutes ten, inside the incoming track's silent
        // opening, and the outro anchor dragged a bar either way. Each drag
        // slides that silent opening through the frames already rendered
        // without changing one of them, so the render goes on through each of
        // them, bringing the incoming track up from a run-in.
        let current = two_tracks();
        for beats in [-4.0, 4.0, -16.0, 16.0] {
            assert_eq!(
                renders_the_same_frames(&current, &dragged(&current, beats), POSITION, REACH),
                Ok(()),
                "dragging the outro anchor by {beats} beats"
            );
        }
    }
}
