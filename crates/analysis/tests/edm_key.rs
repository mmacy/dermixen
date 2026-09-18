//! Acceptance tests for the electronic-music key analyzer. A coder agent makes
//! these pass without editing them.

use std::f64::consts::TAU;

use dermixen_analysis::{EdmKey, Key, KeyAnalyzer, Mode, PitchClass};
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
    let found = EdmKey.analyze(&audio).unwrap();
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
    let found = EdmKey.analyze(&audio).unwrap();
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
    let found = EdmKey.analyze(&audio).unwrap();
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
    assert_eq!(EdmKey.name(), "edm");
    assert!(EdmKey.analyze(&Audio::new()).is_err());
}

#[test]
fn windows_that_agree_score_a_higher_confidence_than_windows_that_do_not() {
    // Eight seconds in C major throughout, against eight seconds that spend
    // half their time in C major and half in F sharp major, the key furthest
    // from it around the circle of fifths.
    let steady = progression(&[&[0, 4, 7], &[5, 9, 12], &[7, 11, 14], &[0, 4, 7]]);
    let split = progression(&[&[0, 4, 7], &[5, 9, 12], &[6, 10, 13], &[11, 15, 18]]);
    let steady = EdmKey.analyze(&steady).unwrap();
    let split = EdmKey.analyze(&split).unwrap();
    assert!((0.0..=1.0).contains(&steady.confidence));
    assert!((0.0..=1.0).contains(&split.confidence));
    assert!(
        steady.confidence > split.confidence,
        "steady {} against split {}",
        steady.confidence,
        split.confidence
    );
}
