//! The content hash that identifies an audio file independently of its path.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A 256-bit BLAKE3 hash of an audio file's bytes.
///
/// The library index and project files refer to audio by this hash as well as
/// by path, so a file that is moved or renamed is still recognized, and a file
/// whose bytes change is treated as a different track. In text form it is
/// sixty-four hexadecimal digits; it is written in lowercase and read in
/// either case.
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ContentHash(pub [u8; 32]);

/// The reason a text form could not be read as a [`ContentHash`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a content hash is 64 hexadecimal digits, got {0:?}")]
pub struct ContentHashParseError(pub String);

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({self})")
    }
}

impl FromStr for ContentHash {
    type Err = ContentHashParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 64 {
            return Err(ContentHashParseError(text.to_owned()));
        }
        let mut out = [0u8; 32];
        for (i, pair) in bytes.chunks(2).enumerate() {
            let hi = hex_digit(pair[0]);
            let lo = hex_digit(pair[1]);
            match (hi, lo) {
                (Some(hi), Some(lo)) => out[i] = hi << 4 | lo,
                _ => return Err(ContentHashParseError(text.to_owned())),
            }
        }
        Ok(ContentHash(out))
    }
}

fn hex_digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn round_trips_through_text() {
        let hash: ContentHash = HEX.parse().unwrap();
        assert_eq!(hash.0[1], 1);
        assert_eq!(hash.0[31], 31);
        assert_eq!(hash.to_string(), HEX);
    }

    #[test]
    fn accepts_uppercase_and_prints_lowercase() {
        let hash: ContentHash = HEX.to_uppercase().parse().unwrap();
        assert_eq!(hash.to_string(), HEX);
    }

    #[test]
    fn rejects_wrong_length_and_bad_digits() {
        assert!("abc".parse::<ContentHash>().is_err());
        assert!(HEX.replace('0', "g").parse::<ContentHash>().is_err());
    }

    #[test]
    fn serializes_as_a_string() {
        let hash: ContentHash = HEX.parse().unwrap();
        let json = serde_json::to_string(&hash).unwrap();
        assert_eq!(json, format!("\"{HEX}\""));
        assert_eq!(serde_json::from_str::<ContentHash>(&json).unwrap(), hash);
        assert!(serde_json::from_str::<ContentHash>("\"zz\"").is_err());
    }
}
