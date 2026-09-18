//! Reading audio files into the internal format.

use std::io::{Cursor, Read};
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};

use dermixen_core::files::open_regular;
use dermixen_core::{ContentHash, LONGEST_TRACK, SAMPLE_RATE};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::{Audio, Frame};

/// The lowest sample rate a file may state, in samples per second.
pub const LOWEST_SOURCE_RATE: u32 = 8_000;

/// The highest sample rate a file may state, in samples per second.
pub const HIGHEST_SOURCE_RATE: u32 = 384_000;

/// The largest audio file [`decode`] reads, in bytes.
pub const LARGEST_AUDIO_FILE: u64 = 2 * 1024 * 1024 * 1024;

/// The largest magnitude of a decoded sample, where 1 is full scale.
///
/// [`decode`] replaces a sample that is not a number with silence and clamps
/// a larger one to this, so every consumer of decoded audio, from the
/// analyzers and the stretcher to the audio device, receives finite samples.
pub const SAMPLE_LIMIT: f32 = 8.0;

/// A decoded file: its audio in the internal format, its identity, and what the file itself held.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    /// The audio, stereo at the internal sample rate.
    pub audio: Audio,
    /// The hash of the file's bytes.
    pub hash: ContentHash,
    /// The sample rate the file was stored at.
    pub source_sample_rate: u32,
    /// The number of channels the file was stored with.
    pub source_channels: u16,
}

/// The reason a file could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// The file could not be opened or read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file is in a format Dermixen does not import, or has more than two channels.
    #[error("{path}: {message}")]
    Unsupported {
        /// The file.
        path: PathBuf,
        /// What is unsupported about it.
        message: String,
    },
    /// The file claims a supported format but its contents cannot be decoded.
    #[error("{path}: {message}")]
    Corrupt {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        message: String,
    },
}

/// Decodes a WAV, MP3, FLAC, MP4, or M4A file to stereo 32-bit float at the internal sample rate.
///
/// Dermixen reads the AAC-LC audio in an MP4 or M4A file and decodes the
/// file's default audio track. A mono file is copied to both channels. A file
/// with more than two channels is refused. A file stored at another sample
/// rate is resampled, so its length changes in proportion. The whole file is
/// read. There is no streaming.
///
/// The path must name a regular file of at most [`LARGEST_AUDIO_FILE`]
/// bytes, which is checked before the file is read. A stated sample rate
/// outside [`LOWEST_SOURCE_RATE`] to [`HIGHEST_SOURCE_RATE`], and audio
/// longer than [`dermixen_core::LONGEST_TRACK`] once it is at the internal
/// rate, are [`DecodeError::Unsupported`]. The length is refused as soon as
/// the decoded packets pass it, before the rest of the file is decoded.
/// Every decoded sample is a finite number within [`SAMPLE_LIMIT`].
pub fn decode(path: &Path) -> Result<Decoded, DecodeError> {
    let bytes = read_whole_file(path)?;
    let hash = ContentHash(*blake3::hash(&bytes).as_bytes());

    let (frames, source_sample_rate, source_channels) = decode_without_panicking(path, bytes)?;
    let frames = if source_sample_rate == SAMPLE_RATE {
        frames
    } else {
        resample(path, frames, source_sample_rate)?
    };

    Ok(Decoded {
        audio: Audio { frames },
        hash,
        source_sample_rate,
        source_channels,
    })
}

/// Reads every byte of a regular file of at most [`LARGEST_AUDIO_FILE`] bytes.
///
/// The file is opened as [`open_regular`] opens one, so a folder, a device, or
/// a named pipe is refused at once rather than read. The size comes from the
/// open handle, before any of the file is read, and the read itself stops one
/// byte past the limit, so a file that grows while it is being read is bounded
/// too.
fn read_whole_file(path: &Path) -> Result<Vec<u8>, DecodeError> {
    let read = |source| DecodeError::Read {
        path: path.to_path_buf(),
        source,
    };
    let too_large = |size: u64| DecodeError::Unsupported {
        path: path.to_path_buf(),
        message: format!(
            "the file is {size} bytes, and Dermixen imports an audio file of at most {LARGEST_AUDIO_FILE} bytes"
        ),
    };

    let file = open_regular(path).map_err(|error| read(crate::hash::as_io_error(error)))?;
    let size = file.metadata().map_err(read)?.len();
    if size > LARGEST_AUDIO_FILE {
        return Err(too_large(size));
    }

    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
    file.take(LARGEST_AUDIO_FILE.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(read)?;
    if bytes.len() as u64 > LARGEST_AUDIO_FILE {
        return Err(too_large(bytes.len() as u64));
    }
    Ok(bytes)
}

/// Decodes the whole file, and reports a panic from the decoding library as a
/// damaged file.
///
/// Two headers reach arithmetic in symphonia 0.6.1 that overflows: a WAV file
/// that states 32,770 channels or more, and an MP4 file whose `stts` box
/// states 4,294,967,295 samples. A debug build of symphonia panics on the
/// overflow and prints the panic to the standard error. Catching the panic
/// here is what keeps one such file from ending a library scan or a render.
fn decode_without_panicking(
    path: &Path,
    bytes: Vec<u8>,
) -> Result<(Vec<Frame>, u32, u16), DecodeError> {
    let work = AssertUnwindSafe(move || decode_samples(path, bytes));
    std::panic::catch_unwind(work).unwrap_or_else(|_| {
        Err(DecodeError::Corrupt {
            path: path.to_path_buf(),
            message: "the file's header states a number the decoder cannot work with".to_owned(),
        })
    })
}

/// Where the audio starts in `bytes`: after an ID3v2 tag, or at the beginning.
///
/// Symphonia looks for the first MPEG frame header from the first byte of the
/// stream. An ID3v2 tag whose text is stored as UTF-16 begins each text frame
/// with the byte order mark `FF FE`, and those two bytes read as the start of
/// an MPEG-1 layer I frame header. Symphonia can lock onto one, report the
/// track as layer I, and then find no decoder for it, because this build
/// enables symphonia's MP3 decoder alone. Handing over only the bytes after
/// the tag keeps the tag out of that search. Nothing is lost by leaving it
/// out: the artist, title, and year are read from the tag separately, by the
/// library crate, and the hash covers the whole file either way.
fn audio_start(bytes: &[u8]) -> usize {
    // An ID3v2 tag opens with the three letters `ID3`, a two-byte version, a
    // flags byte, and the length of the rest of the tag as four bytes of seven
    // bits each. The length counts neither that ten-byte header nor the
    // ten-byte footer that the flags byte can call for.
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    if bytes[6..10].iter().any(|byte| byte & 0x80 != 0) {
        // The eighth bit of a length byte is always zero in a real tag, so
        // these four bytes are something else and the file starts where it is.
        return 0;
    }
    let length = bytes[6..10]
        .iter()
        .fold(0usize, |total, byte| (total << 7) | usize::from(*byte));
    let footer = if bytes[5] & 0x10 == 0 { 0 } else { 10 };
    let start = 10 + length + footer;
    // A tag that claims to run past the end of the file is not one to trust.
    if start <= bytes.len() { start } else { 0 }
}

/// Decodes the whole file to stereo frames at the rate the file was stored
/// at, and reports that rate and the channel count the file held.
///
/// The frames are built packet by packet, so the decoded audio is held once
/// rather than as interleaved samples and as frames at the same time.
///
/// The file's bytes are handed over rather than borrowed because the decoder
/// reads from them for as long as it runs.
fn decode_samples(path: &Path, bytes: Vec<u8>) -> Result<(Vec<Frame>, u32, u16), DecodeError> {
    // The file name extension only tells the probe which format to try first.
    // A file whose name does not match its contents is still recognized.
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    // Symphonia seeks its own stream back to the first byte it is given, so
    // the tag has to be cut off rather than skipped over.
    let mut bytes = bytes;
    bytes.drain(..audio_start(&bytes));
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            stream,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| from_symphonia(path, error))?;

    let track = reader
        .default_track(TrackType::Audio)
        .ok_or_else(|| DecodeError::Unsupported {
            path: path.to_path_buf(),
            message: "the file holds no audio track".to_owned(),
        })?;
    let track_id = track.id;
    let stated_frames = track.num_frames;
    let Some(CodecParameters::Audio(parameters)) = track.codec_params.clone() else {
        return Err(DecodeError::Unsupported {
            path: path.to_path_buf(),
            message: "the audio track is in a format Dermixen does not import".to_owned(),
        });
    };

    // The rate the header states decides whether the file is imported at all,
    // and it is read before a decoder is made, so a rate that no resampler can
    // work from never reaches one.
    if let Some(rate) = parameters.sample_rate {
        check_rate(path, rate)?;
        if let Some(frames) = stated_frames
            && frames > longest_at(rate)
        {
            return Err(too_long(path));
        }
    }

    // Gapless decoding drops the delay an encoder inserts before the first
    // real sample and the padding it appends after the last one. Without it,
    // an MP3 begins with about twenty-five milliseconds of silence that no
    // listener was ever meant to hear. Dermixen places every beat, cue, and
    // transition by sample position, so that leading silence would leave the
    // whole track a fraction of a beat out of step with the rest of the mix.
    let decoder_options = AudioDecoderOptions::default().gapless(true);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&parameters, &decoder_options)
        .map_err(|error| from_symphonia(path, error))?;

    let mut frames: Vec<Frame> = Vec::new();
    let mut packet_samples: Vec<f32> = Vec::new();
    let mut source: Option<(u32, u16)> = None;
    let mut packets_seen = false;
    // How many frames at the rate the file is stored at are as long as a
    // track may be. The first packet that names a rate sets it.
    let mut longest = u64::MAX;
    loop {
        let packet = match reader.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // A reader that runs out of bytes mid-packet has reached the end of
            // what the file holds. Whatever was decoded up to here stands.
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(from_symphonia(path, error)),
        };
        if packet.track_id != track_id {
            continue;
        }
        packets_seen = true;
        let buffer = match decoder.decode(&packet) {
            Ok(buffer) => buffer,
            // Dermixen skips a packet the decoder cannot read, so one damaged
            // packet in an otherwise good file costs a fraction of a second
            // instead of the whole track. If every packet fails, the decode
            // ends with no frames and the file is refused below.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(from_symphonia(path, error)),
        };

        let channels = stereo_or_fewer(path, buffer.spec().channels().count())?;
        let rate = buffer.spec().rate();
        if source.map(|(seen, _)| seen) != Some(rate) {
            check_rate(path, rate)?;
            longest = longest_at(rate);
        }
        source = Some((rate, channels));

        buffer.copy_to_vec_interleaved(&mut packet_samples);
        // The length is measured on every packet, so a file longer than a
        // track may be is refused as soon as its packets pass that length
        // rather than after the rest of the file is decoded.
        let more = packet_samples.len() / usize::from(channels);
        if frames.len() as u64 + more as u64 > longest {
            return Err(too_long(path));
        }
        make_room(&mut frames, more, longest);
        if channels == 1 {
            frames.extend(packet_samples.iter().map(|sample| {
                let sample = within_the_limit(*sample);
                [sample, sample]
            }));
        } else {
            frames.extend(
                packet_samples
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|frame| [within_the_limit(frame[0]), within_the_limit(frame[1])]),
            );
        }
    }

    if let Some((rate, channels)) = source {
        return Ok((frames, rate, channels));
    }
    if packets_seen {
        return Err(DecodeError::Corrupt {
            path: path.to_path_buf(),
            message: "none of the file's audio could be decoded".to_owned(),
        });
    }

    // Nothing was decoded because the file holds nothing to decode. An empty
    // mix exported by Dermixen is exactly such a file, and the app has to be
    // able to read back what it wrote, so a well-formed file with no audio in
    // it comes back as silence of zero length rather than as damage. The
    // header still states the format the file was written in.
    let (Some(rate), Some(channels)) = (parameters.sample_rate, parameters.channels.as_ref())
    else {
        return Err(DecodeError::Corrupt {
            path: path.to_path_buf(),
            message: "the file holds no audio and states no sample rate or channel count"
                .to_owned(),
        });
    };
    Ok((Vec::new(), rate, stereo_or_fewer(path, channels.count())?))
}

/// Accepts a sample rate within [`LOWEST_SOURCE_RATE`] to
/// [`HIGHEST_SOURCE_RATE`] and refuses any other.
///
/// The resampler works out how much room its output needs from the two rates,
/// so a file that states one hertz asks it for tens of thousands of samples
/// for every sample the file holds. Reading the rate first is what keeps that
/// from happening.
fn check_rate(path: &Path, rate: u32) -> Result<(), DecodeError> {
    if (LOWEST_SOURCE_RATE..=HIGHEST_SOURCE_RATE).contains(&rate) {
        return Ok(());
    }
    Err(DecodeError::Unsupported {
        path: path.to_path_buf(),
        message: format!(
            "the file states a sample rate of {rate} Hz, and Dermixen imports a sample rate from {LOWEST_SOURCE_RATE} Hz to {HIGHEST_SOURCE_RATE} Hz"
        ),
    })
}

/// How many frames at `rate` last as long as [`LONGEST_TRACK`] does at the
/// internal sample rate.
fn longest_at(rate: u32) -> u64 {
    LONGEST_TRACK.0.max(0) as u64 * u64::from(rate) / u64::from(SAMPLE_RATE)
}

/// The error for a file that holds more audio than a track may.
fn too_long(path: &Path) -> DecodeError {
    let minutes = LONGEST_TRACK.0 / (i64::from(SAMPLE_RATE) * 60);
    DecodeError::Unsupported {
        path: path.to_path_buf(),
        message: format!(
            "Dermixen imports a track of at most {minutes} minutes, and this one is longer"
        ),
    }
}

/// A sample the rest of the app can work with: silence in place of a value
/// that is not a number, and anything larger cut down to [`SAMPLE_LIMIT`].
///
/// A sample already within the limit comes back unchanged, down to its bits,
/// so a file of ordinary audio decodes to exactly what it holds.
fn within_the_limit(sample: f32) -> f32 {
    if sample.is_nan() {
        0.0
    } else {
        sample.clamp(-SAMPLE_LIMIT, SAMPLE_LIMIT)
    }
}

/// Makes room in `frames` for `more` frames, never reserving room for more
/// than `longest` frames in all.
///
/// A vector that grows on its own doubles, so a file that runs right up to the
/// length limit would end up with room for twice the limit. Reserving the
/// exact amount once the doubling passes the limit keeps the memory a decode
/// asks for close to the memory the audio takes.
fn make_room(frames: &mut Vec<Frame>, more: usize, longest: u64) {
    if frames.capacity() - frames.len() >= more {
        return;
    }
    let longest = usize::try_from(longest).unwrap_or(usize::MAX);
    let doubled = frames
        .capacity()
        .saturating_mul(2)
        .max(frames.len().saturating_add(more));
    frames.reserve_exact(doubled.min(longest) - frames.len());
}

/// Accepts a mono or stereo channel count and refuses any other.
fn stereo_or_fewer(path: &Path, channels: usize) -> Result<u16, DecodeError> {
    if channels == 0 || channels > 2 {
        return Err(DecodeError::Unsupported {
            path: path.to_path_buf(),
            message: format!(
                "Dermixen imports mono and stereo files, and this one has {channels} channels"
            ),
        });
    }
    Ok(channels as u16)
}

/// Resamples stereo frames from `rate` to the internal sample rate.
///
/// The pitch is unchanged and the length changes in proportion to the two
/// rates, so a file at 48 kHz comes out about eight percent shorter in frames
/// and exactly as long in seconds.
///
/// The frames are handed over rather than borrowed, so that the memory the
/// file was decoded into is given back before the resampled frames are built.
fn resample(path: &Path, frames: Vec<Frame>, rate: u32) -> Result<Vec<Frame>, DecodeError> {
    let unsupported = || DecodeError::Unsupported {
        path: path.to_path_buf(),
        message: format!("the file is at {rate} Hz, which cannot be resampled to {SAMPLE_RATE} Hz"),
    };
    if rate == 0 {
        return Err(unsupported());
    }
    if frames.is_empty() {
        return Ok(Vec::new());
    }

    let ratio = f64::from(SAMPLE_RATE) / f64::from(rate);
    // The window is long enough to keep the resampling artifacts far below the
    // noise floor of any source Dermixen imports.
    let parameters = SincInterpolationParameters::new(256, WindowFunction::BlackmanHarris2)
        .oversampling_factor(256)
        .interpolation(SincInterpolationType::Quadratic);
    let mut resampler = Async::<f32>::new_sinc(ratio, 1.1, &parameters, 1024, 2, FixedAsync::Input)
        .map_err(|_| unsupported())?;

    // The borrow of the frames ends with this block, so the frames the file
    // decoded to are given back before the resampled frames are built.
    let output = {
        let input = InterleavedSlice::new(frames.as_flattened(), 2, frames.len())
            .map_err(|_| unsupported())?;
        resampler
            .process_all(&input, frames.len(), None)
            .map_err(|_| unsupported())?
    };
    drop(frames);

    // Resampling a sample within the limit can overshoot it, so the output is
    // held to the limit the way the decoded packets are.
    let resampled = output.take_data();
    Ok(resampled
        .as_chunks::<2>()
        .0
        .iter()
        .map(|frame| [within_the_limit(frame[0]), within_the_limit(frame[1])])
        .collect())
}

/// Describes a decoding failure in terms of the file it happened on.
///
/// The file's bytes were read into memory before the probe and the decoder
/// ever saw them, so a stream that fails to read means the contents end sooner
/// than the format says they should. That is a damaged file rather than a
/// filesystem problem.
fn from_symphonia(path: &Path, error: SymphoniaError) -> DecodeError {
    let path = path.to_path_buf();
    match error {
        SymphoniaError::Unsupported(feature) => DecodeError::Unsupported {
            path,
            message: format!("Dermixen does not import this file: {feature}"),
        },
        other => DecodeError::Corrupt {
            path,
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a WAV file with a header and no samples, and returns where it went.
    fn write_empty_wav(dir: &Path, name: &str, rate: u32, channels: u16) -> PathBuf {
        let path = dir.join(name);
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        hound::WavWriter::create(&path, spec)
            .unwrap()
            .finalize()
            .unwrap();
        path
    }

    /// Builds an ID3v2 header for a tag whose body is `length` bytes.
    fn id3_header(length: u32, flags: u8) -> Vec<u8> {
        let mut header = b"ID3".to_vec();
        header.extend([3, 0, flags]);
        for shift in [21, 14, 7, 0] {
            header.push(((length >> shift) & 0x7F) as u8);
        }
        header
    }

    #[test]
    fn a_file_with_no_id3_tag_starts_at_its_first_byte() {
        assert_eq!(audio_start(b"RIFF....WAVEfmt "), 0);
        assert_eq!(audio_start(b"\xff\xfb\x90\x64"), 0);
        assert_eq!(audio_start(b""), 0);
        assert_eq!(audio_start(b"ID3"), 0);
    }

    #[test]
    fn an_id3_tag_moves_the_start_past_it() {
        let mut file = id3_header(500, 0);
        file.resize(10 + 500 + 4, 0);
        assert_eq!(audio_start(&file), 510);
    }

    #[test]
    fn a_tag_that_calls_for_a_footer_moves_the_start_ten_bytes_further() {
        let mut file = id3_header(500, 0x10);
        file.resize(10 + 500 + 10 + 4, 0);
        assert_eq!(audio_start(&file), 520);
    }

    #[test]
    fn a_tag_longer_than_the_file_is_not_trusted() {
        let mut file = id3_header(5_000, 0);
        file.resize(64, 0);
        assert_eq!(audio_start(&file), 0);
    }

    #[test]
    fn four_bytes_that_cannot_be_a_tag_length_are_not_read_as_one() {
        // A real length byte never has its eighth bit set.
        let mut file = b"ID3".to_vec();
        file.extend([3, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        file.resize(4_096, 0);
        assert_eq!(audio_start(&file), 0);
    }

    #[test]
    fn a_valid_file_with_no_audio_decodes_to_silence_of_zero_length() {
        let dir = tempfile::tempdir().unwrap();

        let stereo = write_empty_wav(dir.path(), "empty-stereo.wav", 44_100, 2);
        let decoded = decode(&stereo).unwrap();
        assert!(decoded.audio.is_empty());
        assert_eq!(decoded.source_sample_rate, 44_100);
        assert_eq!(decoded.source_channels, 2);
        assert_eq!(decoded.hash, crate::hash_file(&stereo).unwrap());

        // The rate and channel count come from the file itself, not from what
        // Dermixen would have preferred to find there.
        let mono = write_empty_wav(dir.path(), "empty-mono.wav", 22_050, 1);
        let decoded = decode(&mono).unwrap();
        assert!(decoded.audio.is_empty());
        assert_eq!(decoded.source_sample_rate, 22_050);
        assert_eq!(decoded.source_channels, 1);
    }
}
