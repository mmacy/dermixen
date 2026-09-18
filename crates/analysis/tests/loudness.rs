//! Acceptance tests for loudness measurement. A coder agent makes these pass
//! without editing them.
//!
//! The expectations are the ones EBU Tech 3341 gives for a loudness meter:
//! a stereo 1 kHz sine at minus twenty-three decibels below full scale in
//! both channels measures minus twenty-three LUFS within a tenth, and the
//! relative gate leaves a passage ten loudness units quieter than the rest
//! out of the measurement.

use std::f64::consts::TAU;

use dermixen_analysis::measure_loudness;
use dermixen_core::Seconds;
use dermixen_media::Audio;
use dermixen_testkit::synth;

/// A 1 kHz sine of the given length, each channel at the level given in
/// decibels below full scale, or silent where no level is given.
fn tone(seconds: f64, left_db: Option<f64>, right_db: Option<f64>) -> Audio {
    let count = (seconds * 44_100.0).round() as usize;
    let amplitude = |db: Option<f64>| db.map_or(0.0, |db| 10f64.powf(db / 20.0));
    let (left, right) = (amplitude(left_db), amplitude(right_db));
    Audio {
        frames: (0..count)
            .map(|n| {
                let value = (TAU * 1000.0 * n as f64 / 44_100.0).sin();
                [(value * left) as f32, (value * right) as f32]
            })
            .collect(),
    }
}

fn within(value: f64, expected: f64, tolerance: f64) -> bool {
    (value - expected).abs() <= tolerance
}

#[test]
fn a_stereo_1khz_sine_measures_its_own_level() {
    let found = measure_loudness(&tone(10.0, Some(-23.0), Some(-23.0))).unwrap();
    assert!(within(found.integrated.0, -23.0, 0.1), "{found:?}");
    assert!(within(found.true_peak.0, -23.0, 0.3), "{found:?}");

    let found = measure_loudness(&tone(10.0, Some(-33.0), Some(-33.0))).unwrap();
    assert!(within(found.integrated.0, -33.0, 0.1), "{found:?}");
    assert!(within(found.true_peak.0, -33.0, 0.3), "{found:?}");
}

#[test]
fn a_quiet_passage_below_the_relative_gate_is_left_out() {
    // Three seconds at minus thirty-six, thirty at minus twenty-three, and
    // three more at minus thirty-six. The ungated mean is about minus
    // twenty-four, so the relative gate sits near minus thirty-four and the
    // quiet passages fall below it: the measurement is the loud part alone,
    // within two tenths, since the blocks that straddle each edge count too.
    let mut audio = tone(3.0, Some(-36.0), Some(-36.0));
    audio
        .frames
        .extend(tone(30.0, Some(-23.0), Some(-23.0)).frames);
    audio
        .frames
        .extend(tone(3.0, Some(-36.0), Some(-36.0)).frames);
    let found = measure_loudness(&audio).unwrap();
    assert!(within(found.integrated.0, -23.0, 0.2), "{found:?}");
}

#[test]
fn a_single_channel_counts_three_decibels_less() {
    // The loudness sums the channels' power, so one channel at minus
    // twenty-three is 3.01 loudness units quieter than both. The true peak
    // is the peak of either channel, so it is unchanged.
    let found = measure_loudness(&tone(10.0, Some(-23.0), None)).unwrap();
    assert!(within(found.integrated.0, -26.01, 0.1), "{found:?}");
    assert!(within(found.true_peak.0, -23.0, 0.3), "{found:?}");
}

#[test]
fn silence_and_audio_too_short_or_too_quiet_to_gate_have_no_loudness() {
    assert_eq!(measure_loudness(&synth::silence(Seconds(5.0))), None);
    // Two tenths of a second is shorter than one measurement block.
    assert_eq!(measure_loudness(&tone(0.2, Some(-23.0), Some(-23.0))), None);
    // Minus eighty is below the absolute gate of minus seventy.
    assert_eq!(measure_loudness(&tone(5.0, Some(-80.0), Some(-80.0))), None);
    assert_eq!(measure_loudness(&Audio::new()), None);
}

#[test]
fn the_true_peak_sees_between_the_samples() {
    // A sine at a quarter of the sample rate with its phase turned an eighth
    // of a turn has samples at 0.707 of its amplitude and its peaks between
    // them: the sample peak is minus 9.03 dBFS and the true peak minus 6.02.
    let count = 5 * 44_100;
    let audio = Audio {
        frames: (0..count)
            .map(|n| {
                let value = 0.5 * (TAU * (n as f64 / 4.0 + 1.0 / 8.0)).sin();
                [value as f32, value as f32]
            })
            .collect(),
    };
    let sample_peak = audio
        .frames
        .iter()
        .map(|frame| f64::from(frame[0].abs()))
        .fold(0.0, f64::max);
    assert!(
        within(20.0 * sample_peak.log10(), -9.03, 0.05),
        "the samples peak at {sample_peak}"
    );
    let found = measure_loudness(&audio).unwrap();
    assert!(within(found.true_peak.0, -6.02, 0.7), "{found:?}");
}

#[test]
#[ignore = "engine-guards"]
fn audio_that_overflows_the_meter_has_no_loudness() {
    // Sixty-four frames of the largest float overflow the true peak filter,
    // and a gain worked out from an infinite peak cannot be written to a mix.
    let mut audio = tone(5.0, Some(-20.0), Some(-20.0));
    for frame in &mut audio.frames[44_100..44_164] {
        *frame = [f32::MAX, f32::MAX];
    }
    match measure_loudness(&audio) {
        None => {}
        Some(loudness) => {
            assert!(loudness.true_peak.0.is_finite(), "{loudness:?}");
            assert!(loudness.integrated.0 <= 0.0, "{loudness:?}");
        }
    }
}

#[test]
#[ignore = "engine-guards"]
fn a_loudness_above_full_scale_is_no_loudness() {
    // A square wave at eight times full scale, which is the most the decoder
    // lets through, measures far above 0 LUFS. No real master does.
    let mut audio = tone(5.0, Some(0.0), Some(0.0));
    for (n, frame) in audio.frames.iter_mut().enumerate() {
        let value = if (n / 50) % 2 == 0 { 8.0 } else { -8.0 };
        *frame = [value, value];
    }
    assert_eq!(measure_loudness(&audio), None);
    // A tone six decibels under full scale in both channels measures near
    // minus six LUFS, louder than any real master, and still has a loudness.
    let loud = tone(5.0, Some(-6.0), Some(-6.0));
    assert!(measure_loudness(&loud).is_some());
}
