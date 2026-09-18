//! Writing MP3 files, which is the second export format `DESIGN.md` names
//! under "Supported formats": 320 kilobits per second at a constant bit
//! rate, through the LAME encoder.
//!
//! The file is written frame by frame as the render produces them, the way
//! [`WavFile`](crate::WavFile) writes a WAV, so a render of a long mix never
//! keeps the whole mix in memory. This module exists only in a build with
//! the `mp3` feature, because the encoder is C code linked from outside the
//! workspace, and `dermixen render` refuses an MP3 output in a build without
//! it and says why. The encoder is LAME, reached through the
//! `mp3lame-encoder` crate, which keeps every unsafe call outside this
//! workspace.

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use dermixen_core::{SAMPLE_RATE, Samples};
use mp3lame_encoder::{
    Bitrate, Builder, Encoder, FlushGap, InterleavedPcm, Mode, Quality, VbrMode,
    max_required_buffer_size,
};

use crate::{Audio, Frame};

/// The bit rate every MP3 is written at, in kilobits per second.
pub const MP3_BITRATE_KBPS: u32 = 320;

/// The sample rate every MP3 is written at, in the type the encoder's
/// setter takes.
const OUTPUT_SAMPLE_RATE: NonZeroU32 = NonZeroU32::new(SAMPLE_RATE).unwrap();

/// How many frames go to the encoder in one call. Every write fills a
/// buffer this size and the leftover waits for the next write, so the
/// encoder gets the same blocks whatever sizes the caller writes in, and
/// the bytes come out the same.
const ENCODE_BLOCK_FRAMES: usize = 4_096;

/// How much room the final flush of the encoder needs, in bytes. The
/// `mp3lame-encoder` documentation names this number.
const FLUSH_CAPACITY: usize = 7_200;

/// The reason an MP3 file could not be written.
#[derive(Debug, thiserror::Error)]
#[error("cannot write {path}: {message}")]
pub struct Mp3Error {
    /// The file.
    pub path: PathBuf,
    /// What went wrong.
    pub message: String,
}

/// Converts one sample to what the encoder takes. A sample that is not a
/// number becomes silence, because LAME stops the whole process on one of
/// those: it fails an assertion in its `psymodel.c` and never returns, so
/// no error could reach the caller. Anything past full scale clamps to
/// full scale, which covers plus and minus infinity too.
fn to_encoder_sample(sample: f32) -> f32 {
    if sample.is_nan() {
        0.0
    } else {
        sample.clamp(-1.0, 1.0)
    }
}

/// Builds an [`Mp3Error`] for `path` out of anything that can print itself.
fn cannot_write(path: &Path, message: impl std::fmt::Display) -> Mp3Error {
    Mp3Error {
        path: path.to_path_buf(),
        message: message.to_string(),
    }
}

/// Starts a LAME encoder set to the one format this crate writes: two
/// channels at [`SAMPLE_RATE`], joint stereo, [`MP3_BITRATE_KBPS`] at a
/// constant bit rate, the encoder's best quality setting, and no tags.
///
/// This also turns on the encoder's own information frame, which sits at
/// the front of the file and names the encoder, the length, and the
/// samples of silence the encoder adds at each end, so a player can leave
/// the added silence out.
fn start_encoder() -> Result<Encoder, String> {
    let mut builder = Builder::new().ok_or_else(|| "cannot start the LAME encoder".to_string())?;
    let set = |what: &str, result: Result<(), mp3lame_encoder::BuildError>| {
        result.map_err(|error| format!("the LAME encoder did not accept the {what}: {error}"))
    };
    set("channel count", builder.set_num_channels(2))?;
    set("input sample rate", builder.set_sample_rate(SAMPLE_RATE))?;
    set(
        "output sample rate",
        builder.set_output_sample_rate(Some(OUTPUT_SAMPLE_RATE)),
    )?;
    set("channel mode", builder.set_mode(Mode::JointStereo))?;
    set("bit rate mode", builder.set_vbr_mode(VbrMode::Off))?;
    set("bit rate", builder.set_brate(Bitrate::Kbps320))?;
    set("quality", builder.set_quality(Quality::Best))?;
    set(
        "information frame setting",
        builder.set_to_write_vbr_tag(true),
    )?;
    builder
        .build()
        .map_err(|error| format!("the LAME encoder did not start: {error}"))
}

/// An MP3 file being written frame by frame: stereo at the internal sample
/// rate, at [`MP3_BITRATE_KBPS`] constant bit rate, with no tags.
///
/// The encoder keeps a few frames of audio until [`Mp3File::finish`]
/// flushes them and writes the encoder's own information frame, so a file
/// that drops before `finish` completes is incomplete. `Mp3File` removes
/// that file in its `Drop`. A caller that needs to know the file completed
/// must call `finish`.
pub struct Mp3File {
    /// The LAME encoder this file's audio goes through.
    encoder: Encoder,
    /// The file on disk, kept open so `finish` can go back to the front of
    /// the file and put the information frame in the room the encoder
    /// reserved there.
    file: File,
    /// Where the file is. Every [`Mp3Error`] names this path, and the
    /// `Drop` for `Mp3File` removes this file when `finish` has not
    /// completed.
    path: PathBuf,
    /// Samples given to [`Mp3File::write`] that have not gone to the
    /// encoder yet, interleaved and put through [`to_encoder_sample`], at
    /// most one block of them.
    pending: Vec<f32>,
    /// The buffer the encoder writes its bytes into, reused between calls.
    encoded: Vec<u8>,
    /// How many frames [`Mp3File::write`] has taken.
    frames_written: i64,
    /// How many bytes are in the file so far.
    bytes_written: u64,
    /// Whether [`Mp3File::finish`] completed. The `Drop` for `Mp3File`
    /// removes the file unless this field is true.
    finished: bool,
}

impl Mp3File {
    /// Starts an MP3 file in a file that is already open, which is how a
    /// caller writes through [`dermixen_core::files::AtomicFile`]. `path`
    /// names the file in error messages and is never opened or removed.
    pub fn from_file(file: File, path: &Path) -> Result<Mp3File, Mp3Error> {
        let _ = file;
        Err(cannot_write(
            path,
            "writing to an open file is not implemented",
        ))
    }

    /// Starts an MP3 file at `path`, replacing any file already there.
    pub fn create(path: &Path) -> Result<Mp3File, Mp3Error> {
        // The encoder starts first, because `File::create` truncates any
        // file already at the path and an encoder that does not start would
        // leave an empty file where a previous export was.
        let encoder = start_encoder().map_err(|message| cannot_write(path, message))?;
        let file = File::create(path).map_err(|error| cannot_write(path, error))?;
        Ok(Mp3File {
            encoder,
            file,
            path: path.to_path_buf(),
            pending: Vec::with_capacity(ENCODE_BLOCK_FRAMES * 2),
            encoded: Vec::with_capacity(max_required_buffer_size(ENCODE_BLOCK_FRAMES)),
            frames_written: 0,
            bytes_written: 0,
            finished: false,
        })
    }

    /// Appends frames to the file. Values outside minus one to one are
    /// clamped, as they are for a 16-bit WAV, and a sample that is not a
    /// number is written as silence.
    pub fn write(&mut self, frames: &[Frame]) -> Result<(), Mp3Error> {
        for frame in frames {
            self.pending.push(to_encoder_sample(frame[0]));
            self.pending.push(to_encoder_sample(frame[1]));
            self.frames_written += 1;
            if self.pending.len() >= ENCODE_BLOCK_FRAMES * 2 {
                self.encode_pending()?;
            }
        }
        Ok(())
    }

    /// Hands everything waiting in `pending` to the encoder and appends
    /// whatever bytes the encoder gives back.
    fn encode_pending(&mut self) -> Result<(), Mp3Error> {
        self.encoded.clear();
        self.encoded
            .reserve(max_required_buffer_size(self.pending.len() / 2));
        // The buffer empties either way, so a caller that keeps writing
        // after an error does not hand the encoder the same samples again.
        let encoded = self
            .encoder
            .encode_to_vec(InterleavedPcm(&self.pending[..]), &mut self.encoded);
        self.pending.clear();
        encoded.map_err(|error| cannot_write(&self.path, error))?;
        self.file
            .write_all(&self.encoded)
            .map_err(|error| cannot_write(&self.path, error))?;
        self.bytes_written += self.encoded.len() as u64;
        Ok(())
    }

    /// Flushes the frames the encoder still keeps, writes the encoder's
    /// information frame, and closes the file.
    pub fn finish(mut self) -> Result<(), Mp3Error> {
        if !self.pending.is_empty() {
            self.encode_pending()?;
        }

        self.encoded.clear();
        self.encoded.reserve(FLUSH_CAPACITY);
        self.encoder
            .flush_to_vec::<FlushGap>(&mut self.encoded)
            .map_err(|error| cannot_write(&self.path, error))?;
        self.file
            .write_all(&self.encoded)
            .map_err(|error| cannot_write(&self.path, error))?;
        self.bytes_written += self.encoded.len() as u64;

        self.write_information_frame()?;
        self.file
            .flush()
            .map_err(|error| cannot_write(&self.path, error))?;
        self.finished = true;
        Ok(())
    }

    /// Puts the encoder's information frame at the front of the file, over
    /// the empty frame the encoder writes there to reserve the room. The
    /// file contains no ID3 tag, so that room starts at the first byte.
    ///
    /// The encoder reserves no room for that frame at some settings, and
    /// this method then writes nothing.
    fn write_information_frame(&mut self) -> Result<(), Mp3Error> {
        let size = self.encoder.lame_tag_size();
        if !self.encoder.is_lame_tag_written() || size == 0 || self.bytes_written < size as u64 {
            return Ok(());
        }
        self.encoded.clear();
        self.encoded.reserve(size);
        self.encoder
            .lame_tag_encode_to_vec(&mut self.encoded)
            .ok_or_else(|| {
                cannot_write(&self.path, "the LAME encoder returned no information frame")
            })?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(|error| cannot_write(&self.path, error))?;
        self.file
            .write_all(&self.encoded)
            .map_err(|error| cannot_write(&self.path, error))?;
        Ok(())
    }

    /// How many frames have been given to the file so far, whether or not
    /// the encoder has written them out yet.
    pub fn len(&self) -> Samples {
        Samples(self.frames_written)
    }

    /// Whether no frames have been given yet.
    pub fn is_empty(&self) -> bool {
        self.len() == Samples::ZERO
    }
}

impl Drop for Mp3File {
    /// Removes the file when [`Mp3File::finish`] has not completed, because
    /// the file on disk is then missing both the frames the encoder still
    /// keeps and the information frame at the front.
    fn drop(&mut self) {
        if !self.finished {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl std::fmt::Debug for Mp3File {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mp3File").finish_non_exhaustive()
    }
}

/// Writes whole audio to an MP3 file at `path`, as [`Mp3File`] would frame
/// by frame. The bytes written are the same either way.
pub fn write_mp3(path: &Path, audio: &Audio) -> Result<(), Mp3Error> {
    let mut file = Mp3File::create(path)?;
    file.write(&audio.frames)?;
    file.finish()
}
