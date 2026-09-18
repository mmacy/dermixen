//! Acceptance tests for the libkeyfinder baseline. A coder agent makes these pass
//! without editing them. They compile only with the `keyfinder` feature.

#![cfg(feature = "keyfinder")]

use std::f64::consts::TAU;

use dermixen_analysis::{Key, KeyAnalyzer, KeyfinderKey, Mode, PitchClass};
use dermixen_core::SAMPLE_RATE;
use dermixen_media::Audio;

/// The frequency of a note given as semitones above middle C.
fn frequency(semitones: i32) -> f64 {
    261.625_565 * 2f64.powf(f64::from(semitones) / 12.0)
}

/// A progression of block chords, two seconds each, every chord a set of
/// notes given as semitones above middle C, played as sines with their octaves.
fn progression(chords: &[&[i32]]) -> Audio {
    let rate = f64::from(SAMPLE_RATE);
    let per_chord = 2 * SAMPLE_RATE as usize;
    let mut frames = Vec::with_capacity(per_chord * chords.len());
    for chord in chords {
        for i in 0..per_chord {
            let t = i as f64 / rate;
            let mut value = 0.0;
            for note in *chord {
                for octave in [0, 12] {
                    value += (TAU * frequency(note + octave) * t).sin();
                }
            }
            let value = (value / (chord.len() as f64 * 2.0) * 0.5) as f32;
            frames.push([value, value]);
        }
    }
    Audio { frames }
}

#[test]
fn a_progression_in_c_major_is_heard_in_c_major() {
    // C, F, G, C: the tonic, subdominant, and dominant of C major.
    let audio = progression(&[&[0, 4, 7], &[5, 9, 12], &[7, 11, 14], &[0, 4, 7]]);
    let found = KeyfinderKey.analyze(&audio).unwrap();
    assert_eq!(
        found.key,
        Key {
            tonic: PitchClass::C,
            mode: Mode::Major
        }
    );
    assert!(
        (0.0..=1.0).contains(&found.confidence),
        "{}",
        found.confidence
    );
}

#[test]
fn a_progression_in_a_minor_is_heard_in_a_minor() {
    // A minor, D minor, E major, A minor: the tonic, subdominant, and
    // dominant of A minor, with the raised leading tone in the dominant.
    let audio = progression(&[&[-3, 0, 4], &[2, 5, 9], &[4, 8, 11], &[-3, 0, 4]]);
    let found = KeyfinderKey.analyze(&audio).unwrap();
    assert_eq!(
        found.key,
        Key {
            tonic: PitchClass::A,
            mode: Mode::Minor
        }
    );
}

#[test]
fn a_progression_in_f_sharp_minor_is_heard_in_f_sharp_minor() {
    // F sharp minor, B minor, C sharp major, F sharp minor.
    let audio = progression(&[&[6, 9, 13], &[11, 14, 18], &[13, 17, 20], &[6, 9, 13]]);
    let found = KeyfinderKey.analyze(&audio).unwrap();
    assert_eq!(
        found.key,
        Key {
            tonic: PitchClass::Fs,
            mode: Mode::Minor
        }
    );
}

#[test]
fn the_analyzer_is_named_for_the_scoreboard_and_refuses_empty_audio() {
    assert_eq!(KeyfinderKey.name(), "keyfinder");
    assert!(KeyfinderKey.analyze(&Audio::new()).is_err());
}
