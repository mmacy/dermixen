//! Manual beat grid correction: the state behind the grid editor's drag,
//! halve and double buttons, exact tempo field, and tap tempo, which
//! `DESIGN.md` names under "Manual beatgrid correction".
//!
//! An editor works on a copy of one track's grid, and every adjustment
//! takes effect on the track the moment the gesture that made it ends. The
//! window puts the grid on the track through
//! [`Timeline::set_grid`](crate::Timeline::set_grid), which makes one
//! undoable step of consecutive changes, so the whole correction is undone
//! in one go however many taps and nudges went into it. The editor keeps a
//! history of its own beside that step, so that one adjustment can be taken
//! back without the correction going with it, which
//! [`GridEditor::undo`] describes.
//!
//! The editor also keeps a view of the track's own waveform, which the
//! window paints as a strip with the grid's beats drawn over it. The view
//! can be narrowed until one pixel is one frame of audio, and a drag across
//! the strip slides the grid by the frames those pixels cover, which is how
//! a grid is adjusted down to the sample. The view follows the playhead of
//! the audition that plays the track with a metronome on the grid, so the
//! person sees the beat lines pass the kicks as the click is heard.

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, Bpm, Edit, SAMPLE_RATE, Samples, Seconds};
use dermixen_media::Frame;

/// How long a pause between taps may last before the next tap starts a new
/// count.
pub const TAP_GAP: Seconds = Seconds(2.0);

/// How many taps a tempo needs before the editor reports one.
pub const TAPS_FOR_A_TEMPO: usize = 4;

/// The narrowest the view can be: one frame of audio per pixel, at which a
/// drag of one pixel moves the grid by one frame.
pub const MIN_SAMPLES_PER_PX: f64 = 1.0;

/// How far the pointer must travel from a press before the press counts as
/// a drag of the grid rather than a click on the strip. At one frame per
/// pixel a hand that moves one pixel during a click would otherwise slide
/// the grid by a frame.
pub const DRAG_PX: f32 = 2.0;

/// The slowest tempo the editor accepts, in beats per minute, which is
/// [`Bpm::LOWEST`]. No music the app is for is slower, and a grid below it is
/// a mistake rather than a correction. A document holds no slower tempo
/// either, so a grid the editor takes is one a document may hold.
pub const LOWEST_BPM: f64 = Bpm::LOWEST.0;

/// The fastest tempo the editor accepts, in beats per minute, which is
/// [`Bpm::HIGHEST`]. A grid above it puts beats closer together than the
/// metronome's click is long, and its beat lines would crowd every pixel of
/// the strip. A document holds no faster tempo either.
pub const HIGHEST_BPM: f64 = Bpm::HIGHEST.0;

/// The part of the track on the strip.
///
/// Two mappings follow from a view. A whole pixel column `x` of the
/// waveform covers the frames from `start + floor(x * samples_per_px)` up
/// to but not including `start + floor((x + 1) * samples_per_px)`, so that
/// every frame belongs to exactly one column. A point on the strip, at a
/// column that need not be whole, stands for the frame
/// `start + x * samples_per_px` rounded to the nearest frame, with a half
/// rounding away from zero, and a frame is drawn at the column
/// `(frame - start) / samples_per_px`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridView {
    /// The track frame at the left edge.
    pub start: Samples,
    /// How many frames one pixel covers, never below [`MIN_SAMPLES_PER_PX`]
    /// and never more than the track's length divided by the width. A track
    /// shorter than the strip is shown at one frame per pixel rather than
    /// stretched to fill it.
    pub samples_per_px: f64,
    /// The width of the strip in pixels.
    pub width_px: f32,
}

/// One pixel column of the strip's waveform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridColumn {
    /// The column, counted from the left edge of the strip.
    pub x: f32,
    /// The loudest sample among the frames the column covers, in either
    /// channel, as a number from zero to one, a sample louder than one
    /// counting as one. A column past the end of the audio covers no
    /// frames and gives zero, and a sample that is not a finite number
    /// counts for nothing.
    pub peak: f32,
}

/// One beat of the grid, where it is drawn on the strip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridBeat {
    /// The column of the beat's frame.
    pub x: f32,
    /// The beat's index on the grid: zero at beat zero, negative before it.
    pub beat: i64,
    /// Whether the beat starts a bar, which is every beat whose index is a
    /// whole multiple of [`BEATS_PER_BAR`](dermixen_core::BEATS_PER_BAR),
    /// negative multiples included.
    pub downbeat: bool,
}

/// Everything the window draws on the strip for one view.
#[derive(Debug, Clone, PartialEq)]
pub struct GridScene {
    /// One column per pixel of the width, from the left edge, the width
    /// rounded up to a whole number of columns.
    pub columns: Vec<GridColumn>,
    /// The beats whose frame lies at or after the left edge and before the
    /// right edge, from the earliest. The list is empty when the grid's
    /// tempo is not a finite number between [`LOWEST_BPM`] and
    /// [`HIGHEST_BPM`], since a grid the editor's own controls refuse can
    /// still arrive with the track.
    pub beats: Vec<GridBeat>,
    /// The column of the audition's playhead, when one was given and its
    /// frame lies in the view.
    pub playhead: Option<f32>,
}

/// The column where a press on the strip began and the grid at that moment,
/// kept until the press is released so a drag can measure the distance moved
/// from the press itself rather than from the previous position reported.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    /// The column the press began at.
    x: f32,
    /// The grid as it stood when the press began. The drag moves beat zero
    /// out from this grid, and an undo or a redo puts this grid back before
    /// taking an adjustment off the editor's own history.
    grid: BeatGrid,
    /// Whether the pointer has travelled [`DRAG_PX`] from the press, which
    /// makes the press a drag of the grid rather than a click on the strip.
    dragging: bool,
}

/// A copy of one track's grid being corrected by hand.
#[derive(Debug, Clone, PartialEq)]
pub struct GridEditor {
    /// The track's position in the playlist.
    track: usize,
    /// The grid as corrected so far.
    grid: BeatGrid,
    /// The times of the taps in the current count.
    taps: Vec<Seconds>,
    /// The track's length in frames, once given.
    length: Option<Samples>,
    /// The part of the track on the strip.
    view: GridView,
    /// Whether the view has already been set to show the whole track once,
    /// which happens the first time both a length and a positive width are
    /// known.
    framed: bool,
    /// The press in progress, if any.
    press: Option<Press>,
    /// The grid as it stood before each adjustment made since the editor
    /// opened, the earliest first, which is what
    /// [`undo`](GridEditor::undo) walks back through.
    past: Vec<BeatGrid>,
    /// The grid each undo took back, the earliest undo first, which is what
    /// [`redo`](GridEditor::redo) walks forward through. An adjustment
    /// empties it.
    future: Vec<BeatGrid>,
}

impl GridEditor {
    /// An editor over a copy of `grid`, the grid of the track at `track`.
    pub fn new(track: usize, grid: BeatGrid) -> GridEditor {
        GridEditor {
            track,
            grid,
            taps: Vec::new(),
            length: None,
            view: GridView {
                start: Samples::ZERO,
                samples_per_px: MIN_SAMPLES_PER_PX,
                width_px: 0.0,
            },
            framed: false,
            press: None,
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    /// The track's position in the playlist.
    pub fn track(&self) -> usize {
        self.track
    }

    /// The grid as corrected so far.
    pub fn grid(&self) -> BeatGrid {
        self.grid
    }

    /// Slides the grid by `by` frames, later for a positive count and
    /// earlier for a negative one, as a person dragging the grid does. Beat
    /// zero may end up before the first frame of the audio.
    pub fn nudge(&mut self, by: Samples) {
        let before = self.grid;
        self.grid.first_beat += by;
        self.adjusted(before);
    }

    /// Doubles the tempo, for a grid that found half the true tempo. Beat
    /// zero stays where it is. A doubled tempo above [`HIGHEST_BPM`] is
    /// refused and nothing changes.
    pub fn double(&mut self) {
        let doubled = self.grid.bpm.0 * 2.0;
        if tempo_in_range(doubled) {
            let before = self.grid;
            self.grid.bpm = Bpm(doubled);
            self.adjusted(before);
        }
    }

    /// Halves the tempo, for a grid that found twice the true tempo. Beat
    /// zero stays where it is. A halved tempo below [`LOWEST_BPM`] is
    /// refused and nothing changes.
    pub fn halve(&mut self) {
        let halved = self.grid.bpm.0 / 2.0;
        if tempo_in_range(halved) {
            let before = self.grid;
            self.grid.bpm = Bpm(halved);
            self.adjusted(before);
        }
    }

    /// Sets the tempo to a number a person typed and returns true, or
    /// returns false and changes nothing when the number is not a finite
    /// tempo between [`LOWEST_BPM`] and [`HIGHEST_BPM`], both included.
    pub fn set_bpm(&mut self, bpm: Bpm) -> bool {
        if !tempo_in_range(bpm.0) {
            return false;
        }
        let before = self.grid;
        self.grid.bpm = bpm;
        self.adjusted(before);
        true
    }

    /// Records a tap at a time on the clock the taps are counted on, and
    /// returns the tempo once there are enough taps to say one.
    ///
    /// A tap more than [`TAP_GAP`] after the previous tap, or at or before
    /// it, starts a new count with that tap alone. Once the count contains
    /// [`TAPS_FOR_A_TEMPO`] taps or more, the tempo is sixty divided by the
    /// mean interval in seconds between consecutive taps of the count, the
    /// grid's tempo is set to it, and it is returned. With fewer taps
    /// nothing changes and nothing is returned. A tapped tempo outside
    /// [`LOWEST_BPM`] to [`HIGHEST_BPM`] leaves the grid as it was and
    /// returns nothing, though the count goes on. A tap whose time is not a
    /// finite number is ignored: the count and the grid stay as they were
    /// and nothing is returned.
    pub fn tap(&mut self, at: Seconds) -> Option<Bpm> {
        if !at.0.is_finite() {
            return None;
        }
        if let Some(&previous) = self.taps.last()
            && (at.0 <= previous.0 || at.0 - previous.0 > TAP_GAP.0)
        {
            self.taps.clear();
        }
        self.taps.push(at);

        if self.taps.len() < TAPS_FOR_A_TEMPO {
            return None;
        }
        // The span from the first tap to the last, divided by the number of
        // intervals between them, is the mean interval: an interior tap
        // enters that mean only through the two intervals it sits between,
        // which cancel against it when the intervals are summed.
        let span = self.taps.last().unwrap().0 - self.taps[0].0;
        let intervals = (self.taps.len() - 1) as f64;
        let bpm = Bpm(60.0 / (span / intervals));
        if !tempo_in_range(bpm.0) {
            return None;
        }
        let before = self.grid;
        self.grid.bpm = bpm;
        self.adjusted(before);
        Some(bpm)
    }

    /// How many taps the current count contains.
    pub fn taps(&self) -> usize {
        self.taps.len()
    }

    /// The edit that puts the corrected grid on the track.
    pub fn edit(&self) -> Edit {
        Edit::SetGrid {
            track: self.track,
            grid: self.grid,
        }
    }

    /// Tells the editor how long the track is, in frames, which bounds the
    /// view. The view is set to show the whole track from its first frame
    /// the first time both a length and a positive width are known,
    /// however the second of the two arrives. Every length after that only
    /// re-applies the bounds to the view as it stands. A length below zero
    /// counts as zero.
    pub fn set_length(&mut self, length: Samples) {
        let length = Samples(length.0.max(0));
        self.length = Some(length);
        if self.framed {
            self.view = self.bounded(
                self.view.start.0 as f64,
                self.view.samples_per_px,
                self.view.width_px,
            );
        } else if self.view.width_px > 0.0 {
            self.frame_whole_track(length, self.view.width_px);
        }
    }

    /// Tells the editor how wide the strip is, in pixels. The view is set
    /// to show the whole track from its first frame the first time both a
    /// length and a positive width are known, however the second of the
    /// two arrives. Every width after that keeps the view's left edge and
    /// its frames per pixel, within the bounds. A width that is not a
    /// positive finite number is ignored.
    pub fn set_width(&mut self, width_px: f32) {
        if !width_px.is_finite() || width_px <= 0.0 {
            return;
        }
        if let Some(length) = self.length
            && !self.framed
        {
            self.frame_whole_track(length, width_px);
            return;
        }
        self.view = self.bounded(self.view.start.0 as f64, self.view.samples_per_px, width_px);
    }

    /// The part of the track on the strip. Before a length is given, the
    /// view starts at the first frame at one frame per pixel.
    pub fn view(&self) -> GridView {
        self.view
    }

    /// Sets the part of the track on the strip, brought within the bounds:
    /// the frames per pixel is held between [`MIN_SAMPLES_PER_PX`] and the
    /// length divided by the width, or [`MIN_SAMPLES_PER_PX`] when that is
    /// smaller, and the start between the first frame and the last start
    /// at which the view still ends within the track, which is the first
    /// frame when the track is shorter than the strip.
    /// A view whose width is not a positive finite number, or whose frames
    /// per pixel is not a finite number, is ignored.
    pub fn set_view(&mut self, view: GridView) {
        if !view.width_px.is_finite() || view.width_px <= 0.0 || !view.samples_per_px.is_finite() {
            return;
        }
        self.view = self.bounded(view.start.0 as f64, view.samples_per_px, view.width_px);
    }

    /// The column at which a track frame is drawn, which may lie outside
    /// the strip.
    pub fn x_of(&self, frame: Samples) -> f32 {
        (frame.0.saturating_sub(self.view.start.0) as f64 / self.view.samples_per_px) as f32
    }

    /// The track frame drawn at a column, rounded to the nearest frame,
    /// which may lie outside the track. A column that is not a finite
    /// number gives the frame at the left edge.
    pub fn sample_at(&self, x: f32) -> Samples {
        if !x.is_finite() {
            return self.view.start;
        }
        let frame = self.view.start.0 as f64 + f64::from(x) * self.view.samples_per_px;
        Samples(frame.round() as i64)
    }

    /// Narrows the view by `factor` around the frame at column `around_x`,
    /// which stays at that column, or widens it for a factor below one,
    /// within the bounds [`set_view`](GridEditor::set_view) applies. A
    /// factor that is not a positive finite number, or a column that is not
    /// finite, changes nothing.
    pub fn zoom_by(&mut self, factor: f64, around_x: f32) {
        if !factor.is_finite() || factor <= 0.0 || !around_x.is_finite() {
            return;
        }
        let anchor = self.view.start.0 as f64 + f64::from(around_x) * self.view.samples_per_px;
        let samples_per_px = self.view.samples_per_px / factor;
        let start = anchor - f64::from(around_x) * samples_per_px;
        self.view = self.bounded(start, samples_per_px, self.view.width_px);
    }

    /// Moves the view later by `dx_px` pixels' worth of frames, or earlier
    /// for a negative count, within the bounds. A count that is not finite
    /// changes nothing.
    pub fn scroll_by(&mut self, dx_px: f32) {
        if !dx_px.is_finite() {
            return;
        }
        let start = self.view.start.0 as f64 + f64::from(dx_px) * self.view.samples_per_px;
        self.view = self.bounded(start, self.view.samples_per_px, self.view.width_px);
    }

    /// Takes hold of the grid at column `x`, as a press on the strip does.
    /// The grid does not move until the pointer has travelled [`DRAG_PX`]
    /// from the press. A column that is not a finite number is ignored,
    /// leaving the grid as it was.
    pub fn press(&mut self, x: f32) {
        if !x.is_finite() {
            return;
        }
        self.press = Some(Press {
            x,
            grid: self.grid,
            dragging: false,
        });
    }

    /// Drags the grid to column `x`. A press becomes a drag once `x` is
    /// [`DRAG_PX`] or more from the pressed column, in either direction,
    /// and stays a drag until the grid is let go, however close to the press
    /// the pointer comes back. Before that the grid does not move. Once the
    /// press is a drag, beat zero moves from where it was at the press by the
    /// frames between the pressed column and this one, which is the pixel
    /// distance times the frames per pixel, rounded to the nearest frame,
    /// so a drag at one frame per pixel is exact to the frame. The grid
    /// changes as the drag goes, so the window can hand it to the audition
    /// on every move. A move without a press changes nothing. A move to a
    /// column that is not finite changes nothing and does not make the
    /// press a drag.
    pub fn drag_to(&mut self, x: f32) {
        let Some(press) = &mut self.press else {
            return;
        };
        if !x.is_finite() {
            return;
        }
        let moved_px = f64::from(x) - f64::from(press.x);
        if moved_px.abs() >= f64::from(DRAG_PX) {
            press.dragging = true;
        }
        if !press.dragging {
            return;
        }
        let by = moved_px * self.view.samples_per_px;
        self.grid.first_beat = Samples(press.grid.first_beat.0.saturating_add(by.round() as i64));
    }

    /// Lets go of the grid. A press that never became a drag is a click,
    /// and the frame the pressed column stands for comes back, with the
    /// grid left as it was. Once a length is known the frame is held
    /// between the first frame of the track and its last frame, and a
    /// track of no frames gives the first frame. Before a length is known
    /// the frame is held only to at least the first frame. Letting go after
    /// a drag gives nothing and leaves the grid where the last move put it.
    /// Letting go with no press held gives nothing. A drag that left the grid
    /// somewhere other than where the press found it is one adjustment, which
    /// [`undo`](GridEditor::undo) takes back in one go however many moves the
    /// drag took.
    pub fn release(&mut self) -> Option<Samples> {
        let press = self.press.take()?;
        if press.dragging {
            self.adjusted(press.grid);
            return None;
        }
        let frame = self.sample_at(press.x).0;
        let last = match self.length {
            Some(length) => (length.0 - 1).max(0),
            None => i64::MAX,
        };
        Some(Samples(frame.clamp(0, last)))
    }

    /// Replaces the grid as corrected so far, as the window does when the
    /// mix's own undo or redo changes the track's grid while the editor is
    /// open. A press in progress is let go, since the grid it held is gone.
    /// The taps counted so far are kept. The adjustments the editor could
    /// take back with [`undo`](GridEditor::undo) or make again with
    /// [`redo`](GridEditor::redo) are dropped, because every one of them
    /// described a grid the track no longer has, so
    /// [`can_undo`](GridEditor::can_undo) and
    /// [`can_redo`](GridEditor::can_redo) are both false afterwards.
    pub fn set_grid(&mut self, grid: BeatGrid) {
        self.grid = grid;
        self.press = None;
        self.past.clear();
        self.future.clear();
    }

    /// Whether the grid is held, which is true from a press until it is let
    /// go, before the press has travelled [`DRAG_PX`] as well as after.
    pub fn dragging(&self) -> bool {
        self.press.is_some()
    }

    /// Takes back the last adjustment made since the editor opened and
    /// returns true, or returns false and changes nothing when no
    /// adjustment is left to take back. [`can_undo`](GridEditor::can_undo)
    /// says whether one is left without calling this.
    ///
    /// An adjustment is one call of [`nudge`](GridEditor::nudge),
    /// [`double`](GridEditor::double), [`halve`](GridEditor::halve),
    /// [`set_bpm`](GridEditor::set_bpm), or [`tap`](GridEditor::tap) that
    /// left the grid different from before the call, or one drag, from its
    /// press to its [`release`](GridEditor::release), that moved the grid. A
    /// call that leaves the grid as it was, like a nudge of zero frames, a
    /// refused tempo, or a tap that gives no tempo, is no adjustment. The
    /// window calls this for the command key with Z while the editor is
    /// open, and then puts the grid on the track as it does after any other
    /// adjustment, so the mix's own history still keeps the whole correction
    /// as one step. A press in progress is let go when there is an
    /// adjustment to take back, since the grid the press held is gone, and a
    /// call that finds nothing to take back leaves the press where it is. The
    /// taps counted so far are kept. The editor keeps every adjustment since
    /// it opened, however many there are.
    pub fn undo(&mut self) -> bool {
        let Some(grid) = self.past.pop() else {
            return false;
        };
        self.let_go_of_the_press();
        self.future.push(self.grid);
        self.grid = grid;
        true
    }

    /// Makes the last adjustment that [`undo`](GridEditor::undo) took back
    /// again and returns true, or returns false and changes nothing when
    /// there is none. [`can_redo`](GridEditor::can_redo) says whether one
    /// is there without calling this. An adjustment made after an undo drops what could
    /// have been redone. A press in progress is let go when there is an
    /// adjustment to make again, and a call that finds none leaves the press
    /// where it is. The taps counted so far are kept. The window calls this
    /// for the command key with shift and Z while the editor is open.
    pub fn redo(&mut self) -> bool {
        let Some(grid) = self.future.pop() else {
            return false;
        };
        self.let_go_of_the_press();
        self.past.push(self.grid);
        self.grid = grid;
        true
    }

    /// Whether [`undo`](GridEditor::undo) would change the grid: true once
    /// an adjustment has been made since the editor opened and has not been
    /// taken back. Reading this changes nothing, so a caller that has to
    /// know before it calls [`undo`](GridEditor::undo) asks here rather
    /// than calling and reading what comes back.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Whether [`redo`](GridEditor::redo) would change the grid: true once
    /// [`undo`](GridEditor::undo) has taken an adjustment back and no
    /// adjustment has been made since. Reading this changes nothing, like
    /// reading [`can_undo`](GridEditor::can_undo).
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Keeps `before`, the grid as it stood before a gesture, as one
    /// adjustment for [`undo`](GridEditor::undo) to take back, and drops
    /// every adjustment [`redo`](GridEditor::redo) could have made again,
    /// since the gesture that was made is what follows the adjustments left
    /// in the past. A gesture that ended with the grid it began on is no
    /// adjustment, and nothing is kept for such a gesture.
    fn adjusted(&mut self, before: BeatGrid) {
        if self.grid == before {
            return;
        }
        self.past.push(before);
        self.future.clear();
    }

    /// Lets go of a press in progress and puts back the grid the press began
    /// on, which cancels a drag halfway through. [`undo`](GridEditor::undo)
    /// and [`redo`](GridEditor::redo) both start here, because each of the
    /// two leaves the editor on a grid from before the press.
    fn let_go_of_the_press(&mut self) {
        if let Some(press) = self.press.take() {
            self.grid = press.grid;
        }
    }

    /// Keeps a track frame on the strip, as the view does for the
    /// audition's playhead: a frame already in the view changes nothing,
    /// and one outside it moves the view so that the frame sits at the left
    /// edge, within the bounds, which turns the page as the playhead runs
    /// off the right edge.
    pub fn follow(&mut self, frame: Samples) {
        if self.on_screen(self.x_of(frame)) {
            return;
        }
        self.view = self.bounded(frame.0 as f64, self.view.samples_per_px, self.view.width_px);
    }

    /// What to draw for the view over `audio`, the track's frames, with the
    /// audition's playhead at `playhead` if one is running. Every frame the
    /// view covers is read, so the window keeps the scene until the view,
    /// the grid, or the playhead changes rather than asking on every
    /// repaint.
    pub fn scene(&self, audio: &[Frame], playhead: Option<Samples>) -> GridScene {
        let playhead = playhead
            .map(|frame| self.x_of(frame))
            .filter(|&x| self.on_screen(x));
        GridScene {
            columns: self.columns(audio),
            beats: self.beats_in_view(),
            playhead,
        }
    }

    /// Whether a column lies at or after the left edge and before the right
    /// edge.
    fn on_screen(&self, x: f32) -> bool {
        x >= 0.0 && x < self.view.width_px
    }

    /// Sets the view to show the whole track from its first frame, at the
    /// track's length divided by the width, or [`MIN_SAMPLES_PER_PX`] when
    /// that would be narrower, and marks the view as having received its
    /// one-time framing.
    fn frame_whole_track(&mut self, length: Samples, width_px: f32) {
        let samples_per_px = (length.0 as f64 / f64::from(width_px)).max(MIN_SAMPLES_PER_PX);
        self.view = GridView {
            start: Samples::ZERO,
            samples_per_px,
            width_px,
        };
        self.framed = true;
    }

    /// Fits a candidate view against what the track allows: the frames per
    /// pixel held to at least [`MIN_SAMPLES_PER_PX`], and the start held to
    /// at least the first frame. Once the track's length is known, the
    /// frames per pixel is also held to at most the length divided by the
    /// width, and the start to at most the last position at which the view
    /// still ends within the track. Before the length is known, neither the
    /// frames per pixel nor the start has an upper bound.
    fn bounded(&self, start: f64, samples_per_px: f64, width_px: f32) -> GridView {
        let max_samples_per_px = match self.length {
            Some(length) => (length.0 as f64 / f64::from(width_px)).max(MIN_SAMPLES_PER_PX),
            None => f64::INFINITY,
        };
        let samples_per_px = samples_per_px.clamp(MIN_SAMPLES_PER_PX, max_samples_per_px);
        let max_start = match self.length {
            Some(length) => (length.0 as f64 - f64::from(width_px) * samples_per_px).max(0.0),
            None => f64::INFINITY,
        };
        // Rounded to the nearest frame before it is held to the bounds, so
        // that the result never lands past the last valid start the way
        // rounding a value already at that fractional bound could.
        let start = start.round().clamp(0.0, max_start);
        GridView {
            start: Samples(start as i64),
            samples_per_px,
            width_px,
        }
    }

    /// One column per pixel of the view's width, each with the peak among
    /// the frames of `audio` that column covers.
    fn columns(&self, audio: &[Frame]) -> Vec<GridColumn> {
        let count = self.view.width_px.ceil().max(0.0) as usize;
        (0..count)
            .map(|i| {
                let x = i as f64;
                let from = self.view.start.0 + (x * self.view.samples_per_px).floor() as i64;
                let to = self.view.start.0 + ((x + 1.0) * self.view.samples_per_px).floor() as i64;
                GridColumn {
                    x: i as f32,
                    peak: peak_over(audio, from, to),
                }
            })
            .collect()
    }

    /// The grid's beats whose frame lies at or after the left edge and
    /// before the right edge of the view, or none when the grid's tempo is
    /// not a finite number between [`LOWEST_BPM`] and [`HIGHEST_BPM`].
    fn beats_in_view(&self) -> Vec<GridBeat> {
        if !tempo_in_range(self.grid.bpm.0) {
            return Vec::new();
        }
        let start = self.view.start.0;
        let end =
            self.view.start.0 as f64 + f64::from(self.view.width_px) * self.view.samples_per_px;
        let start_time = self.view.start.to_seconds();
        let end_time = Seconds(end / f64::from(SAMPLE_RATE));
        // The beat's own continuous position only estimates which beats
        // might belong in view. Widen the estimate by one beat on each
        // side so that a beat whose continuous position sits just outside
        // the view still gets checked below once its frame is rounded to
        // the nearest sample, which is the same rounding the reported
        // column uses, so the check and the report never disagree.
        let first = self.grid.beat_at(start_time).0.floor() as i64 - 1;
        let last = self.grid.beat_at(end_time).0.ceil() as i64 + 1;
        (first..=last)
            .filter_map(|beat| {
                let frame = self.grid.position_of(Beats(beat as f64));
                // The interval is half open, matching how a column covers
                // its frames: a beat exactly at the right edge belongs to
                // the view one page over, not to this one.
                let in_view = frame.0 >= start && (frame.0 as f64) < end;
                in_view.then(|| GridBeat {
                    x: self.x_of(frame),
                    beat,
                    downbeat: beat.rem_euclid(i64::from(BEATS_PER_BAR)) == 0,
                })
            })
            .collect()
    }
}

/// How far an arrow key moves the grid: one frame on its own, ten frames
/// with shift held, and a hundred frames with shift and the command or the
/// alt key held. `shift` says whether shift is held, and `command` whether
/// the command key (control, on Linux) or the alt key is. The command or
/// alt key without shift moves one frame, since the bigger steps are
/// reached through shift. The count is always positive: the caller
/// subtracts it for the left arrow.
pub fn arrow_step(shift: bool, command: bool) -> Samples {
    match (shift, command) {
        (true, true) => Samples(100),
        (true, false) => Samples(10),
        (false, _) => Samples(1),
    }
}

/// Whether a tempo is a finite number between [`LOWEST_BPM`] and
/// [`HIGHEST_BPM`], both included.
fn tempo_in_range(bpm: f64) -> bool {
    bpm.is_finite() && (LOWEST_BPM..=HIGHEST_BPM).contains(&bpm)
}

/// The loudest sample among the frames of `audio` from `from` up to but not
/// including `to`, in either channel, capped at one; a sample that is not a
/// finite number, or a position outside `audio`, contributes nothing.
fn peak_over(audio: &[Frame], from: i64, to: i64) -> f32 {
    let len = audio.len() as i64;
    let from = from.clamp(0, len) as usize;
    let to = to.clamp(0, len) as usize;
    let mut peak = 0.0f32;
    for frame in &audio[from..to] {
        for sample in frame {
            if sample.is_finite() {
                peak = peak.max(sample.abs().min(1.0));
            }
        }
    }
    peak
}
