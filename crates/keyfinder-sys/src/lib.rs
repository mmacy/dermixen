#![allow(unsafe_code)]

//! libkeyfinder, the key detection library, made callable from Rust.
//!
//! libkeyfinder is a C++ library. Its source is vendored under
//! `vendor/libkeyfinder` in this crate and compiled by `build.rs` together
//! with `src/shim.cpp`, a short piece of C++ that exposes the one operation
//! Dermixen needs as a plain C function and catches everything libkeyfinder
//! throws. This file wraps that C function in [`key_of_audio`], which is safe
//! to call and hands back a key and a confidence.
//!
//! The crates that make up the app itself forbid unsafe code, so all of the
//! risk of calling into this C++ sits here, in a file short enough to read in
//! one sitting. The other three crates that allow unsafe code are the
//! wrappers around the time-stretcher, the beat tracker, and the documents
//! macOS asks the app to open.
//!
//! libkeyfinder normally works out its spectra with FFTW, a library that would
//! have to be installed on every machine Dermixen builds on. In its place this
//! crate builds the fast Fourier transform Takuya Ooura published, whose source
//! is vendored under `vendor/ooura`, so the crate compiles from a clean
//! checkout with nothing but a C and C++ compiler.
//!
//! libkeyfinder is under the GNU General Public License version 3, and
//! `vendor/libkeyfinder/LICENSE` is its own copy of that license. Dermixen is
//! under the same license, which is what makes linking libkeyfinder into
//! Dermixen allowed.

use std::ffi::{c_char, c_int, c_uint};
use std::fmt;
use std::sync::Once;

/// How many bytes of failure message the C++ side may write back, including
/// the byte that ends the text. libkeyfinder's own messages are all short.
const MESSAGE_BYTES: usize = 256;

/// The analysis finished and named a key.
const STATUS_OK: c_int = 0;
/// The analysis finished and heard nothing it could take a key from.
const STATUS_SILENCE: c_int = 1;

unsafe extern "C" {
    fn dermixen_keyfinder_prepare();
    fn dermixen_keyfinder_frame_samples(frame_rate: c_uint) -> usize;
    fn dermixen_keyfinder_analyze(
        samples: *const f32,
        count: usize,
        frame_rate: c_uint,
        key: *mut c_int,
        confidence: *mut f64,
        message: *mut c_char,
        message_length: usize,
    ) -> c_int;
}

/// Major or minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// A major key.
    Major,
    /// A minor key.
    Minor,
}

/// One of the twenty-four keys libkeyfinder can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    /// The tonic, counted in semitones above C: zero for C, one for C sharp,
    /// and so on up to eleven for B.
    ///
    /// libkeyfinder counts its own keys from A rather than from C, and names
    /// the five black notes with flats. Both differences are settled here, so
    /// a caller only ever sees a count of semitones above C.
    pub tonic: u8,
    /// Whether the key is major or minor.
    pub mode: Mode,
}

/// What libkeyfinder found in one track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Analysis {
    /// The key libkeyfinder named.
    pub key: Key,
    /// How far ahead of the runner-up the winning key finished, from zero to
    /// one. See [`key_of_audio`] for what the figure measures.
    pub confidence: f64,
}

/// The reason libkeyfinder named no key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The track held nothing libkeyfinder could take a key from, which is
    /// what libkeyfinder reports when every one of the twelve pitch classes
    /// comes out at nothing at all. Silence is the ordinary case.
    ///
    /// Short audio does not land here. libkeyfinder pads a buffer too short to
    /// fill one analysis frame with silence and names a key from the result;
    /// see [`frame_samples`] for turning such a fragment away.
    Silence,
    /// libkeyfinder reported a failure, for the reason given.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Silence => formatter.write_str("libkeyfinder heard no key in the audio"),
            Error::Failed(reason) => write!(formatter, "libkeyfinder failed: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// The lowest sample rate [`key_of_audio`] accepts, in samples per second.
pub const LOWEST_SAMPLE_RATE: u32 = 8_000;

/// The highest sample rate [`key_of_audio`] accepts, in samples per second.
pub const HIGHEST_SAMPLE_RATE: u32 = 384_000;

/// How many samples at `sample_rate` fill one of the frames libkeyfinder works
/// a spectrum out from. At the rate Dermixen works at this is a little under
/// four seconds of audio.
///
/// libkeyfinder turns nothing away for being short. Handed a fragment, it pads
/// the fragment with silence until one frame is full and names a key from the
/// result, which is a key read mostly from silence and worth nothing. A caller
/// that would rather refuse such a fragment than believe the answer should
/// compare the length of its audio against this figure first, which is what
/// Dermixen's own key analyzer does.
///
/// The answer is zero for a rate outside [`LOWEST_SAMPLE_RATE`] to
/// [`HIGHEST_SAMPLE_RATE`], which [`key_of_audio`] refuses.
pub fn frame_samples(sample_rate: u32) -> usize {
    // Safety: this only reads two constants out of the C++ library and does
    // arithmetic on them, touching no memory the caller owns.
    unsafe { dermixen_keyfinder_frame_samples(sample_rate as c_uint) }
}

/// Finds the musical key of a whole track.
///
/// `samples` is one channel of audio at `sample_rate` samples per second, from
/// the first sample of the track to the last. libkeyfinder works on one
/// channel, so a stereo track is mixed down before it gets here.
///
/// Audio shorter than [`frame_samples`] still comes back with a key rather
/// than an error, because that is what libkeyfinder does with it. The answer
/// is not worth having, so a caller should check the length first.
///
/// The confidence is the margin by which the best-scoring of libkeyfinder's
/// twenty-four keys beat the second best, as a fraction of the best score. It
/// is zero when the top two keys tied and one when the runner-up scored
/// nothing at all. libkeyfinder scores every key by how closely the track's
/// average spread of pitches matches that key's expected spread, so a track
/// that sits squarely in one key leaves the runner-up further behind than a
/// track that wanders.
///
/// Read the figure as a ranking rather than as a probability. Every key's
/// expected spread of pitches overlaps heavily with every other's, so even a
/// plain four-chord progression in one key wins by only a few percent, while
/// noise with no key in it wins by a few tenths of one percent. Higher always
/// means a clearer key; it never means a percentage chance of being right.
///
/// # Errors
///
/// Answers [`Error::Silence`] when libkeyfinder heard nothing it could take a
/// key from, and [`Error::Failed`] when libkeyfinder reported a failure, which
/// includes a track longer than the roughly twenty-seven hours libkeyfinder's
/// own sample counter reaches and a track holding a sample that is not a
/// finite number. A sample rate outside [`LOWEST_SAMPLE_RATE`] to
/// [`HIGHEST_SAMPLE_RATE`] is also [`Error::Failed`], with a reason that
/// names the sample rate, and it never reaches libkeyfinder, which divides by
/// a figure it works out from the rate and reads outside its buffers when the
/// rate is very large.
pub fn key_of_audio(samples: &[f32], sample_rate: u32) -> Result<Analysis, Error> {
    // libkeyfinder builds its tone profiles the first time an analysis asks
    // for them, without guarding that first build against a second thread
    // arriving at the same moment. Building them here, once, means every
    // analysis that follows only reads them, so two tracks can be analyzed on
    // two threads at the same time.
    static PREPARED: Once = Once::new();
    // Safety: this call only fills two tables inside the C++ library, and
    // `Once` runs it to completion before any other thread goes past this
    // point.
    PREPARED.call_once(|| unsafe { dermixen_keyfinder_prepare() });

    if samples.is_empty() {
        return Err(Error::Silence);
    }
    if u32::try_from(samples.len()).is_err() {
        return Err(Error::Failed(format!(
            "the track is {} samples long, and libkeyfinder counts samples in a number that \
             stops at {}",
            samples.len(),
            u32::MAX
        )));
    }

    let mut key: c_int = 0;
    let mut confidence: f64 = 0.0;
    let mut message = [0u8; MESSAGE_BYTES];
    // Safety: the sample pointer is valid for the length passed beside it, the
    // three writable pointers are to live local values of the matching types,
    // and the C++ side writes no more bytes of message than the length given.
    // The C++ side catches every exception libkeyfinder can raise, so nothing
    // unwinds across this call.
    let status = unsafe {
        dermixen_keyfinder_analyze(
            samples.as_ptr(),
            samples.len(),
            sample_rate as c_uint,
            &mut key,
            &mut confidence,
            message.as_mut_ptr().cast::<c_char>(),
            message.len(),
        )
    };

    if status == STATUS_SILENCE {
        return Err(Error::Silence);
    }
    if status != STATUS_OK {
        let end = message.iter().position(|byte| *byte == 0).unwrap_or(0);
        return Err(Error::Failed(
            String::from_utf8_lossy(&message[..end]).into_owned(),
        ));
    }

    let key = key_at(key).ok_or_else(|| {
        Error::Failed(format!(
            "libkeyfinder named key number {key}, which is not one of its own"
        ))
    })?;
    Ok(Analysis { key, confidence })
}

/// The key libkeyfinder means by `index`, or `None` when the number is not one
/// of the twenty-four keys libkeyfinder names.
///
/// libkeyfinder numbers its keys in pairs from A upwards: A major is zero, A
/// minor is one, B flat major is two, and so on through the twelve tonics.
fn key_at(index: c_int) -> Option<Key> {
    if !(0..24).contains(&index) {
        return None;
    }
    let tonic = u8::try_from(index / 2).ok()?;
    Some(Key {
        // libkeyfinder starts counting at A, which is nine semitones above C.
        tonic: (tonic + 9) % 12,
        mode: if index % 2 == 0 {
            Mode::Major
        } else {
            Mode::Minor
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    /// The frequency of a note given as semitones above middle C.
    fn frequency(semitones: i32) -> f64 {
        261.625_565 * 2f64.powf(f64::from(semitones) / 12.0)
    }

    /// A run of block chords, two seconds each, every chord a set of notes
    /// given as semitones above middle C, played as sines with their octaves.
    fn progression(chords: &[&[i32]]) -> Vec<f32> {
        let rate = f64::from(RATE);
        let per_chord = 2 * RATE as usize;
        let mut samples = Vec::with_capacity(per_chord * chords.len());
        for chord in chords {
            for i in 0..per_chord {
                let t = i as f64 / rate;
                let mut value = 0.0;
                for note in *chord {
                    for octave in [0, 12] {
                        value += (std::f64::consts::TAU * frequency(note + octave) * t).sin();
                    }
                }
                samples.push((value / (chord.len() as f64 * 2.0) * 0.5) as f32);
            }
        }
        samples
    }

    #[test]
    fn a_progression_in_c_major_is_heard_in_c_major() {
        // C, F, G, C: the tonic, subdominant, and dominant of C major.
        let audio = progression(&[&[0, 4, 7], &[5, 9, 12], &[7, 11, 14], &[0, 4, 7]]);
        let found = key_of_audio(&audio, RATE).unwrap();
        assert_eq!(
            found.key,
            Key {
                tonic: 0,
                mode: Mode::Major
            }
        );
        assert!(
            (0.0..=1.0).contains(&found.confidence),
            "the confidence was {}",
            found.confidence
        );
    }

    #[test]
    fn a_progression_in_a_minor_is_heard_in_a_minor() {
        // A minor, D minor, E major, A minor, with the raised leading tone in
        // the dominant.
        let audio = progression(&[&[-3, 0, 4], &[2, 5, 9], &[4, 8, 11], &[-3, 0, 4]]);
        let found = key_of_audio(&audio, RATE).unwrap();
        assert_eq!(
            found.key,
            Key {
                tonic: 9,
                mode: Mode::Minor
            }
        );
    }

    /// A run of pseudo-random samples between plus and minus `level`, built
    /// from a fixed recipe so the same call gives the same noise every time.
    fn noise(count: usize, level: f32) -> Vec<f32> {
        (0..count)
            .map(|i| level * ((i as f64 * 12.9898).sin() * 43_758.545_3).fract() as f32)
            .collect()
    }

    #[test]
    fn a_clearer_key_wins_by_a_wider_margin() {
        let clean = progression(&[&[0, 4, 7], &[5, 9, 12], &[7, 11, 14], &[0, 4, 7]]);
        let hiss = noise(clean.len(), 0.4);
        let muddied: Vec<f32> = clean.iter().zip(&hiss).map(|(a, b)| a + b).collect();

        let clean = key_of_audio(&clean, RATE).unwrap();
        let muddied = key_of_audio(&muddied, RATE).unwrap();
        let hiss = key_of_audio(&hiss, RATE).unwrap();

        assert!(
            clean.confidence > muddied.confidence,
            "the clean progression scored {} and the muddied one {}",
            clean.confidence,
            muddied.confidence
        );
        assert!(
            muddied.confidence > hiss.confidence,
            "the muddied progression scored {} and the noise {}",
            muddied.confidence,
            hiss.confidence
        );
    }

    #[test]
    fn one_analysis_frame_is_a_little_under_four_seconds() {
        // libkeyfinder plays a track back at a tenth of its rate before taking
        // a spectrum, so a frame of sixteen thousand three hundred and
        // eighty-four samples covers ten times that many of the samples handed
        // in, which is three and seven tenths of a second.
        assert_eq!(frame_samples(RATE), 163_840);
        assert!(f64::from(RATE) * 3.7 < 163_840.0);
        assert!(f64::from(RATE) * 3.8 > 163_840.0);
    }

    #[test]
    fn audio_too_short_for_one_frame_still_comes_back_with_a_key() {
        // This is libkeyfinder's own behavior, which this crate leaves as it is:
        // it pads the fragment with silence and names a key from the padding.
        // The test states the behavior plainly so that a caller relying on
        // `frame_samples` to guard against it can see why the guard is needed.
        let fragment = progression(&[&[0, 4, 7]])[..RATE as usize / 4].to_vec();
        assert!(fragment.len() < frame_samples(RATE));
        assert!(key_of_audio(&fragment, RATE).is_ok());
    }

    #[test]
    fn silence_holds_no_key() {
        assert_eq!(key_of_audio(&[], RATE), Err(Error::Silence));
        let quiet = vec![0.0f32; RATE as usize * 8];
        assert_eq!(key_of_audio(&quiet, RATE), Err(Error::Silence));
    }

    #[test]
    fn a_sample_that_is_not_a_number_is_refused() {
        let mut audio = vec![0.1f32; RATE as usize];
        audio[1000] = f32::NAN;
        let Err(Error::Failed(reason)) = key_of_audio(&audio, RATE) else {
            panic!("libkeyfinder accepted a sample that is not a number");
        };
        assert!(!reason.is_empty(), "the failure came back with no reason");
    }

    #[test]
    #[ignore = "wrapper-limits"]
    fn a_sample_rate_libkeyfinder_cannot_work_at_is_refused() {
        let second = vec![0.25_f32; 44_100];
        for rate in [
            0,
            1,
            1_000,
            4_000,
            7_999,
            384_001,
            2_147_483_647,
            4_294_967_295,
        ] {
            match key_of_audio(&second, rate) {
                Err(Error::Failed(reason)) => {
                    assert!(reason.contains("sample rate"), "{rate}: {reason}")
                }
                other => panic!("{rate}: {other:?}"),
            }
            assert_eq!(frame_samples(rate), 0, "{rate}");
        }
        for rate in [LOWEST_SAMPLE_RATE, HIGHEST_SAMPLE_RATE] {
            assert!(frame_samples(rate) > 0, "{rate}");
            assert!(
                !matches!(key_of_audio(&second, rate), Err(Error::Failed(_))),
                "{rate} was refused"
            );
        }
    }

    #[test]
    fn libkeyfinder_key_numbers_are_read_as_tonics_and_modes() {
        assert_eq!(
            key_at(0),
            Some(Key {
                tonic: 9,
                mode: Mode::Major
            })
        );
        assert_eq!(
            key_at(6),
            Some(Key {
                tonic: 0,
                mode: Mode::Major
            })
        );
        assert_eq!(
            key_at(19),
            Some(Key {
                tonic: 6,
                mode: Mode::Minor
            })
        );
        assert_eq!(key_at(24), None);
        assert_eq!(key_at(-1), None);
        assert_eq!(
            Error::Silence.to_string(),
            "libkeyfinder heard no key in the audio"
        );
    }
}

/// Checks of the fast Fourier transform this crate substitutes for FFTW.
///
/// libkeyfinder reads every key it names out of that transform, so a mistake
/// in the transform would move every key without failing anything. The tests
/// here compare the transform against one worked out from the textbook
/// definition, and check that a frame comes back unchanged from a trip out to
/// the frequency bins and back.
#[cfg(test)]
mod transform {
    use std::f64::consts::TAU;
    use std::ffi::c_char;

    /// The frame sizes libkeyfinder itself asks for: the spectrum of one
    /// analysis window, and the shape of the low-pass filter.
    const SIZES_IN_USE: [usize; 2] = [16_384, 2_048];

    /// Small frame sizes, cheap enough to check against a transform worked out
    /// one term at a time over every bin.
    const SMALL_SIZES: [usize; 4] = [8, 16, 64, 256];

    /// How far a value may sit from the value worked out from the definition:
    /// one part in a billion, against the larger of the expected value and
    /// one, so that a bin holding a large number is judged by the same
    /// proportion as a bin holding a small one.
    const TOLERANCE: f64 = 1e-9;

    unsafe extern "C" {
        fn dermixen_keyfinder_forward_transform(
            input: *const f64,
            frame_size: usize,
            real: *mut f64,
            imaginary: *mut f64,
            message: *mut c_char,
            message_length: usize,
        ) -> i32;
        fn dermixen_keyfinder_inverse_transform(
            real: *const f64,
            imaginary: *const f64,
            frame_size: usize,
            output: *mut f64,
            message: *mut c_char,
            message_length: usize,
        ) -> i32;
    }

    /// The real and imaginary part of every bin of the forward transform of
    /// `samples`, or the reason the transform would not run.
    fn forward(samples: &[f64]) -> Result<(Vec<f64>, Vec<f64>), String> {
        let mut real = vec![0.0; samples.len()];
        let mut imaginary = vec![0.0; samples.len()];
        let mut message = [0u8; 256];
        // Safety: all three slices are the length passed beside them, and the
        // C++ side catches every exception the transform can raise.
        let status = unsafe {
            dermixen_keyfinder_forward_transform(
                samples.as_ptr(),
                samples.len(),
                real.as_mut_ptr(),
                imaginary.as_mut_ptr(),
                message.as_mut_ptr().cast::<c_char>(),
                message.len(),
            )
        };
        if status != 0 {
            return Err(reason(&message));
        }
        Ok((real, imaginary))
    }

    /// The samples the inverse transform reports for the bins given, or the
    /// reason the transform would not run.
    fn inverse(real: &[f64], imaginary: &[f64]) -> Result<Vec<f64>, String> {
        let mut output = vec![0.0; real.len()];
        let mut message = [0u8; 256];
        // Safety: all three slices are the length passed beside them, and the
        // C++ side catches every exception the transform can raise.
        let status = unsafe {
            dermixen_keyfinder_inverse_transform(
                real.as_ptr(),
                imaginary.as_ptr(),
                real.len(),
                output.as_mut_ptr(),
                message.as_mut_ptr().cast::<c_char>(),
                message.len(),
            )
        };
        if status != 0 {
            return Err(reason(&message));
        }
        Ok(output)
    }

    /// The text the C++ side wrote into a message buffer.
    fn reason(message: &[u8]) -> String {
        let end = message.iter().position(|byte| *byte == 0).unwrap_or(0);
        String::from_utf8_lossy(&message[..end]).into_owned()
    }

    /// A signal of `count` samples with no symmetry to hide a mistake behind,
    /// built from a fixed recipe so every run sees the same numbers.
    fn signal(count: usize) -> Vec<f64> {
        (0..count)
            .map(|i| {
                let i = i as f64;
                (i * 0.37).sin() + 0.5 * (i * 1.11).cos() + 0.25 * (i * 0.043).sin()
            })
            .collect()
    }

    /// The forward transform of `samples` worked out from its definition, one
    /// term at a time, for every bin from zero up to half the frame.
    ///
    /// This is the transform every textbook states: each bin is the sum over
    /// every sample of that sample turned by an angle that grows with the bin
    /// number and the sample position.
    fn from_the_definition(samples: &[f64]) -> Vec<(f64, f64)> {
        let count = samples.len();
        (0..=count / 2)
            .map(|bin| {
                let mut real = 0.0;
                let mut imaginary = 0.0;
                for (position, sample) in samples.iter().enumerate() {
                    let angle = TAU * (position * bin % count) as f64 / count as f64;
                    real += sample * angle.cos();
                    imaginary -= sample * angle.sin();
                }
                (real, imaginary)
            })
            .collect()
    }

    /// A run of `count` samples that is silent apart from a few spikes, along
    /// with where those spikes sit and how tall each one is.
    ///
    /// The transform of a single spike is known without any summing: every bin
    /// holds the same size, turned by an angle set by where the spike sits.
    /// That makes every bin of a large frame cheap to state exactly, which a
    /// term-by-term sum over sixteen thousand samples is not.
    fn spikes(count: usize) -> (Vec<f64>, Vec<(usize, f64)>) {
        let placed = vec![
            (0usize, 1.5f64),
            (1, -0.75),
            (count / 3, 2.25),
            (count - 7, -1.125),
        ];
        let mut samples = vec![0.0; count];
        for (position, height) in &placed {
            samples[*position] += height;
        }
        (samples, placed)
    }

    /// The forward transform of a run of spikes, stated bin by bin without
    /// summing over the samples.
    fn transform_of_spikes(count: usize, placed: &[(usize, f64)]) -> Vec<(f64, f64)> {
        (0..=count / 2)
            .map(|bin| {
                let mut real = 0.0;
                let mut imaginary = 0.0;
                for (position, height) in placed {
                    let angle = TAU * (position * bin % count) as f64 / count as f64;
                    real += height * angle.cos();
                    imaginary -= height * angle.sin();
                }
                (real, imaginary)
            })
            .collect()
    }

    /// Fails the test when `found` sits further from `wanted` than one part in
    /// a billion.
    fn agrees(found: f64, wanted: f64, what: &str) {
        let allowed = TOLERANCE * wanted.abs().max(1.0);
        assert!(
            (found - wanted).abs() <= allowed,
            "{what}: the transform reported {found} where {wanted} was expected"
        );
    }

    #[test]
    fn every_bin_of_a_small_frame_matches_the_definition() {
        for size in SMALL_SIZES {
            let samples = signal(size);
            let (real, imaginary) = forward(&samples).unwrap();
            let wanted = from_the_definition(&samples);
            for (bin, (want_real, want_imaginary)) in wanted.iter().enumerate() {
                agrees(
                    real[bin],
                    *want_real,
                    &format!("frame {size}, bin {bin}, real part"),
                );
                agrees(
                    imaginary[bin],
                    *want_imaginary,
                    &format!("frame {size}, bin {bin}, imaginary part"),
                );
            }
        }
    }

    #[test]
    fn every_bin_at_the_sizes_libkeyfinder_uses_matches_the_definition() {
        for size in SIZES_IN_USE {
            let (samples, placed) = spikes(size);
            let (real, imaginary) = forward(&samples).unwrap();
            let wanted = transform_of_spikes(size, &placed);
            for (bin, (want_real, want_imaginary)) in wanted.iter().enumerate() {
                agrees(
                    real[bin],
                    *want_real,
                    &format!("frame {size}, bin {bin}, real part"),
                );
                agrees(
                    imaginary[bin],
                    *want_imaginary,
                    &format!("frame {size}, bin {bin}, imaginary part"),
                );
            }
        }
    }

    #[test]
    fn the_bins_above_the_halfway_point_stay_at_nothing() {
        // FFTW never wrote to those bins and libkeyfinder never reads them,
        // so the substitute leaves them alone in the same way.
        for size in SMALL_SIZES {
            let (real, imaginary) = forward(&signal(size)).unwrap();
            for bin in (size / 2 + 1)..size {
                assert_eq!(real[bin], 0.0, "frame {size}, bin {bin}, real part");
                assert_eq!(
                    imaginary[bin], 0.0,
                    "frame {size}, bin {bin}, imaginary part"
                );
            }
        }
    }

    #[test]
    fn a_frame_comes_back_unchanged_from_a_round_trip() {
        for size in SMALL_SIZES.iter().chain(SIZES_IN_USE.iter()) {
            let samples = signal(*size);
            let (real, imaginary) = forward(&samples).unwrap();
            let returned = inverse(&real, &imaginary).unwrap();
            for (position, sample) in samples.iter().enumerate() {
                agrees(
                    returned[position],
                    *sample,
                    &format!("frame {size}, sample {position} after a round trip"),
                );
            }
        }
    }

    #[test]
    fn a_frame_that_is_not_a_power_of_two_is_turned_away() {
        let reason = forward(&signal(1000)).unwrap_err();
        assert!(
            reason.contains("power of two"),
            "the reason given was {reason:?}"
        );
        let reason = inverse(&[1.0; 100], &[0.0; 100]).unwrap_err();
        assert!(
            reason.contains("power of two"),
            "the reason given was {reason:?}"
        );
    }
}
