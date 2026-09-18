//! Acceptance tests for the time-stretchers. A coder agent makes these pass without editing them.
//!
//! Both implementations are driven through the `TimeStretcher` trait by the
//! same code, so the tests state what any stretcher must do and then what
//! only a pitch-preserving one must do. The Signalsmith tests compile only
//! with the `signalsmith` feature:
//!
//! ```text
//! cargo test -p dermixen-engine --test stretch --features signalsmith
//! ```

use dermixen_core::{Bpm, Seconds};
use dermixen_engine::{Resampler, TimeStretcher};
use dermixen_media::{Audio, Frame};
use dermixen_testkit::spectrum::dominant_frequency;
use dermixen_testkit::synth;

const INPUT_BLOCK: usize = 1024;

/// The number of output frames asked for per input block at a nominal speed.
fn output_block(speed: f64) -> usize {
    (INPUT_BLOCK as f64 / speed).round() as usize
}

/// The speed the stretcher is really driven at, since blocks hold whole
/// frames: 1024 input frames per `output_block` output frames.
fn effective_speed(speed: f64) -> f64 {
    INPUT_BLOCK as f64 / output_block(speed) as f64
}

/// The number of output frames to discard so that the rest lines up with
/// the input: the input-side latency converted to output frames at the
/// effective speed, plus the output-side latency.
fn alignment(stretcher: &dyn TimeStretcher, speed: f64) -> usize {
    (stretcher.input_latency().0 as f64 / effective_speed(speed)).round() as usize
        + stretcher.output_latency().0 as usize
}

/// Runs `input` through a stretcher at about `speed` input frames per output
/// frame, feeding fixed input blocks, flushing with silence, and dropping the
/// frames the two latencies account for, so the result lines up with the
/// input scaled by the effective speed.
fn run(stretcher: &mut dyn TimeStretcher, input: &Audio, speed: f64) -> Audio {
    let output_block = output_block(speed);
    let expected = (input.frames.len() as f64 / effective_speed(speed)).round() as usize;
    let latency = alignment(stretcher, speed);
    let mut produced: Vec<Frame> = Vec::with_capacity(expected + latency + output_block);
    let mut fed = 0;
    while produced.len() < expected + latency {
        let mut block = vec![[0.0, 0.0]; INPUT_BLOCK];
        let available = input.frames.len().saturating_sub(fed).min(INPUT_BLOCK);
        let from = fed.min(input.frames.len());
        block[..available].copy_from_slice(&input.frames[from..from + available]);
        fed += INPUT_BLOCK;
        let mut out = vec![[0.0, 0.0]; output_block];
        stretcher.process(&block, &mut out);
        produced.extend_from_slice(&out);
        assert!(
            fed < input.frames.len() * 4 + 1_000_000,
            "the stretcher never produced enough output"
        );
    }
    Audio {
        frames: produced[latency..latency + expected].to_vec(),
    }
}

fn middle(audio: &Audio) -> &[Frame] {
    let len = audio.frames.len();
    &audio.frames[len / 4..len * 3 / 4]
}

fn rms(frames: &[Frame]) -> f64 {
    let sum: f64 = frames
        .iter()
        .map(|f| f64::from(f[0]) * f64::from(f[0]))
        .sum();
    (sum / frames.len() as f64).sqrt()
}

fn largest_step(frames: &[Frame]) -> f32 {
    frames
        .windows(2)
        .map(|w| (w[1][0] - w[0][0]).abs())
        .fold(0.0, f32::max)
}

/// The positions of the loudest frame in each beat-length window, which is where each kick lands.
fn kick_positions(audio: &Audio, bpm: Bpm, speed: f64) -> Vec<usize> {
    let period = (bpm.beat_period().0 * 44_100.0 / speed).round() as usize;
    audio
        .frames
        .chunks(period)
        .enumerate()
        .filter(|(_, chunk)| chunk.len() == period)
        .map(|(i, chunk)| {
            i * period
                + chunk
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1[0].abs().total_cmp(&b.1[0].abs()))
                    .map(|(j, _)| j)
                    .unwrap()
        })
        .collect()
}

/// What every stretcher must do, whether or not it preserves pitch.
fn check_any_stretcher(make: &dyn Fn() -> Box<dyn TimeStretcher>, preserves_pitch: bool) {
    let stretcher = make();
    assert!(stretcher.input_latency().0 >= 0);
    assert!(stretcher.output_latency().0 >= 0);
    assert!(
        stretcher.output_latency().0 < 22_050,
        "output latency is over half a second"
    );

    let tone = synth::sine(440.0, 0.5, Seconds(5.0));
    let tone_rms = rms(middle(&tone));

    for speed in [1.0, 1.0 / 1.02, 1.05, 0.9] {
        let mut stretcher = make();
        let out = run(stretcher.as_mut(), &tone, speed);
        let speed = effective_speed(speed);
        let expected_len = (tone.frames.len() as f64 / speed).round() as usize;
        assert_eq!(out.frames.len(), expected_len);
        let level = rms(middle(&out));
        assert!(
            (level - tone_rms).abs() < 0.1 * tone_rms,
            "speed {speed}: level {level} against {tone_rms}"
        );
        let step = largest_step(middle(&out));
        assert!(
            step < 0.1,
            "speed {speed}: a jump of {step} between neighbouring samples"
        );
        let expected_pitch = if preserves_pitch {
            440.0
        } else {
            440.0 * speed
        };
        let pitch = dominant_frequency(middle(&out), 0);
        assert!(
            (pitch - expected_pitch).abs() < 2.0,
            "speed {speed}: pitch {pitch} Hz, expected {expected_pitch}"
        );
    }
}

/// Kicks keep their timing: at any speed each kick lands where the input kick lands, scaled.
fn check_kick_timing(make: &dyn Fn() -> Box<dyn TimeStretcher>, tolerance_frames: usize) {
    let bpm = Bpm(130.0);
    let kicks = synth::kicks(bpm, Seconds(0.2), Seconds(6.0));
    let reference = kick_positions(&kicks, bpm, 1.0);
    assert!(reference.len() >= 12);
    for speed in [1.0, 0.98, 1.03] {
        let mut stretcher = make();
        let out = run(stretcher.as_mut(), &kicks, speed);
        let speed = effective_speed(speed);
        let found = kick_positions(&out, bpm, speed);
        for (n, (expected, got)) in reference.iter().zip(&found).enumerate() {
            let expected = (*expected as f64 / speed).round() as i64;
            let error = (*got as i64 - expected).abs();
            assert!(
                error <= tolerance_frames as i64,
                "speed {speed}: kick {n} at {got}, expected {expected}, off by {error} frames"
            );
        }
    }
}

/// Changing speed between blocks does not click.
fn check_variable_speed(make: &dyn Fn() -> Box<dyn TimeStretcher>) {
    let tone = synth::sine(440.0, 0.5, Seconds(4.0));
    let mut stretcher = make();
    let mut produced: Vec<Frame> = Vec::new();
    for (i, block) in tone.frames.chunks(INPUT_BLOCK).enumerate() {
        if block.len() < INPUT_BLOCK {
            break;
        }
        let speed = if i % 2 == 0 { 0.98 } else { 1.02 };
        let mut out = vec![[0.0, 0.0]; output_block(speed)];
        stretcher.process(block, &mut out);
        produced.extend_from_slice(&out);
    }
    let latency = alignment(stretcher.as_ref(), 1.0);
    let steady = &produced[latency + 4096..produced.len() - 4096];
    let step = largest_step(steady);
    assert!(
        step < 0.1,
        "a jump of {step} between neighbouring samples while the speed varies"
    );
}

/// After a reset the stretcher behaves as if new.
fn check_reset(make: &dyn Fn() -> Box<dyn TimeStretcher>) {
    let tone = synth::sine(330.0, 0.5, Seconds(1.0));
    let mut stretcher = make();
    let first = run(stretcher.as_mut(), &tone, 1.01);
    stretcher.reset();
    let again = run(stretcher.as_mut(), &tone, 1.01);
    assert_eq!(first, again);
}

#[test]
fn the_resampler_changes_speed_and_pitch_together() {
    check_any_stretcher(&|| Box::new(Resampler::new()), false);
}

#[test]
fn the_resampler_keeps_kicks_in_time() {
    check_kick_timing(&|| Box::new(Resampler::new()), 22);
}

#[test]
fn the_resampler_handles_a_changing_speed() {
    check_variable_speed(&|| Box::new(Resampler::new()));
}

#[test]
fn the_resampler_resets() {
    check_reset(&|| Box::new(Resampler::new()));
}

#[test]
fn the_resampler_at_speed_one_is_exact() {
    let tone = synth::sine(440.0, 0.5, Seconds(2.0));
    let mut stretcher = Resampler::new();
    let out = run(&mut stretcher, &tone, 1.0);
    assert_eq!(out, tone);
}

#[cfg(feature = "signalsmith")]
mod signalsmith {
    use super::*;
    use dermixen_engine::SignalsmithStretcher;

    #[test]
    fn signalsmith_changes_speed_and_keeps_pitch() {
        check_any_stretcher(&|| Box::new(SignalsmithStretcher::new()), true);
    }

    #[test]
    fn signalsmith_keeps_kicks_in_time() {
        check_kick_timing(&|| Box::new(SignalsmithStretcher::new()), 132);
    }

    #[test]
    fn signalsmith_handles_a_changing_speed() {
        check_variable_speed(&|| Box::new(SignalsmithStretcher::new()));
    }

    #[test]
    fn signalsmith_resets() {
        check_reset(&|| Box::new(SignalsmithStretcher::new()));
    }
}
