//! Acceptance tests for the three-band EQ. A coder agent makes these pass without editing them.
//!
//! Every test judges the EQ by its effect on white noise or a tone, measured
//! with the spectrum helpers in `dermixen-testkit`.

use dermixen_core::{Decibels, Seconds};
use dermixen_engine::{HIGH_CROSSOVER_HZ, LOW_CROSSOVER_HZ, ThreeBandEq};
use dermixen_media::{Audio, Frame};
use dermixen_testkit::spectrum::band_level_db;
use dermixen_testkit::synth;

const SECONDS: f64 = 6.0;

/// Bands well inside each of the three regions, and one across each crossover.
const LOW_BAND: (f64, f64) = (40.0, 120.0);
const MID_BAND: (f64, f64) = (700.0, 1000.0);
const HIGH_BAND: (f64, f64) = (6000.0, 12000.0);

fn noise() -> Audio {
    synth::white_noise(5, 0.4, Seconds(SECONDS))
}

fn processed(eq: &mut ThreeBandEq, audio: &Audio) -> Audio {
    let mut out = audio.clone();
    eq.process(&mut out.frames);
    out
}

/// The change in level from `before` to `after` in a band, in decibels, for the left channel.
fn change(before: &Audio, after: &Audio, band: (f64, f64)) -> f64 {
    band_level_db(&after.frames, 0, band.0, band.1)
        - band_level_db(&before.frames, 0, band.0, band.1)
}

fn max_abs(frames: &[Frame]) -> f32 {
    frames
        .iter()
        .map(|f| f[0].abs().max(f[1].abs()))
        .fold(0.0, f32::max)
}

#[test]
fn the_crossovers_are_where_the_contract_says() {
    assert_eq!(LOW_CROSSOVER_HZ, 250.0);
    assert_eq!(HIGH_CROSSOVER_HZ, 2500.0);
    let mut eq = ThreeBandEq::new();
    assert_eq!(eq.gains(), [Decibels::UNITY; 3]);
    eq.set_gains(Decibels(-3.0), Decibels(0.0), Decibels(6.0));
    assert_eq!(eq.gains(), [Decibels(-3.0), Decibels(0.0), Decibels(6.0)]);
}

#[test]
fn a_flat_eq_is_transparent() {
    let input = noise();
    let output = processed(&mut ThreeBandEq::new(), &input);
    assert_eq!(output.len(), input.len());
    for band in [
        LOW_BAND,
        (200.0, 300.0),
        MID_BAND,
        (2000.0, 3000.0),
        HIGH_BAND,
        (15000.0, 20000.0),
    ] {
        let delta = change(&input, &output, band);
        assert!(delta.abs() < 1.0, "band {band:?} changed by {delta} dB");
    }
    // Both channels are treated alike.
    let right = band_level_db(&output.frames, 1, 100.0, 10000.0)
        - band_level_db(&input.frames, 1, 100.0, 10000.0);
    assert!(right.abs() < 1.0, "right channel changed by {right} dB");
}

#[test]
fn killing_the_low_band_removes_the_bass_and_leaves_the_rest() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::SILENCE, Decibels::UNITY, Decibels::UNITY);
    let output = processed(&mut eq, &input);
    let low = change(&input, &output, LOW_BAND);
    assert!(low < -40.0, "low band only dropped by {low} dB");
    let mid = change(&input, &output, MID_BAND);
    assert!(mid.abs() < 1.0, "mid band changed by {mid} dB");
    let high = change(&input, &output, HIGH_BAND);
    assert!(high.abs() < 1.0, "high band changed by {high} dB");
}

#[test]
fn killing_the_mid_band_leaves_the_bass_and_the_treble() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::UNITY, Decibels::SILENCE, Decibels::UNITY);
    let output = processed(&mut eq, &input);
    let mid = change(&input, &output, MID_BAND);
    assert!(mid < -30.0, "mid band only dropped by {mid} dB");
    let low = change(&input, &output, LOW_BAND);
    assert!(low.abs() < 1.0, "low band changed by {low} dB");
    let high = change(&input, &output, HIGH_BAND);
    assert!(high.abs() < 1.0, "high band changed by {high} dB");
}

#[test]
fn killing_the_high_band_removes_the_treble_and_leaves_the_rest() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::UNITY, Decibels::UNITY, Decibels::SILENCE);
    let output = processed(&mut eq, &input);
    let high = change(&input, &output, HIGH_BAND);
    assert!(high < -40.0, "high band only dropped by {high} dB");
    let low = change(&input, &output, LOW_BAND);
    assert!(low.abs() < 1.0, "low band changed by {low} dB");
    let mid = change(&input, &output, MID_BAND);
    assert!(mid.abs() < 1.0, "mid band changed by {mid} dB");
}

#[test]
fn a_partial_cut_and_a_boost_change_the_band_by_that_much() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels(-6.0), Decibels::UNITY, Decibels(3.0));
    let output = processed(&mut eq, &input);
    let low = change(&input, &output, LOW_BAND);
    assert!(
        (low + 6.0).abs() < 1.0,
        "low band changed by {low} dB, expected -6"
    );
    let high = change(&input, &output, HIGH_BAND);
    assert!(
        (high - 3.0).abs() < 1.0,
        "high band changed by {high} dB, expected +3"
    );
    let mid = change(&input, &output, MID_BAND);
    assert!(mid.abs() < 1.0, "mid band changed by {mid} dB");
}

#[test]
fn a_tone_keeps_its_level_when_another_band_is_killed() {
    let bass = synth::sine(100.0, 0.5, Seconds(SECONDS));
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::UNITY, Decibels::UNITY, Decibels::SILENCE);
    let output = processed(&mut eq, &bass);
    let delta = change(&bass, &output, (90.0, 110.0));
    assert!(
        delta.abs() < 1.0,
        "100 Hz tone changed by {delta} dB with the high band killed"
    );

    let treble = synth::sine(8000.0, 0.5, Seconds(SECONDS));
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::SILENCE, Decibels::UNITY, Decibels::UNITY);
    let output = processed(&mut eq, &treble);
    let delta = change(&treble, &output, (7900.0, 8100.0));
    assert!(
        delta.abs() < 1.0,
        "8 kHz tone changed by {delta} dB with the low band killed"
    );

    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::SILENCE, Decibels::UNITY, Decibels::UNITY);
    let output = processed(&mut eq, &bass);
    let delta = change(&bass, &output, (90.0, 110.0));
    assert!(
        delta < -40.0,
        "100 Hz tone only dropped by {delta} dB with the low band killed"
    );
}

#[test]
fn every_band_killed_is_silence() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::SILENCE, Decibels::SILENCE, Decibels::SILENCE);
    let output = processed(&mut eq, &input);
    assert!(max_abs(&output.frames) < 1e-6);
}

#[test]
fn processing_in_blocks_gives_the_same_result_as_one_call() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels(-6.0), Decibels(2.0), Decibels(-12.0));
    let whole = processed(&mut eq, &input);

    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels(-6.0), Decibels(2.0), Decibels(-12.0));
    let mut blocks = input.clone();
    for block in blocks.frames.chunks_mut(1024) {
        eq.process(block);
    }
    let worst = whole
        .frames
        .iter()
        .zip(&blocks.frames)
        .map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()))
        .fold(0.0, f32::max);
    assert!(worst < 1e-5, "block processing differs by up to {worst}");
}

#[test]
fn reset_forgets_the_past() {
    let input = noise();
    let mut eq = ThreeBandEq::new();
    eq.set_gains(Decibels::SILENCE, Decibels::UNITY, Decibels(-3.0));
    let first = processed(&mut eq, &input);
    let carried = processed(&mut eq, &input);
    eq.reset();
    let after_reset = processed(&mut eq, &input);
    assert_eq!(after_reset, first);
    // Without the reset the filter state from the first pass leaks into the second.
    assert_ne!(carried.frames[..64], first.frames[..64]);
    assert_eq!(
        eq.gains(),
        [Decibels::SILENCE, Decibels::UNITY, Decibels(-3.0)]
    );
}
