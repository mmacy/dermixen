//! The intro and outro anchors that decide where one track overlaps the next.

use serde::{Deserialize, Serialize};

use crate::units::Beats;

/// Where a track joins its neighboring tracks on the timeline.
///
/// Both anchors are whole beats within the track's own grid. On the timeline
/// the outro anchor of each track sits at the same mix beat as the intro
/// anchor of the track after it; that single alignment is what places every
/// track, and the overlap between two tracks follows from it. An anchor may
/// lie before the track's first beat or after its last, which is how a gap of
/// silence between two tracks is expressed.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchors {
    /// The beat the previous track's outro anchor lines up with.
    #[serde(rename = "intro_beat")]
    pub intro: Beats,
    /// The beat the next track's intro anchor lines up with.
    #[serde(rename = "outro_beat")]
    pub outro: Beats,
}

impl Anchors {
    /// The distance from the intro anchor to the outro anchor.
    ///
    /// Because the next track's intro anchor lands on this track's outro
    /// anchor, this is also how far the timeline moves from this track's
    /// intro anchor to the next track's intro anchor.
    pub fn span(&self) -> Beats {
        self.outro - self.intro
    }
}
