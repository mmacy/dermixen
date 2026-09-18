//! The transitions between adjacent tracks: the blend that is the default,
//! the beat-matched crossfade, and the presets built on those.
//!
//! A preset places its nodes from a track's anchors, its grid, and its
//! length, and every function here takes those numbers to be ones a mix
//! document may hold, which [`Mix::check`](crate::Mix::check) is what
//! decides. Each node then lands on a finite beat at a finite level, which
//! is why placing one cannot fail. [`apply_edit`](crate::apply_edit) checks
//! a track before it writes a preset onto it.

use crate::anchors::Anchors;
use crate::envelope::EnvelopeNode;
use crate::mix::Track;
use crate::tempo::TempoNode;
use crate::units::{Beats, Decibels};

/// The six fractions that shape a fade. Each is used twice: as a position
/// along the fade's span and as the amplitude the fade reaches there, so the
/// fade is a straight line in amplitude sampled at these points.
const FADE_FRACTIONS: [f64; 6] = [0.0, 1.0 / 8.0, 1.0 / 4.0, 1.0 / 2.0, 3.0 / 4.0, 1.0];

/// Replaces the tempo node at the same beat as `node`, or adds it if the
/// track has no node there yet.
fn set_tempo_node(tempo: &mut Vec<TempoNode>, node: TempoNode) {
    match tempo.iter_mut().find(|existing| existing.at.0 == node.at.0) {
        Some(existing) => *existing = node,
        None => tempo.push(node),
    }
}

/// The six nodes of a fade whose amplitude rises from silence at `start` to
/// unity `span` beats later.
fn fade_in(start: Beats, span: Beats) -> [EnvelopeNode; 6] {
    std::array::from_fn(|i| EnvelopeNode {
        at: start + span * FADE_FRACTIONS[i],
        value: Decibels::from_linear(FADE_FRACTIONS[i]),
    })
}

/// The six nodes of a fade whose amplitude falls from unity at `start` to
/// silence `span` beats later.
fn fade_out(start: Beats, span: Beats) -> [EnvelopeNode; 6] {
    std::array::from_fn(|i| EnvelopeNode {
        at: start + span * (1.0 - FADE_FRACTIONS[i]),
        value: Decibels::from_linear(FADE_FRACTIONS[i]),
    })
}

/// Writes a beat-matched crossfade between an outgoing track and the track
/// after it, spanning `bars` bars from the aligned anchors.
///
/// On the outgoing track it places a tempo node at the outro anchor holding
/// that track's own tempo, and a fade from unity at the outro anchor to
/// silence `bars` bars later. On the incoming track it places a tempo node
/// `bars` bars after the intro anchor holding that track's own tempo, and a
/// fade from silence at the intro anchor to unity `bars` bars later. Because
/// the two anchors share one mix beat, the mix tempo ramps from the outgoing
/// tempo to the incoming tempo across exactly the fade.
///
/// Each fade is six volume nodes that approximate a fade that is straight in
/// amplitude. The fade in has nodes at fractions 0, 1/8, 1/4, 1/2, 3/4, and 1
/// of the span holding [`Decibels::SILENCE`](crate::Decibels::SILENCE) and
/// then the decibel equivalents of amplitudes 1/8, 1/4, 1/2, 3/4, and 1. The
/// fade out is its mirror in time: nodes at fractions 0, 1/4, 1/2, 3/4, 7/8,
/// and 1 holding unity and then amplitudes 3/4, 1/2, 1/4, 1/8, and silence.
/// Nodes already at those positions are replaced and every other node is
/// left alone, so a fade written earlier with a different length leaves its
/// other nodes in place; EQ envelopes are not touched. `bars` must be at
/// least one, since a span of zero beats collapses each fade to a single node
/// at full level.
pub fn beatmix(outgoing: &mut Track, incoming: &mut Track, bars: u32) {
    let span = Beats::from_bars(f64::from(bars));

    set_tempo_node(
        &mut outgoing.tempo,
        TempoNode {
            at: outgoing.anchors.outro,
            bpm: outgoing.grid.bpm,
        },
    );
    set_tempo_node(
        &mut incoming.tempo,
        TempoNode {
            at: incoming.anchors.intro + span,
            bpm: incoming.grid.bpm,
        },
    );

    for node in fade_out(outgoing.anchors.outro, span) {
        outgoing
            .volume
            .insert(node)
            .expect("fade node positions and levels are always finite");
    }
    for node in fade_in(incoming.anchors.intro, span) {
        incoming
            .volume
            .insert(node)
            .expect("fade node positions and levels are always finite");
    }
}

/// The length of the default transition in bars, which is also the overlap
/// the anchors analysis places are made for.
pub const DEFAULT_BARS: u32 = 8;

/// A transition preset: the shape of the nodes written across an overlap.
///
/// A preset is nothing beyond the nodes it writes, so a transition can be
/// edited freely afterwards. The names are the ones `dermixen mix add
/// --preset` accepts.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Preset {
    /// The long blend of [`blend`], which is the default: the incoming
    /// track rises from silence over the eight bars before its intro anchor
    /// and the twenty-eight bars after it, and the outgoing track eases down
    /// from its outro anchor to its last sample without ever fading out. It
    /// has no length of its own, so the bar count given with it moves the
    /// outro anchor as it does for every preset and changes nothing else.
    Blend,
    /// The beat-matched crossfade of [`beatmix`], `bars` bars long.
    Beatmix {
        /// The length of the fade in bars, at least one.
        bars: u32,
    },
    /// The crossfade of [`beatmix`], `bars` bars long, with the low band
    /// handed from the outgoing track to the incoming one at the middle of
    /// the fade: the incoming track's lows are killed until the swap beat and
    /// the outgoing track's lows are killed from it, each over the beat
    /// before the swap. The swap beat is the anchor plus half the fade, which
    /// is a whole beat because a fade is whole bars.
    BassSwap {
        /// The length of the fade in bars, at least one.
        bars: u32,
    },
    /// No overlap to speak of: the outgoing track drops to silence over the
    /// quarter beat before its outro anchor, the incoming track rises from
    /// silence over the quarter beat before its intro anchor, and the tempo
    /// changes at the shared beat rather than ramping, with a node on each
    /// track at its anchor holding its own tempo.
    Cut,
}

impl Preset {
    /// The preset with this name, or `None` for a name that is not one:
    /// `blend`, `beatmix`, `bass-swap`, or `cut`. `bars` is the fade length
    /// for the presets that have one and is ignored by `blend` and `cut`.
    pub fn from_name(name: &str, bars: u32) -> Option<Preset> {
        match name {
            "blend" => Some(Preset::Blend),
            "beatmix" => Some(Preset::Beatmix { bars }),
            "bass-swap" => Some(Preset::BassSwap { bars }),
            "cut" => Some(Preset::Cut),
            _ => None,
        }
    }

    /// The preset's name, as [`Preset::from_name`] reads it.
    pub fn name(self) -> &'static str {
        match self {
            Preset::Blend => "blend",
            Preset::Beatmix { .. } => "beatmix",
            Preset::BassSwap { .. } => "bass-swap",
            Preset::Cut => "cut",
        }
    }
}

/// Writes the transition `preset` describes between an outgoing track and
/// the track after it, placing nodes the way [`beatmix`] does: a node
/// already at one of the exact beats written is replaced, and every other
/// node is left alone.
pub fn apply(preset: Preset, outgoing: &mut Track, incoming: &mut Track) {
    match preset {
        Preset::Blend => blend(outgoing, incoming),
        Preset::Beatmix { bars } => beatmix(outgoing, incoming, bars),
        Preset::BassSwap { bars } => bass_swap(outgoing, incoming, bars),
        Preset::Cut => cut(outgoing, incoming),
    }
}

/// Writes everything [`beatmix`] writes, and in addition hands the low band
/// from the outgoing track to the incoming one at the middle of the span.
///
/// The swap beat is each track's own anchor plus half the span, which is a
/// whole beat because the span is whole bars. The incoming track's `low`
/// envelope gets [`Decibels::SILENCE`] one beat before its swap beat and
/// [`Decibels::UNITY`] at it; the outgoing track's `low` envelope gets
/// [`Decibels::UNITY`] one beat before its swap beat and
/// [`Decibels::SILENCE`] at it. Only the outgoing bassline is heard before
/// the swap, and only the incoming one after it.
fn bass_swap(outgoing: &mut Track, incoming: &mut Track, bars: u32) {
    beatmix(outgoing, incoming, bars);

    let half_span = Beats::from_bars(f64::from(bars)) * 0.5;
    let outgoing_swap = outgoing.anchors.outro + half_span;
    let incoming_swap = incoming.anchors.intro + half_span;

    for node in [
        EnvelopeNode {
            at: outgoing_swap - Beats::ONE,
            value: Decibels::UNITY,
        },
        EnvelopeNode {
            at: outgoing_swap,
            value: Decibels::SILENCE,
        },
    ] {
        outgoing
            .eq
            .low
            .insert(node)
            .expect("swap node positions and levels are always finite");
    }
    for node in [
        EnvelopeNode {
            at: incoming_swap - Beats::ONE,
            value: Decibels::SILENCE,
        },
        EnvelopeNode {
            at: incoming_swap,
            value: Decibels::UNITY,
        },
    ] {
        incoming
            .eq
            .low
            .insert(node)
            .expect("swap node positions and levels are always finite");
    }
}

/// Writes a hard cut: no overlap to speak of.
///
/// The outgoing track's volume drops from unity to silence over the quarter
/// beat before its outro anchor, and the incoming track's volume rises from
/// silence to unity over the quarter beat before its intro anchor. Each
/// track gets a tempo node at its own anchor holding its own original tempo,
/// so the tempo changes at the shared beat rather than ramping: the two
/// nodes land on one mix beat, the layout keeps them in playlist order, and
/// the later node, the incoming track's, is the tempo at that beat.
fn cut(outgoing: &mut Track, incoming: &mut Track) {
    let quarter_beat = Beats(0.25);

    set_tempo_node(
        &mut outgoing.tempo,
        TempoNode {
            at: outgoing.anchors.outro,
            bpm: outgoing.grid.bpm,
        },
    );
    set_tempo_node(
        &mut incoming.tempo,
        TempoNode {
            at: incoming.anchors.intro,
            bpm: incoming.grid.bpm,
        },
    );

    for node in [
        EnvelopeNode {
            at: outgoing.anchors.outro - quarter_beat,
            value: Decibels::UNITY,
        },
        EnvelopeNode {
            at: outgoing.anchors.outro,
            value: Decibels::SILENCE,
        },
    ] {
        outgoing
            .volume
            .insert(node)
            .expect("cut node positions and levels are always finite");
    }
    for node in [
        EnvelopeNode {
            at: incoming.anchors.intro - quarter_beat,
            value: Decibels::SILENCE,
        },
        EnvelopeNode {
            at: incoming.anchors.intro,
            value: Decibels::UNITY,
        },
    ] {
        incoming
            .volume
            .insert(node)
            .expect("cut node positions and levels are always finite");
    }
}

/// How many bars before its intro anchor the incoming track of a blend
/// starts rising from silence.
pub const BLEND_LEAD_BARS: u32 = 8;

/// How many bars after its intro anchor the incoming track of a blend takes
/// to reach full level, which is also how much of a track a blend needs
/// between the track's own two anchors.
pub const BLEND_RISE_BARS: u32 = 28;

/// The incoming track's volume nodes in a blend: each node's beat counted
/// from the intro anchor, and its level.
///
/// The first node sits [`BLEND_LEAD_BARS`] before the anchor and the last
/// [`BLEND_RISE_BARS`] after it. The track is at -12 dB when its anchor
/// lands on the outgoing track's outro anchor, which is where its beat
/// comes in under the outgoing track, and it reaches full level
/// twenty-eight bars later.
const BLEND_RISE: [(Beats, Decibels); 8] = [
    (Beats(-32.0), Decibels::SILENCE),
    (Beats(-24.0), Decibels(-24.0)),
    (Beats(-12.0), Decibels(-16.0)),
    (Beats(0.0), Decibels(-12.0)),
    (Beats(32.0), Decibels(-5.5)),
    (Beats(64.0), Decibels(-2.5)),
    (Beats(96.0), Decibels(-0.5)),
    (Beats(112.0), Decibels::UNITY),
];

/// The outgoing track's volume nodes in a blend: each node's place as a
/// fraction of the way from the outro anchor to the track's last sample,
/// and its level. The track is never faded to silence. It reaches -7 dB
/// as its audio ends.
const BLEND_EASE: [(f64, Decibels); 5] = [
    (0.0, Decibels::UNITY),
    (0.25, Decibels(-1.0)),
    (0.5, Decibels(-2.5)),
    (0.75, Decibels(-4.5)),
    (1.0, Decibels(-7.0)),
];

/// The shortest tail that fits the three middle nodes of [`BLEND_EASE`].
/// Each middle node is rounded to a whole beat, so the tail needs one whole
/// beat for each of the three, strictly between the outro anchor and the
/// last sample. A shorter tail rounds the middle nodes onto the anchor's
/// own beat, where the lowest of the three would replace the full-level
/// node and leave the track quiet from its first sample, so a blend out of
/// a tail this short writes the first and last nodes only.
const BLEND_EASE_MIN_TAIL: Beats = Beats(4.0);

/// Writes a long blend between an outgoing track and the track after it,
/// the transition a DJ draws by hand when the incoming track is meant to be
/// established before the outgoing one leaves.
///
/// The tempo nodes are the ones an eight-bar [`beatmix`] writes: one on the
/// outgoing track at its outro anchor holding that track's own tempo, and
/// one on the incoming track [`DEFAULT_BARS`] bars after its intro anchor
/// holding its own, so the mix tempo ramps from one to the other over the
/// eight bars after the aligned anchors.
///
/// The incoming track gets the eight volume nodes of [`BLEND_RISE`],
/// counted from its intro anchor: silence eight bars before the anchor,
/// -24 dB six bars before, -16 dB three bars before, -12 dB at the anchor,
/// then -5.5 dB eight bars after, -2.5 dB sixteen bars after, -0.5 dB
/// twenty-four bars after, and full level twenty-eight bars after. A node
/// before the track's first sample does no harm: the envelope is read from
/// the first sample on, so a track whose intro anchor is near its start
/// enters at whatever level the rise has reached there.
///
/// The outgoing track gets the five volume nodes of [`BLEND_EASE`], spread
/// from its outro anchor to the beat of its last sample: full level at the
/// anchor, -1 dB a quarter of the way to the end, -2.5 dB halfway, -4.5 dB
/// three quarters of the way, and -7 dB at the last sample. The three
/// middle nodes are rounded to whole beats. The track is never faded out:
/// it plays to its end under the incoming track, so the blend suits an
/// outro anchor in the last minute or two of a track, which is where
/// analysis places one.
///
/// The length of the tail, the beats from the outro anchor to the last
/// sample, decides how many of the five nodes are written. A tail of
/// [`BLEND_EASE_MIN_TAIL`] or more gets all five, because each middle node
/// then has a whole beat of its own strictly between the anchor and the
/// last sample. A shorter tail gets two nodes, full level at the anchor and
/// -7 dB at the last sample, so the track plays at full level up to its
/// anchor. An outro anchor at or past the last sample gets a single node at
/// full level, since there is no tail to ease.
///
/// Nodes already at the exact beats written are replaced and every other
/// node is left alone, as with [`beatmix`], and EQ envelopes are not
/// touched.
pub fn blend(outgoing: &mut Track, incoming: &mut Track) {
    set_tempo_node(
        &mut outgoing.tempo,
        TempoNode {
            at: outgoing.anchors.outro,
            bpm: outgoing.grid.bpm,
        },
    );
    set_tempo_node(
        &mut incoming.tempo,
        TempoNode {
            at: incoming.anchors.intro + Beats::from_bars(f64::from(DEFAULT_BARS)),
            bpm: incoming.grid.bpm,
        },
    );

    let outro = outgoing.anchors.outro;
    let end = outgoing.grid.beat_at_position(outgoing.length);
    let tail = end - outro;
    if tail.0 > 0.0 {
        let middle_nodes_fit = tail.0 >= BLEND_EASE_MIN_TAIL.0;
        for (fraction, value) in BLEND_EASE {
            let is_middle = fraction > 0.0 && fraction < 1.0;
            if is_middle && !middle_nodes_fit {
                continue;
            }
            let at = if fraction == 1.0 {
                end
            } else {
                (outro + tail * fraction).round()
            };
            outgoing
                .volume
                .insert(EnvelopeNode { at, value })
                .expect("ease node positions and levels are always finite");
        }
    } else {
        outgoing
            .volume
            .insert(EnvelopeNode {
                at: outro,
                value: Decibels::UNITY,
            })
            .expect("an anchor is always a finite beat");
    }
    for (offset, value) in BLEND_RISE {
        incoming
            .volume
            .insert(EnvelopeNode {
                at: incoming.anchors.intro + offset,
                value,
            })
            .expect("rise node positions and levels are always finite");
    }
}

/// The outro anchor for a transition `bars` bars long, given the anchors
/// analysis placed for the default length.
///
/// Analysis chooses where the outgoing track should be gone, which is the
/// end of an eight-bar overlap from its outro anchor. A longer or shorter
/// transition keeps that end and moves the outro anchor so the overlap still
/// finishes there, so the anchor moves earlier by four beats for every bar
/// beyond eight and later by four beats for every bar short of it.
pub fn outro_for(analyzed: Anchors, bars: u32) -> Beats {
    analyzed.outro + Beats::from_bars(f64::from(DEFAULT_BARS)) - Beats::from_bars(f64::from(bars))
}
