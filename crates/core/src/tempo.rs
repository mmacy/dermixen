//! The mix tempo curve and the mapping between mix time and track time.
//!
//! One tempo curve governs the whole mix. It is a function from mix beats to
//! tempo, and every track plays at whatever tempo the curve gives at each
//! moment, so tracks that overlap cannot drift apart.
//!
//! # Coordinates
//!
//! Three coordinate systems meet here.
//!
//! - **Mix beats** count beats along the timeline. Mix beat zero is beat zero
//!   of the first track's grid. Every track's beats fall on this same lattice,
//!   because tracks are placed by aligning whole beats.
//! - **Mix time** is elapsed time along the timeline in seconds, with time zero
//!   at mix beat zero. The tempo curve is what relates it to mix beats.
//! - **Track time** is time within one track's audio file, measured from the
//!   file's first sample. A track's own constant tempo grid relates it to
//!   that track's beats.
//!
//! # The curve between nodes
//!
//! A [`TempoCurve`] holds nodes at mix beats. At a node's own beat the tempo
//! is that node's tempo exactly. Between two consecutive nodes the tempo
//! changes linearly in *time*, so a ramp from tempo `a` at beat `p` to tempo
//! `b` at beat `q` lasts `120 * (q - p) / (a + b)` seconds, and the beat
//! count within the ramp is a quadratic function of time. Before the first
//! node the tempo is the first node's tempo; after the last node it is the
//! last node's tempo. Two nodes at the same beat make an instantaneous
//! change, and at exactly that beat the later node's tempo applies. Every
//! mapping is closed-form, and mix time is a strictly increasing function of
//! mix beats, so each direction is the exact inverse of the other.

use serde::{Deserialize, Serialize};

use crate::beat_grid::BeatGrid;
use crate::units::{Beats, Bpm, Samples, Seconds};

/// One node of the tempo curve: a tempo reached at a beat.
///
/// In a [`TempoCurve`], `at` is a mix beat. In a track's own tempo list inside
/// a project file, `at` is a beat of that track, and the mix layout turns it
/// into a mix beat.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoNode {
    /// The beat at which the curve reaches `bpm`.
    #[serde(rename = "beat")]
    pub at: Beats,
    /// The tempo at that beat.
    pub bpm: Bpm,
}

/// The tempo of the whole mix as a function of mix beats.
///
/// A curve always has at least one node, and nodes are kept in increasing
/// order of position.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoCurve {
    nodes: Vec<TempoNode>,
    segments: Vec<Segment>,
}

/// One stretch of the curve, in the form the mappings work in.
///
/// A segment begins at mix beat `beat`, which the mix reaches at mix time
/// `time`, and the tempo there is `bpm`. Along the segment the *square* of the
/// tempo grows by `slope` for every beat, which is the same thing as the tempo
/// changing linearly in time. The first and the last segment of a curve are
/// flat, with a slope of zero, and the formulas of those two segments stay
/// correct however far beyond the first node or the last node the code looks.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Segment {
    /// The mix beat at which the segment begins.
    beat: f64,
    /// The mix time at which the segment begins.
    time: f64,
    /// The tempo at the beginning of the segment, in beats per minute.
    bpm: f64,
    /// How much the square of the tempo grows for every beat along the segment.
    slope: f64,
}

impl Segment {
    /// The tempo at a mix beat.
    fn bpm_at_beat(&self, beat: f64) -> f64 {
        (self.bpm * self.bpm + self.slope * (beat - self.beat)).sqrt()
    }

    /// The tempo at a mix time.
    fn bpm_at_time(&self, time: f64) -> f64 {
        self.bpm + self.slope * (time - self.time) / 120.0
    }

    /// The mix time at a mix beat.
    ///
    /// A stretch of curve that runs from tempo `a` to tempo `b` across `n`
    /// beats lasts `120 * n / (a + b)` seconds. The time from the beginning of
    /// the segment to a beat within it is that same expression, with the tempo
    /// at that beat standing in for `b`.
    fn time_at(&self, beat: f64) -> f64 {
        self.time + 120.0 * (beat - self.beat) / (self.bpm + self.bpm_at_beat(beat))
    }

    /// The mix beat at a mix time, which is the exact inverse of [`Segment::time_at`].
    fn beat_at(&self, time: f64) -> f64 {
        self.beat + (time - self.time) * (self.bpm + self.bpm_at_time(time)) / 120.0
    }
}

/// The reason a list of nodes could not become a [`TempoCurve`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TempoCurveError {
    /// No nodes were given.
    #[error("a tempo curve needs at least one node")]
    Empty,
    /// A node's tempo was zero, negative, or not finite.
    #[error("node {index} has tempo {bpm}, which is not a positive finite number")]
    InvalidBpm {
        /// The index of the offending node in the list as given.
        index: usize,
        /// The tempo that was given.
        bpm: f64,
    },
    /// A node's position was not finite.
    #[error("node {index} has a position that is not a finite number")]
    InvalidPosition {
        /// The index of the offending node in the list as given.
        index: usize,
    },
}

/// The segments of a curve, with mix time measured from the first node.
///
/// The list begins with the flat stretch that reaches back before the first
/// node, holds one stretch for every pair of neighboring nodes that sit at
/// different beats, and ends with the flat stretch that runs on after the last
/// node. Two nodes at one beat make an instantaneous tempo change: this
/// function builds no segment between them, and no time passes at that beat.
///
/// The list of nodes must not be empty, and it must already be sorted by
/// position.
fn segments_of(nodes: &[TempoNode]) -> Vec<Segment> {
    let first = nodes[0];
    let last = nodes[nodes.len() - 1];
    let mut previous = Segment {
        beat: first.at.0,
        time: 0.0,
        bpm: first.bpm.0,
        slope: 0.0,
    };
    let mut segments = vec![previous];
    for pair in nodes.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let span = to.at.0 - from.at.0;
        if span <= 0.0 {
            continue;
        }
        let segment = Segment {
            beat: from.at.0,
            time: previous.time_at(from.at.0),
            bpm: from.bpm.0,
            slope: (to.bpm.0 - from.bpm.0) * (to.bpm.0 + from.bpm.0) / span,
        };
        segments.push(segment);
        previous = segment;
    }
    segments.push(Segment {
        beat: last.at.0,
        time: previous.time_at(last.at.0),
        bpm: last.bpm.0,
        slope: 0.0,
    });
    segments
}

/// The index of the segment that governs a mix beat: the last one that begins
/// at or before that beat.
///
/// Where two nodes sit at one beat, the segment beginning there is the one
/// after the change, which is how that beat comes to be heard at the later
/// node's tempo.
fn index_at_beat(segments: &[Segment], beat: f64) -> usize {
    segments
        .partition_point(|segment| segment.beat <= beat)
        .saturating_sub(1)
}

/// The index of the segment that governs a mix time: the last one that begins
/// at or before that time.
fn index_at_time(segments: &[Segment], time: f64) -> usize {
    segments
        .partition_point(|segment| segment.time <= time)
        .saturating_sub(1)
}

impl TempoCurve {
    /// Builds a curve from nodes given in any order.
    ///
    /// Nodes are sorted by position; nodes at the same position keep the order
    /// they were given in. Fails if the list is empty, a tempo is not a
    /// positive finite number, or a position is not finite.
    pub fn new(nodes: Vec<TempoNode>) -> Result<Self, TempoCurveError> {
        if nodes.is_empty() {
            return Err(TempoCurveError::Empty);
        }
        for (index, node) in nodes.iter().enumerate() {
            if !node.at.0.is_finite() {
                return Err(TempoCurveError::InvalidPosition { index });
            }
            if !node.bpm.is_valid() {
                return Err(TempoCurveError::InvalidBpm {
                    index,
                    bpm: node.bpm.0,
                });
            }
        }

        let mut nodes = nodes;
        nodes.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
        let mut segments = segments_of(&nodes);

        // Mix beat zero is heard at mix time zero, and the mapping has to say
        // so exactly rather than to within a rounding error, so the table
        // always holds a segment that begins at beat zero. Where no node sits
        // there, the segment covering beat zero is cut in two at that beat;
        // both halves describe the same stretch of curve. The constructor then
        // subtracts the time of beat zero from every segment's time, which puts
        // beat zero at time zero and leaves every gap between beats unchanged.
        let index = index_at_beat(&segments, 0.0);
        let zero_time = segments[index].time_at(0.0);
        if segments[index].beat != 0.0 {
            let half = Segment {
                beat: 0.0,
                time: zero_time,
                bpm: segments[index].bpm_at_beat(0.0),
                slope: segments[index].slope,
            };
            let at = if segments[index].beat < 0.0 {
                index + 1
            } else {
                index
            };
            segments.insert(at, half);
        }
        for segment in &mut segments {
            segment.time -= zero_time;
        }

        Ok(TempoCurve { nodes, segments })
    }

    /// A curve that holds one tempo everywhere.
    ///
    /// # Panics
    ///
    /// Panics if the tempo is not a positive finite number.
    pub fn constant(bpm: Bpm) -> Self {
        Self::new(vec![TempoNode {
            at: Beats::ZERO,
            bpm,
        }])
        .expect("a constant curve needs a tempo that is a positive finite number")
    }

    /// The nodes in increasing order of position.
    pub fn nodes(&self) -> &[TempoNode] {
        &self.nodes
    }

    /// The tempo at a mix beat.
    pub fn bpm_at_beat(&self, beat: Beats) -> Bpm {
        let segment = self.segments[index_at_beat(&self.segments, beat.0)];
        Bpm(segment.bpm_at_beat(beat.0))
    }

    /// The tempo at a mix time.
    pub fn bpm_at_time(&self, time: Seconds) -> Bpm {
        let segment = self.segments[index_at_time(&self.segments, time.0)];
        Bpm(segment.bpm_at_time(time.0))
    }

    /// The mix time at which a mix beat is reached. Mix beat zero is at time zero.
    pub fn time_at(&self, beat: Beats) -> Seconds {
        let segment = self.segments[index_at_beat(&self.segments, beat.0)];
        Seconds(segment.time_at(beat.0))
    }

    /// The mix beat reached at a mix time. This is the inverse of [`TempoCurve::time_at`].
    pub fn beat_at(&self, time: Seconds) -> Beats {
        let segment = self.segments[index_at_time(&self.segments, time.0)];
        Beats(segment.beat_at(time.0))
    }
}

/// A track placed on the timeline.
///
/// The placement is the mix beat at which the track's beat zero falls. Every
/// other relation between the track and the timeline follows from that, the
/// track's grid, and the tempo curve.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PlacedTrack {
    /// The mix beat at which the track's beat zero falls.
    pub origin: Beats,
    /// The track's own grid.
    pub grid: BeatGrid,
    /// The length of the track's audio.
    pub length: Samples,
}

impl PlacedTrack {
    /// The mix beat at which a track time is heard.
    pub fn mix_beat_of(&self, track_time: Seconds) -> Beats {
        self.origin + self.grid.beat_at(track_time)
    }

    /// The track time heard at a mix beat.
    pub fn track_time_at_beat(&self, mix_beat: Beats) -> Seconds {
        self.grid.time_of(mix_beat - self.origin)
    }

    /// The mix time at which a track time is heard.
    pub fn mix_time_of(&self, curve: &TempoCurve, track_time: Seconds) -> Seconds {
        curve.time_at(self.mix_beat_of(track_time))
    }

    /// The track time heard at a mix time. This is the inverse of [`PlacedTrack::mix_time_of`].
    pub fn track_time_at(&self, curve: &TempoCurve, mix_time: Seconds) -> Seconds {
        self.track_time_at_beat(curve.beat_at(mix_time))
    }

    /// How fast the track plays at a mix time, as track seconds per mix second.
    ///
    /// This is the mix tempo divided by the track's original tempo: one means
    /// the track plays at its original speed, and values above one mean it is
    /// sped up.
    pub fn rate_at(&self, curve: &TempoCurve, mix_time: Seconds) -> f64 {
        curve.bpm_at_time(mix_time).0 / self.grid.bpm.0
    }

    /// The mix time at which the track's first sample is heard.
    pub fn start(&self, curve: &TempoCurve) -> Seconds {
        self.mix_time_of(curve, Seconds::ZERO)
    }

    /// The mix time at which the track's last sample has been heard.
    pub fn end(&self, curve: &TempoCurve) -> Seconds {
        self.mix_time_of(curve, self.length.to_seconds())
    }
}
