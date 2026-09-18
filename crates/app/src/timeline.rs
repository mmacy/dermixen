//! The timeline view-model: the mix laid out for drawing, and the clicks and
//! drags that become edits.
//!
//! The window paints what [`Timeline::scene`] describes and hands every
//! press, drag, and release back to the timeline in pixels. The timeline
//! turns them into edits through a [`History`], so every change is
//! undoable and nothing in the window changes the document any other way.
//! Positions come in and go out as pixels of a [`View`], so the tests place
//! clicks the way a person would and check where marks are drawn, without a
//! window. `DESIGN.md` describes the editing model under "Feature kernel".

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

use dermixen_core::{
    Anchor, BEATS_PER_BAR, BeatGrid, Beats, Bpm, ContentHash, Curve, Decibels, Edit, EditError,
    Envelope, EnvelopeNode, History, Mix, Samples, Seconds, TempoNode, Timeline as Layout, Track,
    apply_edit, owner_of,
};
use dermixen_engine::mix_length;
use dermixen_library::PhraseRecord;
use dermixen_media::Overview;

use crate::corrections::write_correction;

/// How close, in pixels on each axis, a press must be to a node or an
/// anchor to take hold of it, and to a curve's line to add a node on it.
pub const HIT_PX: f32 = 6.0;

/// The narrowest a bar may be drawn before bar lines are left out.
pub const MIN_BAR_PX: f32 = 16.0;

/// The shortest a drag of a lane's bottom edge leaves the lane, and the
/// shortest the window fits a lane to. A lane that tall has room for a
/// track's name above its waveform.
pub const MIN_LANE_PX: f32 = 44.0;

/// The tallest a drag of a lane's bottom edge makes it.
pub const MAX_LANE_PX: f32 = 600.0;

/// How close, in pixels above or below, a press must be to a lane's bottom
/// edge to take hold of the edge and resize the lane.
pub const EDGE_PX: f32 = 4.0;

/// How far the tempo lane's range extends beyond the lowest and highest
/// tempo on the curve.
pub const TEMPO_MARGIN: Bpm = Bpm(2.0);

/// The tempo the tempo lane centers on when the mix has no tracks.
pub const DEFAULT_TEMPO: Bpm = Bpm(120.0);

/// How far the wheel travels, in pixels, to narrow or widen the view by a
/// factor of e (about 2.72) with the command or the alt key held. It is the
/// rate egui itself zooms at for the command key with the wheel, which its
/// `scroll_zoom_speed` input option gives as one part in two hundred per
/// pixel. A larger number makes the wheel travel further for the same
/// change of the view. [`wheel_gesture`] applies it.
pub const ZOOM_WHEEL_PX: f64 = 200.0;

/// How far past the frame the render has reached the master BPM control
/// writes, so that the render, which keeps advancing between the repaint
/// that read the frame and the edit, has not passed the pin by the time the
/// transport is handed the document.
pub const REACH_MARGIN: Samples = Samples(4_410);

/// The shortest the view may be made, in seconds.
const MIN_SPAN: f64 = 1.0;

/// What the wheel over the lanes does, as [`wheel_gesture`] works it out
/// from the wheel's movement and the modifier keys held.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WheelGesture {
    /// No modifier key that zooms is held: the lanes scroll up or down by
    /// `lanes_px`, which the window hands to the scroll area around the
    /// lanes as the wheel gave it, and the view scrolls in time by
    /// `time_px`, which the window hands to
    /// [`Timeline::scroll_by`]. A trackpad's two-finger sideways swipe is
    /// what gives a sideways movement with no key held, so that swipe is
    /// what scrolls the view in time.
    Scroll {
        /// The wheel's vertical movement, in pixels, as the wheel gave it.
        lanes_px: f32,
        /// How far the view moves later, in pixels, which is the wheel's
        /// sideways movement with its sign turned round, since the content
        /// moves the way the fingers go.
        time_px: f32,
    },
    /// The command or the alt key is held without shift: the view narrows
    /// or widens around the pointer by `factor`, which the window hands to
    /// [`Timeline::zoom_by`]. A factor above one narrows the view.
    Zoom {
        /// e raised to the wheel's movement, sideways and vertical added
        /// together, over [`ZOOM_WHEEL_PX`], so a wheel that goes up
        /// narrows the view.
        factor: f64,
    },
    /// The command or the alt key is held with shift: the view scrolls in
    /// time by `time_px`, which the window hands to
    /// [`Timeline::scroll_by`].
    Time {
        /// How far the view moves later, in pixels, which is the wheel's
        /// sideways and vertical movement added together with the sign
        /// turned round, since the content moves the way the wheel goes.
        time_px: f32,
    },
}

/// What the wheel over the lanes does, from how far the wheel moved and
/// which modifier keys were held.
///
/// `dx_px` and `dy_px` are the wheel's sideways and vertical movement in
/// pixels, in the direction egui reports them, which is the direction the
/// content moves: a wheel that goes up and fingers that swipe right count
/// as positive. `zoom_key` is whether the command key or the alt key is
/// held, and `shift` whether shift is. The window reads the wheel's events
/// itself and calls this for each one, so that the alt key and shift are
/// read as the timeline means them rather than as egui's own zoom and
/// sideways scroll. The rules are the ones `docs/window.md` states under
/// "The mouse". With no zoom key the lanes scroll, and a sideways movement
/// scrolls the view in time. A zoom key without shift zooms around the
/// pointer. A zoom key with shift scrolls the view in time. A movement
/// that is not a finite number counts as zero, so a wheel that reports
/// nothing usable scrolls nothing and zooms by a factor of one. Pinching
/// two fingers is not a wheel and is handed to [`Timeline::zoom_by`]
/// directly.
pub fn wheel_gesture(dx_px: f32, dy_px: f32, zoom_key: bool, shift: bool) -> WheelGesture {
    // A movement that is not a finite number counts as no movement at all.
    // `Timeline::scroll_by` and `Timeline::zoom_by` refuse such a value
    // themselves, but the scroll area the window hands `lanes_px` to takes
    // whatever it is given, so the guard belongs here.
    let dx = if dx_px.is_finite() { dx_px } else { 0.0 };
    let dy = if dy_px.is_finite() { dy_px } else { 0.0 };
    match (zoom_key, shift) {
        (false, _) => WheelGesture::Scroll {
            lanes_px: dy,
            time_px: -dx,
        },
        (true, false) => WheelGesture::Zoom {
            factor: (f64::from(dx + dy) / ZOOM_WHEEL_PX).exp(),
        },
        (true, true) => WheelGesture::Time {
            time_px: -(dx + dy),
        },
    }
}

/// How far past the end of the mix the view may reach when it is widened as
/// far as it goes.
const SPAN_BEYOND_THE_MIX: f64 = 60.0;

/// The part of the mix on screen and the size it is drawn at.
///
/// Time runs left to right across `width_px` pixels from `from` to `to`, so
/// a pixel column `x` shows the mix from `from + x * (to - from) / width_px`
/// to the same for `x + 1`. Mix time here is the clock `mix show` and the
/// rendered file use: zero is the first frame the render writes, which is
/// mix time as the tempo curve counts it less the mix time the earliest
/// track starts at, which
/// [`Timeline::start`](dermixen_core::Timeline::start) gives. Track lanes
/// are stacked from the top in playlist order, and the tempo lane follows
/// them. Each lane is `lane_height_px` tall unless
/// [`set_lane_height`](Timeline::set_lane_height) or a drag of its bottom
/// edge has given it a height of its own. A row `y` belongs to the lane
/// whose top is at or before it and whose bottom is after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The width of the timeline in pixels.
    pub width_px: f32,
    /// The height in pixels of every lane that has not been given a height
    /// of its own.
    pub lane_height_px: f32,
    /// The mix time at the left edge.
    pub from: Seconds,
    /// The mix time at the right edge, after `from`.
    pub to: Seconds,
}

/// What the person has selected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selection {
    /// Nothing.
    Nothing,
    /// A track, by its position in the playlist.
    Track(usize),
    /// A node of one of a track's curves, by the beat it sits at.
    Node {
        /// The track's position in the playlist.
        track: usize,
        /// The curve.
        curve: Curve,
        /// The node's beat.
        at: Beats,
    },
    /// One of a track's anchors.
    Anchor {
        /// The track's position in the playlist.
        track: usize,
        /// Which anchor.
        anchor: Anchor,
    },
    /// A tempo node, by the track it belongs to and its beat of that track.
    TempoNode {
        /// The track's position in the playlist.
        track: usize,
        /// The node's beat of the track.
        at: Beats,
    },
}

/// A point in pixels, with `y` growing downward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Pixels from the left edge of the timeline.
    pub x: f32,
    /// Pixels from the top of the timeline.
    pub y: f32,
}

/// One pixel column of a waveform: the bar to draw from `top` to `bottom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveColumn {
    /// The column.
    pub x: f32,
    /// The top of the column: the lane's middle, moved up by the product of
    /// the peak's `high` and half the lane's height.
    pub top: f32,
    /// The bottom of the column: the lane's middle, moved up by the product
    /// of the peak's `low` and half the lane's height, which moves it down
    /// when `low` is negative.
    pub bottom: f32,
}

/// A node of the selected curve, where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeMark {
    /// Where to draw it.
    pub at_px: Point,
    /// The node's beat of its track.
    pub at: Beats,
    /// The node's level.
    pub level: Decibels,
    /// Whether it is selected.
    pub selected: bool,
}

/// An anchor, where it is drawn: a vertical line across the lane at `x`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorMark {
    /// The column.
    pub x: f32,
    /// Which anchor.
    pub anchor: Anchor,
    /// The anchor's beat of its track.
    pub at: Beats,
    /// Whether it is selected.
    pub selected: bool,
}

/// A phrase start, where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhraseMark {
    /// The column.
    pub x: f32,
    /// The longest phrase that starts there, in bars.
    pub bars: u32,
}

/// One track's lane: everything to draw for it within the view.
///
/// Every `x` here is a pixel column of the view, and only marks whose
/// column lies from zero to the view's width inclusive are included. The
/// track's own `start_x` and `end_x` may lie outside the view.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// The track's position in the playlist.
    pub track: usize,
    /// The track's file name without its extension.
    pub name: String,
    /// The top of the lane in pixels.
    pub top: f32,
    /// The height of the lane in pixels.
    pub height: f32,
    /// The column at which the track's audio starts.
    pub start_x: f32,
    /// The column at which the track's audio ends.
    pub end_x: f32,
    /// The track's original tempo.
    pub bpm: Bpm,
    /// Whether keylock is on.
    pub keylock: bool,
    /// The track's gain, which volume leveling wrote when the track joined
    /// the mix.
    pub gain: Decibels,
    /// One column per whole pixel column from the later of zero and
    /// `start_x` to the earlier of the view's width and `end_x`, both
    /// inclusive, from the track's overview. The list is empty when no
    /// overview has been given for the track.
    pub waveform: Vec<WaveColumn>,
    /// The selected curve as a line: one point per whole pixel column across
    /// the same columns, at the level the curve gives at the mix time of
    /// that column.
    pub curve: Vec<Point>,
    /// The selected curve's nodes within the view, in beat order.
    pub nodes: Vec<NodeMark>,
    /// The anchors within the view.
    pub anchors: Vec<AnchorMark>,
    /// The columns of the bar lines within the view, or none when a bar is
    /// narrower than [`MIN_BAR_PX`]. Bars start at the downbeat the track's
    /// phrase record gives, or at beat zero when it has none.
    pub bars: Vec<f32>,
    /// The phrase starts within the view.
    pub phrases: Vec<PhraseMark>,
    /// The columns of the section changes within the view.
    pub sections: Vec<f32>,
}

/// A tempo node, where it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoMark {
    /// Where to draw it.
    pub at_px: Point,
    /// The track the node belongs to.
    pub track: usize,
    /// The node's beat of that track.
    pub at: Beats,
    /// The node's tempo.
    pub bpm: Bpm,
    /// Whether it is selected.
    pub selected: bool,
}

/// The tempo curve's lane, beneath the track lanes.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoLane {
    /// The top of the lane in pixels.
    pub top: f32,
    /// The height of the lane in pixels.
    pub height: f32,
    /// The tempo at the bottom of the lane.
    pub low: Bpm,
    /// The tempo at the top of the lane.
    pub high: Bpm,
    /// The curve as a line: one point per whole pixel column of the view
    /// from zero to the width, at the mix tempo at that column's time.
    pub curve: Vec<Point>,
    /// The tempo nodes within the view, in mix order. The node the layout
    /// puts at mix beat zero to start the curve is not among them, since it
    /// belongs to no track.
    pub nodes: Vec<TempoMark>,
}

/// Everything to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// One lane per track, in playlist order.
    pub lanes: Vec<Lane>,
    /// The tempo lane.
    pub tempo: TempoLane,
    /// The playhead's column, when it lies from zero to the view's width
    /// inclusive.
    pub playhead_x: Option<f32>,
    /// The length of the mix as [`mix_length`](dermixen_engine::mix_length)
    /// gives it, on the same clock as the view.
    pub length: Seconds,
}

/// What a press took hold of, and where a drag would put it.
///
/// The beat and level a press found are kept beside the ones a drag has
/// moved them to, so that a release commits nothing when the two are still
/// the same, and so that a refused edit can put the selection back where it
/// was.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grip {
    /// A node of one of a track's curves.
    Node {
        /// The track's position in the playlist.
        track: usize,
        /// The curve the node belongs to.
        curve: Curve,
        /// The node as the press found it.
        from: EnvelopeNode,
        /// The node as a drag has moved it.
        to: EnvelopeNode,
    },
    /// One of a track's anchors.
    Anchor {
        /// The track's position in the playlist.
        track: usize,
        /// Which anchor.
        anchor: Anchor,
        /// The beat the press found it on.
        from: Beats,
        /// The beat a drag has moved it to.
        to: Beats,
    },
    /// A tempo node of a track.
    Tempo {
        /// The track's position in the playlist.
        track: usize,
        /// The node as the press found it.
        from: TempoNode,
        /// The node as a drag has moved it.
        to: TempoNode,
    },
}

impl Grip {
    /// The edit that commits the move, or nothing when the drag has not
    /// moved what the press took hold of.
    fn edit(self) -> Option<Edit> {
        match self {
            Grip::Node {
                track,
                curve,
                from,
                to,
            } => (to.at.0 != from.at.0 || to.value.0 != from.value.0).then_some(Edit::MoveNode {
                track,
                curve,
                from: from.at,
                to,
            }),
            Grip::Anchor {
                track,
                anchor,
                from,
                to,
            } => (to.0 != from.0).then_some(Edit::MoveAnchor { track, anchor, to }),
            Grip::Tempo { track, from, to } => (to.at.0 != from.at.0 || to.bpm.0 != from.bpm.0)
                .then_some(Edit::MoveTempoNode {
                    track,
                    from: from.at,
                    to,
                }),
        }
    }

    /// What the selection names while the press is held.
    fn selection(self) -> Selection {
        match self {
            Grip::Node {
                track, curve, to, ..
            } => Selection::Node {
                track,
                curve,
                at: to.at,
            },
            Grip::Anchor { track, anchor, .. } => Selection::Anchor { track, anchor },
            Grip::Tempo { track, to, .. } => Selection::TempoNode { track, at: to.at },
        }
    }

    /// What the selection named before any drag moved it, which is what a
    /// refused move goes back to.
    fn selection_before_the_drag(self) -> Selection {
        match self {
            Grip::Node {
                track, curve, from, ..
            } => Selection::Node {
                track,
                curve,
                at: from.at,
            },
            Grip::Anchor { track, anchor, .. } => Selection::Anchor { track, anchor },
            Grip::Tempo { track, from, .. } => Selection::TempoNode { track, at: from.at },
        }
    }
}

/// A press being held: what it took hold of, and the tempo lane's range as
/// it stood when the press began.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    /// What the press took hold of.
    grip: Grip,
    /// The tempo at the bottom of the tempo lane.
    low: Bpm,
    /// The tempo at the top of the tempo lane.
    high: Bpm,
}

/// What a press at a point would do, worked out without changing anything.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Outcome {
    /// Nothing lies under the point: it is outside every lane, or it is in
    /// the tempo lane away from the curve.
    Nothing,
    /// Take hold of a node that is already on the selected curve.
    Node {
        /// The track's position in the playlist.
        track: usize,
        /// The node's beat of the track.
        at: Beats,
    },
    /// Take hold of an anchor.
    Anchor {
        /// The track's position in the playlist.
        track: usize,
        /// Which anchor.
        anchor: Anchor,
    },
    /// Take hold of a tempo node that is already on the curve.
    Tempo {
        /// The track's position in the playlist.
        track: usize,
        /// The node's beat of the track.
        at: Beats,
    },
    /// Add a node to a track's selected curve and take hold of it.
    AddNode {
        /// The track's position in the playlist.
        track: usize,
        /// The node to add.
        node: EnvelopeNode,
    },
    /// Add a tempo node to a track and take hold of it.
    AddTempo {
        /// The track's position in the playlist.
        track: usize,
        /// The node to add.
        node: TempoNode,
    },
    /// Select a track.
    Track(usize),
}

/// The mix laid out for drawing under one view.
///
/// A frame holds the tempo curve and the placement of every track, together
/// with the mix time the render's first frame falls at, and turns beats,
/// times, and pixel columns into one another. One is built from scratch
/// whenever the timeline needs to lay the mix out, so nothing has to be kept
/// in step with the document.
struct Frame<'a> {
    /// The part of the mix on screen.
    view: View,
    /// The document the layout was built from.
    mix: &'a Mix,
    /// The tempo curve of the mix and the placement of every track.
    layout: Layout,
    /// The mix time the render's first frame falls at, which the view's
    /// clock counts from.
    start: Seconds,
    /// The height in pixels of every lane, from the top, with the tempo
    /// lane last, which [`Timeline::heights_of`] works out.
    heights: Vec<f64>,
}

impl<'a> Frame<'a> {
    /// The mix laid out under a view at the heights of its lanes, or
    /// nothing for a mix with no tracks.
    fn of(view: View, mix: &'a Mix, heights: Vec<f64>) -> Option<Frame<'a>> {
        let layout = mix.timeline()?;
        let start = layout.start();
        Some(Frame {
            view,
            mix,
            layout,
            start,
            heights,
        })
    }

    /// The width of the view in pixels.
    fn width(&self) -> f64 {
        f64::from(self.view.width_px)
    }

    /// The height of one lane in pixels. Lanes are numbered from the top: a
    /// track's lane by the track's position in the playlist, and the tempo
    /// lane by the number of tracks.
    fn lane_height(&self, lane: usize) -> f64 {
        self.heights
            .get(lane)
            .copied()
            .unwrap_or_else(|| f64::from(self.view.lane_height_px))
    }

    /// The lane on the other side of an edge that lies within [`EDGE_PX`]
    /// of a row in `lane`, which is the lane below when the row is near
    /// that lane's bottom edge and the lane above when it is near its top.
    /// Nothing when no edge is that close, and nothing past the tempo lane
    /// or above the first lane, neither of which has a lane beyond it.
    fn lane_across_a_near_edge(&self, lane: usize, y: f64) -> Option<usize> {
        let reach = f64::from(EDGE_PX);
        let top = self.lane_top(lane);
        if (y - (top + self.lane_height(lane))).abs() <= reach && lane < self.mix.tracks.len() {
            return Some(lane + 1);
        }
        if (y - top).abs() <= reach && lane > 0 {
            return Some(lane - 1);
        }
        None
    }

    /// The lane a row falls in, which is the lane whose top is at or before
    /// the row and whose bottom is after it. A row above the first lane or
    /// at or below the bottom of the tempo lane falls in no lane.
    fn lane_at(&self, y: f64) -> Option<usize> {
        if !y.is_finite() || y < 0.0 {
            return None;
        }
        let mut bottom = 0.0;
        for lane in 0..=self.mix.tracks.len() {
            bottom += self.lane_height(lane);
            if y < bottom {
                return Some(lane);
            }
        }
        None
    }

    /// The column a mix time is drawn at.
    fn column_of(&self, time: Seconds) -> f64 {
        (time.0 - self.view.from.0) * self.width() / (self.view.to.0 - self.view.from.0)
    }

    /// The mix time drawn at a column.
    fn time_at(&self, x: f64) -> Seconds {
        Seconds(self.view.from.0 + x * (self.view.to.0 - self.view.from.0) / self.width())
    }

    /// The column a mix beat is drawn at.
    fn column_of_mix_beat(&self, beat: Beats) -> f64 {
        self.column_of(self.layout.curve.time_at(beat) - self.start)
    }

    /// The mix beat drawn at a column.
    fn mix_beat_at(&self, x: f64) -> Beats {
        self.layout.curve.beat_at(self.time_at(x) + self.start)
    }

    /// The column one of a track's own beats is drawn at.
    fn column_of_track_beat(&self, track: usize, beat: Beats) -> f64 {
        self.column_of_mix_beat(self.layout.tracks[track].origin + beat)
    }

    /// The track's own beat at a column. Track beats and mix beats advance
    /// together, so this is the mix beat less the beat the track begins at.
    fn track_beat_at(&self, track: usize, x: f64) -> Beats {
        self.mix_beat_at(x) - self.layout.tracks[track].origin
    }

    /// The columns at which a track's audio starts and ends, either of which
    /// may lie outside the view.
    fn track_span(&self, track: usize) -> (f64, f64) {
        let placed = &self.layout.tracks[track];
        (
            self.column_of(placed.start(&self.layout.curve) - self.start),
            self.column_of(placed.end(&self.layout.curve) - self.start),
        )
    }

    /// The first and last whole pixel column of a track's audio that lie
    /// within the view. The first is after the last when no column does.
    fn columns_of(&self, track: usize) -> (i64, i64) {
        let (start, end) = self.track_span(track);
        let first = start.max(0.0).ceil();
        let last = end.min(self.width()).floor();
        (first as i64, last as i64)
    }

    /// Whether a column lies from zero to the view's width inclusive, which
    /// is what puts a mark on screen.
    fn on_screen(&self, x: f64) -> bool {
        (0.0..=self.width()).contains(&x)
    }

    /// The top of a lane in pixels, which is the height of every lane above
    /// it. Lanes are numbered as for
    /// [`lane_height`](Frame::lane_height).
    fn lane_top(&self, lane: usize) -> f64 {
        // The sum starts at zero rather than at the identity `sum` folds
        // from, which is negative zero, so the first lane's top is plain
        // zero.
        (0..lane).fold(0.0, |top, above| top + self.lane_height(above))
    }
}

/// The row a level is drawn at in a lane whose top is `top`: the top of the
/// lane is unity and the bottom is silence, and the level sits above the
/// bottom by the lane's height times its amplitude, clamped to the lane.
fn level_row(top: f64, height: f64, level: Decibels) -> f64 {
    let amplitude = level.to_linear().clamp(0.0, 1.0);
    top + height * (1.0 - amplitude)
}

/// The level a row gives in a lane whose top is `top`, which is how a drag
/// reads a level back. A row at or below the bottom of the lane gives
/// [`Decibels::SILENCE`] and one at or above the top gives unity.
fn row_level(top: f64, height: f64, y: f64) -> Decibels {
    let amplitude = ((top + height - y) / height).clamp(0.0, 1.0);
    Decibels::from_linear(amplitude)
}

/// The row a tempo is drawn at in the tempo lane, which runs in a straight
/// line from `low` at the bottom to `high` at the top.
fn tempo_row(top: f64, height: f64, low: Bpm, high: Bpm, bpm: Bpm) -> f64 {
    top + height * (high.0 - bpm.0) / (high.0 - low.0)
}

/// The tempo a row gives in the tempo lane, which is how a drag reads a
/// tempo back.
fn row_tempo(top: f64, height: f64, low: Bpm, high: Bpm, y: f64) -> Bpm {
    Bpm(high.0 - (y - top) / height * (high.0 - low.0))
}

/// Whether a press with this outcome takes hold of a node, an anchor, or a
/// tempo node that is already there, which is what comes before a lane's
/// bottom edge when a press falls near one.
fn takes_hold(outcome: Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Node { .. } | Outcome::Anchor { .. } | Outcome::Tempo { .. }
    )
}

/// One of a track's four envelopes, by the curve that names it.
fn envelope_of(track: &Track, curve: Curve) -> &Envelope {
    match curve {
        Curve::Volume => &track.volume,
        Curve::Low => &track.eq.low,
        Curve::Mid => &track.eq.mid,
        Curve::High => &track.eq.high,
    }
}

/// The lowest and highest tempo the tempo lane spans for a document: the
/// extremes of every node on the mix's tempo curve, the node that starts the
/// curve included, widened by [`TEMPO_MARGIN`] at each end. A mix with no
/// tracks centers on [`DEFAULT_TEMPO`].
fn tempo_range_of(mix: &Mix) -> (Bpm, Bpm) {
    let Some(layout) = mix.timeline() else {
        return (
            Bpm(DEFAULT_TEMPO.0 - TEMPO_MARGIN.0),
            Bpm(DEFAULT_TEMPO.0 + TEMPO_MARGIN.0),
        );
    };
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for node in layout.curve.nodes() {
        low = low.min(node.bpm.0);
        high = high.max(node.bpm.0);
    }
    (Bpm(low - TEMPO_MARGIN.0), Bpm(high + TEMPO_MARGIN.0))
}

/// The track a correction is written for when an edit is applied, which is
/// the track whose anchors or grid the edit changed.
fn correction_for(edit: &Edit) -> Option<usize> {
    match edit {
        Edit::MoveAnchor { track, .. } | Edit::SetGrid { track, .. } => Some(*track),
        _ => None,
    }
}

/// The mix on the timeline, with everything the window needs to draw it and
/// everything a person does to it.
///
/// Levels are drawn by amplitude: the top of a lane is unity and its bottom
/// is silence, and a level is drawn above the bottom by the lane's height
/// times its amplitude, which is ten to the power of its decibels over
/// twenty, clamped to the lane. A drag reads the level back the same way,
/// with the bottom of the lane giving
/// [`Decibels::SILENCE`](dermixen_core::Decibels::SILENCE). A level above
/// unity is a boost, and the clamp draws every boost at the top of the lane,
/// so dragging such a node sideways reads unity from the row and sets the
/// node to unity. Nothing in the app writes a boost and no transition preset
/// holds one, so a person meets that only on a document edited by hand.
///
/// Tempos in the tempo lane run in a straight line from `low` at the bottom
/// to `high` at the top, where `low` and `high` are the lowest and highest
/// tempo of any node on the committed curve, the starting node included,
/// less and plus [`TEMPO_MARGIN`]; they are computed when a press begins and
/// kept for as long as the press is held, and a mix with no tracks centers
/// on [`DEFAULT_TEMPO`]. A node dragged along a track snaps to the nearest
/// whole beat of the track, an anchor to the nearest whole beat, and a tempo
/// node to the nearest whole mix beat. A tempo node stays on the track it
/// belongs to however far it is dragged, because the edit that moves it
/// names that track. [`owner_of`](dermixen_core::owner_of) names the track a
/// tempo node is added to, and the press settles that once, when it adds the
/// node. A press may be dragged to a point outside the view, and an anchor
/// may be dragged past the end of its track.
///
/// A drag is previewed: while a press is held, [`scene`](Timeline::scene)
/// describes the document with the held node or anchor moved to where it
/// would land, laid out whole, so the tracks after a dragged anchor slide
/// with it. [`mix`](Timeline::mix) still gives the committed document. The
/// edit is committed on release, as one undoable step, and only if the
/// position changed. After a committed move the selection names the node
/// or anchor at its new beat. A press that adds a node commits the addition
/// at once, so a press that adds and then drags is two undoable steps. An
/// edit the history refuses leaves the document and the selection as they
/// were. A correction is written only for an anchor move and a grid change,
/// after the edit has been applied, from the document as it then stands.
pub struct Timeline {
    /// The document and its history.
    history: History,
    /// The part of the mix on screen.
    view: View,
    /// The curve the lanes show and the presses edit.
    curve: Curve,
    /// What is selected.
    selection: Selection,
    /// The playhead, as an output frame.
    playhead: Samples,
    /// How far the transport's render has reached, as an output frame, once
    /// the window has given a frame. The master BPM control writes from
    /// there rather than from the playhead, so its change lands past the
    /// audio the render has already made.
    reach: Option<Samples>,
    /// The overviews given so far, by content hash.
    overviews: HashMap<ContentHash, Overview>,
    /// The phrase records given so far, by content hash.
    phrases: HashMap<ContentHash, PhraseRecord>,
    /// Where corrections are written, if anywhere.
    corrections: Option<PathBuf>,
    /// The height a person gave a track's lane, by the track's content
    /// hash, for every track whose lane has one. Keying by the hash is what
    /// makes a height follow its track through a move, a removal, and an
    /// undo.
    heights: HashMap<ContentHash, f32>,
    /// The height a person gave the tempo lane, if they gave it one. The
    /// tempo lane belongs to no track, so its height is kept on its own.
    tempo_height: Option<f32>,
    /// The lane whose bottom edge a press took hold of, while one is held.
    edge: Option<usize>,
    /// The press being held, if one is.
    press: Option<Press>,
    /// The request to move the transport that a click on the ruler left.
    seek: Option<Samples>,
    /// Whether the document has changed since the window last asked.
    changed: bool,
    /// The message of the last correction that could not be written.
    failure: Option<String>,
    /// The track of the grid editor's correction in progress, while one is.
    /// Another [`set_grid`](Timeline::set_grid) for that track replaces the
    /// change the correction has already made rather than stacking a second
    /// step on the history.
    correction_track: Option<usize>,
}

impl Timeline {
    /// A timeline over `mix` with a view of the first minute at a thousand
    /// pixels wide and lanes a hundred pixels tall, showing the volume
    /// curve, with nothing selected and the playhead at the start.
    pub fn new(mix: Mix) -> Timeline {
        Timeline {
            history: History::new(mix),
            view: View {
                width_px: 1000.0,
                lane_height_px: 100.0,
                from: Seconds::ZERO,
                to: Seconds(60.0),
            },
            curve: Curve::Volume,
            selection: Selection::Nothing,
            playhead: Samples::ZERO,
            reach: None,
            overviews: HashMap::new(),
            phrases: HashMap::new(),
            corrections: None,
            heights: HashMap::new(),
            tempo_height: None,
            edge: None,
            press: None,
            seek: None,
            changed: false,
            failure: None,
            correction_track: None,
        }
    }

    /// The committed document.
    pub fn mix(&self) -> &Mix {
        self.history.mix()
    }

    /// The part of the mix on screen.
    pub fn view(&self) -> View {
        self.view
    }

    /// Sets the part of the mix on screen. A `to` at or before `from`, a
    /// width or lane height at or below zero, or any value that is not a
    /// finite number, is ignored.
    pub fn set_view(&mut self, view: View) {
        // Every value has to be a number for the view to be usable at all,
        // since one that is not would poison every mapping through it.
        let usable = view.from.0.is_finite()
            && view.to.0.is_finite()
            && view.to.0 > view.from.0
            && view.width_px.is_finite()
            && view.width_px > 0.0
            && view.lane_height_px.is_finite()
            && view.lane_height_px > 0.0;
        if !usable {
            return;
        }
        self.view = view;
    }

    /// Gives one lane a height of its own, in pixels, clamped to
    /// [`MIN_LANE_PX`] to [`MAX_LANE_PX`]. Lanes are numbered from the top:
    /// a track's lane by the track's position in the playlist, and the
    /// tempo lane by the number of tracks. A track's height follows the
    /// track through a move, a removal, and an undo, since it is kept by
    /// the track's content hash. A lane past the tempo lane or a height that
    /// is not a finite number is ignored.
    pub fn set_lane_height(&mut self, lane: usize, height_px: f32) {
        if !height_px.is_finite() {
            return;
        }
        let height = height_px.clamp(MIN_LANE_PX, MAX_LANE_PX);
        let tracks = self.history.mix().tracks.len();
        if lane < tracks {
            let hash = self.history.mix().tracks[lane].hash;
            self.heights.insert(hash, height);
        } else if lane == tracks {
            self.tempo_height = Some(height);
        }
    }

    /// The height of a lane in pixels: the one it was given, else the
    /// view's `lane_height_px`. Lanes are numbered as for
    /// [`set_lane_height`](Timeline::set_lane_height).
    pub fn lane_height(&self, lane: usize) -> f32 {
        let tracks = &self.history.mix().tracks;
        let own = match tracks.get(lane) {
            Some(track) => self.heights.get(&track.hash).copied(),
            None if lane == tracks.len() => self.tempo_height,
            None => None,
        };
        own.unwrap_or(self.view.lane_height_px)
    }

    /// The height of every lane together, in pixels, which is the height
    /// the window allocates for the lanes.
    pub fn lanes_height(&self) -> f32 {
        (0..=self.history.mix().tracks.len())
            .map(|lane| self.lane_height(lane))
            .sum()
    }

    /// The lane whose bottom edge lies within [`EDGE_PX`] of the point,
    /// above or below, whatever else lies there. The window uses it to
    /// shape the cursor. A mix with no tracks has no edge.
    pub fn edge_at(&self, at: Point) -> Option<usize> {
        if !at.y.is_finite() {
            return None;
        }
        let tracks = self.history.mix().tracks.len();
        if tracks == 0 {
            return None;
        }
        let mut bottom = 0.0;
        for lane in 0..=tracks {
            bottom += self.lane_height(lane);
            if (at.y - bottom).abs() <= EDGE_PX {
                return Some(lane);
            }
        }
        None
    }

    /// Takes a double-click at the point: when the point lies within
    /// [`EDGE_PX`] of a lane's bottom edge, as [`edge_at`](Timeline::edge_at)
    /// finds it, the lane becomes [`MAX_LANE_PX`] tall if it is shorter
    /// than that, and [`MIN_LANE_PX`] tall if it is already at
    /// [`MAX_LANE_PX`], and the lanes below move with it. Returns whether a
    /// lane's height changed. A point on no edge changes nothing. A node,
    /// an anchor, or a tempo node within [`HIT_PX`] of the point comes
    /// before the edge, as it does for [`press`](Timeline::press), so a
    /// double-click on a node at silence, which is drawn on its lane's
    /// bottom row, changes no height. The lane keeps the height as
    /// [`set_lane_height`](Timeline::set_lane_height) keeps one, so a
    /// track's lane keeps it through a move, a removal, and an undo. This is
    /// no edit: the document, the history, the selection, and a press being
    /// held are left as they are. The window calls this when egui reports a
    /// double-click on the lanes.
    pub fn double_click(&mut self, at: Point) -> bool {
        let (low, high) = tempo_range_of(self.history.mix());
        let outcome = {
            let mix = self.history.mix();
            match Frame::of(self.view, mix, self.heights_of(mix)) {
                Some(frame) => self.outcome_of(&frame, at, low, high),
                None => Outcome::Nothing,
            }
        };
        let Some(lane) = self.edge_press(outcome, at) else {
            return false;
        };
        let before = self.lane_height(lane);
        // A lane the double-click finds at the maximum goes to the minimum,
        // so a second double-click on the same edge puts a lane the first
        // one made tall back out of the way.
        let height = if before >= MAX_LANE_PX {
            MIN_LANE_PX
        } else {
            MAX_LANE_PX
        };
        self.set_lane_height(lane, height);
        self.lane_height(lane) != before
    }

    /// The lane whose bottom edge a press at the point would take hold of,
    /// given what the press found under it: the lane
    /// [`edge_at`](Timeline::edge_at) names, unless the press found a node,
    /// an anchor, or a tempo node, which come first.
    fn edge_press(&self, outcome: Outcome, at: Point) -> Option<usize> {
        if takes_hold(outcome) {
            return None;
        }
        self.edge_at(at)
    }

    /// The top of a lane in pixels, which is the height of every lane above
    /// it. Lanes are numbered as for
    /// [`set_lane_height`](Timeline::set_lane_height).
    fn lane_top(&self, lane: usize) -> f32 {
        (0..lane).fold(0.0, |top, above| top + self.lane_height(above))
    }

    /// The height in pixels of every lane of a document, from the top, with
    /// the tempo lane last. A [`Frame`] is laid out from this, so every
    /// lane is drawn and pressed at the height it has.
    fn heights_of(&self, mix: &Mix) -> Vec<f64> {
        mix.tracks
            .iter()
            .map(|track| self.heights.get(&track.hash).copied())
            .chain(std::iter::once(self.tempo_height))
            .map(|height| f64::from(height.unwrap_or(self.view.lane_height_px)))
            .collect()
    }

    /// The column at which a mix time is drawn, which may lie outside the
    /// view.
    pub fn x_of(&self, time: Seconds) -> f32 {
        let span = self.view.to.0 - self.view.from.0;
        ((time.0 - self.view.from.0) * f64::from(self.view.width_px) / span) as f32
    }

    /// The mix time drawn at a column, which may lie outside the view.
    pub fn time_at(&self, x: f32) -> Seconds {
        let span = self.view.to.0 - self.view.from.0;
        Seconds(self.view.from.0 + f64::from(x) * span / f64::from(self.view.width_px))
    }

    /// Narrows the view by `factor` around the time at column `around_x`,
    /// which stays at that column, or widens it for a factor below one. The
    /// view is never narrower than one second nor wider than the mix's
    /// length plus a minute, and never starts before the mix.
    pub fn zoom_by(&mut self, factor: f64, around_x: f32) {
        if !factor.is_finite() || factor <= 0.0 || !around_x.is_finite() {
            return;
        }
        let anchor = self.time_at(around_x).0;
        let fraction = f64::from(around_x) / f64::from(self.view.width_px);
        let widest =
            (mix_length(self.history.mix()).to_seconds().0 + SPAN_BEYOND_THE_MIX).max(MIN_SPAN);
        let span = ((self.view.to.0 - self.view.from.0) / factor).clamp(MIN_SPAN, widest);
        self.place_view(anchor - fraction * span, span);
    }

    /// Moves the view later by `dx_px` pixels' worth of time, or earlier for
    /// a negative count, never starting before the mix.
    pub fn scroll_by(&mut self, dx_px: f32) {
        if !dx_px.is_finite() {
            return;
        }
        let span = self.view.to.0 - self.view.from.0;
        let by = f64::from(dx_px) * span / f64::from(self.view.width_px);
        self.place_view(self.view.from.0 + by, span);
    }

    /// Puts the view at a start and a span, never starting before the mix.
    fn place_view(&mut self, from: f64, span: f64) {
        if !from.is_finite() || !span.is_finite() {
            return;
        }
        let from = from.max(0.0);
        self.view.from = Seconds(from);
        self.view.to = Seconds(from + span);
    }

    /// Gives the timeline a track's overview, keyed by the track's content
    /// hash so it follows the track through reordering.
    pub fn set_overview(&mut self, hash: ContentHash, overview: Overview) {
        self.overviews.insert(hash, overview);
    }

    /// Gives the timeline a track's phrase record, keyed by content hash.
    pub fn set_phrases(&mut self, hash: ContentHash, phrases: PhraseRecord) {
        self.phrases.insert(hash, phrases);
    }

    /// Moves the playhead, as the window does on every repaint from the
    /// transport's position.
    pub fn set_playhead(&mut self, at: Samples) {
        self.playhead = at;
    }

    /// Records how far the transport's render has reached. The window calls
    /// this on every repaint with the `reached` field of the transport's
    /// status. The master BPM control writes from the first whole mix beat
    /// at or after this frame plus [`REACH_MARGIN`], so its change lands
    /// where nothing has been rendered yet, with room for the render to
    /// advance between the repaint that read the frame and the edit, and
    /// the render continues without starting over. Until it is set, or when
    /// it lies before the playhead, the playhead stands in for it.
    pub fn set_reach(&mut self, at: Samples) {
        self.reach = Some(at);
    }

    /// Where the playhead is.
    pub fn playhead(&self) -> Samples {
        self.playhead
    }

    /// Chooses which of the four curves the lanes show and the presses edit.
    /// The selection is cleared if it was a node of another curve.
    pub fn select_curve(&mut self, curve: Curve) {
        if let Selection::Node { curve: held, .. } = self.selection
            && held != curve
        {
            self.selection = Selection::Nothing;
        }
        self.curve = curve;
    }

    /// The curve the lanes show.
    pub fn curve(&self) -> Curve {
        self.curve
    }

    /// What is selected.
    pub fn selection(&self) -> Selection {
        self.selection
    }

    /// The document with any drag in progress applied to a copy, which is
    /// what the scene is laid out from.
    fn previewed(&self) -> Cow<'_, Mix> {
        let Some(edit) = self.press.and_then(|press| press.grip.edit()) else {
            return Cow::Borrowed(self.history.mix());
        };
        let mut preview = self.history.mix().clone();
        match apply_edit(&mut preview, &edit) {
            Ok(()) => Cow::Owned(preview),
            // A drag onto another node, or anywhere else the document
            // refuses, is drawn as the document still stands.
            Err(_) => Cow::Borrowed(self.history.mix()),
        }
    }

    /// The tempo lane's range: the one the press is holding, or the one the
    /// committed document gives.
    fn tempo_range(&self) -> (Bpm, Bpm) {
        match self.press {
            Some(press) => (press.low, press.high),
            None => tempo_range_of(self.history.mix()),
        }
    }

    /// Everything to draw, for the committed document with any drag in
    /// progress previewed.
    pub fn scene(&self) -> Scene {
        let mix = self.previewed();
        let length = mix_length(&mix).to_seconds();
        let (low, high) = self.tempo_range();
        let playhead = f64::from(self.x_of(self.playhead.to_seconds()));
        let playhead_x = (0.0..=f64::from(self.view.width_px))
            .contains(&playhead)
            .then_some(playhead as f32);

        let Some(frame) = Frame::of(self.view, &mix, self.heights_of(&mix)) else {
            return Scene {
                lanes: Vec::new(),
                tempo: TempoLane {
                    top: 0.0,
                    height: self.lane_height(0),
                    low,
                    high,
                    curve: Vec::new(),
                    nodes: Vec::new(),
                },
                playhead_x,
                length,
            };
        };

        let lanes = (0..mix.tracks.len())
            .map(|track| self.lane_of(&frame, track))
            .collect();
        Scene {
            lanes,
            tempo: self.tempo_lane_of(&frame, low, high),
            playhead_x,
            length,
        }
    }

    /// Everything to draw for one track's lane.
    fn lane_of(&self, frame: &Frame<'_>, track: usize) -> Lane {
        let held = &frame.mix.tracks[track];
        let top = frame.lane_top(track);
        let height = frame.lane_height(track);
        let (start_x, end_x) = frame.track_span(track);
        let (first, last) = frame.columns_of(track);
        let envelope = envelope_of(held, self.curve);

        let mut waveform = Vec::new();
        if let Some(overview) = self.overviews.get(&held.hash) {
            let placed = &frame.layout.tracks[track];
            let middle = top + height / 2.0;
            for column in first..=last {
                let x = column as f64;
                let from =
                    placed.track_time_at(&frame.layout.curve, frame.time_at(x) + frame.start);
                let to =
                    placed.track_time_at(&frame.layout.curve, frame.time_at(x + 1.0) + frame.start);
                let peak = overview.peak_over(from.to_samples()..to.to_samples());
                waveform.push(WaveColumn {
                    x: x as f32,
                    top: (middle - f64::from(peak.high) * height / 2.0) as f32,
                    bottom: (middle - f64::from(peak.low) * height / 2.0) as f32,
                });
            }
        }

        let mut curve = Vec::new();
        for column in first..=last {
            let x = column as f64;
            let level = envelope.value_at(frame.track_beat_at(track, x));
            curve.push(Point {
                x: x as f32,
                y: level_row(top, height, level) as f32,
            });
        }

        let nodes = envelope
            .nodes()
            .iter()
            .filter_map(|node| {
                let x = frame.column_of_track_beat(track, node.at);
                frame.on_screen(x).then(|| NodeMark {
                    at_px: Point {
                        x: x as f32,
                        y: level_row(top, height, node.value) as f32,
                    },
                    at: node.at,
                    level: node.value,
                    selected: self.selection
                        == Selection::Node {
                            track,
                            curve: self.curve,
                            at: node.at,
                        },
                })
            })
            .collect();

        let anchors = [
            (Anchor::Intro, held.anchors.intro),
            (Anchor::Outro, held.anchors.outro),
        ]
        .into_iter()
        .filter_map(|(anchor, at)| {
            let x = frame.column_of_track_beat(track, at);
            frame.on_screen(x).then(|| AnchorMark {
                x: x as f32,
                anchor,
                at,
                selected: self.selection == Selection::Anchor { track, anchor },
            })
        })
        .collect();

        let record = self.phrases.get(&held.hash);
        let bars = bars_of(frame, track, record.map_or(0, |record| record.downbeat));
        let phrases = record
            .map(|record| {
                record
                    .starts
                    .iter()
                    .filter_map(|start| {
                        let x = frame.column_of_track_beat(track, start.beat);
                        frame.on_screen(x).then_some(PhraseMark {
                            x: x as f32,
                            bars: start.bars,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sections = record
            .map(|record| {
                record
                    .sections
                    .iter()
                    .filter_map(|beat| {
                        let x = frame.column_of_track_beat(track, *beat);
                        frame.on_screen(x).then_some(x as f32)
                    })
                    .collect()
            })
            .unwrap_or_default();

        Lane {
            track,
            name: held
                .path
                .file_stem()
                .unwrap_or_else(|| held.path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            top: top as f32,
            height: height as f32,
            start_x: start_x as f32,
            end_x: end_x as f32,
            bpm: held.grid.bpm,
            keylock: held.keylock,
            gain: held.gain,
            waveform,
            curve,
            nodes,
            anchors,
            bars,
            phrases,
            sections,
        }
    }

    /// Everything to draw for the tempo lane.
    fn tempo_lane_of(&self, frame: &Frame<'_>, low: Bpm, high: Bpm) -> TempoLane {
        let lane = frame.mix.tracks.len();
        let top = frame.lane_top(lane);
        let height = frame.lane_height(lane);
        let width = frame.width();

        let mut curve = Vec::new();
        let mut column = 0.0;
        while column <= width {
            let bpm = frame
                .layout
                .curve
                .bpm_at_time(frame.time_at(column) + frame.start);
            curve.push(Point {
                x: column as f32,
                y: tempo_row(top, height, low, high, bpm) as f32,
            });
            column += 1.0;
        }

        let mut nodes: Vec<(f64, TempoMark)> = Vec::new();
        for (track, held) in frame.mix.tracks.iter().enumerate() {
            for node in &held.tempo {
                let mix_beat = frame.layout.tracks[track].origin + node.at;
                let x = frame.column_of_mix_beat(mix_beat);
                if !frame.on_screen(x) {
                    continue;
                }
                nodes.push((
                    mix_beat.0,
                    TempoMark {
                        at_px: Point {
                            x: x as f32,
                            y: tempo_row(top, height, low, high, node.bpm) as f32,
                        },
                        track,
                        at: node.at,
                        bpm: node.bpm,
                        selected: self.selection == Selection::TempoNode { track, at: node.at },
                    },
                ));
            }
        }
        nodes.sort_by(|a, b| a.0.total_cmp(&b.0));

        TempoLane {
            top: top as f32,
            height: height as f32,
            low,
            high,
            curve,
            nodes: nodes.into_iter().map(|(_, mark)| mark).collect(),
        }
    }

    /// A press at a point, which selects and may begin a drag.
    ///
    /// In a track lane, the first of these that applies:
    ///
    /// 1. A node of the selected curve within [`HIT_PX`] of the point on
    ///    both axes is selected and taken hold of. When several qualify, the
    ///    nearest by distance is taken, and the earlier beat on a tie.
    /// 2. An anchor within [`HIT_PX`] of the column is selected and taken
    ///    hold of, the intro anchor on a tie.
    /// 3. When the row is within [`EDGE_PX`] of the bottom edge of this lane
    ///    or of the lane above, and the lane across that edge has a node of
    ///    the selected curve, an anchor, or a tempo node within [`HIT_PX`]
    ///    of the point, that one is taken hold of as rules 1 and 2 and the
    ///    tempo lane's rule 1 say. A node at silence is drawn on its lane's
    ///    bottom row, and this is what lets a press from either side of the
    ///    row take it. A node that is already there is taken before a node
    ///    is added, here as everywhere else in these rules, so this comes
    ///    before rules 4 and 5.
    /// 4. When the column lies within the track's audio, from `start_x` to
    ///    `end_x` inclusive, and the selected curve of the track has no
    ///    nodes, a first node is added at the nearest whole beat of the
    ///    track to the column's time, at the level the row gives, and is
    ///    selected and taken hold of. This is how a person clicks the track
    ///    to place the first node.
    /// 5. When the column lies within the track's audio and the curve's line
    ///    at that column is within [`HIT_PX`] of the row, a node is added at
    ///    the nearest whole beat of the track to the column's time, at the
    ///    level the curve gives at that beat, and is selected and taken hold
    ///    of. This is how a person clicks the line to add more nodes.
    /// 6. When the row is within [`EDGE_PX`] of the bottom edge of this lane
    ///    or of the lane above, that edge is taken hold of, and a drag then
    ///    sets the height of the lane the edge belongs to, as
    ///    [`set_lane_height`](Timeline::set_lane_height) does. The
    ///    selection stays as it was.
    /// 7. The track is selected.
    ///
    /// In the tempo lane, the first that applies:
    ///
    /// 1. A tempo node within [`HIT_PX`] of the point on both axes is
    ///    selected and taken hold of, the nearest by distance when several
    ///    qualify and the earlier mix beat on a tie.
    /// 2. When the row is within [`EDGE_PX`] of the last track lane's
    ///    bottom edge and that lane has a node of the selected curve or an
    ///    anchor within [`HIT_PX`] of the point, that one is taken hold of.
    ///    A node that is already there is taken before a tempo node is
    ///    added, so this comes before rule 3.
    /// 3. When the curve's line at the column is within [`HIT_PX`] of the
    ///    row, a tempo node is added at the nearest whole mix beat to the
    ///    column's time, at the tempo the curve gives at that beat, on the
    ///    track that owns the beat, and is selected and taken hold of.
    /// 4. When the row is within [`EDGE_PX`] of the tempo lane's bottom
    ///    edge or of the last track lane's bottom edge, that edge is taken
    ///    hold of, as in a track lane.
    /// 5. Nothing changes.
    ///
    /// A press below every lane takes hold of the tempo lane's bottom edge
    /// when the row is within [`EDGE_PX`] of it, and otherwise changes
    /// nothing. A press above the first lane changes nothing.
    pub fn press(&mut self, at: Point) {
        let (low, high) = tempo_range_of(self.history.mix());
        let outcome = {
            let mix = self.history.mix();
            match Frame::of(self.view, mix, self.heights_of(mix)) {
                Some(frame) => self.outcome_of(&frame, at, low, high),
                None => Outcome::Nothing,
            }
        };
        if let Some(lane) = self.edge_press(outcome, at) {
            self.press = None;
            self.edge = Some(lane);
            return;
        }
        self.edge = None;
        let grip = match outcome {
            Outcome::Nothing => return,
            Outcome::Track(track) => {
                self.press = None;
                self.selection = Selection::Track(track);
                return;
            }
            Outcome::Node { track, at } => {
                let node = envelope_of(&self.history.mix().tracks[track], self.curve)
                    .nodes()
                    .iter()
                    .find(|node| node.at.0 == at.0)
                    .copied()
                    .expect("the timeline located this node when it worked out the press");
                Grip::Node {
                    track,
                    curve: self.curve,
                    from: node,
                    to: node,
                }
            }
            Outcome::Anchor { track, anchor } => {
                let anchors = self.history.mix().tracks[track].anchors;
                let beat = match anchor {
                    Anchor::Intro => anchors.intro,
                    Anchor::Outro => anchors.outro,
                };
                Grip::Anchor {
                    track,
                    anchor,
                    from: beat,
                    to: beat,
                }
            }
            Outcome::Tempo { track, at } => {
                let node = self.history.mix().tracks[track]
                    .tempo
                    .iter()
                    .find(|node| node.at.0 == at.0)
                    .copied()
                    .expect("the timeline located this node when it worked out the press");
                Grip::Tempo {
                    track,
                    from: node,
                    to: node,
                }
            }
            Outcome::AddNode { track, node } => {
                let edit = Edit::AddNode {
                    track,
                    curve: self.curve,
                    node,
                };
                if self.apply(edit).is_err() {
                    // A beat that already holds a node leaves the track
                    // selected, which is what a press that adds nothing does.
                    self.press = None;
                    self.selection = Selection::Track(track);
                    return;
                }
                Grip::Node {
                    track,
                    curve: self.curve,
                    from: node,
                    to: node,
                }
            }
            Outcome::AddTempo { track, node } => {
                if self.apply(Edit::AddTempoNode { track, node }).is_err() {
                    self.press = None;
                    return;
                }
                Grip::Tempo {
                    track,
                    from: node,
                    to: node,
                }
            }
        };
        self.selection = grip.selection();
        self.press = Some(Press { grip, low, high });
    }

    /// What a press at a point would do, without doing any of it.
    fn outcome_of(&self, frame: &Frame<'_>, at: Point, low: Bpm, high: Bpm) -> Outcome {
        let x = f64::from(at.x);
        let y = f64::from(at.y);
        if !x.is_finite() {
            return Outcome::Nothing;
        }
        // A row above the first lane, one at or below the bottom of the
        // tempo lane, and one that is not a number all fall in no lane.
        let Some(lane) = frame.lane_at(y) else {
            return Outcome::Nothing;
        };
        let outcome = self.lane_outcome(frame, lane, x, y, low, high);
        if takes_hold(outcome) {
            return outcome;
        }
        // A node at the very foot of a lane and a node at the very top of
        // the next lane are drawn within a few pixels of the edge between
        // the two lanes. A press that near an edge therefore looks in the
        // lane across the edge as well, and a node, an anchor, or a tempo
        // node found there is taken before the edge is.
        let Some(across) = frame.lane_across_a_near_edge(lane, y) else {
            return outcome;
        };
        let neighbor = self.lane_outcome(frame, across, x, y, low, high);
        if takes_hold(neighbor) {
            return neighbor;
        }
        outcome
    }

    /// What a press at a column and a row in one lane would do.
    fn lane_outcome(
        &self,
        frame: &Frame<'_>,
        lane: usize,
        x: f64,
        y: f64,
        low: Bpm,
        high: Bpm,
    ) -> Outcome {
        if lane == frame.mix.tracks.len() {
            return self.tempo_outcome(frame, x, y, low, high);
        }
        self.track_outcome(frame, lane, x, y)
    }

    /// What a press in a track's lane would do.
    fn track_outcome(&self, frame: &Frame<'_>, track: usize, x: f64, y: f64) -> Outcome {
        let held = &frame.mix.tracks[track];
        let top = frame.lane_top(track);
        let height = frame.lane_height(track);
        let reach = f64::from(HIT_PX);
        let envelope = envelope_of(held, self.curve);

        let mut nearest: Option<(f64, Beats)> = None;
        for node in envelope.nodes() {
            let node_x = frame.column_of_track_beat(track, node.at);
            let node_y = level_row(top, height, node.value);
            let (dx, dy) = (node_x - x, node_y - y);
            if dx.abs() <= reach && dy.abs() <= reach {
                let distance = dx.hypot(dy);
                if nearest.is_none_or(|(held, _)| distance < held) {
                    nearest = Some((distance, node.at));
                }
            }
        }
        if let Some((_, at)) = nearest {
            return Outcome::Node { track, at };
        }

        let mut nearest: Option<(f64, Anchor)> = None;
        for (anchor, beat) in [
            (Anchor::Intro, held.anchors.intro),
            (Anchor::Outro, held.anchors.outro),
        ] {
            let distance = (frame.column_of_track_beat(track, beat) - x).abs();
            if distance <= reach && nearest.is_none_or(|(held, _)| distance < held) {
                nearest = Some((distance, anchor));
            }
        }
        if let Some((_, anchor)) = nearest {
            return Outcome::Anchor { track, anchor };
        }

        let (start_x, end_x) = frame.track_span(track);
        if !(start_x..=end_x).contains(&x) {
            return Outcome::Track(track);
        }
        let beat = frame.track_beat_at(track, x).round();
        if !beat.0.is_finite() {
            return Outcome::Track(track);
        }
        if envelope.is_empty() {
            return Outcome::AddNode {
                track,
                node: EnvelopeNode {
                    at: beat,
                    value: row_level(top, height, y),
                },
            };
        }
        let line = level_row(
            top,
            height,
            envelope.value_at(frame.track_beat_at(track, x)),
        );
        if (line - y).abs() <= reach {
            return Outcome::AddNode {
                track,
                node: EnvelopeNode {
                    at: beat,
                    value: envelope.value_at(beat),
                },
            };
        }
        Outcome::Track(track)
    }

    /// What a press in the tempo lane would do.
    fn tempo_outcome(&self, frame: &Frame<'_>, x: f64, y: f64, low: Bpm, high: Bpm) -> Outcome {
        let lane = frame.mix.tracks.len();
        let top = frame.lane_top(lane);
        let height = frame.lane_height(lane);
        let reach = f64::from(HIT_PX);

        let mut nearest: Option<(f64, f64, usize, Beats)> = None;
        for (track, held) in frame.mix.tracks.iter().enumerate() {
            for node in &held.tempo {
                let mix_beat = frame.layout.tracks[track].origin + node.at;
                let dx = frame.column_of_mix_beat(mix_beat) - x;
                let dy = tempo_row(top, height, low, high, node.bpm) - y;
                if dx.abs() > reach || dy.abs() > reach {
                    continue;
                }
                let distance = dx.hypot(dy);
                let closer = nearest.is_none_or(|(held_distance, held_beat, _, _)| {
                    distance < held_distance
                        || (distance == held_distance && mix_beat.0 < held_beat)
                });
                if closer {
                    nearest = Some((distance, mix_beat.0, track, node.at));
                }
            }
        }
        if let Some((_, _, track, at)) = nearest {
            return Outcome::Tempo { track, at };
        }

        let line = tempo_row(
            top,
            height,
            low,
            high,
            frame
                .layout
                .curve
                .bpm_at_time(frame.time_at(x) + frame.start),
        );
        if (line - y).abs() > reach {
            return Outcome::Nothing;
        }
        let mix_beat = frame.mix_beat_at(x).round();
        if !mix_beat.0.is_finite() {
            return Outcome::Nothing;
        }
        let Some((track, at)) = owner_of(frame.mix, mix_beat) else {
            return Outcome::Nothing;
        };
        Outcome::AddTempo {
            track,
            node: TempoNode {
                at,
                bpm: frame.layout.curve.bpm_at_beat(mix_beat),
            },
        }
    }

    /// Moves what a press took hold of: a node to the nearest whole beat of
    /// its track at the column and the level at the row, an anchor to the
    /// nearest whole beat at the column, and a tempo node to the nearest
    /// whole mix beat at the column and the tempo at the row. Nothing is
    /// committed until [`release`](Timeline::release). With no press held,
    /// nothing changes.
    ///
    /// A press holding a lane's bottom edge sets that lane's height to the
    /// row less the lane's top, clamped to [`MIN_LANE_PX`] to
    /// [`MAX_LANE_PX`], so the lanes below it move as the drag goes. The
    /// height is set as the drag goes and stays when it ends, and no edit
    /// is made.
    pub fn drag_to(&mut self, at: Point) {
        if let Some(lane) = self.edge {
            if at.y.is_finite() {
                self.set_lane_height(lane, at.y - self.lane_top(lane));
            }
            return;
        }
        let Some(mut press) = self.press else {
            return;
        };
        if !at.x.is_finite() || !at.y.is_finite() {
            return;
        }
        // The drag is measured against the committed layout. Dragging a
        // track's own intro anchor moves that track along the timeline, so
        // measuring against the previewed layout instead would move the
        // ground the drag is being measured on.
        let mix = self.history.mix();
        let Some(frame) = Frame::of(self.view, mix, self.heights_of(mix)) else {
            return;
        };
        let x = f64::from(at.x);
        let y = f64::from(at.y);
        let held = match press.grip {
            Grip::Node { track, .. } | Grip::Anchor { track, .. } | Grip::Tempo { track, .. } => {
                track
            }
        };
        if held >= frame.mix.tracks.len() {
            return;
        }
        match &mut press.grip {
            Grip::Node { track, to, .. } => {
                let top = frame.lane_top(*track);
                let beat = frame.track_beat_at(*track, x).round();
                if !beat.0.is_finite() {
                    return;
                }
                to.at = beat;
                to.value = row_level(top, frame.lane_height(*track), y);
            }
            Grip::Anchor { track, to, .. } => {
                let beat = frame.track_beat_at(*track, x).round();
                if !beat.0.is_finite() {
                    return;
                }
                *to = beat;
            }
            Grip::Tempo { track, to, .. } => {
                let lane = frame.mix.tracks.len();
                let top = frame.lane_top(lane);
                let mix_beat = frame.mix_beat_at(x).round();
                if !mix_beat.0.is_finite() {
                    return;
                }
                to.at = mix_beat - frame.layout.tracks[*track].origin;
                to.bpm = row_tempo(top, frame.lane_height(lane), press.low, press.high, y);
            }
        }
        self.selection = press.grip.selection();
        self.press = Some(press);
    }

    /// Ends a press, committing the move as one edit if the held node or
    /// anchor was moved, and writing a correction for a moved anchor when a
    /// corrections folder is set. A move the history refuses, such as a
    /// node dragged onto another, leaves the document as it was. A press
    /// that held a lane's bottom edge ends with the lane at the height the
    /// drag left it, and commits nothing.
    pub fn release(&mut self) {
        // A resize is not an edit: the lane already has the height the drag
        // gave it, and nothing goes to the history.
        if self.edge.take().is_some() {
            return;
        }
        let Some(press) = self.press.take() else {
            return;
        };
        let Some(edit) = press.grip.edit() else {
            return;
        };
        if self.apply(edit).is_err() {
            self.selection = press.grip.selection_before_the_drag();
        }
    }

    /// What a press at the point would select, without selecting anything
    /// or adding a node: the node, anchor, or tempo node it would take hold
    /// of, the track it would select or add a first node to, or nothing.
    /// A press that would add a tempo node comes back as nothing, since the
    /// tempo lane belongs to no track and holds nothing yet to name, and so
    /// does a press that would take hold of a lane's bottom edge. The
    /// window uses it, with [`edge_at`](Timeline::edge_at), to shape the
    /// cursor.
    pub fn hit(&self, at: Point) -> Selection {
        let (low, high) = tempo_range_of(self.history.mix());
        let mix = self.history.mix();
        let Some(frame) = Frame::of(self.view, mix, self.heights_of(mix)) else {
            return Selection::Nothing;
        };
        let outcome = self.outcome_of(&frame, at, low, high);
        if self.edge_press(outcome, at).is_some() {
            return Selection::Nothing;
        }
        match outcome {
            Outcome::Nothing | Outcome::AddTempo { .. } => Selection::Nothing,
            Outcome::Track(track) | Outcome::AddNode { track, .. } => Selection::Track(track),
            Outcome::Node { track, at } => Selection::Node {
                track,
                curve: self.curve,
                at,
            },
            Outcome::Anchor { track, anchor } => Selection::Anchor { track, anchor },
            Outcome::Tempo { track, at } => Selection::TempoNode { track, at },
        }
    }

    /// Applies any edit through the history, as the window does for a grid
    /// correction from the grid editor, and writes a correction for a grid
    /// change or an anchor move when a corrections folder is set. A refused
    /// edit comes back as its error and changes nothing.
    pub fn apply(&mut self, edit: Edit) -> Result<(), EditError> {
        // An edit the document refuses changes nothing, the grid editor's
        // correction in progress included, so the step ends only once the
        // history has taken the edit.
        self.apply_through_the_history(edit)?;
        self.correction_track = None;
        Ok(())
    }

    /// Applies an edit through the history and writes a correction for a
    /// grid change or an anchor move, without ending the grid editor's
    /// correction in progress. [`apply`](Timeline::apply) calls this and
    /// ends the correction once the history has taken the edit, and
    /// [`set_grid`](Timeline::set_grid) calls this to add a change to the
    /// correction in progress.
    fn apply_through_the_history(&mut self, edit: Edit) -> Result<(), EditError> {
        let correction = correction_for(&edit);
        self.history.apply(edit)?;
        self.changed = true;
        if let Some(track) = correction {
            self.record_correction(track);
        }
        Ok(())
    }

    /// Puts `grid` on `track` as one step of the grid editor's correction,
    /// marks the document changed, and writes a correction for it when a
    /// corrections folder is set.
    ///
    /// Consecutive calls for the same track make one undoable step between
    /// them: the second and every later call replace the document the call
    /// before it made rather than stacking on it, so an undo takes the
    /// track back to the grid it had before the first call, and the anchors
    /// and nodes are placed from that grid each time rather than rounded
    /// again at every call. Any other edit applied through the timeline, an
    /// undo, a redo, a call for another track, or
    /// [`end_grid_correction`](Timeline::end_grid_correction) ends the step,
    /// and the next call starts a new one. Neither the selection nor a
    /// press held on the timeline is touched, which [`undo`](Timeline::undo)
    /// would touch.
    ///
    /// A call the document refuses, because the tempo is not a positive
    /// finite number or the track is not in the playlist, is judged before
    /// the track is compared with the step in progress. It comes back as
    /// its error and leaves everything as it was: the document, what can be
    /// undone and redone, the step in progress, and the corrections folder.
    pub fn set_grid(&mut self, track: usize, grid: BeatGrid) -> Result<(), EditError> {
        let edit = Edit::SetGrid { track, grid };
        if self.correction_track != Some(track) {
            self.apply_through_the_history(edit)?;
            self.correction_track = Some(track);
            return Ok(());
        }
        // The correction's earlier change is undone before this one is
        // applied, so the history holds one step for the whole correction
        // and every grid of the correction places the track's anchors and
        // nodes from the grid the correction began on rather than from the
        // grid the change before it left. A refused edit leaves what can be
        // redone alone, so the undone change goes back exactly as it was.
        self.history.undo();
        if let Err(problem) = self.apply_through_the_history(edit) {
            self.history.redo();
            return Err(problem);
        }
        Ok(())
    }

    /// Ends the grid editor's correction in progress, as the window does
    /// when the editor closes, so that the next
    /// [`set_grid`](Timeline::set_grid) starts a new undoable step. With no
    /// correction in progress nothing changes.
    pub fn end_grid_correction(&mut self) {
        self.correction_track = None;
    }

    /// Writes a track's grid and anchors to the corrections folder, if one
    /// is set, keeping the message of a write that fails.
    fn record_correction(&mut self, track: usize) {
        let Some(dir) = self.corrections.clone() else {
            return;
        };
        let Some(held) = self.history.mix().tracks.get(track) else {
            return;
        };
        if let Err(problem) = write_correction(&dir, held) {
            self.failure = Some(format!(
                "the correction for {} could not be written to {}: {problem}",
                held.path.display(),
                dir.display()
            ));
        }
    }

    /// Selects a track, as a press on its name in the window does.
    pub fn select_track(&mut self, track: usize) {
        self.selection = Selection::Track(track);
    }

    /// Sets the selected tempo node's tempo to a number the person typed,
    /// keeping its beat. With no tempo node selected, or a tempo that is not
    /// a positive finite number, nothing changes.
    pub fn set_selected_tempo(&mut self, bpm: Bpm) {
        let Selection::TempoNode { track, at } = self.selection else {
            return;
        };
        if !bpm.is_valid() {
            return;
        }
        let _ = self.apply(Edit::MoveTempoNode {
            track,
            from: at,
            to: TempoNode { at, bpm },
        });
    }

    /// A click on the ruler at a column, which asks the window to move the
    /// transport to that column's time. The request waits in
    /// [`take_seek`](Timeline::take_seek).
    pub fn click_ruler(&mut self, x: f32) {
        if !x.is_finite() {
            return;
        }
        self.seek = Some(self.time_at(x).to_samples());
    }

    /// The pending request to move the transport, taken once.
    pub fn take_seek(&mut self) -> Option<Samples> {
        self.seek.take()
    }

    /// Whether the document has changed since this was last asked, which is
    /// when the window hands the transport the new document. Undo and redo
    /// count as changes.
    pub fn take_document_change(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Removes what is selected: a node, a tempo node, or a track, after
    /// which nothing is selected. An anchor cannot be removed, and nothing
    /// selected is nothing to remove.
    pub fn delete_selection(&mut self) {
        let edit = match self.selection {
            Selection::Node { track, curve, at } => Edit::RemoveNode { track, curve, at },
            Selection::TempoNode { track, at } => Edit::RemoveTempoNode { track, at },
            Selection::Track(track) => Edit::RemoveTrack { track },
            Selection::Anchor { .. } | Selection::Nothing => return,
        };
        if self.apply(edit).is_ok() {
            self.selection = Selection::Nothing;
        }
    }

    /// Sets the mix tempo from just ahead of the playhead on, as the master
    /// BPM control does when the person releases it: one
    /// [`ChangeTempoFrom`](dermixen_core::Edit::ChangeTempoFrom) at the
    /// first whole mix beat at or after the frame the render has reached
    /// plus [`REACH_MARGIN`] (see [`set_reach`](Timeline::set_reach)), which
    /// pins the curve there and puts the new tempo one beat later, so
    /// nothing the playhead has passed and nothing already rendered changes.
    /// The window calls this once per release of the control rather than on
    /// every movement, so that a turn of the control is one undoable step.
    pub fn set_master_tempo(&mut self, bpm: Bpm) {
        let Some(mix_beat) = self.beat_to_write_from() else {
            return;
        };
        let _ = self.apply(Edit::ChangeTempoFrom { mix_beat, bpm });
    }

    /// The mix beat the master BPM control writes its pin at: the first
    /// whole beat at or after the frame the render has reached plus
    /// [`REACH_MARGIN`], with the playhead standing in for a reach that has
    /// not been set or that lies before the playhead. A mix with no tracks
    /// has no such beat.
    fn beat_to_write_from(&self) -> Option<Beats> {
        let layout = self.history.mix().timeline()?;
        let reached = match self.reach {
            Some(reach) if reach.0 >= self.playhead.0 => reach,
            _ => self.playhead,
        };
        let ahead = (reached + REACH_MARGIN).to_seconds() + layout.start();
        Some(Beats(layout.curve.beat_at(ahead).0.ceil()))
    }

    /// The mix tempo at the playhead, which the master BPM control shows.
    pub fn master_tempo(&self) -> Bpm {
        let Some(layout) = self.history.mix().timeline() else {
            return DEFAULT_TEMPO;
        };
        layout
            .curve
            .bpm_at_time(self.playhead.to_seconds() + layout.start())
    }

    /// Moves a track to another position in the playlist, as
    /// [`MoveTrack`](dermixen_core::Edit::MoveTrack) does. The selection is
    /// cleared, since track positions have changed. The other commands
    /// leave the selection where it is.
    pub fn move_track(&mut self, from: usize, to: usize) {
        if self.apply(Edit::MoveTrack { from, to }).is_ok() {
            self.selection = Selection::Nothing;
        }
    }

    /// Turns a track's keylock on or off.
    pub fn set_keylock(&mut self, track: usize, keylock: bool) {
        let _ = self.apply(Edit::SetKeylock { track, keylock });
    }

    /// Undoes the last edit and returns whether there was one. The selection
    /// is cleared, since what it named may be gone, a press held on the
    /// timeline is dropped for the same reason, and the grid editor's
    /// correction in progress ends, so the next
    /// [`set_grid`](Timeline::set_grid) starts a step of its own.
    pub fn undo(&mut self) -> bool {
        self.correction_track = None;
        if !self.history.undo() {
            return false;
        }
        self.press = None;
        self.edge = None;
        self.selection = Selection::Nothing;
        self.changed = true;
        true
    }

    /// Redoes the last undone edit and returns whether there was one. The
    /// selection is cleared, a press held on the timeline is dropped, and
    /// the grid editor's correction in progress ends.
    pub fn redo(&mut self) -> bool {
        self.correction_track = None;
        if !self.history.redo() {
            return false;
        }
        self.press = None;
        self.edge = None;
        self.selection = Selection::Nothing;
        self.changed = true;
        true
    }

    /// Whether there is an edit to undo.
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is an edit to redo.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Names the folder every anchor move and grid change is written to, as
    /// [`write_correction`](crate::write_correction) writes it. The folder
    /// is not checked here. A correction that cannot be written leaves its
    /// message for [`take_correction_failure`](Timeline::take_correction_failure),
    /// and the edit stands.
    pub fn set_corrections_dir(&mut self, dir: PathBuf) {
        self.corrections = Some(dir);
    }

    /// The message of the last correction that could not be written, taken
    /// once.
    pub fn take_correction_failure(&mut self) -> Option<String> {
        self.failure.take()
    }
}

/// The columns of a track's bar lines within the view.
///
/// Bars are four beats of the track's own grid, counted from the beat the
/// track's phrase record calls a downbeat. They are left out altogether when
/// the bars of the visible stretch of the track would come closer together
/// than [`MIN_BAR_PX`].
fn bars_of(frame: &Frame<'_>, track: usize, downbeat: u32) -> Vec<f32> {
    let (first_column, last_column) = frame.columns_of(track);
    if first_column > last_column {
        return Vec::new();
    }
    let bar = f64::from(BEATS_PER_BAR);
    let downbeat = f64::from(downbeat);
    let from = frame.track_beat_at(track, first_column as f64).0;
    let to = frame.track_beat_at(track, last_column as f64).0;
    if !from.is_finite() || !to.is_finite() {
        return Vec::new();
    }
    // One bar of slack at each end covers the rounding of the column range.
    // Every line is then kept or dropped by the column it falls in.
    let first = ((from - downbeat) / bar).floor() - 1.0;
    let last = ((to - downbeat) / bar).ceil() + 1.0;
    if !first.is_finite() || !last.is_finite() || last < first {
        return Vec::new();
    }
    // When the bar lines average less than MIN_BAR_PX pixels apart, some
    // pair of them is closer than that as well, so the whole set is dropped
    // here without working each line out. Counting the lines first also
    // keeps the loop below short on a mix long enough to hold millions of
    // bars.
    let span_px = (last_column - first_column) as f64;
    if last - first > span_px / f64::from(MIN_BAR_PX) + 4.0 {
        return Vec::new();
    }

    let mut columns = Vec::new();
    let mut index = first;
    while index <= last {
        let x = frame.column_of_track_beat(track, Beats(downbeat + index * bar));
        if frame.on_screen(x) {
            columns.push(x as f32);
        }
        index += 1.0;
    }
    let narrowest = columns
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(f32::INFINITY, f32::min);
    if narrowest < MIN_BAR_PX {
        return Vec::new();
    }
    columns
}
