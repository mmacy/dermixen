//! Reading audio files into the internal format.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use dermixen_core::{ContentHash, SAMPLE_RATE};
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
pub fn decode(path: &Path) -> Result<Decoded, DecodeError> {
    let bytes = std::fs::read(path).map_err(|source| DecodeError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let hash = ContentHash(*blake3::hash(&bytes).as_bytes());

    let (interleaved, source_sample_rate, source_channels) = decode_samples(path, bytes)?;
    let frames = to_stereo(&interleaved, source_channels);
    let frames = if source_sample_rate == SAMPLE_RATE {
        frames
    } else {
        resample(path, &frames, source_sample_rate)?
    };

    Ok(Decoded {
        audio: Audio { frames },
        hash,
        source_sample_rate,
        source_channels,
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

/// Decodes the whole file to interleaved samples, and reports the rate and channel count it held.
///
/// The file's bytes are handed over rather than borrowed because the decoder
/// reads from them for as long as it runs.
fn decode_samples(path: &Path, bytes: Vec<u8>) -> Result<(Vec<f32>, u32, u16), DecodeError> {
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
    let Some(CodecParameters::Audio(parameters)) = track.codec_params.clone() else {
        return Err(DecodeError::Unsupported {
            path: path.to_path_buf(),
            message: "the audio track is in a format Dermixen does not import".to_owned(),
        });
    };

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

    let mut interleaved: Vec<f32> = Vec::new();
    let mut packet_samples: Vec<f32> = Vec::new();
    let mut source: Option<(u32, u16)> = None;
    let mut packets_seen = false;
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
        source = Some((buffer.spec().rate(), channels));

        buffer.copy_to_vec_interleaved(&mut packet_samples);
        interleaved.extend_from_slice(&packet_samples);
    }

    if let Some((rate, channels)) = source {
        return Ok((interleaved, rate, channels));
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

/// Turns interleaved samples into stereo frames, copying a mono file to both channels.
fn to_stereo(interleaved: &[f32], channels: u16) -> Vec<Frame> {
    let channels = usize::from(channels);
    interleaved
        .chunks_exact(channels)
        .map(|frame| {
            if channels == 1 {
                [frame[0], frame[0]]
            } else {
                [frame[0], frame[1]]
            }
        })
        .collect()
}

/// Resamples stereo frames from `rate` to the internal sample rate.
///
/// The pitch is unchanged and the length changes in proportion to the two
/// rates, so a file at 48 kHz comes out about eight percent shorter in frames
/// and exactly as long in seconds.
fn resample(path: &Path, frames: &[Frame], rate: u32) -> Result<Vec<Frame>, DecodeError> {
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

    let input =
        InterleavedSlice::new(frames.as_flattened(), 2, frames.len()).map_err(|_| unsupported())?;
    let output = resampler
        .process_all(&input, frames.len(), None)
        .map_err(|_| unsupported())?;
    Ok(to_stereo(&output.take_data(), 2))
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
