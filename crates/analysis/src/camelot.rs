//! The Camelot wheel: the notation DJs use for keys, and the mapping between
//! it and a tonic and mode.
//!
//! Every key has one Camelot code, a number from 1 to 12 and a letter, `A`
//! for minor and `B` for major. Codes that are one step apart around the
//! wheel, or that share a number, name keys that mix well, which is what the
//! library uses to say which tracks are harmonically compatible. The whole
//! wheel is written out in `docs/library.md`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::analyzer::{Key, Mode, PitchClass};

/// The letter half of a Camelot code: `A` for a minor key, `B` for a major key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Letter {
    A,
    B,
}

/// One position on the Camelot wheel, such as `8A`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Camelot {
    number: u8,
    letter: Letter,
}

/// The reason a text could not be read as a Camelot code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a Camelot code is a number from 1 to 12 followed by A or B, such as 8A, got {0:?}")]
pub struct CamelotParseError(pub String);

/// The whole wheel, in the order printed in `docs/library.md`: each code with
/// the key it names.
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

impl Camelot {
    /// The number, from 1 to 12.
    pub fn number(self) -> u8 {
        self.number
    }

    /// The letter.
    pub fn letter(self) -> Letter {
        self.letter
    }

    /// Builds a code, or returns `None` when the number is not from 1 to 12.
    pub fn new(number: u8, letter: Letter) -> Option<Camelot> {
        (1..=12)
            .contains(&number)
            .then_some(Camelot { number, letter })
    }

    /// The key this code names.
    pub fn key(self) -> Key {
        WHEEL
            .iter()
            .find(|(number, letter, ..)| *number == self.number && *letter == self.letter)
            .map(|(_, _, tonic, mode)| Key {
                tonic: *tonic,
                mode: *mode,
            })
            .expect("every code built through Camelot::new is on the wheel")
    }

    /// Whether two codes name keys that mix well: the same code, the same
    /// number with the other letter, or the same letter with a number one
    /// step away around the wheel, where 12 and 1 are one step apart.
    pub fn is_compatible_with(self, other: Camelot) -> bool {
        if self.number == other.number {
            return true;
        }
        if self.letter != other.letter {
            return false;
        }
        let apart = self.number.abs_diff(other.number);
        apart == 1 || apart == 11
    }
}

impl Key {
    /// The Camelot code of this key.
    pub fn camelot(self) -> Camelot {
        WHEEL
            .iter()
            .find(|(_, _, tonic, mode)| *tonic == self.tonic && *mode == self.mode)
            .map(|(number, letter, ..)| Camelot {
                number: *number,
                letter: *letter,
            })
            .expect("every key is on the wheel")
    }
}

impl fmt::Display for Camelot {
    /// Writes the code as the number followed by the letter, as in `8A`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let letter = match self.letter {
            Letter::A => 'A',
            Letter::B => 'B',
        };
        write!(f, "{}{letter}", self.number)
    }
}

impl FromStr for Camelot {
    type Err = CamelotParseError;

    /// Reads a code such as `8A` or `12b`. Surrounding whitespace is
    /// ignored and the letter may be in either case.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || CamelotParseError(text.to_owned());
        let trimmed = text.trim();
        let mut chars = trimmed.chars();
        let letter = match chars.next_back() {
            Some('A' | 'a') => Letter::A,
            Some('B' | 'b') => Letter::B,
            _ => return Err(error()),
        };
        let digits = chars.as_str();
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(error());
        }
        let number: u8 = digits.parse().map_err(|_| error())?;
        Camelot::new(number, letter).ok_or_else(error)
    }
}

impl Serialize for Camelot {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Camelot {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}
