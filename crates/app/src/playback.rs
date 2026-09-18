//! The play, pause, and stop state around the transport: what the window's
//! transport buttons, the space bar, and a click on the ruler do, as plain
//! state that says what the transport is to be told.
//!
//! The window owns the transport and the audio device; this state owns the
//! playhead, the start point, and what the person believes the transport is
//! doing. The start point is what makes stopping and playing again play the
//! same stretch again, as a studio tool is expected to: playing records it,
//! a click on the ruler sets it, stopping returns the playhead to it, and
//! playing after the mix has played out goes back to it.

use dermixen_core::Samples;
use dermixen_engine::TransportState as EngineState;
use dermixen_engine::TransportStatus;

/// What the person is being shown the transport doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaybackState {
    /// No transport is running.
    Stopped,
    /// The transport is playing, or is about to once it has buffered.
    Playing,
    /// The transport holds its position.
    Paused,
    /// The transport has played the mix out.
    Ended,
    /// The transport stopped on the message, which the person is shown.
    Failed(String),
}

/// What the window is to tell the transport, in the order given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportOrder {
    /// Start a transport at the frame.
    Start {
        /// The output frame to start at.
        at: Samples,
    },
    /// Resume the transport where it is.
    Resume,
    /// Hold the transport where it is.
    Pause,
    /// Move the transport to the frame.
    Seek {
        /// The output frame to move to.
        at: Samples,
    },
    /// Stop the transport and give the device back.
    Stop,
}

/// The frame playing starts from: the beginning when `playhead` is at or
/// past the mix's `length`, since there is nothing left to hear from
/// `playhead` on, and `playhead` itself otherwise.
fn play_from(playhead: Samples, length: Samples) -> Samples {
    if playhead >= length {
        Samples::ZERO
    } else {
        playhead
    }
}

/// The frame stopping returns to: `start_point`, or `length` when an edit
/// has left the mix shorter than the start point.
fn stop_to(start_point: Samples, length: Samples) -> Samples {
    Samples(start_point.0.min(length.0))
}

/// The playhead, the start point, and what the transport is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct Playback {
    /// The output frame the person is hearing, or the frame playing would
    /// start from.
    playhead: Samples,
    /// The frame the last playback started from, which stopping comes back
    /// to.
    start_point: Samples,
    /// What the person is being shown the transport doing.
    state: PlaybackState,
}

impl Playback {
    /// Stopped, with the playhead and the start point at `playhead`.
    pub fn new(playhead: Samples) -> Playback {
        Playback {
            playhead,
            start_point: playhead,
            state: PlaybackState::Stopped,
        }
    }

    /// What the transport is doing.
    pub fn state(&self) -> PlaybackState {
        self.state.clone()
    }

    /// The output frame the person is hearing, or the frame playing would
    /// start from.
    pub fn playhead(&self) -> Samples {
        self.playhead
    }

    /// The frame the last playback started from, which stopping comes back
    /// to.
    pub fn start_point(&self) -> Samples {
        self.start_point
    }

    /// Plays, given the mix's length: starts a transport at the playhead
    /// when none is running, recording the playhead as the start point,
    /// or at the first frame when the playhead is at or past the length;
    /// resumes a paused transport, leaving the start point alone; moves a
    /// transport that has played the mix out back to the start point and
    /// resumes it, or to the first frame when the mix no longer reaches the
    /// start point, recording where it went as the start point; stops a
    /// failed transport, which returns the playhead to the start point as
    /// [`stop`](Playback::stop) does, and starts a fresh one there as a
    /// stopped one is started; and does nothing to a transport that is
    /// playing.
    pub fn play(&mut self, length: Samples) -> Vec<TransportOrder> {
        match &self.state {
            PlaybackState::Stopped => {
                let from = play_from(self.playhead, length);
                self.playhead = from;
                self.start_point = from;
                self.state = PlaybackState::Playing;
                vec![TransportOrder::Start { at: from }]
            }
            PlaybackState::Paused => {
                self.state = PlaybackState::Playing;
                vec![TransportOrder::Resume]
            }
            PlaybackState::Ended => {
                let to = play_from(self.start_point, length);
                self.start_point = to;
                self.playhead = to;
                self.state = PlaybackState::Playing;
                vec![TransportOrder::Seek { at: to }, TransportOrder::Resume]
            }
            PlaybackState::Failed(_) => {
                let stopped_at = stop_to(self.start_point, length);
                let from = play_from(stopped_at, length);
                self.start_point = from;
                self.playhead = from;
                self.state = PlaybackState::Playing;
                vec![TransportOrder::Stop, TransportOrder::Start { at: from }]
            }
            PlaybackState::Playing => vec![],
        }
    }

    /// Holds a playing transport where it is, and does nothing otherwise.
    pub fn pause(&mut self) -> Vec<TransportOrder> {
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
            vec![TransportOrder::Pause]
        } else {
            vec![]
        }
    }

    /// Stops the transport, if one is running, and returns the playhead to
    /// the start point, or to the mix's length when an edit has left the
    /// mix shorter than the start point, in which case the start point
    /// moves there too.
    pub fn stop(&mut self, length: Samples) -> Vec<TransportOrder> {
        if self.state == PlaybackState::Stopped {
            return vec![];
        }
        let back_to = stop_to(self.start_point, length);
        self.start_point = back_to;
        self.playhead = back_to;
        self.state = PlaybackState::Stopped;
        vec![TransportOrder::Stop]
    }

    /// What the space bar does: stops a transport that is playing, and
    /// otherwise plays, so that a pause is resumed rather than started
    /// over.
    pub fn space(&mut self, length: Samples) -> Vec<TransportOrder> {
        if self.state == PlaybackState::Playing {
            self.stop(length)
        } else {
            self.play(length)
        }
    }

    /// A click on the ruler at a frame: the playhead and the start point
    /// move there, and a transport that is playing, paused, or has played
    /// the mix out is moved there too. A playing one plays on from there,
    /// a paused one stays paused there, and one that had played the mix
    /// out is paused there, as the transport leaves it, so that a click
    /// after the end moves the playhead without starting playback. A failed
    /// transport is not moved, since it plays nothing, and neither is a
    /// transport that is not running.
    ///
    /// The frame is stored exactly as given, even one past the mix's
    /// length; a [`Seek`](TransportOrder::Seek) order for such a frame
    /// moves the transport only as far as the length, as
    /// [`Transport::seek`](dermixen_engine::Transport::seek) documents.
    pub fn click_ruler(&mut self, at: Samples) -> Vec<TransportOrder> {
        self.playhead = at;
        self.start_point = at;
        match &self.state {
            PlaybackState::Playing | PlaybackState::Paused => {
                vec![TransportOrder::Seek { at }]
            }
            PlaybackState::Ended => {
                self.state = PlaybackState::Paused;
                vec![TransportOrder::Seek { at }]
            }
            PlaybackState::Stopped | PlaybackState::Failed(_) => vec![],
        }
    }

    /// Takes the transport's own report, as the window reads it on every
    /// repaint while a transport runs: the playhead is the report's
    /// position, and the state is the report's, with buffering shown as
    /// playing. A transport reports a move as its position at once, so a
    /// report never puts the playhead back before a click on the ruler. A
    /// report is ignored while the state is stopped, since no transport is
    /// running for it to describe.
    pub fn heard(&mut self, status: &TransportStatus) {
        if self.state == PlaybackState::Stopped {
            return;
        }
        self.playhead = status.position;
        self.state = match &status.state {
            EngineState::Buffering | EngineState::Playing => PlaybackState::Playing,
            EngineState::Paused => PlaybackState::Paused,
            EngineState::Ended => PlaybackState::Ended,
            EngineState::Failed(problem) => PlaybackState::Failed(problem.clone()),
        };
    }

    /// The window's answer when a [`Start`](TransportOrder::Start) order
    /// could not be carried out, like a missing audio device: no
    /// transport is running, so the state goes back to stopped, with the
    /// playhead and the start point left where playing put them.
    pub fn could_not_start(&mut self) {
        self.state = PlaybackState::Stopped;
    }
}
