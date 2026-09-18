//! Acceptance tests for writing MP3 files. A coder agent makes these pass without
//! editing them.
//!
//! These tests exist only in a build with the `mp3` feature. The command
//! crate turns that feature on by default, so `cargo test --workspace`
//! runs them. On their own they need the flag:
//! `cargo test -p dermixen-media --features mp3 --test mp3`.

#![cfg(feature = "mp3")]

mod common;

use std::f64::consts::TAU;

use common::{rms, zero_crossings};
use dermixen_core::Samples;
use dermixen_media::{Audio, MP3_BITRATE_KBPS, Mp3File, decode, write_mp3};
use dermixen_testkit::mp3::mp3_frames;

/// The expected root mean square of a sine with peak amplitude 0.5.
const HALF_SINE_RMS: f64 = 0.5 / std::f64::consts::SQRT_2;

fn close(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * b.abs()
}

/// Two seconds of a 440 Hz sine, the left channel at half scale and the
/// right at a quarter, which is the tone the decode fixtures contain.
fn tone() -> Audio {
    Audio {
        frames: (0..88_200)
            .map(|n| {
                let value = (TAU * 440.0 * n as f64 / 44_100.0).sin();
                [(value * 0.5) as f32, (value * 0.25) as f32]
            })
            .collect(),
    }
}

#[test]
fn a_tone_written_as_mp3_decodes_to_the_tone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.mp3");
    write_mp3(&path, &tone()).unwrap();

    let decoded = decode(&path).unwrap();
    assert_eq!(decoded.source_sample_rate, 44_100);
    assert_eq!(decoded.source_channels, 2);
    let len = decoded.audio.len().0;
    assert!((len - 88_200).abs() <= 2_400, "length {len}");
    let middle = &decoded.audio.frames[22_050..66_150];
    let crossings = zero_crossings(middle);
    assert!((872..=888).contains(&crossings), "crossings {crossings}");
    assert!(
        close(rms(middle, 0), HALF_SINE_RMS, 0.05),
        "left rms {}",
        rms(middle, 0)
    );
    assert!(
        close(rms(middle, 1), HALF_SINE_RMS / 2.0, 0.10),
        "right rms {}",
        rms(middle, 1)
    );
}

#[test]
fn every_frame_is_320_kilobits_at_44100_and_the_file_is_one_frame_after_another() {
    assert_eq!(MP3_BITRATE_KBPS, 320);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.mp3");
    write_mp3(&path, &tone()).unwrap();
    let frames = mp3_frames(&std::fs::read(&path).unwrap());
    assert!(
        frames.iter().all(|frame| *frame == (320, 44_100)),
        "{frames:?}"
    );
    // Two seconds is 76.6 frames of 1152 samples, so at least 77 frames of
    // audio, plus the encoder's information frame and the frames that flush
    // what it keeps back.
    assert!((77..=82).contains(&frames.len()), "{} frames", frames.len());
}

#[test]
fn frames_written_in_blocks_give_the_same_bytes_as_one_write() {
    let dir = tempfile::tempdir().unwrap();
    let whole = dir.path().join("whole.mp3");
    let blocks = dir.path().join("blocks.mp3");
    let audio = tone();
    write_mp3(&whole, &audio).unwrap();

    let mut file = Mp3File::create(&blocks).unwrap();
    assert!(file.is_empty());
    for block in audio.frames.chunks(1024) {
        file.write(block).unwrap();
    }
    assert_eq!(file.len(), Samples(88_200));
    file.finish().unwrap();

    assert_eq!(
        std::fs::read(&whole).unwrap(),
        std::fs::read(&blocks).unwrap(),
        "the bytes differ"
    );
}

#[test]
fn a_file_that_cannot_be_created_is_an_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no/such/folder/tone.mp3");
    let problem = Mp3File::create(&path).unwrap_err();
    assert_eq!(problem.path, path);
    assert!(problem.to_string().contains("tone.mp3"), "{problem}");
    let problem = write_mp3(&path, &tone()).unwrap_err();
    assert_eq!(problem.path, path);
}

#[test]
fn a_sample_that_is_not_a_number_is_written_as_silence_rather_than_stopping_the_program() {
    // The encoder is C code that fails an assertion on a NaN sample and
    // ends the whole process, so the writer turns such a sample into silence
    // before the encoder sees it.
    let mut audio = tone();
    audio.frames[5_000] = [f32::NAN, f32::NAN];
    audio.frames[60_000][0] = f32::NAN;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nan.mp3");
    write_mp3(&path, &audio).unwrap();
    let decoded = decode(&path).unwrap();
    let len = decoded.audio.len().0;
    assert!((len - 88_200).abs() <= 2_400, "length {len}");
    let middle = &decoded.audio.frames[22_050..66_150];
    assert!(
        close(rms(middle, 0), HALF_SINE_RMS, 0.05),
        "left rms {}",
        rms(middle, 0)
    );
    assert!(
        decoded
            .audio
            .frames
            .iter()
            .all(|frame| frame[0].is_finite() && frame[1].is_finite())
    );
}

#[test]
fn values_past_full_scale_are_clamped_rather_than_wrapped() {
    // A tone at twice full scale decodes as a tone clipped at full scale.
    // A sine clipped at half its amplitude has a root mean square of 0.884,
    // since two thirds of each cycle sits at full scale, and nothing wraps
    // to the opposite sign, which would pull the level far below that.
    let loud = Audio {
        frames: tone()
            .frames
            .iter()
            .map(|frame| [frame[0] * 4.0, frame[1] * 4.0])
            .collect(),
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("loud.mp3");
    write_mp3(&path, &loud).unwrap();
    let decoded = decode(&path).unwrap();
    let middle = &decoded.audio.frames[22_050..66_150];
    let level = rms(middle, 0);
    assert!(
        level > 0.8 && level < 1.0,
        "a sine clipped at half its amplitude has a level near 0.884, not {level}"
    );
}
