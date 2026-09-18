//! Measuring what is in a signal: band levels and the strongest frequency.
//!
//! Tests use these to check filters and stretchers by their effect on
//! audio rather than by their internals. Every measurement averages the
//! spectrum over consecutive windows of [`WINDOW`] frames with a Hann taper,
//! so a signal shorter than one window cannot be measured.

use std::f64::consts::PI;

use dermixen_core::SAMPLE_RATE;
use dermixen_media::Frame;

/// The number of frames in one analysis window, a power of two.
pub const WINDOW: usize = 8192;

/// The width of one spectrum bin in hertz.
pub fn bin_width() -> f64 {
    f64::from(SAMPLE_RATE) / WINDOW as f64
}

/// An in-place radix-two fast Fourier transform of `re` and `im`, whose lengths are a power of two.
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        let (wr, wi) = (angle.cos(), angle.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next_cr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next_cr;
            }
        }
        len <<= 1;
    }
}

/// The power in each bin from zero to half the sample rate, averaged over
/// every whole window of `frames`, for one channel.
///
/// Returns `None` when there are fewer frames than one window.
pub fn power_spectrum(frames: &[Frame], channel: usize) -> Option<Vec<f64>> {
    let windows = frames.len() / WINDOW;
    if windows == 0 {
        return None;
    }
    let taper: Vec<f64> = (0..WINDOW)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / WINDOW as f64).cos())
        .collect();
    let mut power = vec![0.0; WINDOW / 2 + 1];
    let mut re = vec![0.0; WINDOW];
    let mut im = vec![0.0; WINDOW];
    for w in 0..windows {
        for i in 0..WINDOW {
            re[i] = f64::from(frames[w * WINDOW + i][channel]) * taper[i];
            im[i] = 0.0;
        }
        fft(&mut re, &mut im);
        for (bin, p) in power.iter_mut().enumerate() {
            *p += re[bin] * re[bin] + im[bin] * im[bin];
        }
    }
    for p in &mut power {
        *p /= windows as f64;
    }
    Some(power)
}

/// The level in decibels of everything between `low_hz` and `high_hz` in one
/// channel, relative to nothing in particular: compare two calls, not one
/// call with a constant. Silence gives minus two hundred.
pub fn band_level_db(frames: &[Frame], channel: usize, low_hz: f64, high_hz: f64) -> f64 {
    let power = power_spectrum(frames, channel).expect("at least one window of frames");
    let first = (low_hz / bin_width()).ceil() as usize;
    let last = ((high_hz / bin_width()).floor() as usize).min(power.len() - 1);
    let total: f64 = power[first..=last].iter().sum();
    if total <= 0.0 {
        -200.0
    } else {
        10.0 * total.log10()
    }
}

/// The frequency in hertz with the most power in one channel, refined
/// between bins by fitting a parabola through the strongest bin and its
/// neighboring samples.
pub fn dominant_frequency(frames: &[Frame], channel: usize) -> f64 {
    let power = power_spectrum(frames, channel).expect("at least one window of frames");
    let peak = (1..power.len() - 1)
        .max_by(|a, b| power[*a].total_cmp(&power[*b]))
        .expect("more than two bins");
    let (a, b, c) = (power[peak - 1].ln(), power[peak].ln(), power[peak + 1].ln());
    let offset = if (a - 2.0 * b + c).abs() < f64::EPSILON {
        0.0
    } else {
        0.5 * (a - c) / (a - 2.0 * b + c)
    };
    (peak as f64 + offset) * bin_width()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth;
    use dermixen_core::Seconds;

    #[test]
    fn a_tone_is_found_at_its_frequency() {
        let tone = synth::sine(1000.0, 0.5, Seconds(2.0));
        let found = dominant_frequency(&tone.frames, 0);
        assert!((found - 1000.0).abs() < 1.0, "found {found}");
        let low = synth::sine(97.3, 0.5, Seconds(2.0));
        let found = dominant_frequency(&low.frames, 1);
        assert!((found - 97.3).abs() < 1.0, "found {found}");
    }

    #[test]
    fn a_tone_has_far_more_level_in_its_own_band_than_elsewhere() {
        let tone = synth::sine(1000.0, 0.5, Seconds(2.0));
        let inside = band_level_db(&tone.frames, 0, 900.0, 1100.0);
        let outside = band_level_db(&tone.frames, 0, 3000.0, 6000.0);
        assert!(inside - outside > 60.0, "inside {inside} outside {outside}");
    }

    #[test]
    fn white_noise_is_flat_across_bands_of_equal_width() {
        let noise = synth::white_noise(11, 0.5, Seconds(6.0));
        let low = band_level_db(&noise.frames, 0, 100.0, 600.0);
        let mid = band_level_db(&noise.frames, 0, 1000.0, 1500.0);
        let high = band_level_db(&noise.frames, 0, 8000.0, 8500.0);
        assert!((low - mid).abs() < 1.0, "low {low} mid {mid}");
        assert!((high - mid).abs() < 1.0, "high {high} mid {mid}");
    }

    #[test]
    fn a_gain_change_shows_as_the_same_change_in_level() {
        let quiet = synth::white_noise(11, 0.25, Seconds(2.0));
        let loud = synth::white_noise(11, 0.5, Seconds(2.0));
        let difference = band_level_db(&loud.frames, 0, 200.0, 2000.0)
            - band_level_db(&quiet.frames, 0, 200.0, 2000.0);
        assert!(
            (difference - 6.0206).abs() < 0.01,
            "difference {difference}"
        );
    }

    #[test]
    fn silence_and_short_input_are_handled() {
        let silent = synth::silence(Seconds(1.0));
        assert_eq!(band_level_db(&silent.frames, 0, 100.0, 1000.0), -200.0);
        assert!(power_spectrum(&synth::silence(Seconds(0.1)).frames, 0).is_none());
    }
}
