//! Unit types for positions, durations, tempos, and gains.
//!
//! Each quantity has its own type so that mixing them up is a compile error
//! rather than a bug found by ear. The inner values are public because the
//! types are plain wrappers with no invariants; validation happens where a
//! value enters the system, such as when a project file is parsed.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// The one sample rate every buffer in Dermixen uses, in samples per second.
pub const SAMPLE_RATE: u32 = 44_100;

/// The number of beats in a bar. Dermixen assumes four-four time throughout.
pub const BEATS_PER_BAR: u32 = 4;

/// A duration or position in seconds.
///
/// Fractional values are normal; positions become whole samples only when a
/// buffer is read or written.
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seconds(pub f64);

/// A duration or position counted in samples at [`SAMPLE_RATE`].
///
/// The count is signed so that positions before a reference point, such as
/// the audio before a track's first beat, can be expressed.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Samples(pub i64);

/// A count of beats, which may be fractional.
///
/// Within a track, beat zero is the first beat of the track's grid and beats
/// before it are negative. On the mix timeline, beat zero is the first beat of
/// the first track.
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Beats(pub f64);

/// A tempo in beats per minute.
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Bpm(pub f64);

/// A gain in decibels, where zero leaves the signal unchanged.
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Decibels(pub f64);

/// A loudness in loudness units relative to full scale (LUFS), as EBU R 128
/// measures it: the level of the whole signal after K-weighting and gating,
/// where a stereo 1 kHz sine at minus twenty-three decibels below full scale
/// in both channels measures minus twenty-three.
///
/// A loudness is an absolute level and a [`Decibels`] is a change, so the
/// difference between two loudnesses is a gain and a loudness plus a gain is
/// a loudness, and nothing else is defined.
#[derive(Copy, Clone, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Lufs(pub f64);

impl Sub for Lufs {
    type Output = Decibels;
    fn sub(self, rhs: Lufs) -> Decibels {
        Decibels(self.0 - rhs.0)
    }
}

impl Add<Decibels> for Lufs {
    type Output = Lufs;
    fn add(self, rhs: Decibels) -> Lufs {
        Lufs(self.0 + rhs.0)
    }
}

impl Seconds {
    /// Zero seconds.
    pub const ZERO: Seconds = Seconds(0.0);

    /// Converts to a whole number of samples at [`SAMPLE_RATE`], rounding to the nearest sample.
    pub fn to_samples(self) -> Samples {
        Samples((self.0 * f64::from(SAMPLE_RATE)).round() as i64)
    }

    /// The number of beats this duration spans at the given tempo.
    pub fn beats_at(self, bpm: Bpm) -> Beats {
        Beats(self.0 * bpm.0 / 60.0)
    }
}

impl Samples {
    /// Zero samples.
    pub const ZERO: Samples = Samples(0);

    /// Converts to seconds at [`SAMPLE_RATE`].
    pub fn to_seconds(self) -> Seconds {
        Seconds(self.0 as f64 / f64::from(SAMPLE_RATE))
    }
}

impl Beats {
    /// Zero beats.
    pub const ZERO: Beats = Beats(0.0);

    /// One beat.
    pub const ONE: Beats = Beats(1.0);

    /// One bar, which is [`BEATS_PER_BAR`] beats.
    pub const BAR: Beats = Beats(BEATS_PER_BAR as f64);

    /// Builds a beat count from a count of bars.
    pub fn from_bars(bars: f64) -> Beats {
        Beats(bars * f64::from(BEATS_PER_BAR))
    }

    /// The number of bars this beat count spans, which may be fractional.
    pub fn bars(self) -> f64 {
        self.0 / f64::from(BEATS_PER_BAR)
    }

    /// The duration of this many beats at the given tempo.
    pub fn at(self, bpm: Bpm) -> Seconds {
        Seconds(self.0 * 60.0 / bpm.0)
    }

    /// Rounds to the nearest whole beat.
    pub fn round(self) -> Beats {
        Beats(self.0.round())
    }

    /// Whether this count is a whole number of beats.
    pub fn is_whole(self) -> bool {
        self.0.is_finite() && self.0.fract() == 0.0
    }
}

impl Bpm {
    /// The duration of one beat at this tempo.
    pub fn beat_period(self) -> Seconds {
        Seconds(60.0 / self.0)
    }

    /// The lowest tempo a document, an edit, a command, or the window accepts.
    pub const LOWEST: Bpm = Bpm(20.0);

    /// The highest tempo a document, an edit, a command, or the window accepts.
    pub const HIGHEST: Bpm = Bpm(999.0);

    /// Whether this is a usable tempo: a number from [`Bpm::LOWEST`] to
    /// [`Bpm::HIGHEST`].
    ///
    /// The range is what keeps a render finite. A tempo near zero makes a mix
    /// of any length, and the stretcher's start-up work grows as the tempo
    /// falls. A tempo that is not a number fails both comparisons and so is
    /// not valid.
    pub fn is_valid(self) -> bool {
        self.0 >= Self::LOWEST.0 && self.0 <= Self::HIGHEST.0
    }
}

impl Decibels {
    /// No change in level.
    pub const UNITY: Decibels = Decibels(0.0);

    /// The lowest gain or envelope level a document may contain. Every level
    /// at or below [`Decibels::SILENCE`] is silent, so the limit only keeps
    /// the number finite and small.
    pub const LOWEST_LEVEL: Decibels = Decibels(-144.0);

    /// The highest gain or envelope level a document may contain.
    pub const HIGHEST_LEVEL: Decibels = Decibels(24.0);

    /// Whether this is a level a document may contain: a number from
    /// [`Decibels::LOWEST_LEVEL`] to [`Decibels::HIGHEST_LEVEL`].
    pub fn is_level(self) -> bool {
        self.0 >= Self::LOWEST_LEVEL.0 && self.0 <= Self::HIGHEST_LEVEL.0
    }

    /// The level at or below which a signal is treated as silent.
    ///
    /// [`Decibels::to_linear`] returns exactly zero at or below this level, so
    /// a track faded to the floor contributes nothing to the mix rather than an
    /// inaudible residue.
    pub const SILENCE: Decibels = Decibels(-90.0);

    /// The linear amplitude multiplier for this gain, or exactly zero at or below [`Decibels::SILENCE`].
    pub fn to_linear(self) -> f64 {
        if self.0 <= Self::SILENCE.0 {
            0.0
        } else {
            10f64.powf(self.0 / 20.0)
        }
    }

    /// The gain that multiplies amplitude by `linear`. Zero or negative input gives [`Decibels::SILENCE`].
    pub fn from_linear(linear: f64) -> Decibels {
        if linear <= 0.0 {
            Self::SILENCE
        } else {
            Decibels((20.0 * linear.log10()).max(Self::SILENCE.0))
        }
    }
}

macro_rules! float_arithmetic {
    ($t:ident) => {
        impl Add for $t {
            type Output = $t;
            fn add(self, rhs: $t) -> $t {
                $t(self.0 + rhs.0)
            }
        }
        impl Sub for $t {
            type Output = $t;
            fn sub(self, rhs: $t) -> $t {
                $t(self.0 - rhs.0)
            }
        }
        impl AddAssign for $t {
            fn add_assign(&mut self, rhs: $t) {
                self.0 += rhs.0;
            }
        }
        impl SubAssign for $t {
            fn sub_assign(&mut self, rhs: $t) {
                self.0 -= rhs.0;
            }
        }
        impl Mul<f64> for $t {
            type Output = $t;
            fn mul(self, rhs: f64) -> $t {
                $t(self.0 * rhs)
            }
        }
        impl Div<f64> for $t {
            type Output = $t;
            fn div(self, rhs: f64) -> $t {
                $t(self.0 / rhs)
            }
        }
        impl Neg for $t {
            type Output = $t;
            fn neg(self) -> $t {
                $t(-self.0)
            }
        }
    };
}

float_arithmetic!(Seconds);
float_arithmetic!(Beats);
float_arithmetic!(Decibels);

impl Add for Samples {
    type Output = Samples;
    fn add(self, rhs: Samples) -> Samples {
        Samples(self.0 + rhs.0)
    }
}

impl Sub for Samples {
    type Output = Samples;
    fn sub(self, rhs: Samples) -> Samples {
        Samples(self.0 - rhs.0)
    }
}

impl AddAssign for Samples {
    fn add_assign(&mut self, rhs: Samples) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Samples {
    fn sub_assign(&mut self, rhs: Samples) {
        self.0 -= rhs.0;
    }
}

impl Neg for Samples {
    type Output = Samples;
    fn neg(self) -> Samples {
        Samples(-self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_second_is_the_sample_rate() {
        assert_eq!(Seconds(1.0).to_samples(), Samples(44_100));
        assert_eq!(Samples(44_100).to_seconds(), Seconds(1.0));
    }

    #[test]
    fn samples_round_to_nearest() {
        assert_eq!(Seconds(0.5 / 44_100.0).to_samples(), Samples(1));
        assert_eq!(Seconds(0.49 / 44_100.0).to_samples(), Samples(0));
        assert_eq!(Seconds(-1.5 / 44_100.0).to_samples(), Samples(-2));
    }

    #[test]
    fn beats_and_seconds_agree_at_a_tempo() {
        let bpm = Bpm(120.0);
        assert_eq!(Beats(2.0).at(bpm), Seconds(1.0));
        assert_eq!(Seconds(1.0).beats_at(bpm), Beats(2.0));
        assert_eq!(bpm.beat_period(), Seconds(0.5));
    }

    #[test]
    fn bars_hold_four_beats() {
        assert_eq!(Beats::from_bars(2.0), Beats(8.0));
        assert_eq!(Beats(6.0).bars(), 1.5);
        assert_eq!(Beats::BAR, Beats(4.0));
    }

    #[test]
    fn whole_beats_are_recognised() {
        assert!(Beats(3.0).is_whole());
        assert!(Beats(-3.0).is_whole());
        assert!(!Beats(3.5).is_whole());
        assert!(!Beats(f64::NAN).is_whole());
        assert_eq!(Beats(3.4).round(), Beats(3.0));
        assert_eq!(Beats(3.5).round(), Beats(4.0));
    }

    #[test]
    fn tempo_validity() {
        assert!(Bpm(138.0).is_valid());
        assert!(!Bpm(0.0).is_valid());
        assert!(!Bpm(-1.0).is_valid());
        assert!(!Bpm(f64::NAN).is_valid());
        assert!(!Bpm(f64::INFINITY).is_valid());
    }

    #[test]
    fn decibels_convert_to_linear_gain() {
        assert_eq!(Decibels::UNITY.to_linear(), 1.0);
        assert!((Decibels(-6.0206).to_linear() - 0.5).abs() < 1e-4);
        assert!((Decibels(20.0).to_linear() - 10.0).abs() < 1e-12);
        assert_eq!(Decibels::SILENCE.to_linear(), 0.0);
        assert_eq!(Decibels(-200.0).to_linear(), 0.0);
    }

    #[test]
    fn decibels_convert_from_linear_gain() {
        assert_eq!(Decibels::from_linear(1.0), Decibels::UNITY);
        assert!((Decibels::from_linear(10.0).0 - 20.0).abs() < 1e-12);
        assert_eq!(Decibels::from_linear(0.0), Decibels::SILENCE);
        assert_eq!(Decibels::from_linear(-1.0), Decibels::SILENCE);
        assert_eq!(Decibels::from_linear(1e-9), Decibels::SILENCE);
    }

    #[test]
    fn arithmetic_stays_in_type() {
        assert_eq!(Seconds(1.5) + Seconds(0.5), Seconds(2.0));
        assert_eq!(Seconds(1.5) - Seconds(0.5), Seconds(1.0));
        assert_eq!(Seconds(1.5) * 2.0, Seconds(3.0));
        assert_eq!(Seconds(3.0) / 2.0, Seconds(1.5));
        assert_eq!(-Seconds(1.0), Seconds(-1.0));
        assert_eq!(Beats(3.0) + Beats(1.0), Beats(4.0));
        assert_eq!(Decibels(-3.0) + Decibels(-3.0), Decibels(-6.0));
        assert_eq!(Samples(10) - Samples(15), Samples(-5));
        let mut s = Samples(1);
        s += Samples(2);
        assert_eq!(s, Samples(3));
    }

    #[test]
    fn units_serialize_as_bare_numbers() {
        assert_eq!(serde_json::to_string(&Seconds(1.5)).unwrap(), "1.5");
        assert_eq!(serde_json::to_string(&Samples(44100)).unwrap(), "44100");
        assert_eq!(serde_json::to_string(&Beats(64.0)).unwrap(), "64.0");
        assert_eq!(serde_json::to_string(&Bpm(138.0)).unwrap(), "138.0");
        assert_eq!(serde_json::to_string(&Decibels(-6.0)).unwrap(), "-6.0");
        assert_eq!(serde_json::from_str::<Samples>("12").unwrap(), Samples(12));
        assert!(serde_json::from_str::<Samples>("12.5").is_err());
    }
}
