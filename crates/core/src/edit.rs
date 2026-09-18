//! The edits a person makes to a mix on the timeline, applied one at a time
//! with undo and redo.
//!
//! Every change the timeline can make to a document is an [`Edit`]: a value
//! that names the track, the curve, the node, or the anchor concerned and
//! what becomes of it. [`apply_edit`] makes the change to a [`Mix`] or
//! refuses it and leaves the mix as it was, and a [`History`] applies edits
//! to the document it holds and undoes and redoes them. The window's
//! view-models turn a click or a drag into an edit and hand it to the
//! history; nothing in the window changes a document any other way, which
//! is what makes every change undoable and every view-model testable
//! without a window. `DESIGN.md` describes the editing model under "Feature
//! kernel".

use crate::anchors::Anchors;
use crate::beat_grid::BeatGrid;
use crate::envelope::{Envelope, EnvelopeNode};
use crate::mix::{Mix, Track};
use crate::tempo::TempoNode;
use crate::transition::{BLEND_LEAD_BARS, BLEND_RISE_BARS, Preset, apply as apply_preset};
use crate::units::{BEATS_PER_BAR, Beats, Bpm};

/// The quarter beat ahead of an anchor that belongs to the anchor's own
/// transition. The transition generator puts the first node of a cut there,
/// and it is the margin [`outgoing_from`] takes off the outro anchor.
const QUARTER_BEAT: Beats = Beats(0.25);

/// Eight bars, the furthest ahead of an intro anchor that a node counts as
/// part of the transition into the track when that anchor moves. It is
/// [`BLEND_LEAD_BARS`], where the blend's rise begins.
const INTRO_LEAD: Beats = Beats(BLEND_LEAD_BARS as f64 * BEATS_PER_BAR as f64);

/// Twenty-eight bars, the furthest past an intro anchor that a node counts
/// as part of the transition into the track when that anchor moves. It is
/// [`BLEND_RISE_BARS`], where the blend's rise reaches full level.
const INTRO_REACH: Beats = Beats(BLEND_RISE_BARS as f64 * BEATS_PER_BAR as f64);

/// One of a track's two anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The intro anchor, where the track joins the track before it.
    Intro,
    /// The outro anchor, where the track joins the track after it.
    Outro,
}

/// One of a track's four envelopes, which the timeline edits the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    /// The volume envelope.
    Volume,
    /// The low band of the EQ.
    Low,
    /// The middle band of the EQ.
    Mid,
    /// The high band of the EQ.
    High,
}

/// A change to a mix document.
///
/// Track positions count from zero in playlist order. Beats are beats of the
/// track's own grid, except in [`SetTempoAt`](Edit::SetTempoAt), which names
/// a mix beat because the master BPM control works on the mix's clock.
/// An edit that adds, moves, or renumbers a track's tempo nodes leaves them
/// in increasing order of beat. An edit that names a track the mix does
/// not have, a node that is not there, or a beat or level that is not a
/// finite number is refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// Moves an anchor of a track to a whole beat of its grid.
    ///
    /// The nodes belonging to that anchor's transition move with it by the
    /// same number of beats. For the outro anchor, that is every volume, EQ,
    /// and tempo node from a quarter beat before the anchor on, which is the
    /// whole of the transition out of the track. For the intro anchor, it is
    /// every node from eight bars before the beat the anchor is leaving up
    /// to and including twenty-eight bars after that same beat, but not
    /// from the beat [`outgoing_from`] names on, which belongs to the
    /// transition out. Eight bars before the anchor and twenty-eight bars
    /// after it are the extent of a blend's rise, which is the longest
    /// transition into a track that a preset writes, so every node of a
    /// blend whose rise ends before the transition out travels with the
    /// intro anchor. The fit rule of [`fits_between_the_anchors`] applies to
    /// every track that a track follows and not to the last track of a mix,
    /// so a last track whose anchors are under twenty-eight bars apart keeps
    /// the rise nodes past its outro anchor where they are when its intro
    /// anchor moves. The reach is a limit the document does not
    /// record and the presets do not enforce: a node the person placed in
    /// the body of the track stays where it sits in the music, and the last
    /// nodes of a transition drawn by hand beyond twenty-eight bars stay
    /// behind too. A moved node that lands on the exact beat of a node that
    /// did not move replaces it, as a preset's node replaces one already at
    /// its beat. A move to the beat the anchor already sits on changes
    /// nothing. The beat must be whole, and the track's outro anchor must
    /// stay after its intro anchor.
    MoveAnchor {
        /// The track's position in the playlist.
        track: usize,
        /// Which anchor.
        anchor: Anchor,
        /// The whole beat to move it to.
        to: Beats,
    },
    /// Adds a node to one of a track's curves. A node already at that beat
    /// is refused rather than replaced; the timeline moves or removes the
    /// existing node instead. A beat or level that is not a finite number
    /// is refused as an invalid node.
    AddNode {
        /// The track's position in the playlist.
        track: usize,
        /// The curve.
        curve: Curve,
        /// The node to add.
        node: EnvelopeNode,
    },
    /// Moves the node at a beat of a curve to a new beat and level. The new
    /// beat may be the old one; any other beat that already holds a node is
    /// refused, as is a new beat or level that is not a finite number.
    MoveNode {
        /// The track's position in the playlist.
        track: usize,
        /// The curve.
        curve: Curve,
        /// The beat of the node to move.
        from: Beats,
        /// Where and what it becomes.
        to: EnvelopeNode,
    },
    /// Removes the node at a beat of a curve.
    RemoveNode {
        /// The track's position in the playlist.
        track: usize,
        /// The curve.
        curve: Curve,
        /// The beat of the node to remove.
        at: Beats,
    },
    /// Adds a tempo node to a track. A node already at that beat is refused,
    /// as is a tempo that is not a positive finite number or a beat that is
    /// not finite.
    AddTempoNode {
        /// The track's position in the playlist.
        track: usize,
        /// The node to add.
        node: TempoNode,
    },
    /// Moves the tempo node at a beat of a track to a new beat and tempo, on
    /// the same terms as [`MoveNode`](Edit::MoveNode), with the tempo held
    /// to a positive finite number as [`AddTempoNode`](Edit::AddTempoNode)
    /// holds it.
    MoveTempoNode {
        /// The track's position in the playlist.
        track: usize,
        /// The beat of the node to move.
        from: Beats,
        /// Where and what it becomes.
        to: TempoNode,
    },
    /// Removes the tempo node at a beat of a track.
    RemoveTempoNode {
        /// The track's position in the playlist.
        track: usize,
        /// The beat of the node to remove.
        at: Beats,
    },
    /// Sets the mix tempo at a mix beat: a tempo node holding `bpm` is
    /// written at that beat on the track [`owner_of`] names, at the track
    /// beat that lands on the mix beat, and a node of that track already at
    /// that beat is replaced.
    /// The curve then ramps from that node to the next node and from the
    /// previous node to it, as it does between any two nodes, so a second
    /// such edit at a later beat, holding the tempo the mix should return
    /// to, ends the excursion. Removing the nodes an excursion wrote returns
    /// the curve to the ramp between the nodes that remain, which are the
    /// transitions' own unless the control wrote onto the exact beat of one
    /// of them and replaced it; undo is the way back from that. A mix with
    /// no tracks is refused, as is a tempo that is not a positive finite
    /// number.
    SetTempoAt {
        /// The mix beat.
        mix_beat: Beats,
        /// The tempo from that beat on.
        bpm: Bpm,
    },
    /// Changes the mix tempo from a mix beat on, as the master BPM control
    /// does during playback, without changing the curve before that beat.
    /// On the track [`owner_of`] names, a node is written at the mix beat
    /// holding the tempo the curve has there, unless a node of that track
    /// already sits at that beat, in which case that node is kept as it is
    /// and no pin is written. Then a node one beat later is written on the
    /// same track holding `bpm`, replacing a node of that track already at
    /// that beat. The first node pins the curve: every tempo before the mix
    /// beat is what it was, so the part of the mix a person has already
    /// heard is not changed under them. The curve ramps over the one beat
    /// between the two nodes, and from the second node it continues as any
    /// curve does, ramping toward the next node or holding `bpm` when there
    /// is none. The track's tempo nodes are left in increasing order of
    /// beat. A mix with no tracks is refused as an empty mix, a tempo that
    /// is not a positive finite number as an invalid tempo, and a mix beat
    /// that is not finite with the same error an invalid node gets.
    ChangeTempoFrom {
        /// The mix beat.
        mix_beat: Beats,
        /// The tempo from one beat later on.
        bpm: Bpm,
    },
    /// Moves a track to another position in the playlist, which is also the
    /// timeline. Every node travels with its track, and the transitions
    /// between the tracks now adjacent are whatever their nodes make.
    MoveTrack {
        /// The track's position before the move.
        from: usize,
        /// Its position after the move, counting positions in the playlist
        /// with the track removed. A position past the last one is refused
        /// as a track the mix does not have.
        to: usize,
    },
    /// Removes a track from the playlist. The tracks on either side keep
    /// their nodes, and the transition between them is whatever those nodes
    /// make.
    RemoveTrack {
        /// The track's position in the playlist.
        track: usize,
    },
    /// Inserts a track at a position from zero to the number of tracks, and
    /// joins it to its neighbors with a preset: on the track before, every
    /// node from a quarter beat before its outro anchor on is removed, on
    /// the track after every node before a quarter beat before its outro
    /// anchor is removed, and the transition generator then writes the
    /// preset between the track before and the new track and between the
    /// new track and the track after. The track before is cleared on every
    /// insert, appending to the end of the playlist included. That is where
    /// this edit parts company with `dermixen mix add`, which clears only
    /// when it inserts between two tracks. The new track's anchors
    /// must be whole beats with the outro after the intro, and a preset
    /// whose span does not fit between the anchors of the track it fades
    /// out of, whether the track before or the new track, is refused with
    /// the span and the length named. A track inserted at position zero of
    /// a mix with tracks becomes the first, and the track that was first is
    /// the track after; a track inserted into an empty mix gets no
    /// transition, since it has no neighbor.
    InsertTrack {
        /// The position the track takes.
        at: usize,
        /// The track, with its grid, anchors, and any nodes it already has.
        track: Box<Track>,
        /// The transition to write on each side.
        preset: Preset,
    },
    /// Turns a track's keylock on or off.
    SetKeylock {
        /// The track's position in the playlist.
        track: usize,
        /// Whether to keep the track's pitch when its speed changes.
        keylock: bool,
    },
    /// Replaces a track's beat grid, as the grid editor does when a person
    /// drags the grid, halves or doubles the tempo, or taps one in.
    ///
    /// The anchors and every node keep their place in the music: each is
    /// moved to the beat of the new grid that falls at the same time in the
    /// track as its beat of the old grid, and the anchors are then rounded
    /// to the nearest whole beat, with a half beat rounding away from zero.
    /// A grid coarse enough to bring two nodes onto one beat, on an envelope
    /// or in the tempo list, keeps the later of the two and discards the
    /// earlier, since the document holds no two envelope nodes at one beat.
    /// The grid's tempo must be a positive finite number.
    SetGrid {
        /// The track's position in the playlist.
        track: usize,
        /// The new grid.
        grid: BeatGrid,
    },
}

/// Why an edit was refused. The mix is unchanged when one of these comes back.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EditError {
    /// The track position is not in the playlist.
    #[error("there is no track {track}; the mix has {tracks}")]
    NoSuchTrack {
        /// The position asked for.
        track: usize,
        /// How many tracks the mix has.
        tracks: usize,
    },
    /// A track cannot be inserted at the position.
    #[error("a track cannot be inserted at position {position}; the mix has {tracks} tracks")]
    NoSuchPosition {
        /// The position asked for.
        position: usize,
        /// How many tracks the mix has.
        tracks: usize,
    },
    /// No node sits at the beat named.
    #[error("there is no node at beat {}", at.0)]
    NoSuchNode {
        /// The beat named.
        at: Beats,
    },
    /// A node already sits at the beat a node was to be added or moved to.
    #[error("there is already a node at beat {}", at.0)]
    NodeInTheWay {
        /// The beat.
        at: Beats,
    },
    /// An anchor was to be put on a beat that is not whole.
    #[error("an anchor must sit on a whole beat, not {}", beat.0)]
    NotAWholeBeat {
        /// The beat asked for.
        beat: Beats,
    },
    /// The anchors would not leave the outro after the intro.
    #[error("the outro anchor at beat {} must be after the intro anchor at beat {}", outro.0, intro.0)]
    AnchorsOutOfOrder {
        /// The intro anchor.
        intro: Beats,
        /// The outro anchor.
        outro: Beats,
    },
    /// A tempo is not one a document may contain.
    #[error("a tempo must be from {} to {} beats per minute, not {}", Bpm::LOWEST.0, Bpm::HIGHEST.0, bpm.0)]
    InvalidTempo {
        /// The tempo given.
        bpm: Bpm,
    },
    /// A node's beat or level is not a finite number.
    #[error("a node's beat and level must be finite numbers")]
    InvalidNode,
    /// A transition's span does not fit between the anchors of the track it
    /// fades out of.
    #[error("a transition of {} beats does not fit in the {} beats between the anchors", length.0, span.0)]
    TransitionDoesNotFit {
        /// The beats between the outgoing track's anchors.
        span: Beats,
        /// The beats the transition needs.
        length: Beats,
    },
    /// The edit would leave a mix that [`Mix::check`] refuses: a beat, a
    /// tempo, a level, or a length out of range, or a mix too long to render.
    #[error("{0}")]
    OutOfRange(crate::mix::MixFileError),
    /// The mix has no tracks, so there is nothing to set a tempo on.
    #[error("the mix has no tracks")]
    EmptyMix,
}

/// The track at a position, or the refusal that names the position and how
/// many tracks the mix has.
fn track_at(tracks: &mut [Track], position: usize) -> Result<&mut Track, EditError> {
    let held = tracks.len();
    tracks.get_mut(position).ok_or(EditError::NoSuchTrack {
        track: position,
        tracks: held,
    })
}

/// The envelope a curve names within a track.
fn envelope_of(track: &mut Track, curve: Curve) -> &mut Envelope {
    match curve {
        Curve::Volume => &mut track.volume,
        Curve::Low => &mut track.eq.low,
        Curve::Mid => &mut track.eq.mid,
        Curve::High => &mut track.eq.high,
    }
}

/// All four envelopes of a track, for the edits that treat them alike.
fn every_envelope(track: &mut Track) -> [&mut Envelope; 4] {
    [
        &mut track.volume,
        &mut track.eq.low,
        &mut track.eq.mid,
        &mut track.eq.high,
    ]
}

/// Whether a node's beat and level are both finite numbers.
fn is_finite(node: EnvelopeNode) -> bool {
    node.at.0.is_finite() && node.value.0.is_finite()
}

/// Where the node at a beat sits among an envelope's nodes.
fn node_at(envelope: &Envelope, at: Beats) -> Option<usize> {
    envelope.nodes().iter().position(|node| node.at.0 == at.0)
}

/// Where the tempo node at a beat sits in a track's list.
fn tempo_node_at(tempo: &[TempoNode], at: Beats) -> Option<usize> {
    tempo.iter().position(|node| node.at.0 == at.0)
}

/// Puts a track's tempo nodes in increasing order of beat, which is the
/// order every edit leaves them in.
fn sort_tempo(track: &mut Track) {
    track.tempo.sort_by(|a, b| a.at.0.total_cmp(&b.at.0));
}

/// Removes from an envelope every node except the ones whose beat `keep`
/// accepts.
fn keep_nodes(envelope: &mut Envelope, keep: impl Fn(Beats) -> bool) {
    let mut index = envelope.len();
    while index > 0 {
        index -= 1;
        if !keep(envelope.nodes()[index].at) {
            envelope.remove(index);
        }
    }
}

/// Moves by `by` beats every node of an envelope whose beat `moves`
/// accepts, and leaves the rest where they are. A moved node that lands on
/// the exact beat of a node that did not move replaces it.
///
/// A node that lands on a beat that is not a finite number is refused as an
/// invalid node. A mix [`Mix::check`] accepts never produces such a node: a
/// beat of at most [`MAX_BEAT`](crate::MAX_BEAT), moved by the distance
/// between two beats of that size, stays far inside the range of an `f64`.
fn move_nodes(
    envelope: &mut Envelope,
    moves: impl Fn(Beats) -> bool,
    by: Beats,
) -> Result<(), EditError> {
    let mut after = Envelope::new();
    for node in envelope.nodes() {
        if !moves(node.at) {
            after.insert(*node).map_err(|_| EditError::InvalidNode)?;
        }
    }
    for node in envelope.nodes() {
        if moves(node.at) {
            after
                .insert(EnvelopeNode {
                    at: node.at + by,
                    value: node.value,
                })
                .map_err(|_| EditError::InvalidNode)?;
        }
    }
    *envelope = after;
    Ok(())
}

/// Moves a track's tempo nodes by the rule [`move_nodes`] follows for an
/// envelope: `moves` chooses which nodes travel, and a moved node that lands
/// on the exact beat of a node that did not move replaces it.
fn move_tempo_nodes(tempo: &mut Vec<TempoNode>, moves: impl Fn(Beats) -> bool, by: Beats) {
    let mut after: Vec<TempoNode> = tempo
        .iter()
        .copied()
        .filter(|node| !moves(node.at))
        .collect();
    for node in tempo.iter().filter(|node| moves(node.at)) {
        let moved = TempoNode {
            at: node.at + by,
            bpm: node.bpm,
        };
        after.retain(|existing| existing.at.0 != moved.at.0);
        after.push(moved);
    }
    *tempo = after;
}

/// Moves one anchor of a track and takes the nodes of that anchor's
/// transition along with it.
fn move_anchor(
    track: &mut Track,
    index: usize,
    anchor: Anchor,
    to: Beats,
) -> Result<(), EditError> {
    if !to.is_whole() {
        return Err(EditError::NotAWholeBeat { beat: to });
    }
    // The beat is held to the document's limits before any node moves,
    // because the distance every node travels is measured from it.
    let field = match anchor {
        Anchor::Intro => "intro_beat",
        Anchor::Outro => "outro_beat",
    };
    crate::mix::check_beat(|| format!("tracks[{index}].anchors.{field}"), to)
        .map_err(EditError::OutOfRange)?;
    let anchors = track.anchors;
    let (from, moved) = match anchor {
        Anchor::Intro => (
            anchors.intro,
            Anchors {
                intro: to,
                outro: anchors.outro,
            },
        ),
        Anchor::Outro => (
            anchors.outro,
            Anchors {
                intro: anchors.intro,
                outro: to,
            },
        ),
    };
    if moved.outro.0 <= moved.intro.0 {
        return Err(EditError::AnchorsOutOfOrder {
            intro: moved.intro,
            outro: moved.outro,
        });
    }
    if to.0 == from.0 {
        return Ok(());
    }

    // A node belongs to the anchor's transition when its beat is at or after
    // `low`, at or before `high`, and before `limit`. The transition out of a
    // track runs from a quarter beat before the outro anchor to the end of
    // the track, so it has no upper bound. The transition into a track runs
    // from eight bars before the intro anchor to twenty-eight bars after it,
    // which is the blend's extent, and stops where the transition out begins.
    let transition_out = outgoing_from(anchors);
    let no_bound = Beats(f64::INFINITY);
    let (low, high, limit) = match anchor {
        Anchor::Outro => (transition_out, no_bound, no_bound),
        Anchor::Intro => (from - INTRO_LEAD, from + INTRO_REACH, transition_out),
    };
    let belongs = |at: Beats| at.0 >= low.0 && at.0 <= high.0 && at.0 < limit.0;

    let by = to - from;
    for envelope in every_envelope(track) {
        move_nodes(envelope, belongs, by)?;
    }
    move_tempo_nodes(&mut track.tempo, belongs, by);
    track.anchors = moved;
    sort_tempo(track);
    Ok(())
}

/// Puts a track on a new beat grid, keeping its anchors and every node at
/// the same time in the music.
fn set_grid(track: &mut Track, grid: BeatGrid) -> Result<(), EditError> {
    if !grid.bpm.is_valid() {
        return Err(EditError::InvalidTempo { bpm: grid.bpm });
    }
    let old = track.grid;
    // The beat of the new grid that falls at the same time in the track as
    // this beat of the old one.
    let regrid = |beat: Beats| grid.beat_at(old.time_of(beat));

    track.anchors = Anchors {
        intro: regrid(track.anchors.intro).round(),
        outro: regrid(track.anchors.outro).round(),
    };
    for envelope in every_envelope(track) {
        let mut after = Envelope::new();
        for node in envelope.nodes() {
            after
                .insert(EnvelopeNode {
                    at: regrid(node.at),
                    value: node.value,
                })
                .map_err(|_| EditError::InvalidNode)?;
        }
        *envelope = after;
    }
    // Taking the tempo nodes in beat order is what leaves the later of two
    // that land on one beat as the one that survives, which is what an
    // envelope does with the same pair.
    sort_tempo(track);
    let mut tempo: Vec<TempoNode> = Vec::with_capacity(track.tempo.len());
    for node in &track.tempo {
        let node = TempoNode {
            at: regrid(node.at),
            bpm: node.bpm,
        };
        tempo.retain(|existing| existing.at.0 != node.at.0);
        tempo.push(node);
    }
    track.tempo = tempo;
    track.grid = grid;
    sort_tempo(track);
    Ok(())
}

/// Puts a track into the playlist and writes the preset on each side of it.
fn insert_track(mix: &mut Mix, at: usize, track: &Track, preset: Preset) -> Result<(), EditError> {
    let held = mix.tracks.len();
    if at > held {
        return Err(EditError::NoSuchPosition {
            position: at,
            tracks: held,
        });
    }
    let anchors = track.anchors;
    for beat in [anchors.intro, anchors.outro] {
        if !beat.is_whole() {
            return Err(EditError::NotAWholeBeat { beat });
        }
    }
    if anchors.outro.0 <= anchors.intro.0 {
        return Err(EditError::AnchorsOutOfOrder {
            intro: anchors.intro,
            outro: anchors.outro,
        });
    }
    // The new track is held to the document's limits before a preset writes
    // anything, because a preset places its nodes from the track's anchors,
    // its grid, and its length.
    crate::mix::check_track(at, track).map_err(EditError::OutOfRange)?;
    // Each transition about to be written has an outgoing track: the track
    // before this one, and this one itself when a track follows it. Both are
    // held to the rule before any node is written, so a transition that will
    // not fit leaves the mix as it was.
    let length = span_of(preset);
    if at > 0 {
        fits_between_the_anchors(&mix.tracks[at - 1], length)?;
    }
    if at < held {
        fits_between_the_anchors(track, length)?;
    }

    // Whatever joined the neighbors comes off both of them before the two
    // new transitions go on.
    if at > 0 {
        clear_outgoing(&mut mix.tracks[at - 1]);
    }
    if at < held {
        clear_incoming(&mut mix.tracks[at]);
    }
    mix.tracks.insert(at, track.clone());
    if at > 0 {
        let (earlier, rest) = mix.tracks.split_at_mut(at);
        let outgoing = earlier
            .last_mut()
            .expect("a position past the front has a track before it");
        apply_preset(preset, outgoing, &mut rest[0]);
    }
    if at + 1 < mix.tracks.len() {
        let (through_new, later) = mix.tracks.split_at_mut(at + 1);
        let outgoing = through_new
            .last_mut()
            .expect("the track just inserted is the last of this half");
        apply_preset(preset, outgoing, &mut later[0]);
    }
    let last = mix.tracks.len() - 1;
    for index in at.saturating_sub(1)..=(at + 1).min(last) {
        sort_tempo(&mut mix.tracks[index]);
    }
    Ok(())
}

/// Makes the change `edit` describes to `mix`, or refuses it and leaves the
/// mix exactly as it was.
///
/// The change is made on a copy, which becomes the mix only once
/// [`Mix::check`] accepts it, so an edit that would put a number outside the
/// limits of a document, or make a mix longer than
/// [`LONGEST_MIX`](crate::LONGEST_MIX), comes back as
/// [`EditError::OutOfRange`] with the field named and the mix untouched. A
/// mix that [`Mix::check`] already refuses is refused the same way before the
/// edit is tried, since a document outside the limits has no layout to edit.
/// Between those two checks, no edit panics, whatever numbers it holds.
pub fn apply_edit(mix: &mut Mix, edit: &Edit) -> Result<(), EditError> {
    mix.check().map_err(EditError::OutOfRange)?;
    let mut next = mix.clone();
    apply_to(&mut next, edit)?;
    next.check().map_err(EditError::OutOfRange)?;
    *mix = next;
    Ok(())
}

/// Makes the change `edit` describes, leaving the mix as far along as the
/// change got when it is refused. [`apply_edit`] is what callers use: it
/// works on a copy, so a refusal here never reaches the caller's mix.
fn apply_to(mix: &mut Mix, edit: &Edit) -> Result<(), EditError> {
    match edit {
        Edit::MoveAnchor { track, anchor, to } => {
            move_anchor(track_at(&mut mix.tracks, *track)?, *track, *anchor, *to)
        }
        Edit::AddNode { track, curve, node } => {
            let node = *node;
            let track = track_at(&mut mix.tracks, *track)?;
            if !is_finite(node) {
                return Err(EditError::InvalidNode);
            }
            let envelope = envelope_of(track, *curve);
            if node_at(envelope, node.at).is_some() {
                return Err(EditError::NodeInTheWay { at: node.at });
            }
            envelope
                .insert(node)
                .expect("the node was found finite above");
            Ok(())
        }
        Edit::MoveNode {
            track,
            curve,
            from,
            to,
        } => {
            let (from, to) = (*from, *to);
            let track = track_at(&mut mix.tracks, *track)?;
            if !is_finite(to) {
                return Err(EditError::InvalidNode);
            }
            let envelope = envelope_of(track, *curve);
            let index = node_at(envelope, from).ok_or(EditError::NoSuchNode { at: from })?;
            if to.at.0 != from.0 && node_at(envelope, to.at).is_some() {
                return Err(EditError::NodeInTheWay { at: to.at });
            }
            envelope.remove(index);
            envelope
                .insert(to)
                .expect("the node was found finite above");
            Ok(())
        }
        Edit::RemoveNode { track, curve, at } => {
            let at = *at;
            let track = track_at(&mut mix.tracks, *track)?;
            let envelope = envelope_of(track, *curve);
            let index = node_at(envelope, at).ok_or(EditError::NoSuchNode { at })?;
            envelope.remove(index);
            Ok(())
        }
        Edit::AddTempoNode { track, node } => {
            let node = *node;
            let track = track_at(&mut mix.tracks, *track)?;
            if !node.bpm.is_valid() {
                return Err(EditError::InvalidTempo { bpm: node.bpm });
            }
            if !node.at.0.is_finite() {
                return Err(EditError::InvalidNode);
            }
            if tempo_node_at(&track.tempo, node.at).is_some() {
                return Err(EditError::NodeInTheWay { at: node.at });
            }
            track.tempo.push(node);
            sort_tempo(track);
            Ok(())
        }
        Edit::MoveTempoNode { track, from, to } => {
            let (from, to) = (*from, *to);
            let track = track_at(&mut mix.tracks, *track)?;
            if !to.bpm.is_valid() {
                return Err(EditError::InvalidTempo { bpm: to.bpm });
            }
            if !to.at.0.is_finite() {
                return Err(EditError::InvalidNode);
            }
            let index =
                tempo_node_at(&track.tempo, from).ok_or(EditError::NoSuchNode { at: from })?;
            if to.at.0 != from.0 && tempo_node_at(&track.tempo, to.at).is_some() {
                return Err(EditError::NodeInTheWay { at: to.at });
            }
            track.tempo[index] = to;
            sort_tempo(track);
            Ok(())
        }
        Edit::RemoveTempoNode { track, at } => {
            let at = *at;
            let track = track_at(&mut mix.tracks, *track)?;
            let index = tempo_node_at(&track.tempo, at).ok_or(EditError::NoSuchNode { at })?;
            track.tempo.remove(index);
            Ok(())
        }
        Edit::ChangeTempoFrom { mix_beat, bpm } => {
            let (mix_beat, bpm) = (*mix_beat, *bpm);
            if !bpm.is_valid() {
                return Err(EditError::InvalidTempo { bpm });
            }
            if !mix_beat.0.is_finite() {
                return Err(EditError::InvalidNode);
            }
            // The pin holds the tempo the curve already has at the mix
            // beat. Between two nodes the curve is a straight line in time,
            // so a node placed on the line the curve already follows
            // preserves every tempo before it to within floating-point
            // rounding, and the part of the mix a person has already heard
            // does not change.
            let held = mix
                .timeline()
                .ok_or(EditError::EmptyMix)?
                .curve
                .bpm_at_beat(mix_beat);
            let (index, at) = owner_of(mix, mix_beat).ok_or(EditError::EmptyMix)?;
            let track = &mut mix.tracks[index];
            // A node of this track already at the beat is kept as it is
            // and serves as the pin. It is the tempo heard at that beat,
            // since the later of two nodes at one mix beat is the one that
            // takes effect there, so leaving it alone holds the earlier
            // tempos just as a written pin would.
            if tempo_node_at(&track.tempo, at).is_none() {
                track.tempo.push(TempoNode { at, bpm: held });
            }
            let after = at + Beats(1.0);
            let node = TempoNode { at: after, bpm };
            match tempo_node_at(&track.tempo, after) {
                Some(index) => track.tempo[index] = node,
                None => track.tempo.push(node),
            }
            sort_tempo(track);
            Ok(())
        }
        Edit::SetTempoAt { mix_beat, bpm } => {
            let (mix_beat, bpm) = (*mix_beat, *bpm);
            if !bpm.is_valid() {
                return Err(EditError::InvalidTempo { bpm });
            }
            if !mix_beat.0.is_finite() {
                return Err(EditError::InvalidNode);
            }
            let (index, at) = owner_of(mix, mix_beat).ok_or(EditError::EmptyMix)?;
            let track = &mut mix.tracks[index];
            let node = TempoNode { at, bpm };
            match tempo_node_at(&track.tempo, at) {
                Some(index) => track.tempo[index] = node,
                None => track.tempo.push(node),
            }
            sort_tempo(track);
            Ok(())
        }
        Edit::MoveTrack { from, to } => {
            let held = mix.tracks.len();
            for position in [*from, *to] {
                if position >= held {
                    return Err(EditError::NoSuchTrack {
                        track: position,
                        tracks: held,
                    });
                }
            }
            let track = mix.tracks.remove(*from);
            mix.tracks.insert(*to, track);
            Ok(())
        }
        Edit::RemoveTrack { track } => {
            let held = mix.tracks.len();
            if *track >= held {
                return Err(EditError::NoSuchTrack {
                    track: *track,
                    tracks: held,
                });
            }
            mix.tracks.remove(*track);
            Ok(())
        }
        Edit::InsertTrack { at, track, preset } => insert_track(mix, *at, track, *preset),
        Edit::SetKeylock { track, keylock } => {
            track_at(&mut mix.tracks, *track)?.keylock = *keylock;
            Ok(())
        }
        Edit::SetGrid { track, grid } => set_grid(track_at(&mut mix.tracks, *track)?, *grid),
    }
}

/// How many edits a [`History`] can undo. Every one of them costs a copy of
/// the document, which is why the number is not larger; going back further
/// than two hundred edits is what reopening the last saved file is for.
pub const HISTORY_DEPTH: usize = 200;

/// A document with its history of edits.
///
/// Every edit goes through [`apply`](History::apply), which either changes
/// the document or refuses the edit and records nothing. [`undo`] restores
/// the document exactly as it was before the last applied edit, and
/// [`redo`] applies that edit again; a new edit after an undo discards what
/// could have been redone, and a refused edit discards nothing. An undone
/// document equals the earlier one in every field. The history does not
/// report which edits it applied: the view-model that builds an edit is
/// what records a correction for the ground truth, at the moment it builds
/// the edit. The last [`HISTORY_DEPTH`] edits are the ones that can be
/// undone; applying one more than that drops the oldest document the
/// history holds.
///
/// [`undo`]: History::undo
/// [`redo`]: History::redo
#[derive(Debug, Clone)]
pub struct History {
    /// The document as it is now.
    mix: Mix,
    /// The document as it stood before each applied edit, the oldest first.
    /// A grid change rounds anchors, so an edit cannot be undone by working
    /// out its opposite; keeping a whole document per edit is what makes
    /// undo exact. The list holds at most [`HISTORY_DEPTH`] documents, and
    /// the oldest one goes when a new edit would make it longer.
    past: Vec<Mix>,
    /// The document as it stood after each undone edit, the most recently
    /// undone last.
    future: Vec<Mix>,
}

impl History {
    /// A history holding `mix` with nothing to undo or redo.
    pub fn new(mix: Mix) -> History {
        History {
            mix,
            past: Vec::new(),
            future: Vec::new(),
        }
    }

    /// The document as it is now.
    pub fn mix(&self) -> &Mix {
        &self.mix
    }

    /// Applies an edit to the document, or refuses it and changes nothing,
    /// not even what can be redone.
    pub fn apply(&mut self, edit: Edit) -> Result<(), EditError> {
        // The change is made on a copy of the document, so a refusal leaves
        // the document itself untouched however far the change had got.
        let mut next = self.mix.clone();
        apply_edit(&mut next, &edit)?;
        let before = std::mem::replace(&mut self.mix, next);
        self.past.push(before);
        if self.past.len() > HISTORY_DEPTH {
            self.past.remove(0);
        }
        self.future.clear();
        Ok(())
    }

    /// Restores the document as it was before the last applied edit and
    /// returns true, or returns false when nothing has been applied that
    /// has not been undone.
    pub fn undo(&mut self) -> bool {
        let Some(before) = self.past.pop() else {
            return false;
        };
        let after = std::mem::replace(&mut self.mix, before);
        self.future.push(after);
        true
    }

    /// Applies the last undone edit again and returns true, or returns
    /// false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(after) = self.future.pop() else {
            return false;
        };
        let before = std::mem::replace(&mut self.mix, after);
        self.past.push(before);
        true
    }

    /// Whether [`undo`](History::undo) would do anything.
    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    /// Whether [`redo`](History::redo) would do anything.
    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }
}

/// A quarter beat before a track's outro anchor, which is the earliest beat
/// the transition out of the track reaches: for a cut the transition
/// generator puts the first outgoing node exactly there, and for a
/// crossfade it puts the outgoing nodes from the anchor on. Everything at
/// or after this beat is the transition out of the track, and everything
/// before it is the transition into the track and whatever a person placed
/// by hand ahead of the outro anchor. [`Edit::InsertTrack`] clears nodes by
/// this rule on every insert and `dermixen mix add` when it inserts between
/// two tracks, and [`Edit::MoveAnchor`] moves them by it.
pub fn outgoing_from(anchors: Anchors) -> Beats {
    anchors.outro - QUARTER_BEAT
}

/// Removes from a track every volume, EQ, and tempo node at or after the
/// beat [`outgoing_from`] names, which is the whole of the transition out
/// of the track, so that a new transition can be written in its place.
pub fn clear_outgoing(track: &mut Track) {
    let from = outgoing_from(track.anchors);
    track.tempo.retain(|node| node.at.0 < from.0);
    for envelope in every_envelope(track) {
        keep_nodes(envelope, |at| at.0 < from.0);
    }
}

/// Removes from a track every volume, EQ, and tempo node before the beat
/// [`outgoing_from`] names, which is the transition into the track and
/// anything else placed ahead of its outro anchor, so that a new transition
/// can be written in its place.
pub fn clear_incoming(track: &mut Track) {
    let from = outgoing_from(track.anchors);
    track.tempo.retain(|node| node.at.0 >= from.0);
    for envelope in every_envelope(track) {
        keep_nodes(envelope, |at| at.0 >= from.0);
    }
}

/// How many beats of a track the nodes of a preset cover after the anchor
/// they start at: the preset's bars for a crossfade, the twenty-eight bars
/// of the incoming track's rise for a blend, and a quarter beat for a cut,
/// where the transition generator writes one node a quarter beat before the
/// anchor and one at it. The nodes a blend writes ahead of the intro anchor
/// and the outgoing track's ease to its last sample are not counted: the
/// span is what must fit between a track's own two anchors.
pub fn span_of(preset: Preset) -> Beats {
    match preset {
        Preset::Blend => Beats::from_bars(f64::from(BLEND_RISE_BARS)),
        Preset::Beatmix { bars } | Preset::BassSwap { bars } => Beats::from_bars(f64::from(bars)),
        Preset::Cut => QUARTER_BEAT,
    }
}

/// Refuses a transition whose span is longer than the beats between the
/// outgoing track's own two anchors, with the span and the length named,
/// because the track would otherwise fade in and fade out at the same time.
/// `dermixen mix add` reports the same refusal with the track's file named
/// as well.
pub fn fits_between_the_anchors(outgoing: &Track, length: Beats) -> Result<(), EditError> {
    let span = outgoing.anchors.span();
    if length.0 > span.0 {
        return Err(EditError::TransitionDoesNotFit { span, length });
    }
    Ok(())
}

/// The track a mix beat belongs to, and that beat as a beat of the track:
/// the track whose beat zero falls latest at or before the mix beat, or the
/// first track when the mix beat is before every track's beat zero. A mix
/// with no tracks owns no beat. This is the track the master BPM control
/// writes on, and the track a click on the tempo curve edits.
pub fn owner_of(mix: &Mix, mix_beat: Beats) -> Option<(usize, Beats)> {
    if mix.tracks.is_empty() {
        return None;
    }
    // Each track's beat zero sits where the layout puts it: the first at mix
    // beat zero, and every later one at the previous track's outro anchor
    // less its own intro anchor. The mix beat belongs to the last track to
    // have begun by then, and to the first track when none has.
    let mut origin = Beats::ZERO;
    let mut owner = (0, Beats::ZERO);
    let mut begun = false;
    for (index, track) in mix.tracks.iter().enumerate() {
        if index > 0 {
            origin += mix.tracks[index - 1].anchors.outro - track.anchors.intro;
        }
        if origin.0 <= mix_beat.0 && (!begun || origin.0 >= owner.1.0) {
            owner = (index, origin);
            begun = true;
        }
    }
    Some((owner.0, mix_beat - owner.1))
}
