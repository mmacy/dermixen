#![forbid(unsafe_code)]

//! Decoding, encoding, and content hashing of audio files.
//!
//! Every buffer that leaves this crate is stereo, 32-bit float, at
//! [`SAMPLE_RATE`](dermixen_core::SAMPLE_RATE); files at other rates are
//! resampled on the way in.

use dermixen_core::{Samples, Seconds};

pub mod decode;
pub mod hash;
#[cfg(feature = "mp3")]
pub mod mp3;
pub mod overview;
pub mod wav;

pub use decode::{DecodeError, Decoded, decode};
pub use hash::hash_file;
#[cfg(feature = "mp3")]
pub use mp3::{MP3_BITRATE_KBPS, Mp3Error, Mp3File, write_mp3};
pub use overview::{Overview, Peak};
pub use wav::{WavDepth, WavError, WavFile, write_wav};

/// One stereo sample: the left channel then the right.
pub type Frame = [f32; 2];

/// Decoded audio at the fixed internal format.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Audio {
    /// The frames in playback order.
    pub frames: Vec<Frame>,
}

impl Audio {
    /// Audio with no frames.
    pub fn new() -> Self {
        Self::default()
    }

    /// The length in samples.
    pub fn len(&self) -> Samples {
        Samples(self.frames.len() as i64)
    }

    /// Whether there are no frames.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// The length in seconds.
    pub fn duration(&self) -> Seconds {
        self.len().to_seconds()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_is_counted_in_frames() {
        let audio = Audio {
            frames: vec![[0.0, 0.0]; 44_100],
        };
        assert_eq!(audio.len(), Samples(44_100));
        assert_eq!(audio.duration(), Seconds(1.0));
        assert!(!audio.is_empty());
        assert!(Audio::new().is_empty());
    }
}
