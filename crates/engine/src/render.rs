//! The offline render: every track stretched to the mix tempo, shaped by its
//! EQ envelopes, scaled by its volume envelope and its gain, and summed into
//! one continuous mix.

use std::ops::Range;
use std::sync::Arc;

use dermixen_core::{
    Beats, Decibels, MAX_BEAT, Mix, PlacedTrack, SAMPLE_RATE, Samples, Seconds, TempoCurve,
    Timeline, Track,
};
use dermixen_media::{Audio, Frame};

use crate::eq::ThreeBandEq;
use crate::stretch::TimeStretcher;

/// The number of frames the render works in at a time.
///
/// Envelope values and stretch rates are read once per block, so this is the
/// finest grain at which a fade or a tempo change is followed.
pub const BLOCK_FRAMES: usize = 1024;

/// How much of a track's own audio the render plays through the track's
/// stretcher and equalizer, before the first frame it delivers of that
/// track, when the track is already sounding where a span begins.
///
/// A stretcher and an equalizer keep state, so a track that is joined
/// partway through is brought up first: the render feeds the stretcher and
/// the equalizer this much of the track's audio from just before the span's
/// start, divided into the same blocks a render of the whole mix divides
/// the track into, and throws away what the two produce. The run-in is
/// counted in the track's own frames, so at any mix tempo it covers the
/// same stretch of the track. A track that began less than a run-in before
/// the span's start is brought up from its first frame, as every track is
/// where it enters the mix.
///
/// Half a second is the figure. The plain resampler remembers one frame, so
/// a track without keylock comes out of a run-in exactly as it comes out of
/// a render of the whole mix, and the equalizer's filters reach the same
/// values well inside half a second. The pitch-preserving stretcher needs
/// about a tenth of a second of audio before it produces anything, and its
/// output after a run-in matches a continuous render in where the beats
/// land and how loud they are, which is what the tests in
/// `tests/preview.rs` hold, though not sample for sample, since the
/// stretcher accumulates phase from everything it has been fed. The run-in
/// costs about five milliseconds of work for a keylocked track, so a
/// preview begins to sound a fraction of a second after it is started at
/// any position in a mix.
pub const RUN_IN: Samples = Samples(SAMPLE_RATE as i64 / 2);

/// The reason a mix could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The number of source buffers does not match the number of tracks.
    #[error("the mix has {tracks} tracks but {sources} sources were given")]
    SourceCount {
        /// The number of tracks in the mix.
        tracks: usize,
        /// The number of source buffers given.
        sources: usize,
    },
    /// A source buffer's length does not match the length the track declares.
    #[error("track {index} declares {expected} samples but its source holds {got}")]
    SourceLength {
        /// The track's position in the playlist.
        index: usize,
        /// The length the track declares.
        expected: i64,
        /// The length of the source buffer.
        got: i64,
    },
    /// The loader could not provide a track's audio.
    #[error("track {index} could not be loaded: {message}")]
    Load {
        /// The track's position in the playlist.
        index: usize,
        /// What the loader said.
        message: String,
    },
    /// The mix is not one a document may hold, so it has no safe layout: a
    /// tempo, a beat, a level, or a length is out of range, or the mix is
    /// longer than the longest mix. [`dermixen_core::Mix::check`] decides.
    #[error("the mix cannot be rendered: {0}")]
    Document(dermixen_core::MixFileError),
    /// The sink could not take a block of the mix.
    #[error("the mix could not be written: {0}")]
    Write(String),
    /// The audio device a preview plays through could not be started, or
    /// stopped taking frames before the span had been played.
    #[error("the audio output failed: {0}")]
    Output(String),
    /// A defect in dermixen stopped a preview: the code filling the feed
    /// panicked, and the text is what the panic said. Rust prints the panic
    /// and the line it happened on to standard error as it happens, so this
    /// tells a person that the preview stopped and where to look rather than
    /// repeating the whole panic. Only [`play`](crate::play) and a
    /// [`Transport`](crate::Transport) report this, because a preview that
    /// stopped without saying so leaves a device playing silence. An offline
    /// render lets a panic reach its caller.
    #[error("the preview stopped on a defect in dermixen: {0}")]
    Defect(String),
}

/// What a panic said, taken from the value
/// [`std::panic::catch_unwind`](std::panic::catch_unwind) hands back.
///
/// A panic raised by `panic!`, by `assert!`, or by `unwrap` on a failure
/// carries its message as a string, which is what a person needs to read.
/// A panic raised with a value of any other type has no message to take, so
/// the words below stand in its place.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the code rendering the mix panicked".to_owned())
}

/// Decoded audio a render reads from.
///
/// The render owns each source only while the track is heard, so a loader
/// may hand over any type that holds frames; the render drops it as soon as
/// the track's last frame has been written.
pub trait Source {
    /// The frames in playback order.
    fn frames(&self) -> &[Frame];
}

impl Source for Audio {
    fn frames(&self) -> &[Frame] {
        &self.frames
    }
}

/// Provides a track's decoded audio when the render first needs it: the
/// track's position in the playlist and the track itself are given, and the
/// error text is what a person should read when the audio cannot be had.
pub type Loader<'a> = dyn FnMut(usize, &Track) -> Result<Box<dyn Source>, String> + 'a;

/// Where a render has got to, reported after each block is written.
///
/// Both counts are output frames from the start of the mix, the same clock
/// the rendered file and `mix show` use, so a render of a span of the mix
/// reports positions within the whole mix rather than within the span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// The output frame just past the last one delivered so far.
    pub written: Samples,
    /// How many frames the whole mix has.
    pub total: Samples,
}

/// Where one track sits on the timeline.
///
/// These four values are what turns an output frame into a moment of the mix,
/// a position in the track's own audio, and a beat of the track, which the
/// render asks for once per block. They are gathered here so that the block
/// loop holds one borrow of the timeline for the track it is working on
/// rather than passing the same four values down through every call.
struct Placement<'a> {
    /// The track itself, which holds the envelopes.
    track: &'a Track,
    /// Where the track was laid out on the timeline.
    placed: &'a PlacedTrack,
    /// The tempo curve of the whole mix.
    curve: &'a TempoCurve,
    /// The mix time of output frame zero, which is the moment the earliest
    /// track begins.
    opening: Seconds,
}

impl Placement<'_> {
    /// The mix time at an output frame, which may fall between two frames.
    fn moment(&self, frame: f64) -> Seconds {
        Seconds(self.opening.0 + frame / f64::from(SAMPLE_RATE))
    }

    /// The position within the track's own audio, counted in frames from its
    /// first sample, that belongs under an output frame.
    ///
    /// It follows the tempo curve, so the span of track a block covers is
    /// whatever the curve gives there rather than a fixed number of frames,
    /// and because every block boundary is placed from the curve directly, a
    /// tempo that changes within a block cannot accumulate an error from one
    /// block to the next.
    fn position(&self, frame: i64) -> i64 {
        self.placed
            .track_time_at(self.curve, self.moment(frame as f64))
            .to_samples()
            .0
    }
}

/// One track while it is being heard: the audio it is read from, the
/// stretcher and the EQ that hold its state from one block to the next, and
/// the frames it has produced but not yet added to the mix.
///
/// A track's own blocks begin at the output frame its first sample is heard
/// at, which is rarely a multiple of [`BLOCK_FRAMES`], so one of them can
/// straddle two blocks of the mix. The frames that fall past the end of the
/// block of the mix being built stay in `waiting` until the mix's next block
/// is built.
struct Playing {
    /// The output frame the track's first sample is heard at.
    entry: i64,
    /// The output frame just past the track's last sample.
    exit: i64,
    /// The track's decoded audio.
    source: Box<dyn Source>,
    /// The stretcher that plays the track at the mix tempo.
    stretcher: Box<dyn TimeStretcher>,
    /// The EQ the track passes through.
    eq: ThreeBandEq,
    /// The span of the track's own audio handed to the stretcher for the
    /// block being produced.
    input: Vec<Frame>,
    /// How much of the track's audio has been handed to the stretcher.
    fed: i64,
    /// How far past the position under a block's own output frame the
    /// render reads for that block. The render read this far into the track
    /// while it primed the stretcher, and every block keeps the same lead.
    lead: i64,
    /// The position in the track's audio that belongs under its first output
    /// frame.
    first_sample: i64,
    /// How many output frames the render has produced for the track since
    /// its entry, so that the entry plus this count is the frame the track's
    /// next block begins at. For a track the render joins at a run-in, the
    /// count includes the frames from the track's entry to the run-in, which
    /// the render never produces.
    produced: i64,
    /// Frames the track has produced that have not been added to the mix
    /// yet, the earliest first. They are what is left over when one of the
    /// track's own blocks runs past the end of the block of the mix being
    /// built, and they go in when the mix's next block is built.
    waiting: Vec<Frame>,
}

impl Playing {
    /// Starts a track playing at the output frame `from`: the stretcher is
    /// primed with the track's own audio just before that frame and what
    /// the priming puts out is thrown away, so that the first frame the
    /// track goes on to produce is the frame of the track that belongs at
    /// `from`.
    ///
    /// `from` is the track's entry where the render reaches the track in
    /// the ordinary way, and the frame the track's run-in begins at where
    /// the render joins a track that is already sounding, which
    /// [`render_range`] describes.
    ///
    /// A stretcher's output lags its input, so the frames it puts out before
    /// that lag has passed belong before `from`. The documentation of the
    /// [`TimeStretcher`] trait states the lag in output frames as the input
    /// latency divided by the speed plus the output latency, where the speed
    /// is input frames per output frame: for a track that is how fast it
    /// plays where it enters the mix. The lag is computed once at that entry
    /// speed, so it is a snapshot: when the tempo is already moving there,
    /// the discarded count is off by a fraction of the latency, far below
    /// the beat tolerances the render tests hold. The frames the priming
    /// takes are the frames of the track just before the ones the block at
    /// `from` reads, so every block from `from` on is fed the frames a
    /// render of the whole mix feeds it.
    fn new(
        source: Box<dyn Source>,
        mut stretcher: Box<dyn TimeStretcher>,
        place: &Placement<'_>,
        entry: i64,
        exit: i64,
        from: i64,
    ) -> Self {
        let speed = place
            .placed
            .rate_at(place.curve, place.placed.start(place.curve));
        let lag = ((stretcher.input_latency().0 as f64 / speed).round()
            + stretcher.output_latency().0 as f64)
            .max(0.0) as i64;

        // The position under the track's first output frame is the track's
        // first sample, give or take the rounding of that frame to a whole
        // sample. Subtracting it lines the first sample up with the first
        // output frame.
        let first_sample = place.position(entry);
        // The position in the track's own audio that belongs under `from`,
        // counted from the track's first sample. The priming reads there, and
        // the blocks the track goes on to produce carry on from there.
        let began = place.position(from) - first_sample;

        let mut input: Vec<Frame> = Vec::new();
        let mut thrown_away = vec![[0.0f32, 0.0f32]; BLOCK_FRAMES];
        let mut fed = began;
        let mut discarded = 0i64;
        while discarded < lag {
            let count = (lag - discarded).min(BLOCK_FRAMES as i64);
            discarded += count;
            let wanted = (began + (discarded as f64 * speed).round() as i64).max(fed);
            gather(&mut input, source.frames(), fed, wanted);
            fed = wanted;
            stretcher.process(&input, &mut thrown_away[..count as usize]);
        }

        Playing {
            entry,
            exit,
            source,
            stretcher,
            eq: ThreeBandEq::new(),
            input,
            fed,
            // The loop above read this far into the track past the position
            // under `from`, and every block the track produces keeps that same
            // lead.
            lead: fed - began,
            first_sample,
            // The frames from the track's entry to `from` are behind the
            // render, so the next block the track produces is the one at
            // `from`.
            produced: from - entry,
            waiting: Vec::new(),
        }
    }

    /// Adds this track's part of one block of the mix.
    ///
    /// `out` is the block being built and `at` is the output frame it begins
    /// at. Frames the track produced for an earlier block but could not fit
    /// into it go in first, then the track produces as many further blocks of
    /// its own as this block of the mix reaches into, and whatever runs past
    /// the end of it waits for the next call.
    fn add_block(&mut self, out: &mut [Frame], at: i64, place: &Placement<'_>) {
        let end = at + out.len() as i64;
        loop {
            // The output frame the earliest waiting frame belongs at. Taking
            // frames off the front of the waiting ones leaves this sum
            // unchanged, so it is worked out afresh on each turn of the loop
            // rather than kept in a field.
            let from = self.entry + self.produced - self.waiting.len() as i64;
            let fits = self.waiting.len().min((end - from).max(0) as usize);
            if fits > 0 {
                debug_assert!(
                    from >= at,
                    "a waiting frame belongs at output frame {from}, \
                     before the block that begins at {at}"
                );
                let offset = (from - at) as usize;
                for (frame, sum) in self.waiting[..fits].iter().zip(&mut out[offset..]) {
                    sum[0] += frame[0];
                    sum[1] += frame[1];
                }
                self.waiting.drain(..fits);
            }
            // Frames are still waiting only when the block being built is full,
            // and there is nothing more to produce once the track has reached
            // either the end of this block or its own last sample.
            if !self.waiting.is_empty() || self.entry + self.produced >= end.min(self.exit) {
                return;
            }
            self.produce(place);
        }
    }

    /// Produces the track's next block of its own.
    ///
    /// The stretcher turns the span of the track's own audio that belongs
    /// under the block into exactly one block of output, and the EQ filters
    /// that block with the gains the three EQ envelopes give. The volume
    /// envelope and the track's gain add up to one level, and that level
    /// scales the block. The frames join the ones waiting to be added to the
    /// mix.
    fn produce(&mut self, place: &Placement<'_>) {
        let at = self.entry + self.produced;
        let count = (self.exit - at).min(BLOCK_FRAMES as i64);
        // One value of each envelope serves the whole block, so it is read at
        // the middle of the block rather than at its first frame: the middle is
        // where the block's average level sits, while the first frame would
        // hold every fade out half a block too loud and start every fade in
        // half a block too quiet. Envelope nodes sit at beats of the track, so
        // the mix beat here is moved back by the beat at which the track was
        // placed.
        let beat = place
            .curve
            .beat_at(place.moment(at as f64 + count as f64 / 2.0))
            - place.placed.origin;

        let wanted = (self.lead + place.position(at + count) - self.first_sample).max(self.fed);
        gather(&mut self.input, self.source.frames(), self.fed, wanted);
        self.fed = wanted;

        let start = self.waiting.len();
        self.waiting.resize(start + count as usize, [0.0, 0.0]);
        let played = &mut self.waiting[start..];
        self.stretcher.process(&self.input, played);
        self.eq.set_gains(
            place.track.eq.low.value_at(beat),
            place.track.eq.mid.value_at(beat),
            place.track.eq.high.value_at(beat),
        );
        self.eq.process(played);
        // The track's level is its volume envelope's value at this beat plus
        // its gain. The envelope alone decides silence: at or below the
        // silence floor the track adds nothing to the mix however much gain it
        // has. The transport reads a document by the same rule when it tells a
        // silent track from a sounding one. A gain of zero multiplies the block
        // by the envelope's value alone.
        let volume = place.track.volume.value_at(beat);
        let level = if volume <= Decibels::SILENCE {
            0.0
        } else {
            (volume + place.track.gain).to_linear() as f32
        };
        for frame in played.iter_mut() {
            frame[0] *= level;
            frame[1] *= level;
        }
        self.produced += count;
    }
}

/// Renders a mix block by block to a sink, loading each track's audio only
/// while it is heard.
///
/// The output is the same audio [`render`] produces, from the timeline's
/// start to its end, delivered to `sink` in order as blocks of
/// [`BLOCK_FRAMES`] frames, the last block shorter when the mix does not
/// divide evenly, and `progress` is called after each block. The return
/// value is the number of frames written. This is [`render_range`] over the
/// whole mix, from frame zero to the mix's length. Each track is stretched to
/// the mix tempo, filtered by its three EQ envelopes, and scaled by the level
/// its volume envelope and its gain add up to before the tracks are summed.
///
/// `load` is called once per track, in playlist order, when the render
/// reaches the first block that holds any of that track's frames, and the
/// source it returns is dropped after the last block that does. A mix of
/// many tracks therefore needs only the tracks that overlap the current
/// block in memory at once, which is what lets a ninety-minute mix render
/// on an ordinary machine.
///
/// A mix that [`dermixen_core::Mix::check`] refuses is refused here with
/// [`RenderError::Document`] naming the field that is out of range, before
/// the mix is laid out, before a track is loaded, and before a stretcher is
/// made. A source sample that is not a finite number is read as silence, so
/// no stretcher, no equalizer, and no sum receives one, and a sample that is
/// a finite number is passed on unchanged.
///
/// A source whose length differs from the track's
/// declared length is refused with [`RenderError::SourceLength`] when it is
/// loaded, a loader that fails stops the render with [`RenderError::Load`],
/// and a sink that fails stops it with [`RenderError::Write`]; in each case
/// the blocks already written stay written. An empty mix writes nothing,
/// loads nothing, and returns zero.
pub fn render_to(
    mix: &Mix,
    load: &mut Loader<'_>,
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
    sink: &mut dyn FnMut(&[Frame]) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
) -> Result<Samples, RenderError> {
    render_range(
        mix,
        Samples::ZERO..Samples(i64::MAX),
        load,
        stretchers,
        sink,
        progress,
    )
}

/// Renders the span of a mix between two output frames, loading only the
/// tracks that are heard within it.
///
/// The frames delivered are the frames [`render_to`] would deliver at the
/// same positions: from the span's start up to but not including its end,
/// both counted in output frames from the start of the mix, the clock the
/// rendered file and `mix show` use. For a track without keylock they are
/// those frames exactly, and for a keylocked track they are the same
/// music, in the sense defined below under the run-in. An end past the end of the mix is
/// clipped to the mix's length, a start before zero is clipped to zero, and
/// a span that is empty after clipping, including one whose start is at or
/// past the end of the mix, delivers nothing, loads nothing, and returns
/// zero. The frames arrive at `sink` in order, in blocks of at most
/// [`BLOCK_FRAMES`] frames, and the return value is the number of frames
/// delivered.
///
/// A track whose last frame falls before the span is never loaded. A track
/// already being heard when the span starts is loaded and brought up from a
/// run-in before the first block is delivered: its stretcher and its EQ are
/// fed the [`RUN_IN`] of the track's own audio that precedes the span's
/// start, or all of the track's audio before the span when the track began
/// less than that before it, divided into the same blocks a render of the
/// whole mix divides the track into, and what the two produce for the
/// run-in is thrown away. The blocks of a track begin at its entry and run
/// in steps of [`BLOCK_FRAMES`] output frames, so the run-in begins at the
/// latest of the track's first frame and the block boundary of the track at
/// or before the point in the track that lies one [`RUN_IN`] before the
/// span's start. The stretcher is primed at the run-in's first block as it
/// is primed where the track enters the mix, with the lag worked out from
/// the speed at the track's entry, and the frames the priming consumes are
/// the frames of the track just before that block, so every block from
/// there on is fed exactly the frames a render of the whole mix feeds it.
/// Each track sounding at the span's start has a run-in of its own, and no
/// track is fed a frame before its own run-in begins, whichever track's
/// run-in begins earliest. A track without keylock is then delivered
/// exactly as [`render_to`] delivers it, since the resampler remembers one
/// frame and the equalizer reaches the same values inside the run-in. A
/// keylocked track is delivered as the same music: the pitch-preserving
/// stretcher accumulates phase from everything it has been fed and places
/// its analysis windows from wherever it was started, so its frames after a
/// run-in are not the frames of a render of the whole mix. The tests hold
/// them instead to the whole render by measurement: every beat lands within
/// a millisecond and within a decibel of where the whole render puts it,
/// the level of every quarter second, overall and around each tone, is
/// within half a decibel of the whole render's, and every tone is at its
/// pitch within ten hertz. `progress` is
/// called after each block with the position within the whole mix, as
/// [`Progress`] describes, so its `written` begins past the span's start
/// and finishes at the span's end. Errors are as for [`render_to`].
pub fn render_range(
    mix: &Mix,
    span: Range<Samples>,
    load: &mut Loader<'_>,
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
    sink: &mut dyn FnMut(&[Frame]) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
) -> Result<Samples, RenderError> {
    render_carrying(mix, span, load, stretchers, sink, progress, &mut |_| None)
}

/// A document handed to a run of the render that is already under way, to be
/// rendered from the block boundary at which it is handed over.
pub(crate) struct Handover {
    /// The document to render from that boundary on.
    pub(crate) mix: Arc<Mix>,
    /// The output frame the run now ends at, which is the new document's
    /// length.
    pub(crate) until: i64,
}

/// Asked at the boundary between two blocks, before the later of the two is
/// built, and once more as the run stops, however it stops.
///
/// The argument is the output frame at which the render will ask again, or
/// `None` when this run will ask no more, so that a caller knows which frame
/// a document it hands over now would take effect at, and learns that the run
/// will take nothing further before the run returns. The answer is the
/// document to go on with from this boundary, or `None` to go on with the one
/// being rendered. A document given to the final ask is dropped, since there
/// is no longer a run to render it.
pub(crate) type Carry<'a> = dyn FnMut(Option<i64>) -> Option<Handover> + 'a;

/// The document one run of the render is working from: the one its caller lent
/// it, or one handed over while it ran.
enum Held<'a> {
    /// The document the caller lent the render for the whole run.
    Lent(&'a Mix),
    /// A document handed over while the run was under way.
    Given(Arc<Mix>),
}

impl Held<'_> {
    /// The document itself.
    fn mix(&self) -> &Mix {
        match self {
            Held::Lent(mix) => mix,
            Held::Given(mix) => mix,
        }
    }
}

/// The run of output frames each track of a laid-out mix occupies, between the
/// moment its first sample is heard and the moment its last sample has been
/// heard.
///
/// `opening` is the mix time at which the earliest track begins, which is
/// output frame zero, and `length` is how many output frames the whole mix
/// has. Working these runs out before any audio is loaded is what lets the
/// block loop know which tracks the block it is about to build needs, and a
/// [`Transport`](crate::Transport) asks the same question of two documents
/// when it decides whether a render can go on through a replacement.
pub(crate) fn track_spans(timeline: &Timeline, opening: Seconds, length: i64) -> Vec<(i64, i64)> {
    timeline
        .tracks
        .iter()
        .map(|placed| {
            let entry = (placed.start(&timeline.curve) - opening)
                .to_samples()
                .0
                .clamp(0, length);
            let exit = (placed.end(&timeline.curve) - opening)
                .to_samples()
                .0
                .clamp(entry, length);
            (entry, exit)
        })
        .collect()
}

/// Loads a track's audio and starts it playing at the output frame `from`,
/// which [`Playing::new`] describes.
///
/// A loader that fails stops the render with [`RenderError::Load`], and a
/// source whose length is not the length the track declares stops it with
/// [`RenderError::SourceLength`], wherever the render reached the track.
fn start_playing(
    index: usize,
    place: &Placement<'_>,
    load: &mut Loader<'_>,
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
    entry: i64,
    exit: i64,
    from: i64,
) -> Result<Playing, RenderError> {
    let source =
        load(index, place.track).map_err(|message| RenderError::Load { index, message })?;
    let got = source.frames().len() as i64;
    if got != place.track.length.0 {
        return Err(RenderError::SourceLength {
            index,
            expected: place.track.length.0,
            got,
        });
    }
    Ok(Playing::new(
        source,
        stretchers(place.track),
        place,
        entry,
        exit,
        from,
    ))
}

/// The output frame at which the render begins feeding a track that is
/// already sounding at `from`, which is where that track's run-in begins.
///
/// A track's own blocks begin at its entry and run in steps of
/// [`BLOCK_FRAMES`] output frames, so this is the last of those boundaries
/// whose position in the track's own audio is at or before the position one
/// [`RUN_IN`] earlier than `from`, and the track's entry when the track
/// began less than a run-in before `from`. Searching the boundaries rather
/// than counting back a fixed number of output frames is what makes the
/// run-in the same stretch of the track at any mix tempo, since a block of
/// the mix covers as many of the track's frames as the tempo curve gives
/// there.
fn run_in_start(place: &Placement<'_>, entry: i64, from: i64) -> i64 {
    let block = BLOCK_FRAMES as i64;
    let target = place.position(from) - RUN_IN.0;
    // A later boundary stands at a later position in the track, so halving
    // the range of boundaries between the track's entry and `from` finds the
    // last one at or before that position.
    let mut low = 0i64;
    let mut high = (from - entry).max(0) / block;
    while low < high {
        let middle = (low + high + 1) / 2;
        if place.position(entry + middle * block) <= target {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    entry + low * block
}

/// Renders the span of a mix as [`render_range`] does, asking `carry` at the
/// boundary between two blocks whether to go on with another document.
///
/// A run that is offered no other document delivers exactly what
/// `render_range` delivers. A run that is given one goes on from that boundary
/// with the state it has built rather than beginning again: every track
/// sounding there keeps its stretcher, its equalizer, and its position within
/// its own audio, and every value the render reads from the document from that
/// frame on is the new document's: the tempo curve, the envelopes, and where
/// each track sits on the timeline. A track the new document has sounding at
/// that boundary that the old one did not is loaded there and brought up
/// from a run-in to the boundary, as [`render_range`] brings up a track
/// sounding where a span begins, and what its stretcher and its EQ produce
/// for the run-in is thrown away. The frames that follow are the new
/// document's own render only when the new document renders the same frames as
/// the old one up to that boundary, so whoever hands a document over is
/// responsible for that;
/// [`Transport::replace`](crate::Transport::replace) describes the check that
/// decides it. The run then ends at the new document's length rather than the
/// old one's, and the return value is the number of frames delivered, which is
/// the span's length when no document is handed over.
///
/// Every way out of this call (the end of the span, a loader that failed, a
/// sink that failed) asks `carry` one last time with `None` before it
/// returns, so a caller learns that this run will take no document while the
/// call is still under way rather than afterwards. That leaves no moment in
/// which a caller could hand a document to a run that has already stopped.
pub(crate) fn render_carrying(
    mix: &Mix,
    span: Range<Samples>,
    load: &mut Loader<'_>,
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
    sink: &mut dyn FnMut(&[Frame]) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
    carry: &mut Carry<'_>,
) -> Result<Samples, RenderError> {
    let rendered = render_blocks(mix, span, load, stretchers, sink, progress, carry);
    // The run is over, so it takes nothing more. A document handed over just
    // before it stopped is answered here and dropped, since there is no run
    // left to render it.
    carry(None);
    rendered
}

/// The block loop of [`render_carrying`], which makes the final ask this
/// function does not.
fn render_blocks(
    mix: &Mix,
    span: Range<Samples>,
    load: &mut Loader<'_>,
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
    sink: &mut dyn FnMut(&[Frame]) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
    carry: &mut Carry<'_>,
) -> Result<Samples, RenderError> {
    // A mix no document may hold has no safe layout, so the run refuses it
    // here, before a track is loaded, a stretcher is made, or the timeline is
    // worked out. Every entry point that renders a span reaches this
    // function, so this one check covers all of them.
    mix.check().map_err(RenderError::Document)?;

    let mut held = Held::Lent(mix);
    let Some(mut timeline) = held.mix().timeline() else {
        return Ok(Samples::ZERO);
    };

    // Mix time zero is the first track's beat zero, which is usually not the
    // moment the mix first makes a sound, so every output frame below is
    // counted from the moment the earliest track begins.
    let mut opening = timeline.start();
    let mut length = (timeline.end() - opening).to_samples().0.max(0);
    let span = clip(&span, length);
    if span.is_empty() {
        return Ok(Samples::ZERO);
    }
    // The output frame the run ends at, which is the length of the document
    // being rendered. A document handed over while the run is under way moves
    // it, since that document may be longer or shorter than the one before it.
    let mut until = span.end;
    let mut spans = track_spans(&timeline, opening, length);

    // The output frame at which the render begins feeding each track, which
    // is where a track enters the mix unless the track is already being
    // heard where the span starts.
    let mut fed_from: Vec<i64> = spans.iter().map(|&(entry, _)| entry).collect();
    let mut playing: Vec<Option<Playing>> = (0..held.mix().tracks.len()).map(|_| None).collect();
    // A track that is already being heard where the span starts is brought
    // up from a run-in of its own, so that its stretcher and its EQ hold the
    // state a render of the whole mix would leave them in there. The render
    // loads every such track here, in playlist order, regardless of the
    // order in which their run-ins begin: a run-in is half a second of a
    // track's own audio, so it covers fewer output frames of a track that
    // plays fast than of one that plays slow, and the earliest run-in is not
    // the earliest track of the playlist. The blocks the loop below throws
    // away are what bring each of these tracks up.
    for (index, &(entry, exit)) in spans.iter().enumerate() {
        if entry >= span.start || exit <= span.start {
            continue;
        }
        let place = Placement {
            track: &held.mix().tracks[index],
            placed: &timeline.tracks[index],
            curve: &timeline.curve,
            opening,
        };
        let from = run_in_start(&place, entry, span.start);
        fed_from[index] = from;
        playing[index] = Some(start_playing(
            index, &place, load, stretchers, entry, exit, from,
        )?);
    }
    // The block loop begins at the earliest of those run-ins, and the blocks
    // it builds before the span's start are thrown away instead of
    // delivered. A track whose run-in begins later than that is left alone
    // until the loop reaches it, so no track is fed a frame before its own
    // run-in.
    let mut at = fed_from
        .iter()
        .zip(&spans)
        .filter(|(_, (entry, exit))| *entry < span.start && *exit > span.start)
        .map(|(from, _)| *from)
        .min()
        .unwrap_or(span.start);

    let mut buffer = vec![[0.0f32, 0.0f32]; BLOCK_FRAMES];
    let mut delivered = 0i64;
    while at < until {
        // The last of the blocks that are thrown away ends exactly where the
        // span starts, so every block from there on begins where a block of
        // the delivered span begins. A track's own blocks run from its first
        // frame in steps of its own, so where the blocks of the mix fall makes
        // no difference to what any track produces.
        let limit = if at < span.start { span.start } else { until };
        let count = (limit - at).min(BLOCK_FRAMES as i64) as usize;
        let end = at + count as i64;

        // Where this run will ask for a document again, which is the frame at
        // which one handed over now takes effect. The blocks before the span
        // are thrown away rather than heard, and no boundary follows the last
        // block of the run, so neither is offered as a place to take one.
        let asking_again = (at >= span.start && end < until).then_some(end);
        // A document that Mix::check refuses has no safe layout, so the run
        // leaves it and goes on with the document it already holds.
        // Transport::replace turns such a document away for the same reason
        // and keeps the document the transport is playing, and a transport is
        // the only caller that hands a document over, so the two agree.
        let handover = carry(asking_again).filter(|handover| handover.mix.check().is_ok());
        if let Some(handover) = handover {
            // A document with no tracks has no timeline and nothing left to
            // render, so the run ends here rather than going on with a
            // document it cannot lay out.
            let Some(laid_out) = handover.mix.timeline() else {
                break;
            };
            held = Held::Given(handover.mix);
            timeline = laid_out;
            opening = timeline.start();
            length = (timeline.end() - opening).to_samples().0.max(0);
            until = handover.until;
            spans = track_spans(&timeline, opening, length);
            // Every track the new document has sounding at this boundary is
            // brought up below, so from here on the block loop starts only
            // tracks that enter later, each at its own entry.
            fed_from = spans.iter().map(|&(entry, _)| entry).collect();
            // The tracks being heard at this boundary go on as they are, with
            // the run of output frames they now occupy: an edit past the
            // boundary can move the frame a sounding track ends at without
            // moving the frame it entered at. A track the new document does
            // not hold in the same place is dropped. The check a transport
            // makes before it hands a document over never allows that.
            playing.resize_with(held.mix().tracks.len(), || None);
            for (index, slot) in playing.iter_mut().enumerate() {
                let Some(voice) = slot.as_mut() else {
                    continue;
                };
                match spans.get(index).copied() {
                    Some((entry, exit)) if entry == voice.entry => {
                        // A track cannot end before the frames it has already
                        // produced, so an edit that pulls its ending back
                        // leaves it sounding until those have gone into the
                        // mix. The run of frames kept here is written back, so
                        // that the block loop below and this track agree on
                        // where it ends.
                        let exit = exit.max(voice.entry + voice.produced);
                        spans[index] = (entry, exit);
                        voice.exit = exit;
                    }
                    _ => *slot = None,
                }
            }
            // A track the new document has sounding here that the old one did
            // not is brought up from a run-in to this boundary, so that its
            // stretcher and its equalizer hold the state a render of the
            // whole new document would leave them in, and so that the block
            // below has no frames of it left over from before the boundary
            // to place. The frames it makes on the way are thrown away as
            // they are made: they belong to output frames the render has
            // already delivered to the feed.
            for (index, &(entry, exit)) in spans.iter().enumerate() {
                let already_playing = playing.get(index).is_some_and(Option::is_some);
                if already_playing || entry >= at || exit <= at {
                    continue;
                }
                let track = &held.mix().tracks[index];
                let place = Placement {
                    track,
                    placed: &timeline.tracks[index],
                    curve: &timeline.curve,
                    opening,
                };
                let from = run_in_start(&place, entry, at);
                let mut voice = start_playing(index, &place, load, stretchers, entry, exit, from)?;
                while voice.entry + voice.produced < at {
                    voice.produce(&place);
                    // Everything the track has made that belongs before the
                    // boundary goes now rather than piling up, so bringing a
                    // track up across a long silent opening costs one block of
                    // frames rather than all of them.
                    let from = voice.entry + voice.produced - voice.waiting.len() as i64;
                    let stale = (at - from).clamp(0, voice.waiting.len() as i64) as usize;
                    voice.waiting.drain(..stale);
                }
                playing[index] = Some(voice);
            }
        }

        let mix = held.mix();
        let block = &mut buffer[..count];
        block.fill([0.0, 0.0]);

        for (index, track) in mix.tracks.iter().enumerate() {
            let (entry, exit) = spans[index];
            // A track that has been heard out before the span starts is never
            // loaded, which is what keeps a span near the end of a long mix
            // from decoding everything that came before it. A track whose
            // run-in has not begun by the end of this block is fed nothing
            // until the block its run-in begins in, so no stretcher is fed a
            // frame of its track before that track's run-in.
            if entry >= exit || exit <= span.start || fed_from[index] >= end || exit <= at {
                continue;
            }
            let place = Placement {
                track,
                placed: &timeline.tracks[index],
                curve: &timeline.curve,
                opening,
            };
            // Every track still to be loaded enters within the span, and
            // walking the playlist in order and starting such a track at the
            // first block that contains any of its frames is what makes the
            // loads come in playlist order without the render having to sort
            // them.
            let voice = match &mut playing[index] {
                Some(voice) => voice,
                slot @ None => slot.insert(start_playing(
                    index,
                    &place,
                    load,
                    stretchers,
                    entry,
                    exit,
                    fed_from[index],
                )?),
            };
            voice.add_block(block, at, &place);
        }

        // A track whose last sample falls in this block is never read again, so
        // the render drops its audio before the next block is built and holds
        // only the tracks that are still being heard.
        for slot in &mut playing {
            if slot.as_ref().is_some_and(|voice| voice.exit <= end) {
                *slot = None;
            }
        }

        if end > span.start {
            sink(block).map_err(RenderError::Write)?;
            delivered += count as i64;
            progress(Progress {
                written: Samples(end),
                total: Samples(length),
            });
        }
        at = end;
    }
    Ok(Samples(delivered))
}

/// The output frames a span covers once it has been clipped to a mix of
/// `length` frames.
///
/// A start before the mix begins is moved to its first frame, an end past the
/// mix's last frame is moved to that, and a span with nothing left in it,
/// including one whose start is at or past the end of the mix, comes back
/// empty. Rendering a span and previewing one clip through this one function,
/// so the two always agree on which frames a span holds.
pub(crate) fn clip(span: &Range<Samples>, length: i64) -> Range<i64> {
    let from = span.start.0.clamp(0, length);
    from..span.end.0.clamp(from, length)
}

/// How many output frames the whole mix has, on the clock a rendered file
/// holds and `mix show` reports: from the moment the earliest track begins to
/// the moment the latest track ends. A mix with no tracks has none. This is
/// the length a span is clipped to, so a command that refuses a start past
/// the end of the mix measures the end here.
///
/// A mix whose values would make [`dermixen_core::Mix::timeline`] reach the
/// panic its documentation states has none either: this function tests every
/// tempo, every anchor, and every tempo node's beat first, and answers zero
/// rather than laying such a mix out. That test is narrower than
/// [`dermixen_core::Mix::check`], so a mix refused for a value the layout
/// survives, such as a gain out of range, still has its real length here,
/// and every entry point of the engine that renders runs the whole of
/// `Mix::check` and refuses it. A window calls this on every repaint, so the
/// mix is laid out once here and not again.
pub fn mix_length(mix: &Mix) -> Samples {
    if !lays_out(mix) {
        return Samples::ZERO;
    }
    Samples(mix.timeline().map_or(0, |timeline| {
        (timeline.end() - timeline.start()).to_samples().0.max(0)
    }))
}

/// Whether [`dermixen_core::Mix::timeline`] lays this mix out without the
/// panic its documentation states under "Panics".
///
/// Three kinds of value make that layout panic: a tempo the tempo curve
/// refuses, which is a tempo outside the range from
/// [`Bpm::LOWEST`](dermixen_core::Bpm::LOWEST) to
/// [`Bpm::HIGHEST`](dermixen_core::Bpm::HIGHEST), a beat that is not a
/// finite number, and a running sum of anchors that reaches a number that is
/// not finite. Testing that every tempo is one
/// [`Bpm::is_valid`](dermixen_core::Bpm::is_valid) accepts, and that every
/// anchor and every tempo node's beat is within [`MAX_BEAT`] of zero, rules
/// out all three: each track moves the running sum by at most twice
/// [`MAX_BEAT`], so a mix would need more tracks than a machine can hold
/// before the sum stopped being finite.
///
/// This is narrower than [`dermixen_core::Mix::check`], which also bounds a
/// track's length, a gain, an envelope level, and the length of the whole
/// mix. A mix that fails only one of those lays out without a panic, so
/// [`mix_length`] answers its real length. Every entry point of the engine
/// that renders runs the whole of `Mix::check` and refuses such a mix.
fn lays_out(mix: &Mix) -> bool {
    // A comparison against NaN is false, so each test below rules out a beat
    // that is not a number as well as one that is out of range.
    let beat_in_range = |beat: Beats| beat.0.abs() <= MAX_BEAT.0;
    mix.tracks.iter().all(|track| {
        track.grid.bpm.is_valid()
            && beat_in_range(track.anchors.intro)
            && beat_in_range(track.anchors.outro)
            && track
                .tempo
                .iter()
                .all(|node| beat_in_range(node.at) && node.bpm.is_valid())
    })
}

/// Renders a mix to audio held in memory.
///
/// This is [`render_to`] with every source already decoded and the whole
/// mix collected into one buffer, which suits tests and short mixes.
/// `sources` holds the decoded audio of each track in playlist order, and
/// `stretchers` makes a fresh stretcher for a track, which is how the caller
/// chooses between pitch-preserving stretching and plain resampling for that
/// track's keylock setting. A render owns the audio it reads, so each source
/// is copied when its track is first heard, and the render drops that copy
/// once the track has been heard out.
///
/// The output runs from the timeline's start to its end, so its first frame
/// is the earliest first sample of any track and its last frame is the
/// latest last sample. Within it, at every instant each track plays at the
/// mix tempo divided by its original tempo, then passes through its EQ with
/// the three band gains its EQ envelopes give at that moment, then is scaled
/// by the level its volume envelope and its gain add up to, and the tracks
/// are summed without any further processing. The envelope alone decides
/// silence: at an instant where a track's volume envelope is at or below
/// [`Decibels::SILENCE`], that track adds nothing to the mix whatever its
/// gain. An empty mix renders to empty audio. The render is deterministic:
/// the same document and sources always give identical output.
///
/// A mix that [`dermixen_core::Mix::check`] refuses is refused with
/// [`RenderError::Document`], and a source sample that is not a finite number
/// is read as silence, both as [`render_to`] describes.
pub fn render(
    mix: &Mix,
    sources: &[Audio],
    stretchers: &mut dyn FnMut(&Track) -> Box<dyn TimeStretcher>,
) -> Result<Audio, RenderError> {
    if mix.tracks.len() != sources.len() {
        return Err(RenderError::SourceCount {
            tracks: mix.tracks.len(),
            sources: sources.len(),
        });
    }
    // The layout below reads the mix, so the mix is checked before it. The
    // block loop checks the mix again, which costs one further pass over the
    // tracks and happens once per render rather than once per block.
    mix.check().map_err(RenderError::Document)?;
    let mut mixed = Audio::new();
    // Laying the mix out gives the finished length before the first block
    // arrives, so the one buffer this form collects into is allocated once
    // rather than doubling in size as the blocks come in.
    if let Some(timeline) = mix.timeline() {
        let length = (timeline.end() - timeline.start()).to_samples().0.max(0);
        mixed.frames.reserve_exact(length as usize);
    }
    render_to(
        mix,
        &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
        stretchers,
        &mut |block| {
            mixed.frames.extend_from_slice(block);
            Ok(())
        },
        &mut |_| {},
    )?;
    Ok(mixed)
}

/// A source sample as the render reads it: the sample itself when it is a
/// finite number, and silence when it is not.
///
/// One sample that is not finite would otherwise pass into the state a
/// stretcher keeps and the state an equalizer keeps, and from there into the
/// sum of every track sounding with it, so a whole passage of the mix would
/// come out as silence or as a value no device can play. The decoder screens
/// the audio it produces. This function screens the audio a render reads,
/// whatever produced that audio.
fn finite(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

/// Fills `input` with the track's frames from `from` up to but not including
/// `to`, where every position before the track's first sample or after its
/// last one is silence, and so is every sample that is not a finite number.
///
/// This is the one place a render reads a track's own audio, so screening
/// the samples here keeps them from every stretcher, every equalizer, and
/// every sum, and costs one test per sample on the way in rather than one
/// per sample at each later stage. A sample that is a finite number is
/// passed on unchanged, so a render of clean audio is the render it was
/// before.
fn gather(input: &mut Vec<Frame>, source: &[Frame], from: i64, to: i64) {
    input.clear();
    input.reserve((to - from).max(0) as usize);
    for index in from..to {
        let frame = usize::try_from(index)
            .ok()
            .and_then(|index| source.get(index))
            .copied()
            .unwrap_or([0.0, 0.0]);
        input.push([finite(frame[0]), finite(frame[1])]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use dermixen_core::{
        Anchors, BeatGrid, Beats, Bpm, ContentHash, Envelope, EqEnvelopes, TempoNode,
    };
    use dermixen_testkit::synth;

    use crate::stretch::Resampler;

    /// A mix of one track: twenty seconds of kicks at 130 beats per minute,
    /// with the grid starting at the first sample.
    fn one_track() -> (Mix, Audio) {
        let audio = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(20.0));
        let track = Track {
            path: std::path::PathBuf::from("synthetic"),
            hash: ContentHash([0; 32]),
            length: audio.len(),
            grid: BeatGrid {
                first_beat: Samples::ZERO,
                bpm: Bpm(130.0),
            },
            anchors: Anchors {
                intro: Beats(0.0),
                outro: Beats(64.0),
            },
            keylock: false,
            gain: dermixen_core::Decibels::UNITY,
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        };
        (
            Mix {
                tracks: vec![track],
            },
            audio,
        )
    }

    /// The plain resampling stretcher, which every render here uses.
    fn resampler(_: &Track) -> Box<dyn TimeStretcher> {
        Box::new(Resampler::new())
    }

    #[test]
    fn a_document_handed_over_mid_render_delivers_its_own_render_from_there() {
        let (before, audio) = one_track();
        // A tempo excursion four beats past the frame the document is handed
        // over at: the mix holds 130 beats per minute up to the pin and falls
        // to 125 over the beat after it, so every frame before the pin is the
        // same in both documents and the frames after it are not. The slower
        // tempo also makes the mix longer, so the run has to end at the new
        // document's length rather than the old one's.
        let handed_over_after = 5 * 44_100;
        let pin = Beats((handed_over_after as f64 / 44_100.0 * 130.0 / 60.0).ceil() + 4.0);
        let mut after = before.clone();
        after.tracks[0].tempo.push(TempoNode {
            at: pin,
            bpm: Bpm(130.0),
        });
        after.tracks[0].tempo.push(TempoNode {
            at: pin + Beats(1.0),
            bpm: Bpm(125.0),
        });

        let sources = [audio];
        let whole_before = render(&before, &sources, &mut resampler).unwrap().frames;
        let whole_after = render(&after, &sources, &mut resampler).unwrap().frames;
        assert!(
            whole_after.len() > whole_before.len() && whole_after != whole_before,
            "the excursion has to change the render for this test to prove anything"
        );

        // The frame at which a document handed over at the next boundary takes
        // effect, learned as a transport learns it, and the frame the document
        // handed over here took effect at.
        let mut boundary: Option<i64> = None;
        let mut applied: Option<i64> = None;
        let mut carried: Vec<Frame> = Vec::new();
        let delivered = render_carrying(
            &before,
            Samples::ZERO..Samples(i64::MAX),
            &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
            &mut resampler,
            &mut |block| {
                carried.extend_from_slice(block);
                Ok(())
            },
            &mut |_| {},
            &mut |asking_again| {
                let handing = boundary
                    .filter(|frame| applied.is_none() && *frame >= handed_over_after)
                    .map(|frame| {
                        applied = Some(frame);
                        Handover {
                            mix: Arc::new(after.clone()),
                            until: mix_length(&after).0,
                        }
                    });
                boundary = asking_again;
                handing
            },
        )
        .unwrap();

        let applied = applied.expect("the render reached the frame to hand the document over at");
        assert!(
            (applied as f64 / 44_100.0) < pin.0 * 60.0 / 130.0,
            "the document was handed over at frame {applied}, which is past the pin"
        );
        assert_eq!(
            delivered.0,
            whole_after.len() as i64,
            "the run ends at the new document's length"
        );
        assert!(
            carried == whole_after,
            "the frames rendered are not the new document's own render"
        );
    }

    #[test]
    fn a_run_that_stops_early_says_it_will_take_no_more_before_it_returns() {
        // The transport hands a document to the frame a run last published,
        // so a run that stops on a failed sink has to take that frame away
        // before it returns; otherwise a replacement landing in between would
        // be recorded against a run that renders nothing.
        let (mix, audio) = one_track();
        let sources = [audio];
        let mut asks: Vec<Option<i64>> = Vec::new();
        let mut blocks = 0;
        let stopped = render_carrying(
            &mix,
            Samples::ZERO..Samples(i64::MAX),
            &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
            &mut resampler,
            &mut |_| {
                blocks += 1;
                if blocks > 4 {
                    Err("the sink gave up".to_owned())
                } else {
                    Ok(())
                }
            },
            &mut |_| {},
            &mut |asking_again| {
                asks.push(asking_again);
                None
            },
        );
        assert!(
            matches!(stopped, Err(RenderError::Write(ref message)) if message == "the sink gave up"),
            "{stopped:?}"
        );
        assert!(
            asks.iter().any(|ask| ask.is_some()),
            "the run published a frame to hand a document over at"
        );
        assert_eq!(
            asks.last(),
            Some(&None),
            "the last thing the run did was take that frame away"
        );
    }

    #[test]
    fn a_mix_that_would_not_lay_out_has_no_length() {
        let (mix, _) = one_track();
        let whole = mix_length(&mix);
        assert!(whole.0 > 0, "the mix this test damages has a length");

        // Every value Mix::timeline names under "Panics", each on its own.
        type Damage = fn(&mut Mix);
        let damages: Vec<(&str, Damage)> = vec![
            ("grid.bpm", |mix| mix.tracks[0].grid.bpm = Bpm(f64::NAN)),
            ("grid.bpm", |mix| mix.tracks[0].grid.bpm = Bpm(0.0)),
            ("grid.bpm", |mix| mix.tracks[0].grid.bpm = Bpm(1e-300)),
            ("anchors.intro", |mix| {
                mix.tracks[0].anchors.intro = Beats(f64::INFINITY);
            }),
            ("anchors.outro", |mix| {
                mix.tracks[0].anchors.outro = Beats(1e18);
            }),
            ("tempo[0].beat", |mix| {
                mix.tracks[0].tempo.push(TempoNode {
                    at: Beats(f64::NAN),
                    bpm: Bpm(130.0),
                });
            }),
            ("tempo[0].bpm", |mix| {
                mix.tracks[0].tempo.push(TempoNode {
                    at: Beats(8.0),
                    bpm: Bpm(f64::INFINITY),
                });
            }),
        ];
        for (field, damage) in damages {
            let mut damaged = mix.clone();
            damage(&mut damaged);
            assert!(
                damaged.check().is_err(),
                "{field}: the document limits accept this value"
            );
            assert_eq!(mix_length(&damaged), Samples::ZERO, "{field}");
        }
    }

    #[test]
    fn a_mix_refused_only_for_a_value_the_layout_survives_still_has_a_length() {
        // A gain far outside the range a document may hold is refused by
        // Mix::check and changes nothing about where the tracks sit, so the
        // mix lays out and has the length it had.
        let (mix, _) = one_track();
        let mut loud = mix.clone();
        loud.tracks[0].gain = dermixen_core::Decibels(1e9);
        assert!(
            loud.check().is_err(),
            "the document limits accept this gain"
        );
        assert_eq!(mix_length(&loud), mix_length(&mix));
    }
}
