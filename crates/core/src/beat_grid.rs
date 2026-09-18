//! A track's beat grid: a constant tempo and the position of its first beat.

use serde::{Deserialize, Serialize};

use crate::units::{BEATS_PER_BAR, Beats, Bpm, Samples, Seconds};

/// The beat grid of one track.
///
/// Every track is assumed to hold one constant tempo, so a grid is fully
/// described by where its first beat falls and how fast the beats come. Beat
/// zero is the first beat and is treated as the start of a bar; beats before
/// it have negative indices. Positions within a track are measured from the
/// first sample of the audio file.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeatGrid {
    /// The position of beat zero within the track.
    #[serde(rename = "first_beat_sample")]
    pub first_beat: Samples,
    /// The track's original tempo, which analysis finds once and which never changes.
    pub bpm: Bpm,
}

impl BeatGrid {
    /// The duration of one beat.
    pub fn beat_period(&self) -> Seconds {
        self.bpm.beat_period()
    }

    /// The fractional beat index at a track time.
    ///
    /// The result is zero at `first_beat`, negative before it, and increases by
    /// one every beat period.
    pub fn beat_at(&self, time: Seconds) -> Beats {
        (time - self.first_beat.to_seconds()).beats_at(self.bpm)
    }

    /// The track time of a beat index, which may be fractional.
    pub fn time_of(&self, beat: Beats) -> Seconds {
        self.first_beat.to_seconds() + beat.at(self.bpm)
    }

    /// The fractional beat index at a track position.
    pub fn beat_at_position(&self, position: Samples) -> Beats {
        self.beat_at(position.to_seconds())
    }

    /// The track position of a beat index, rounded to the nearest sample.
    pub fn position_of(&self, beat: Beats) -> Samples {
        self.time_of(beat).to_samples()
    }

    /// The whole beat nearest to a track time.
    pub fn nearest_beat(&self, time: Seconds) -> Beats {
        self.beat_at(time).round()
    }

    /// The bar start nearest to a track time, as a beat index that is a whole multiple of [`BEATS_PER_BAR`].
    pub fn nearest_bar(&self, time: Seconds) -> Beats {
        let beats_per_bar = f64::from(BEATS_PER_BAR);
        let bar_at_time = self.beat_at(time).0 / beats_per_bar;
        Beats(bar_at_time.round() * beats_per_bar)
    }
}
