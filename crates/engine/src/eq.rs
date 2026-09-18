//! The per-track three-band EQ: a DJ mixer's channel strip and nothing more.

use std::f64::consts::PI;

use dermixen_core::{Decibels, SAMPLE_RATE};
use dermixen_media::Frame;

/// The frequency in hertz below which the low band runs.
pub const LOW_CROSSOVER_HZ: f64 = 250.0;

/// The frequency in hertz above which the high band runs. The mid band lies between the two crossovers.
pub const HIGH_CROSSOVER_HZ: f64 = 2500.0;

/// The number of channels one [`Frame`] holds. Dermixen audio is always stereo.
const CHANNELS: usize = 2;

/// The delay registers are flushed to exact zero once both fall below this
/// magnitude.
///
/// A biquad fed silence decays its state toward zero exponentially and never
/// quite reaches it. Left alone, the state eventually enters the range of
/// subnormal (denormal) floating-point numbers, and on Intel processors
/// arithmetic on subnormal values falls back to a much slower microcode path;
/// a render or a live preview that goes on to hit a long silent passage in a
/// track would then run dramatically slower than the same filter fed sound.
/// The threshold sits far below anything audible (silence in this crate's
/// own gain scale, [`Decibels::SILENCE`], is already many orders of magnitude
/// larger than this) and far above the subnormal boundary, so flushing here
/// changes nothing a listener could hear while keeping every section's
/// arithmetic on the normal, fast path.
const DENORMAL_FLOOR: f64 = 1e-20;

/// One second-order section in transposed direct form II, with independent
/// state per channel so one instance can filter a whole stereo stream.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    /// The two delay registers, one pair per channel.
    state: [[f64; 2]; CHANNELS],
}

impl Biquad {
    /// A Butterworth low-pass section at `cutoff_hz` with quality factor `q`,
    /// built from Robert Bristow-Johnson's audio filter cookbook's
    /// bilinear-transformed formula so that it lines up exactly with
    /// [`Self::high_pass`] at the same cutoff.
    fn low_pass(cutoff_hz: f64, sample_rate_hz: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate_hz;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) / 2.0) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            state: [[0.0; 2]; CHANNELS],
        }
    }

    /// A Butterworth high-pass section at `cutoff_hz` with quality factor `q`.
    fn high_pass(cutoff_hz: f64, sample_rate_hz: f64, q: f64) -> Self {
        let w0 = 2.0 * PI * cutoff_hz / sample_rate_hz;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cos_w0) / 2.0) / a0,
            b1: (-(1.0 + cos_w0)) / a0,
            b2: ((1.0 + cos_w0) / 2.0) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            state: [[0.0; 2]; CHANNELS],
        }
    }

    /// Filters one sample of `channel`, updating that channel's state.
    fn process(&mut self, x: f64, channel: usize) -> f64 {
        let [s1, s2] = self.state[channel];
        let y = self.b0 * x + s1;
        let mut next1 = self.b1 * x - self.a1 * y + s2;
        let mut next2 = self.b2 * x - self.a2 * y;
        if next1.abs() < DENORMAL_FLOOR && next2.abs() < DENORMAL_FLOOR {
            next1 = 0.0;
            next2 = 0.0;
        }
        self.state[channel] = [next1, next2];
        y
    }

    /// Clears the delay registers of both channels.
    fn reset(&mut self) {
        self.state = [[0.0; 2]; CHANNELS];
    }
}

/// The two quality factors that turn a pair of Butterworth low-pass or
/// high-pass sections into a fourth-order Butterworth filter. Running that
/// fourth-order filter twice in series (an eighth-order Linkwitz-Riley
/// crossover) is what gives a crossover split whose two sides sum back to
/// the original signal's level at every frequency: that identity is the
/// classic reason loudspeaker crossovers are built from Linkwitz-Riley
/// filters rather than plain Butterworth ones.
fn fourth_order_butterworth_qs() -> [f64; 2] {
    [
        1.0 / (2.0 * (PI / 8.0).cos()),
        1.0 / (2.0 * (3.0 * PI / 8.0).cos()),
    ]
}

/// One side (low-pass or high-pass) of a Linkwitz-Riley crossover at a fixed
/// cutoff, built from four cascaded [`Biquad`] sections.
#[derive(Debug, Clone)]
struct Crossover {
    stages: [Biquad; 4],
}

impl Crossover {
    fn low_pass(cutoff_hz: f64, sample_rate_hz: f64) -> Self {
        let [q1, q2] = fourth_order_butterworth_qs();
        Self {
            stages: [
                Biquad::low_pass(cutoff_hz, sample_rate_hz, q1),
                Biquad::low_pass(cutoff_hz, sample_rate_hz, q2),
                Biquad::low_pass(cutoff_hz, sample_rate_hz, q1),
                Biquad::low_pass(cutoff_hz, sample_rate_hz, q2),
            ],
        }
    }

    fn high_pass(cutoff_hz: f64, sample_rate_hz: f64) -> Self {
        let [q1, q2] = fourth_order_butterworth_qs();
        Self {
            stages: [
                Biquad::high_pass(cutoff_hz, sample_rate_hz, q1),
                Biquad::high_pass(cutoff_hz, sample_rate_hz, q2),
                Biquad::high_pass(cutoff_hz, sample_rate_hz, q1),
                Biquad::high_pass(cutoff_hz, sample_rate_hz, q2),
            ],
        }
    }

    /// Filters one sample of `channel` through all four stages in series.
    fn process(&mut self, x: f64, channel: usize) -> f64 {
        self.stages
            .iter_mut()
            .fold(x, |v, stage| stage.process(v, channel))
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }
}

/// A three-band equalizer at fixed frequencies with a gain per band.
///
/// The signal is split into a low band below [`LOW_CROSSOVER_HZ`], a high
/// band above [`HIGH_CROSSOVER_HZ`], and a mid band between them; each band is
/// scaled by its gain and the three are summed. With every gain at unity the
/// output has the same level as the input at every frequency, so a flat EQ is
/// transparent. A gain at or below [`Decibels::SILENCE`] removes its band
/// entirely, which is the full kill a bass swap relies on.
///
/// The filters keep state between calls, so one instance belongs to one track
/// for the length of a render, and a change of gains takes effect from the
/// next frame processed.
///
/// Internally the split is a two-stage Linkwitz-Riley crossover network: the
/// signal is first divided at [`LOW_CROSSOVER_HZ`] into the low band and
/// everything above it, and that remainder is divided again at
/// [`HIGH_CROSSOVER_HZ`] into the mid and high bands. Each divide keeps its
/// two sides' combined level equal to the undivided signal's level at every
/// frequency, but only when both sides have passed through the same filters:
/// the mid and high bands have both passed through the second divide, while
/// the low band has passed through neither, so before the three are summed
/// the low band is also run through the second divide's low-pass and
/// high-pass filters in parallel and their outputs added back together. That
/// combination reproduces the second divide's phase shift without changing
/// which frequencies belong to the low band, and with it in place the three
/// bands sum to the input's level at every frequency to within a small
/// fraction of a decibel, the rest being ordinary floating-point rounding.
#[derive(Debug, Clone)]
pub struct ThreeBandEq {
    gains: [Decibels; 3],
    low: Crossover,
    above_low: Crossover,
    mid: Crossover,
    high: Crossover,
    /// Reproduces the phase shift that [`Self::mid`] and [`Self::high`]
    /// impose on their signal, so the low band can be added back in step
    /// with them. See the struct documentation.
    low_phase_match_lp: Crossover,
    /// The high-pass counterpart of `low_phase_match_lp`.
    low_phase_match_hp: Crossover,
}

impl Default for ThreeBandEq {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreeBandEq {
    /// A flat EQ with every band at unity.
    pub fn new() -> Self {
        let sample_rate_hz = f64::from(SAMPLE_RATE);
        Self {
            gains: [Decibels::UNITY; 3],
            low: Crossover::low_pass(LOW_CROSSOVER_HZ, sample_rate_hz),
            above_low: Crossover::high_pass(LOW_CROSSOVER_HZ, sample_rate_hz),
            mid: Crossover::low_pass(HIGH_CROSSOVER_HZ, sample_rate_hz),
            high: Crossover::high_pass(HIGH_CROSSOVER_HZ, sample_rate_hz),
            low_phase_match_lp: Crossover::low_pass(HIGH_CROSSOVER_HZ, sample_rate_hz),
            low_phase_match_hp: Crossover::high_pass(HIGH_CROSSOVER_HZ, sample_rate_hz),
        }
    }

    /// Sets the gain of the low, mid, and high bands.
    pub fn set_gains(&mut self, low: Decibels, mid: Decibels, high: Decibels) {
        self.gains = [low, mid, high];
    }

    /// The gains of the low, mid, and high bands.
    pub fn gains(&self) -> [Decibels; 3] {
        self.gains
    }

    /// Filters `frames` in place.
    pub fn process(&mut self, frames: &mut [Frame]) {
        let [gain_low, gain_mid, gain_high] = self.gains.map(Decibels::to_linear);
        for frame in frames {
            for (channel, sample) in frame.iter_mut().enumerate() {
                let input = f64::from(*sample);
                let low = self.low.process(input, channel);
                let above_low = self.above_low.process(input, channel);
                let mid = self.mid.process(above_low, channel);
                let high = self.high.process(above_low, channel);
                let low_matched = self.low_phase_match_lp.process(low, channel)
                    + self.low_phase_match_hp.process(low, channel);
                let output = low_matched * gain_low + mid * gain_mid + high * gain_high;
                *sample = output as f32;
            }
        }
    }

    /// Clears the filter state, keeping the gains.
    pub fn reset(&mut self) {
        self.low.reset();
        self.above_low.reset();
        self.mid.reset();
        self.high.reset();
        self.low_phase_match_lp.reset();
        self.low_phase_match_hp.reset();
    }
}
