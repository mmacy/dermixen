//! A key analyzer with profiles tuned for electronic dance music.
//!
//! The libkeyfinder baseline scores each of the twenty-four keys against
//! the track's average spread of pitch classes using profiles derived from
//! listening experiments on classical music. Published work on Beatport
//! material shows that profiles fitted to electronic dance music, where the
//! bass note and the fifth dominate and the leading tone is rare, name the
//! key more often. This analyzer is that idea: a chromagram of the track
//! built in pure Rust, scored against electronic-music profiles for major
//! and minor, with the key reported per window so that how often the
//! windows agree becomes the confidence.

use std::f64::consts::{PI, TAU};

use dermixen_core::SAMPLE_RATE;
use dermixen_media::Audio;
use rustfft::FftPlanner;
use rustfft::num_complex::Complex32;

use crate::analyzer::{AnalysisError, Key, KeyAnalysis, KeyAnalyzer, Mode, PitchClass};

/// The twelve pitch classes in semitone order from C, so that a count of
/// semitones above C is a position in this list.
const TONICS: [PitchClass; 12] = [
    PitchClass::C,
    PitchClass::Cs,
    PitchClass::D,
    PitchClass::Ds,
    PitchClass::E,
    PitchClass::F,
    PitchClass::Fs,
    PitchClass::G,
    PitchClass::Gs,
    PitchClass::A,
    PitchClass::As,
    PitchClass::B,
];

// The two profiles below are the `edma` profiles from Ángel Faraldo's work on
// key estimation in electronic dance music, fitted to a corpus of Beatport
// tracks rather than to listening experiments on classical music. The papers
// are Ángel Faraldo, Emilia Gómez, Sergi Jordà and Perfecto Herrera, "Key
// Estimation in Electronic Dance Music", in the Proceedings of the 38th
// European Conference on Information Retrieval, pages 335 to 347, 2016; and
// Ángel Faraldo, Sergi Jordà and Perfecto Herrera, "A Multi-Profile Method for
// Key Estimation in EDM", at the 2017 Audio Engineering Society International
// Conference on Semantic Audio. The twelve-element vectors are copied from the
// reference implementation in Essentia, the file
// `src/algorithms/tonal/key.cpp`, where they are the rows labelled `edma`.
// Essentia is licensed under the AGPL-3.0, and the vectors are published
// research data from the papers cited above rather than Essentia's code.
//
// Faraldo's other published set, called `edmm`, is not used here. Its major
// profile is flat, twelve equal weights, which is how that set deliberately
// reports every major-key track as its relative minor. Dermixen needs both
// modes named, so the flat profile would make the answer wrong by
// construction for every major track.

/// The weight expected of each pitch class in a major key, counted in
/// semitones above the tonic.
const MAJOR_PROFILE: [f64; 12] = [
    1.00, 0.29, 0.50, 0.40, 0.60, 0.56, 0.32, 0.80, 0.31, 0.45, 0.42, 0.39,
];

/// The weight expected of each pitch class in a minor key, counted in
/// semitones above the tonic.
const MINOR_PROFILE: [f64; 12] = [
    1.00, 0.31, 0.44, 0.58, 0.33, 0.49, 0.29, 0.78, 0.43, 0.29, 0.53, 0.32,
];

/// How much audio one window covers, in seconds.
///
/// A window has to be long enough to hold a whole bass figure, and short
/// enough that a track which changes key gives windows that disagree rather
/// than one smeared average. Two seconds is a bar of four beats at one
/// hundred and twenty beats per minute, which is the shortest stretch of this
/// music that reliably states a key.
const WINDOW_SECONDS: f64 = 2.0;

/// How many samples one window covers.
const WINDOW_SAMPLES: usize = (WINDOW_SECONDS * SAMPLE_RATE as f64) as usize;

/// How many samples go into one spectrum.
///
/// At the sample rate Dermixen works at this frame spans about three hundred
/// and seventy milliseconds and puts the frequency bins 2.7 hertz apart. The
/// bins have to be close enough together to separate two neighbouring
/// semitones down in the bass, where a semitone is under four hertz, because
/// the bass note is what names the key in this music.
const FRAME: usize = 16_384;

/// How far one frame starts after the frame before it, in samples.
const HOP: usize = 8_192;

/// The lowest frequency a spectral peak may sit at and still count, in hertz.
///
/// This is a little above the C two octaves below middle C. Below it lies the
/// kick drum, which is loud, has no settled pitch, and would otherwise put a
/// strong peak into every frame.
const MIN_HZ: f64 = 65.0;

/// The highest frequency a spectral peak may sit at and still count, in hertz.
///
/// This is the A two octaves above the A above middle C, about four and a
/// half octaves above the lowest note considered. Everything above it is
/// mostly cymbals and the upper partials of notes whose fundamentals are
/// already counted, and letting it in measurably costs accuracy.
const MAX_HZ: f64 = 1_760.0;

/// The frequency of the C five octaves below middle C, in hertz, which is
/// where the count of semitones above C starts.
const C0_HZ: f64 = 16.351_597_831_287_414;

/// How far below the loudest peak of a frame a peak may be and still count,
/// as a fraction of that loudest peak. Fifteen hundredths is about sixteen
/// decibels down.
///
/// Drums and the noise floor of a lossy file put small bumps all over the
/// spectrum, and every one of them is a local maximum. Keeping only the
/// peaks close to the loudest one leaves the notes and throws the bumps
/// away.
const PEAK_FLOOR: f64 = 0.15;

/// The magnitude below which a frame counts as holding nothing at all.
///
/// A frame of digital silence gives exactly zero. Decoded silence from a
/// lossy file gives a little noise, and over sixteen thousand samples that
/// noise still lands far below this figure, while any audible note lands far
/// above it.
const SILENCE: f64 = 1e-3;

/// How far from a pitch class, in semitones, a peak can sit and still add
/// anything to it.
///
/// A peak exactly on a pitch class adds its whole magnitude there and nothing
/// anywhere else. A peak halfway between two pitch classes splits evenly
/// between them, which is what keeps a track tuned slightly away from concert
/// pitch from falling into the gap.
const SPREAD_SEMITONES: f64 = 1.0;

/// The electronic-music key analyzer.
///
/// The audio is mixed to one channel and cut into windows of a few seconds.
/// Each window becomes a twelve-bin chromagram, the energy at each pitch
/// class across several octaves, and each window's chromagram is scored
/// against the twenty-four key profiles; the track's key is the key with
/// the highest total score across the windows. The confidence is the share
/// of windows whose own best key is the track's key, from zero to one, so a
/// track that stays in one key throughout scores near one and a track whose
/// windows disagree scores low. Audio shorter than one window is refused as
/// too short.
#[derive(Debug, Default, Clone, Copy)]
pub struct EdmKey;

impl KeyAnalyzer for EdmKey {
    fn name(&self) -> &str {
        "edm"
    }

    fn analyze(&self, audio: &Audio) -> Result<KeyAnalysis, AnalysisError> {
        let window_count = audio.frames.len() / WINDOW_SAMPLES;
        if window_count == 0 {
            return Err(AnalysisError::TooShort(audio.len().0));
        }

        let mono: Vec<f32> = audio
            .frames
            .iter()
            .map(|frame| 0.5 * (frame[0] + frame[1]))
            .collect();
        let windows = chromagram(&mono, window_count, WINDOW_SAMPLES);
        let candidates = candidates();

        // Each window is read twice over. Its score against every one of the
        // twenty-four keys goes into the running totals, and those totals are
        // what name the track's key. Separately, the one key that scores
        // highest in that window on its own is written down, and how many
        // windows wrote down the track's key is the confidence. A window with
        // no pitched sound in it is passed over and counts toward neither.
        let mut totals = [0.0f64; 24];
        let mut best_per_window: Vec<usize> = Vec::with_capacity(windows.len());
        for window in &windows {
            let Some(observed) = unit(*window) else {
                continue;
            };
            let mut best_key = 0usize;
            let mut best_score = f64::NEG_INFINITY;
            for (index, candidate) in candidates.iter().enumerate() {
                let score = dot(&observed, &candidate.profile);
                totals[index] += score;
                if score > best_score {
                    best_score = score;
                    best_key = index;
                }
            }
            best_per_window.push(best_key);
        }

        if best_per_window.is_empty() {
            return Err(AnalysisError::Failed(
                "the audio holds no pitched sound to read a key from".to_owned(),
            ));
        }

        let winner = (0..candidates.len())
            .max_by(|left, right| totals[*left].total_cmp(&totals[*right]))
            .expect("there are twenty-four keys to choose between");
        let agreeing = best_per_window.iter().filter(|key| **key == winner).count();
        Ok(KeyAnalysis {
            key: candidates[winner].key,
            confidence: agreeing as f64 / best_per_window.len() as f64,
        })
    }
}

/// One key and the spread of pitch classes expected of it.
struct Candidate {
    /// The key this profile stands for.
    key: Key,
    /// The profile rotated onto this key's tonic, with its mean taken out and
    /// scaled to unit length, so that a dot product with a chromagram
    /// prepared the same way is the correlation between the two.
    profile: [f64; 12],
}

/// The twenty-four keys and their profiles, major then minor for each tonic
/// in semitone order from C.
fn candidates() -> Vec<Candidate> {
    let mut out = Vec::with_capacity(24);
    for (semitone, tonic) in TONICS.iter().enumerate() {
        for (mode, profile) in [(Mode::Major, MAJOR_PROFILE), (Mode::Minor, MINOR_PROFILE)] {
            // The profile is written from the tonic upward, so putting it on
            // a tonic means moving each of its weights up by the tonic's
            // distance above C.
            let mut rotated = [0.0f64; 12];
            for (step, weight) in profile.iter().enumerate() {
                rotated[(semitone + step) % 12] = *weight;
            }
            out.push(Candidate {
                key: Key {
                    tonic: *tonic,
                    mode,
                },
                profile: unit(rotated).expect("a published key profile is not flat"),
            });
        }
    }
    out
}

/// The notes heard in one frame of the track, and the window that frame
/// belongs to.
struct FramePeaks {
    /// Which window the middle of the frame falls in.
    window: usize,
    /// One entry per peak of the frame's spectrum: where the peak sits, in
    /// semitones above the C five octaves below middle C, and how loud it was
    /// next to the loudest peak of the same frame.
    peaks: Vec<(f64, f64)>,
}

/// The chromagram of each window of the track, in order.
///
/// The whole track is stepped through one frame at a time and the notes in
/// each frame are found. How the track is tuned is read from all of those
/// notes together, because in a track recorded or pitched a fraction of a
/// semitone away from concert pitch every note sits that same fraction away
/// from the nearest note of the equal-tempered scale. Every note is then
/// moved back by that fraction, each frame becomes a twelve-bin chromagram of
/// its own, and the frames are added into the window their middle falls in.
/// Audio past the last whole window is added to that last window rather than
/// thrown away.
fn chromagram(mono: &[f32], window_count: usize, window_samples: usize) -> Vec<[f64; 12]> {
    let frames = spectral_peaks(mono, window_count, window_samples);
    let offset = tuning_offset(&frames);

    let mut windows = vec![[0.0f64; 12]; window_count];
    for frame in &frames {
        let mut chroma = [0.0f64; 12];
        for (position, strength) in &frame.peaks {
            spread(&mut chroma, position - offset, *strength);
        }
        // Scaling each frame by its own largest bin makes a loud passage and
        // a quiet one weigh the same on the window they belong to.
        let largest = chroma.iter().copied().fold(0.0f64, f64::max);
        if largest <= 0.0 {
            continue;
        }
        for (total, part) in windows[frame.window].iter_mut().zip(&chroma) {
            *total += part / largest;
        }
    }
    windows
}

/// The notes heard in every frame of the track, in order.
fn spectral_peaks(mono: &[f32], window_count: usize, window_samples: usize) -> Vec<FramePeaks> {
    let mut frames: Vec<FramePeaks> = Vec::new();
    if mono.len() < FRAME {
        return frames;
    }

    let mut planner = FftPlanner::<f32>::new();
    let transform = planner.plan_fft_forward(FRAME);
    let mut scratch = vec![Complex32::new(0.0, 0.0); transform.get_inplace_scratch_len()];
    let mut spectrum = vec![Complex32::new(0.0, 0.0); FRAME];
    let mut magnitudes = vec![0.0f64; FRAME / 2 + 1];

    // A raised cosine taper, so that the ends of a frame fade in and out and
    // a note that does not fit a whole number of cycles into the frame still
    // shows up as one clean peak rather than a smear.
    let taper: Vec<f32> = (0..FRAME)
        .map(|i| (0.5 - 0.5 * (TAU * i as f64 / FRAME as f64).cos()) as f32)
        .collect();
    let bin_hz = f64::from(SAMPLE_RATE) / FRAME as f64;

    let mut start = 0;
    while start + FRAME <= mono.len() {
        for (slot, index) in spectrum.iter_mut().zip(0..FRAME) {
            *slot = Complex32::new(mono[start + index] * taper[index], 0.0);
        }
        transform.process_with_scratch(&mut spectrum, &mut scratch);
        for (magnitude, value) in magnitudes.iter_mut().zip(spectrum.iter()) {
            *magnitude = f64::from(value.norm());
        }

        let peaks = frame_peaks(&magnitudes, bin_hz);
        if !peaks.is_empty() {
            frames.push(FramePeaks {
                window: ((start + FRAME / 2) / window_samples).min(window_count - 1),
                peaks,
            });
        }
        start += HOP;
    }
    frames
}

/// How far the track sits from concert pitch, in semitones, somewhere between
/// half a semitone below and half a semitone above.
///
/// Every note found in the track sits a fraction of a semitone away from the
/// nearest note of the equal-tempered scale. Those fractions run round in a
/// circle, so they are averaged as directions rather than as plain numbers: a
/// note a hundredth of a semitone below and one a hundredth above cancel to
/// nothing rather than averaging to a half. In a track at concert pitch the
/// fractions bunch around zero and the answer comes out near zero; in a track
/// pitched up a quarter of a semitone they bunch around a quarter and the
/// answer comes out there.
///
/// Getting this right matters a great deal. In a track read a quarter of a
/// semitone out of tune, every note is split between two neighbouring pitch
/// classes, and no key profile matches a chromagram smeared that way.
fn tuning_offset(frames: &[FramePeaks]) -> f64 {
    let mut across = 0.0f64;
    let mut up = 0.0f64;
    for frame in frames {
        for (position, strength) in &frame.peaks {
            let angle = TAU * position;
            across += strength * angle.cos();
            up += strength * angle.sin();
        }
    }
    if across == 0.0 && up == 0.0 {
        return 0.0;
    }
    up.atan2(across) / TAU
}

/// The notes heard in one frame, read off that frame's magnitude spectrum.
///
/// Only the peaks of the spectrum count. A peak is a bin louder than both of
/// its neighbours and not too far below the loudest bin of the frame, which
/// leaves out the broadband wash of the drums and keeps the notes. Each peak
/// comes back as its position in semitones above the C five octaves below
/// middle C, paired with how loud it was next to the loudest peak of the same
/// frame. A frame holding nothing but silence comes back with no peaks at all.
fn frame_peaks(magnitudes: &[f64], bin_hz: f64) -> Vec<(f64, f64)> {
    let lowest = ((MIN_HZ / bin_hz).ceil() as usize).max(1);
    let highest = ((MAX_HZ / bin_hz).floor() as usize).min(magnitudes.len() - 2);
    if lowest >= highest {
        return Vec::new();
    }

    let loudest = magnitudes[lowest..=highest]
        .iter()
        .copied()
        .fold(0.0f64, f64::max);
    if loudest <= SILENCE {
        return Vec::new();
    }
    let floor = loudest * PEAK_FLOOR;

    let mut peaks = Vec::new();
    for bin in lowest..=highest {
        let here = magnitudes[bin];
        if here < floor || here <= magnitudes[bin - 1] || here < magnitudes[bin + 1] {
            continue;
        }
        let (frequency, magnitude) =
            refine(magnitudes[bin - 1], here, magnitudes[bin + 1], bin, bin_hz);
        peaks.push((12.0 * (frequency / C0_HZ).log2(), magnitude / loudest));
    }
    peaks
}

/// The true frequency and magnitude of a peak whose three loudest bins are
/// given, with the bin number of the middle one.
///
/// A note rarely lands exactly on a bin, so the bin that came out loudest is
/// only an approximation of where the note is. Fitting a parabola through the
/// logarithms of the three bins recovers the note's frequency far more
/// closely than the spacing of the bins would suggest, which is what lets a
/// bass note be told from its neighbouring semitone.
fn refine(left: f64, middle: f64, right: f64, bin: usize, bin_hz: f64) -> (f64, f64) {
    let left = left.max(SILENCE).ln();
    let peak = middle.max(SILENCE).ln();
    let right = right.max(SILENCE).ln();
    let curvature = left - 2.0 * peak + right;
    let offset = if curvature == 0.0 {
        0.0
    } else {
        (0.5 * (left - right) / curvature).clamp(-0.5, 0.5)
    };
    let frequency = (bin as f64 + offset) * bin_hz;
    let magnitude = (peak - 0.25 * (left - right) * offset).exp();
    (frequency, magnitude)
}

/// Adds a weight at a position given in semitones above C to the pitch
/// classes nearest it.
///
/// The weight is shared between the pitch classes within one semitone of the
/// position, most going to the nearest, in the shape of a raised cosine.
fn spread(chroma: &mut [f64; 12], semitones_above_c: f64, weight: f64) {
    let position = semitones_above_c.rem_euclid(12.0);
    for (bin, value) in chroma.iter_mut().enumerate() {
        // The pitch classes run in a circle, so the distance from a position
        // to a bin is the shorter way round.
        let straight = position - bin as f64;
        let distance = straight - 12.0 * (straight / 12.0).round();
        if distance.abs() >= SPREAD_SEMITONES {
            continue;
        }
        let shaped = (PI / 2.0 * distance / SPREAD_SEMITONES).cos();
        *value += weight * shaped * shaped;
    }
}

/// The twelve values with their mean taken out and scaled to unit length, or
/// nothing at all when every value is the same and there is no shape left to
/// scale.
///
/// Preparing both a chromagram and a profile this way makes the dot product
/// of the two the correlation between them, which is how well the shape of
/// what was heard matches the shape the key expects. Taking the mean out is
/// what stops a track that is simply loud everywhere from matching every key
/// equally well.
fn unit(values: [f64; 12]) -> Option<[f64; 12]> {
    let mean = values.iter().sum::<f64>() / 12.0;
    let mut centered = values.map(|value| value - mean);
    let length = centered
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    for value in centered.iter_mut() {
        *value /= length;
    }
    Some(centered)
}

/// The dot product of two prepared twelve-element vectors.
fn dot(left: &[f64; 12], right: &[f64; 12]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_put_on_a_tonic_names_that_tonic_the_strongest() {
        let candidates = candidates();
        assert_eq!(candidates.len(), 24);
        // Every profile has its largest weight on its own tonic, so after the
        // mean is taken out the tonic's bin is still the largest.
        for candidate in &candidates {
            let tonic = candidate.key.tonic as usize;
            let largest = (0..12)
                .max_by(|left, right| {
                    candidate.profile[*left].total_cmp(&candidate.profile[*right])
                })
                .expect("a profile has twelve bins");
            assert_eq!(largest, tonic, "{}", candidate.key);
        }
    }

    #[test]
    fn a_profile_scores_highest_against_itself() {
        // A chromagram shaped exactly like one key's profile has to name that
        // key, or the scoring is not measuring what it claims to.
        for candidate in &candidates() {
            let mut chroma = [0.0f64; 12];
            for (bin, value) in chroma.iter_mut().enumerate() {
                *value = candidate.profile[bin] + 1.0;
            }
            let observed = unit(chroma).expect("a profile is not flat");
            let best = candidates()
                .into_iter()
                .max_by(|left, right| {
                    dot(&observed, &left.profile).total_cmp(&dot(&observed, &right.profile))
                })
                .expect("there are twenty-four keys");
            assert_eq!(best.key, candidate.key);
        }
    }

    #[test]
    fn a_flat_chromagram_has_no_shape_to_score() {
        assert!(unit([0.0; 12]).is_none());
        assert!(unit([3.5; 12]).is_none());
        let shaped = unit([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0])
            .expect("one raised bin is a shape");
        let length: f64 = shaped.iter().map(|value| value * value).sum();
        assert!((length - 1.0).abs() < 1e-12, "{length}");
    }

    #[test]
    fn a_peak_between_two_pitch_classes_is_shared_between_them() {
        // Half a semitone above C, the weight splits evenly between C and C
        // sharp and reaches nothing else.
        let mut chroma = [0.0f64; 12];
        spread(&mut chroma, 0.5, 1.0);
        assert!((chroma[0] - 0.5).abs() < 1e-12, "{}", chroma[0]);
        assert!((chroma[1] - 0.5).abs() < 1e-12, "{}", chroma[1]);
        for value in chroma.iter().skip(2) {
            assert_eq!(*value, 0.0);
        }

        // Squarely on B, everything goes to B, and the wrap around the circle
        // of pitch classes puts nothing on C.
        let mut chroma = [0.0f64; 12];
        spread(&mut chroma, 11.0, 1.0);
        assert!((chroma[11] - 1.0).abs() < 1e-12, "{}", chroma[11]);
        assert_eq!(chroma[0], 0.0);
    }

    /// A chord progression of block chords, two seconds each, every chord a
    /// set of notes given as semitones above middle C, played as sines with
    /// their octaves, with every frequency multiplied by `detune` so the
    /// whole thing can be pitched off concert pitch on purpose.
    fn progression(chords: &[&[i32]], detune: f64) -> Audio {
        let rate = f64::from(SAMPLE_RATE);
        let per_chord = 2 * SAMPLE_RATE as usize;
        let mut frames = Vec::with_capacity(per_chord * chords.len());
        for chord in chords {
            for i in 0..per_chord {
                let t = i as f64 / rate;
                let mut value = 0.0;
                for note in *chord {
                    for octave in [0, 12] {
                        let hertz =
                            261.625_565 * 2f64.powf(f64::from(note + octave) / 12.0) * detune;
                        value += (TAU * hertz * t).sin();
                    }
                }
                let value = (value / (chord.len() as f64 * 2.0) * 0.5) as f32;
                frames.push([value, value]);
            }
        }
        Audio { frames }
    }

    #[test]
    fn a_track_pitched_off_concert_pitch_is_measured_as_pitched_off() {
        // Three tenths of a semitone sharp, and then the same amount flat.
        for wanted in [0.3f64, -0.3] {
            let audio = progression(&[&[0, 4, 7]], 2f64.powf(wanted / 12.0));
            let mono: Vec<f32> = audio
                .frames
                .iter()
                .map(|frame| 0.5 * (frame[0] + frame[1]))
                .collect();
            let found = tuning_offset(&spectral_peaks(&mono, 1, WINDOW_SAMPLES));
            assert!(
                (found - wanted).abs() < 0.05,
                "wanted {wanted} and measured {found}"
            );
        }
    }

    #[test]
    fn a_track_pitched_off_concert_pitch_still_names_its_key() {
        // Without the tuning measurement every note would land between two
        // pitch classes and be split across both, and no key would match.
        let chords: [&[i32]; 4] = [&[0, 4, 7], &[5, 9, 12], &[7, 11, 14], &[0, 4, 7]];
        for shift in [0.0f64, 0.3, -0.3] {
            let audio = progression(&chords, 2f64.powf(shift / 12.0));
            let found = EdmKey.analyze(&audio).expect("eight seconds is enough");
            assert_eq!(
                found.key,
                Key {
                    tonic: PitchClass::C,
                    mode: Mode::Major
                },
                "pitched {shift} of a semitone away"
            );
        }
    }

    #[test]
    fn silence_holds_no_key() {
        let audio = Audio {
            frames: vec![[0.0, 0.0]; SAMPLE_RATE as usize * 12],
        };
        let Err(AnalysisError::Failed(reason)) = EdmKey.analyze(&audio) else {
            panic!("twelve seconds of silence should hold no key");
        };
        assert!(reason.contains("no pitched sound"), "{reason}");
    }

    #[test]
    fn audio_shorter_than_one_window_is_too_short() {
        let audio = Audio {
            frames: vec![[0.0, 0.0]; WINDOW_SAMPLES - 1],
        };
        assert_eq!(
            EdmKey.analyze(&audio),
            Err(AnalysisError::TooShort(WINDOW_SAMPLES as i64 - 1))
        );
        assert_eq!(
            EdmKey.analyze(&Audio::new()),
            Err(AnalysisError::TooShort(0))
        );
    }
}
