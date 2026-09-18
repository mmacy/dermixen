//! Synthetic audio with known properties, generated when a test needs it.
//!
//! Everything here is stereo with identical channels, at the fixed sample
//! rate, and deterministic: the same arguments give the same frames on every
//! machine.

use std::f64::consts::TAU;

use dermixen_core::{Bpm, SAMPLE_RATE, Seconds};
use dermixen_media::{Audio, Frame};

/// The number of frames in a duration, rounded to the nearest frame.
fn frame_count(duration: Seconds) -> usize {
    duration.to_samples().0.max(0) as usize
}

/// Silence of the given length.
pub fn silence(duration: Seconds) -> Audio {
    Audio {
        frames: vec![[0.0, 0.0]; frame_count(duration)],
    }
}

/// A sine tone at `frequency` hertz with the given peak amplitude, starting at phase zero.
pub fn sine(frequency: f64, amplitude: f32, duration: Seconds) -> Audio {
    let rate = f64::from(SAMPLE_RATE);
    let frames = (0..frame_count(duration))
        .map(|i| {
            let value = (TAU * frequency * i as f64 / rate).sin() as f32 * amplitude;
            [value, value]
        })
        .collect();
    Audio { frames }
}

/// A kick drum on every beat: a 60 hertz burst that decays over about fifty
/// milliseconds, with the first kick at `first_kick` and one every beat of
/// `bpm` after it until `duration` runs out. The rest is silence, so the
/// position of each kick is exact and easy to find.
pub fn kicks(bpm: Bpm, first_kick: Seconds, duration: Seconds) -> Audio {
    let rate = f64::from(SAMPLE_RATE);
    let mut audio = silence(duration);
    let burst_len = (0.1 * rate) as usize;
    let period = bpm.beat_period().0;
    let mut beat = 0;
    loop {
        let at = first_kick.0 + beat as f64 * period;
        let start = (at * rate).round() as i64;
        if start >= audio.frames.len() as i64 {
            break;
        }
        for i in 0..burst_len {
            let index = start + i as i64;
            if index < 0 {
                continue;
            }
            let Some(frame) = audio.frames.get_mut(index as usize) else {
                break;
            };
            let t = i as f64 / rate;
            let value = ((TAU * 60.0 * t).sin() * (-t / 0.015).exp() * 0.9) as f32;
            frame[0] += value;
            frame[1] += value;
        }
        beat += 1;
    }
    audio
}

/// White noise with values uniformly spread between minus and plus
/// `amplitude`, from a small deterministic generator seeded by `seed`.
pub fn white_noise(seed: u64, amplitude: f32, duration: Seconds) -> Audio {
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    let frames = (0..frame_count(duration))
        .map(|_| {
            // A xorshift step, which is plenty for test material.
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            let value = ((unit * 2.0 - 1.0) as f32) * amplitude;
            [value, value]
        })
        .collect();
    Audio { frames }
}

/// The index of the loudest frame in `frames`, by absolute value of the left channel.
pub fn loudest_frame(frames: &[Frame]) -> Option<usize> {
    frames
        .iter()
        .enumerate()
        .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::Samples;

    #[test]
    fn silence_has_the_right_length_and_is_silent() {
        let audio = silence(Seconds(1.5));
        assert_eq!(audio.len(), Samples(66_150));
        assert!(audio.frames.iter().all(|f| *f == [0.0, 0.0]));
        assert!(silence(Seconds(-1.0)).is_empty());
    }

    #[test]
    fn a_sine_has_the_right_length_peak_and_frequency() {
        let audio = sine(440.0, 0.5, Seconds(2.0));
        assert_eq!(audio.len(), Samples(88_200));
        let peak = audio.frames.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!((peak - 0.5).abs() < 1e-3, "peak {peak}");
        assert!(audio.frames.iter().all(|f| f[0] == f[1]));
        let crossings = audio
            .frames
            .windows(2)
            .filter(|w| (w[0][0] < 0.0) != (w[1][0] < 0.0))
            .count();
        // Two zero crossings per cycle, 880 cycles in two seconds.
        assert!((1758..=1762).contains(&crossings), "crossings {crossings}");
    }

    #[test]
    fn kicks_land_on_the_beats() {
        let bpm = Bpm(138.0);
        let first = Seconds(0.25);
        let audio = kicks(bpm, first, Seconds(5.0));
        assert_eq!(audio.len(), Samples(220_500));
        let period = bpm.beat_period().to_samples().0 as usize;
        let mut found = 0;
        for beat in 0..11 {
            let expected = (first.0 * 44_100.0).round() as usize + beat * period;
            let window_start = expected.saturating_sub(period / 2);
            let window_end = (expected + period / 2).min(audio.frames.len());
            if window_end <= window_start {
                break;
            }
            let loudest =
                window_start + loudest_frame(&audio.frames[window_start..window_end]).unwrap();
            // The burst peaks within its first few milliseconds.
            assert!(
                loudest >= expected && loudest < expected + 220,
                "beat {beat}: loudest at {loudest}, expected near {expected}"
            );
            found += 1;
        }
        assert_eq!(found, 11);
        // The first quarter second holds nothing.
        assert!(audio.frames[..11_000].iter().all(|f| *f == [0.0, 0.0]));
    }

    #[test]
    fn noise_is_bounded_and_repeatable() {
        let a = white_noise(7, 0.25, Seconds(0.5));
        let b = white_noise(7, 0.25, Seconds(0.5));
        let c = white_noise(8, 0.25, Seconds(0.5));
        assert_eq!(a.len(), Samples(22_050));
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.frames.iter().all(|f| f[0].abs() <= 0.25 && f[0] == f[1]));
        let mean: f64 = a.frames.iter().map(|f| f64::from(f[0])).sum::<f64>() / 22_050.0;
        assert!(mean.abs() < 0.01, "mean {mean}");
    }
}
