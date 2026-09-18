//! Acceptance tests for the WAV writer. A coder agent makes these pass without editing them.
//!
//! Files are read back with the `hound` reader rather than with Dermixen's own
//! decoder, so this chunk does not wait on the decode chunk.

use dermixen_core::Seconds;
use dermixen_media::{Audio, WavDepth, write_wav};
use dermixen_testkit::synth;
use hound::{SampleFormat, WavReader};

fn test_audio() -> Audio {
    let mut audio = synth::sine(440.0, 0.5, Seconds(0.5));
    let noise = synth::white_noise(3, 0.25, Seconds(0.5));
    for (frame, n) in audio.frames.iter_mut().zip(&noise.frames) {
        frame[0] += n[0];
        frame[1] -= n[1];
    }
    audio
}

#[test]
fn float_wav_holds_the_audio_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("float.wav");
    let audio = test_audio();
    write_wav(&path, &audio, WavDepth::Float32).unwrap();
    let mut reader = WavReader::open(&path).unwrap();
    let spec = reader.spec();
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.sample_rate, 44_100);
    assert_eq!(spec.bits_per_sample, 32);
    assert_eq!(spec.sample_format, SampleFormat::Float);
    let samples: Vec<f32> = reader.samples::<f32>().map(Result::unwrap).collect();
    let expected: Vec<f32> = audio.frames.iter().flat_map(|f| [f[0], f[1]]).collect();
    assert_eq!(samples, expected);
}

#[test]
fn int16_wav_holds_the_audio_rounded_to_the_nearest_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("int16.wav");
    let audio = test_audio();
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    let mut reader = WavReader::open(&path).unwrap();
    let spec = reader.spec();
    assert_eq!(spec.channels, 2);
    assert_eq!(spec.sample_rate, 44_100);
    assert_eq!(spec.bits_per_sample, 16);
    assert_eq!(spec.sample_format, SampleFormat::Int);
    let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    let expected: Vec<i16> = audio
        .frames
        .iter()
        .flat_map(|f| [f[0], f[1]])
        .map(|v| (f64::from(v) * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16)
        .collect();
    assert_eq!(samples, expected);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 4 * 22_050);
}

#[test]
fn int16_wav_clamps_values_beyond_full_scale() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("loud.wav");
    let audio = Audio {
        frames: vec![[1.5, -1.5], [1.0, -1.0], [0.0, 0.5]],
    };
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    let mut reader = WavReader::open(&path).unwrap();
    let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    assert_eq!(samples, vec![32_767, -32_768, 32_767, -32_768, 0, 16_384]);
}

#[test]
fn empty_audio_writes_a_valid_empty_file_and_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.wav");
    write_wav(&path, &test_audio(), WavDepth::Int16).unwrap();
    write_wav(&path, &Audio::new(), WavDepth::Int16).unwrap();
    let reader = WavReader::open(&path).unwrap();
    assert_eq!(reader.len(), 0);
    assert_eq!(reader.spec().channels, 2);
}

#[test]
fn an_unwritable_path_is_an_error_naming_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no such folder").join("out.wav");
    let error = write_wav(&path, &test_audio(), WavDepth::Float32).unwrap_err();
    assert_eq!(error.path, path);
    assert!(!error.message.is_empty());
    assert!(error.to_string().contains("out.wav"));
}

mod streaming {
    //! Acceptance tests for the block-by-block WAV writer. A coder agent makes
    //! these pass without editing them.

    use dermixen_core::{Samples, Seconds};
    use dermixen_media::{WavDepth, WavFile, decode, write_wav};
    use dermixen_testkit::synth;

    #[test]
    fn writing_in_blocks_gives_the_same_file_as_writing_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let audio = synth::sine(440.0, 0.5, Seconds(1.5));
        for depth in [WavDepth::Int16, WavDepth::Float32] {
            let whole = dir.path().join("whole.wav");
            let blocks = dir.path().join("blocks.wav");
            write_wav(&whole, &audio, depth).unwrap();
            let mut file = WavFile::create(&blocks, depth).unwrap();
            assert!(file.is_empty());
            for block in audio.frames.chunks(1000) {
                file.write(block).unwrap();
            }
            assert_eq!(file.len(), audio.len());
            file.finish().unwrap();
            assert_eq!(
                std::fs::read(&whole).unwrap(),
                std::fs::read(&blocks).unwrap(),
                "{depth:?}"
            );
            assert_eq!(decode(&blocks).unwrap().audio.len(), audio.len());
        }
    }

    #[test]
    fn an_empty_file_is_still_a_valid_wav() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wav");
        WavFile::create(&path, WavDepth::Int16)
            .unwrap()
            .finish()
            .unwrap();
        let decoded = decode(&path).unwrap();
        assert_eq!(decoded.audio.len(), Samples::ZERO);
        assert_eq!(decoded.source_sample_rate, 44_100);
    }

    #[test]
    fn a_path_that_cannot_be_created_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no/such/folder/out.wav");
        let problem = WavFile::create(&path, WavDepth::Int16).unwrap_err();
        assert_eq!(problem.path, path);
    }
}
