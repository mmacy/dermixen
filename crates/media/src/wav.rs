//! Writing audio to WAV files.

use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};

use crate::Audio;

/// How samples are stored in a written WAV file.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum WavDepth {
    /// Sixteen-bit signed integers, the export default. Values outside minus
    /// one to one are clamped to the nearest representable value.
    Int16,
    /// Thirty-two-bit floats, which store the internal format exactly.
    Float32,
}

/// The reason a WAV file could not be written.
#[derive(Debug, thiserror::Error)]
#[error("cannot write {path}: {message}")]
pub struct WavError {
    /// The file.
    pub path: PathBuf,
    /// What went wrong.
    pub message: String,
}

/// How many bytes [`WavFile::capacity`] keeps back for the header.
///
/// A WAV file states the size of its audio and the size of the whole file as
/// 32-bit numbers, so the two sizes together stay under 4 GiB. The header
/// hound writes is 44 bytes, and this leaves room for a longer one.
const HEADER_ROOM: u32 = 128;

/// The WAV spec for stereo audio at the internal sample rate, at the given depth.
fn spec_for(depth: WavDepth) -> WavSpec {
    let (bits_per_sample, sample_format) = match depth {
        WavDepth::Int16 => (16, SampleFormat::Int),
        WavDepth::Float32 => (32, SampleFormat::Float),
    };
    WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample,
        sample_format,
    }
}

/// Converts a sample whose full-scale range is minus one to one into the
/// nearest sixteen-bit step, clamping anything beyond full scale.
fn to_i16_sample(sample: f32) -> i16 {
    (f64::from(sample) * 32_768.0)
        .round()
        .clamp(-32_768.0, 32_767.0) as i16
}

/// Writes one frame's samples to `writer` at the given depth: at sixteen
/// bits, each sample is scaled by 32,768, rounded to the nearest integer,
/// and clamped to the representable range; at thirty-two-bit float, each
/// sample is stored as is.
fn write_frame(
    writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>,
    frame: &crate::Frame,
    depth: WavDepth,
) -> Result<(), hound::Error> {
    match depth {
        WavDepth::Int16 => {
            for sample in frame {
                writer.write_sample(to_i16_sample(*sample))?;
            }
        }
        WavDepth::Float32 => {
            for sample in frame {
                writer.write_sample(*sample)?;
            }
        }
    }
    Ok(())
}

/// Writes stereo audio at the internal sample rate to a WAV file, replacing
/// any file already there. Converts samples the way [`write_frame`] does.
pub fn write_wav(path: &Path, audio: &Audio, depth: WavDepth) -> Result<(), WavError> {
    let to_wav_error = |error: hound::Error| WavError {
        path: path.to_path_buf(),
        message: error.to_string(),
    };

    let mut writer = WavWriter::create(path, spec_for(depth)).map_err(to_wav_error)?;
    for frame in &audio.frames {
        write_frame(&mut writer, frame, depth).map_err(to_wav_error)?;
    }
    writer.finalize().map_err(to_wav_error)
}

/// A WAV file being written block by block, for audio too long to hold in
/// memory at once.
///
/// Dropping a `WavFile` without calling [`WavFile::finish`] lets the
/// underlying writer complete the header as best it can, but any error from
/// doing so is discarded. A caller that needs to know the file completed
/// correctly must call [`WavFile::finish`].
pub struct WavFile {
    writer: WavWriter<std::io::BufWriter<std::fs::File>>,
    depth: WavDepth,
    path: PathBuf,
    frames_written: i64,
}

impl WavFile {
    /// Starts a stereo WAV file at the internal sample rate at `path`,
    /// replacing any file already there.
    pub fn create(path: &Path, depth: WavDepth) -> Result<WavFile, WavError> {
        let writer = WavWriter::create(path, spec_for(depth)).map_err(|error| WavError {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        Ok(WavFile {
            writer,
            depth,
            path: path.to_path_buf(),
            frames_written: 0,
        })
    }

    /// Starts a stereo WAV file at the internal sample rate in a file that is
    /// already open, which is how a caller writes through
    /// [`dermixen_core::files::AtomicFile`]. `path` names the file in error
    /// messages and is never opened.
    pub fn from_file(
        file: std::fs::File,
        path: &Path,
        depth: WavDepth,
    ) -> Result<WavFile, WavError> {
        let writer =
            WavWriter::new(std::io::BufWriter::new(file), spec_for(depth)).map_err(|error| {
                WavError {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                }
            })?;
        Ok(WavFile {
            writer,
            depth,
            path: path.to_path_buf(),
            frames_written: 0,
        })
    }

    /// The most frames a WAV file of this depth can hold. A WAV file states
    /// the size of its audio in 32 bits, so the audio and the header together
    /// stay under 4 GiB.
    pub fn capacity(depth: WavDepth) -> dermixen_core::Samples {
        let bytes_per_frame = match depth {
            WavDepth::Int16 => 4,
            WavDepth::Float32 => 8,
        };
        dermixen_core::Samples(i64::from((u32::MAX - HEADER_ROOM) / bytes_per_frame))
    }

    /// Appends frames to the file, converting them the way [`write_frame`] does.
    ///
    /// A write that would take the file past [`WavFile::capacity`] is an
    /// error whose message names the 4 GiB limit, and it writes nothing.
    pub fn write(&mut self, frames: &[crate::Frame]) -> Result<(), WavError> {
        let to_wav_error = |error: hound::Error| WavError {
            path: self.path.clone(),
            message: error.to_string(),
        };
        let capacity = WavFile::capacity(self.depth).0;
        if self.frames_written + frames.len() as i64 > capacity {
            return Err(WavError {
                path: self.path.clone(),
                message: format!(
                    "a WAV file describes at most 4 GiB, which is {capacity} frames at this depth"
                ),
            });
        }
        for frame in frames {
            write_frame(&mut self.writer, frame, self.depth).map_err(to_wav_error)?;
            self.frames_written += 1;
        }
        Ok(())
    }

    /// Completes the header and closes the file.
    pub fn finish(self) -> Result<(), WavError> {
        let path = self.path.clone();
        self.writer.finalize().map_err(|error| WavError {
            path,
            message: error.to_string(),
        })
    }

    /// How many frames have been written so far.
    pub fn len(&self) -> dermixen_core::Samples {
        dermixen_core::Samples(self.frames_written)
    }

    /// Whether no frames have been written yet.
    pub fn is_empty(&self) -> bool {
        self.len() == dermixen_core::Samples::ZERO
    }
}

impl std::fmt::Debug for WavFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WavFile")
            .field("path", &self.path)
            .field("depth", &self.depth)
            .finish()
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn the_capacity_is_what_fits_under_four_gibibytes() {
        // Four bytes a frame at 16 bits and eight at 32. The header is 44
        // bytes at the least, and no header hound writes reaches 128.
        let int16 = WavFile::capacity(WavDepth::Int16).0;
        let float32 = WavFile::capacity(WavDepth::Float32).0;
        let most = i64::from(u32::MAX);
        assert!(int16 * 4 + 44 <= most, "{int16}");
        assert!((int16 + 64) * 4 + 128 > most, "{int16}");
        assert!(float32 * 8 + 44 <= most, "{float32}");
        assert!((float32 + 64) * 8 + 128 > most, "{float32}");
        // Six hours of 16-bit stereo fits, and seven do not.
        assert!(int16 > 6 * 3_600 * 44_100);
        assert!(int16 < 7 * 3_600 * 44_100);
    }

    #[test]
    fn a_write_past_the_capacity_is_refused_and_writes_nothing() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("long.wav");
        let mut file = WavFile::create(&path, WavDepth::Int16).unwrap();
        // The count of frames written is what the limit is measured from, so
        // the test sets it rather than writing four gibibytes.
        file.frames_written = WavFile::capacity(WavDepth::Int16).0 - 1;
        file.write(&[[0.0, 0.0]]).unwrap();
        let problem = file.write(&[[0.0, 0.0]]).unwrap_err();
        assert!(problem.message.contains("4 GiB"), "{problem}");
        assert_eq!(file.len(), WavFile::capacity(WavDepth::Int16));
    }
}
