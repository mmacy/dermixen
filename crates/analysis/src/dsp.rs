//! Small signal-processing pieces the bespoke analyzers share: a
//! second-order filter section and band-limited level envelopes.

use dermixen_core::SAMPLE_RATE;
use dermixen_media::Audio;

/// A second-order filter section in direct form, with coefficients from
/// the standard audio equalizer cookbook.
#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Biquad {
    /// A Butterworth low-pass or high-pass section at `cutoff` hertz.
    pub fn butterworth(cutoff: f64, high_pass: bool) -> Biquad {
        let w0 = std::f64::consts::TAU * cutoff / f64::from(SAMPLE_RATE);
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if high_pass {
            ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0)
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0)
        };
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Filters one sample.
    pub fn step(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A band of frequencies, as the edges of a band-pass made of two
/// second-order Butterworth sections at each edge, which is a fourth-order
/// Linkwitz-Riley response, six decibels down at the edges. `None` at
/// either edge leaves that side open.
#[derive(Debug, Clone, Copy)]
pub struct Band {
    /// The low edge in hertz, or `None` for no high-pass.
    pub low_hz: Option<f64>,
    /// The high edge in hertz, or `None` for no low-pass.
    pub high_hz: Option<f64>,
}

impl Band {
    /// The filter sections that make the band: two high-pass sections at
    /// the low edge and two low-pass sections at the high edge.
    fn sections(self) -> Vec<Biquad> {
        let mut sections = Vec::with_capacity(4);
        if let Some(low) = self.low_hz {
            sections.push(Biquad::butterworth(low, true));
            sections.push(Biquad::butterworth(low, true));
        }
        if let Some(high) = self.high_hz {
            sections.push(Biquad::butterworth(high, false));
            sections.push(Biquad::butterworth(high, false));
        }
        sections
    }
}

/// The level of the mono mix of the audio in each band, as one
/// root-mean-square reading per `hop` samples: one vector per band, each
/// as long as the number of whole hops in the audio.
pub fn band_envelopes(audio: &Audio, bands: &[Band], hop: usize) -> Vec<Vec<f64>> {
    let mut sections: Vec<Vec<Biquad>> = bands.iter().map(|band| band.sections()).collect();
    let hops = audio.frames.len() / hop;
    let mut envelopes: Vec<Vec<f64>> = bands.iter().map(|_| Vec::with_capacity(hops)).collect();
    let mut sums = vec![0.0f64; bands.len()];
    let mut count = 0usize;
    for frame in &audio.frames {
        let mono = 0.5 * (f64::from(frame[0]) + f64::from(frame[1]));
        for (band, filter) in sections.iter_mut().enumerate() {
            let mut sample = mono;
            for section in filter.iter_mut() {
                sample = section.step(sample);
            }
            sums[band] += sample * sample;
        }
        count += 1;
        if count == hop {
            for (band, sum) in sums.iter_mut().enumerate() {
                envelopes[band].push((*sum / hop as f64).sqrt());
                *sum = 0.0;
            }
            count = 0;
        }
    }
    envelopes
}

/// Amplitude as decibels, with a floor far below anything audible.
pub fn decibels(amplitude: f64) -> f64 {
    20.0 * amplitude.max(1e-9).log10()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::Seconds;

    #[test]
    fn a_band_keeps_a_tone_inside_it_and_drops_one_outside() {
        let low_tone = dermixen_testkit::synth::sine(60.0, 0.5, Seconds(2.0));
        let high_tone = dermixen_testkit::synth::sine(2000.0, 0.5, Seconds(2.0));
        let band = Band {
            low_hz: Some(30.0),
            high_hz: Some(130.0),
        };
        let kept: f64 = band_envelopes(&low_tone, &[band], 64)[0][500..]
            .iter()
            .sum();
        let dropped: f64 = band_envelopes(&high_tone, &[band], 64)[0][500..]
            .iter()
            .sum();
        assert!(kept > 20.0 * dropped, "{kept} against {dropped}");
    }
}
