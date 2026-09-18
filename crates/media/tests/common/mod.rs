//! Helpers shared by the media acceptance tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use dermixen_media::Frame;

/// The directory of committed audio fixtures.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

/// The CRC32 of the frames as interleaved 16-bit little-endian samples, which
/// is what `tools/make_audio_fixtures.py` prints for each WAV it writes.
pub fn crc32_of_int16(frames: &[Frame], channels: usize) -> u32 {
    let mut bytes = Vec::with_capacity(frames.len() * channels * 2);
    for frame in frames {
        for value in frame.iter().take(channels) {
            let sample = (f64::from(*value) * 32768.0)
                .round()
                .clamp(-32768.0, 32767.0) as i16;
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
    }
    crc32fast::hash(&bytes)
}

/// The number of sign changes in the left channel over a range of frames.
pub fn zero_crossings(frames: &[Frame]) -> usize {
    frames
        .windows(2)
        .filter(|w| (w[0][0] < 0.0) != (w[1][0] < 0.0))
        .count()
}

/// The root mean square of one channel over a range of frames.
pub fn rms(frames: &[Frame], channel: usize) -> f64 {
    let sum: f64 = frames
        .iter()
        .map(|f| f64::from(f[channel]) * f64::from(f[channel]))
        .sum();
    (sum / frames.len() as f64).sqrt()
}
