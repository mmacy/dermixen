//! Acceptance tests for decoding. A coder agent makes these pass without editing them.
//!
//! The fixtures under `tests/fixtures/audio/` are sine tones written by
//! `tools/make_audio_fixtures.py`, which prints the frame counts and
//! checksums these tests expect.

mod common;

use std::path::Path;

use common::{crc32_of_int16, fixture, rms, zero_crossings};
use dermixen_core::Samples;
use dermixen_media::{DecodeError, decode, hash_file};

/// The expected root mean square of a sine with peak amplitude 0.5.
const HALF_SINE_RMS: f64 = 0.5 / std::f64::consts::SQRT_2;

fn close(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * b.abs()
}

#[test]
fn a_44k_stereo_wav_decodes_exactly() {
    let decoded = decode(&fixture("sine-440-44k.wav")).unwrap();
    assert_eq!(decoded.audio.len(), Samples(88_200));
    assert_eq!(decoded.source_sample_rate, 44_100);
    assert_eq!(decoded.source_channels, 2);
    assert_eq!(crc32_of_int16(&decoded.audio.frames, 2), 0x1dcf_4c6f);
    // Left is a sine at half scale, right the same at quarter scale.
    for (n, frame) in decoded.audio.frames.iter().enumerate() {
        let phase = std::f64::consts::TAU * 440.0 * n as f64 / 44_100.0;
        let left = (phase.sin() * 0.5 * 32_767.0).round() / 32_768.0;
        let right = (phase.sin() * 0.25 * 32_767.0).round() / 32_768.0;
        assert!(
            (f64::from(frame[0]) - left).abs() <= 1.0 / 32_768.0,
            "frame {n} left {} expected {left}",
            frame[0]
        );
        assert!(
            (f64::from(frame[1]) - right).abs() <= 1.0 / 32_768.0,
            "frame {n} right {} expected {right}",
            frame[1]
        );
    }
}

#[test]
fn a_mono_wav_is_copied_to_both_channels() {
    let decoded = decode(&fixture("sine-220-mono-44k.wav")).unwrap();
    assert_eq!(decoded.audio.len(), Samples(44_100));
    assert_eq!(decoded.source_channels, 1);
    assert!(decoded.audio.frames.iter().all(|f| f[0] == f[1]));
    assert_eq!(crc32_of_int16(&decoded.audio.frames, 1), 0x0c51_4aed);
    assert_eq!(zero_crossings(&decoded.audio.frames), 439);
}

#[test]
fn a_48k_wav_is_resampled_to_44k_and_keeps_its_pitch() {
    let decoded = decode(&fixture("sine-440-48k.wav")).unwrap();
    assert_eq!(decoded.source_sample_rate, 48_000);
    assert_eq!(decoded.source_channels, 2);
    let len = decoded.audio.len().0;
    assert!((len - 88_200).abs() <= 441, "length {len}");
    // The middle second of the tone is 880 crossings of 440 Hz, and its level is unchanged.
    let middle = &decoded.audio.frames[22_050..66_150];
    let crossings = zero_crossings(middle);
    assert!((876..=884).contains(&crossings), "crossings {crossings}");
    assert!(
        close(rms(middle, 0), HALF_SINE_RMS, 0.03),
        "left rms {}",
        rms(middle, 0)
    );
    assert!(
        close(rms(middle, 1), HALF_SINE_RMS / 2.0, 0.03),
        "right rms {}",
        rms(middle, 1)
    );
}

#[test]
fn an_mp3_decodes_to_the_tone_it_encodes() {
    let decoded = decode(&fixture("sine-440-44k.mp3")).unwrap();
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
fn an_m4a_decodes_to_the_tone_it_encodes() {
    // The fixture is the tone encoded as AAC in an MP4 container. AAC adds
    // encoder delay and padding, so the length is only close.
    let decoded = decode(&fixture("sine-440-44k.m4a")).unwrap();
    assert_eq!(decoded.source_sample_rate, 44_100);
    assert_eq!(decoded.source_channels, 2);
    let len = decoded.audio.len().0;
    assert!((len - 88_200).abs() <= 4_800, "length {len}");
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
    assert_eq!(
        decoded.hash,
        hash_file(&fixture("sine-440-44k.m4a")).unwrap()
    );
}

#[test]
fn a_flac_decodes_exactly_like_the_wav_it_came_from() {
    let decoded = decode(&fixture("sine-440-44k.flac")).unwrap();
    assert_eq!(decoded.audio.len(), Samples(88_200));
    assert_eq!(decoded.source_sample_rate, 44_100);
    assert_eq!(decoded.source_channels, 2);
    assert_eq!(crc32_of_int16(&decoded.audio.frames, 2), 0x1dcf_4c6f);
    let wav = decode(&fixture("sine-440-44k.wav")).unwrap();
    assert_eq!(decoded.audio, wav.audio);
}

#[test]
fn the_hash_is_of_the_file_bytes() {
    let path = fixture("sine-440-44k.mp3");
    let decoded = decode(&path).unwrap();
    let expected = blake3::hash(&std::fs::read(&path).unwrap());
    assert_eq!(decoded.hash.0, *expected.as_bytes());
    assert_eq!(decoded.hash, hash_file(&path).unwrap());
    let other = decode(&fixture("sine-440-44k.wav")).unwrap();
    assert_ne!(decoded.hash, other.hash);
}

#[test]
fn a_missing_file_is_a_read_error() {
    let path = fixture("does-not-exist.wav");
    match decode(&path) {
        Err(DecodeError::Read { path: reported, .. }) => assert_eq!(reported, path),
        other => panic!("expected a read error, got {other:?}"),
    }
}

#[test]
fn a_file_that_is_not_audio_is_refused_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("words.wav");
    std::fs::write(&path, "this is not a sound file").unwrap();
    match decode(&path) {
        Err(DecodeError::Unsupported {
            path: reported,
            message,
        })
        | Err(DecodeError::Corrupt {
            path: reported,
            message,
        }) => {
            assert_eq!(reported, path);
            assert!(!message.is_empty());
        }
        other => panic!("expected the file to be refused, got {other:?}"),
    }
    let text = fixture("../mix/valid/two-tracks.dmx");
    assert!(decode(Path::new(&text)).is_err());
}

/// An ID3v2.3 text frame holding UTF-16 text: the encoding byte, the byte
/// order mark, the text, and the two zero bytes that end it.
fn utf16_text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut payload = vec![0x01, 0xFF, 0xFE];
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        payload.extend(unit.to_le_bytes());
    }
    let mut frame = id.to_vec();
    frame.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    frame.extend([0, 0]);
    frame.extend(payload);
    frame
}

#[test]
fn an_mp3_decodes_the_same_with_an_id3_tag_in_front_of_it() {
    // The byte order mark that opens each UTF-16 text frame, `FF FE`, reads as
    // the first two bytes of an MPEG-1 layer I frame header, and the character
    // after it supplies a bitrate and a sample rate that pass inspection. The
    // decoder has to start reading past the tag. Reading from the first byte
    // instead, it takes one of those for a real frame header and reports the
    // file as layer I audio it has no decoder for.
    //
    // The four frames below are the shape that provokes it: the lengths decide
    // where the search looks for the next header, so shortening the album title
    // by one character is enough to make the file decode. These are the lengths
    // of the tag on both discs of one released album.
    let mut body = Vec::new();
    for (id, text) in [
        (b"TIT2", "Intro"),
        (b"TALB", "A Blueprint For Survival Vol 2"),
        (b"TPE1", "Blue Planet Corporation"),
        (b"TYER", "1996"),
    ] {
        body.extend(utf16_text_frame(id, text));
    }
    let length = u32::try_from(body.len()).unwrap();
    let mut tagged = b"ID3".to_vec();
    tagged.extend([3, 0, 0]);
    for shift in [21, 14, 7, 0] {
        tagged.push(((length >> shift) & 0x7F) as u8);
    }
    tagged.extend(body);
    tagged.extend(std::fs::read(fixture("sine-440-44k.mp3")).unwrap());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tagged-sine.mp3");
    std::fs::write(&path, &tagged).unwrap();

    let plain = decode(&fixture("sine-440-44k.mp3")).unwrap();
    let decoded = decode(&path).unwrap();
    assert_eq!(decoded.audio.len(), plain.audio.len());
    assert_eq!(decoded.source_sample_rate, plain.source_sample_rate);
    assert_eq!(decoded.source_channels, plain.source_channels);
    assert_eq!(
        crc32_of_int16(&decoded.audio.frames, 2),
        crc32_of_int16(&plain.audio.frames, 2)
    );
}

#[test]
fn an_mp3_decodes_in_time_with_the_wav_it_was_encoded_from() {
    // LAME writes the encoder's delay and its padding into the file. A
    // decoder that takes both off gives back exactly as many frames as the
    // WAV has, with the tone starting on the same frame. A decoder that
    // takes neither off fails the length check, and one that is off by less
    // than a period of the tone fails the lag check. The library stores every
    // beat grid against this decoder's output, so this is the test that says
    // a stored grid names the same sample the WAV would.
    let wav = decode(&fixture("sine-440-44k.wav")).unwrap().audio;
    let mp3 = decode(&fixture("sine-440-44k.mp3")).unwrap().audio;
    assert_eq!(mp3.len(), wav.len());

    // The tone rises from zero on the first frame, so the first frame above
    // a quarter of full scale is a few frames in for both decodes. A lossy
    // codec moves that frame by at most a frame or two.
    let rise = |frames: &[[f32; 2]]| {
        frames
            .iter()
            .position(|frame| frame[0].abs() > 0.25)
            .expect("the tone rises past a quarter of full scale") as i64
    };
    let apart = rise(&mp3.frames) - rise(&wav.frames);
    assert!(apart.abs() <= 2, "the MP3's onset is {apart} frames off");

    // Over the middle second, the lag that lines the two decodes up best is
    // zero. The tone's period is about a hundred frames, so the search stays
    // within sixty frames of zero, where no other lag can tie. The length
    // check above is what rules out a shift of a whole number of periods.
    let middle = 22_050..66_150;
    let product = |lag: i64| -> f64 {
        middle
            .clone()
            .map(|index| {
                let other = index as i64 + lag;
                f64::from(wav.frames[index][0]) * f64::from(mp3.frames[other as usize][0])
            })
            .sum()
    };
    let best = (-60i64..=60)
        .max_by(|a, b| product(*a).total_cmp(&product(*b)))
        .unwrap();
    assert_eq!(best, 0, "the MP3 decodes {best} frames off the WAV");
}
