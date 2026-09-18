//! Acceptance tests for the Camelot wheel. A coder agent makes these pass without editing them.

use dermixen_analysis::{Camelot, Key, Letter, Mode, PitchClass};

/// The whole wheel, as `docs/library.md` prints it: each code with the key it names.
const WHEEL: [(u8, Letter, PitchClass, Mode); 24] = [
    (1, Letter::A, PitchClass::Gs, Mode::Minor),
    (1, Letter::B, PitchClass::B, Mode::Major),
    (2, Letter::A, PitchClass::Ds, Mode::Minor),
    (2, Letter::B, PitchClass::Fs, Mode::Major),
    (3, Letter::A, PitchClass::As, Mode::Minor),
    (3, Letter::B, PitchClass::Cs, Mode::Major),
    (4, Letter::A, PitchClass::F, Mode::Minor),
    (4, Letter::B, PitchClass::Gs, Mode::Major),
    (5, Letter::A, PitchClass::C, Mode::Minor),
    (5, Letter::B, PitchClass::Ds, Mode::Major),
    (6, Letter::A, PitchClass::G, Mode::Minor),
    (6, Letter::B, PitchClass::As, Mode::Major),
    (7, Letter::A, PitchClass::D, Mode::Minor),
    (7, Letter::B, PitchClass::F, Mode::Major),
    (8, Letter::A, PitchClass::A, Mode::Minor),
    (8, Letter::B, PitchClass::C, Mode::Major),
    (9, Letter::A, PitchClass::E, Mode::Minor),
    (9, Letter::B, PitchClass::G, Mode::Major),
    (10, Letter::A, PitchClass::B, Mode::Minor),
    (10, Letter::B, PitchClass::D, Mode::Major),
    (11, Letter::A, PitchClass::Fs, Mode::Minor),
    (11, Letter::B, PitchClass::A, Mode::Major),
    (12, Letter::A, PitchClass::Cs, Mode::Minor),
    (12, Letter::B, PitchClass::E, Mode::Major),
];

fn code(number: u8, letter: Letter) -> Camelot {
    Camelot::new(number, letter).unwrap()
}

#[test]
fn every_key_has_its_place_on_the_wheel() {
    for (number, letter, tonic, mode) in WHEEL {
        let key = Key { tonic, mode };
        let expected = code(number, letter);
        assert_eq!(key.camelot(), expected, "{key:?}");
        assert_eq!(expected.key(), key, "{expected}");
    }
}

#[test]
fn only_numbers_from_one_to_twelve_are_on_the_wheel() {
    assert!(Camelot::new(0, Letter::A).is_none());
    assert!(Camelot::new(13, Letter::B).is_none());
    assert_eq!(code(12, Letter::B).number(), 12);
    assert_eq!(code(12, Letter::B).letter(), Letter::B);
}

#[test]
fn a_code_is_written_and_read_as_a_number_and_a_letter() {
    assert_eq!(code(8, Letter::A).to_string(), "8A");
    assert_eq!(code(12, Letter::B).to_string(), "12B");
    assert_eq!("8A".parse::<Camelot>().unwrap(), code(8, Letter::A));
    assert_eq!("8a".parse::<Camelot>().unwrap(), code(8, Letter::A));
    assert_eq!(" 12B\n".parse::<Camelot>().unwrap(), code(12, Letter::B));
    for text in ["13A", "0A", "8C", "A8", "8", "", "8AB"] {
        assert!(
            text.parse::<Camelot>().is_err(),
            "{text:?} should be refused"
        );
    }
}

#[test]
fn a_code_serializes_as_its_text() {
    let json = serde_json::to_string(&code(8, Letter::A)).unwrap();
    assert_eq!(json, "\"8A\"");
    assert_eq!(
        serde_json::from_str::<Camelot>("\"11B\"").unwrap(),
        code(11, Letter::B)
    );
    assert!(serde_json::from_str::<Camelot>("\"13B\"").is_err());
    assert!(serde_json::from_str::<Camelot>("8").is_err());
}

#[test]
fn neighbors_on_the_wheel_are_compatible() {
    let eight_a = code(8, Letter::A);
    for (other, compatible) in [
        (code(8, Letter::A), true),
        (code(7, Letter::A), true),
        (code(9, Letter::A), true),
        (code(8, Letter::B), true),
        (code(10, Letter::A), false),
        (code(6, Letter::A), false),
        (code(7, Letter::B), false),
        (code(9, Letter::B), false),
        (code(2, Letter::A), false),
    ] {
        assert_eq!(
            eight_a.is_compatible_with(other),
            compatible,
            "8A with {other}"
        );
        assert_eq!(
            other.is_compatible_with(eight_a),
            compatible,
            "{other} with 8A"
        );
    }
    // The wheel wraps: twelve and one are one step apart.
    assert!(code(12, Letter::A).is_compatible_with(code(1, Letter::A)));
    assert!(code(1, Letter::B).is_compatible_with(code(12, Letter::B)));
    assert!(!code(12, Letter::A).is_compatible_with(code(2, Letter::A)));
}
