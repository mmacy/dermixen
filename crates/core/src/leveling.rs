//! Volume leveling: the gain that brings a track to the loudness every
//! track of a mix is played at.
//!
//! `DESIGN.md` asks for volume leveling across tracks by EBU R 128
//! loudness, so that quiet masters and loud masters sit at the same
//! perceived level. The analysis crate measures each track once, the
//! library keeps the measurement, and the gain is written into the mix
//! document when the track joins it, as the track's `gain_db`. The render
//! applies that gain on top of the track's volume envelope. Writing the
//! gain into the document is what keeps a render a function of the
//! document alone, so a mix renders the same whatever the library index
//! contains by then.

use crate::units::{Decibels, Lufs};

/// The loudness every leveled track is brought to.
///
/// Dance masters from the 1990s sit around minus twelve and modern ones
/// around minus eight, so nearly every track is turned down a little and
/// no track is turned up far, which keeps the sum of two tracks across a
/// transition out of clipping. The final level of the mix belongs to the
/// mastering that follows the render, outside the app, as `DESIGN.md` says
/// under "Non-goals".
pub const TARGET_LOUDNESS: Lufs = Lufs(-14.0);

/// The true peak no leveled track is pushed above.
///
/// A quiet master with sharp peaks would clip if it were raised the whole
/// way to the target, so its gain stops where its true peak reaches this
/// ceiling.
pub const TRUE_PEAK_CEILING: Decibels = Decibels(-1.0);

/// The gain that brings a track of loudness `integrated` to
/// [`TARGET_LOUDNESS`], reduced as far as is needed to keep the track's
/// `true_peak` at or below [`TRUE_PEAK_CEILING`] after the gain.
///
/// The gain is the smaller of the two differences: the target minus the
/// loudness, and the ceiling minus the true peak. A loud track is therefore
/// turned down by exactly what brings it to the target, since turning down
/// never lifts a peak, and a quiet track is turned up by that amount or by
/// as much as its peaks allow, whichever is less.
///
/// The result is always a level a mix document may contain, so the gain can
/// be written into a document whatever the measurement was. A measurement
/// that is not a finite number, which a float file can produce, gets
/// [`Decibels::UNITY`], and a gain the two differences put outside the range
/// from [`Decibels::LOWEST_LEVEL`] to [`Decibels::HIGHEST_LEVEL`] stops at
/// the end of that range it passed.
pub fn leveling_gain(integrated: Lufs, true_peak: Decibels) -> Decibels {
    if !integrated.0.is_finite() || !true_peak.0.is_finite() {
        return Decibels::UNITY;
    }
    let to_target = TARGET_LOUDNESS - integrated;
    let to_ceiling = TRUE_PEAK_CEILING - true_peak;
    let gain = to_target.0.min(to_ceiling.0);
    Decibels(gain.clamp(Decibels::LOWEST_LEVEL.0, Decibels::HIGHEST_LEVEL.0))
}
