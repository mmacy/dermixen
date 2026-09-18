//! Text on its way to a terminal.
//!
//! A file name, a tag, and a library record are all text somebody else wrote,
//! and a terminal reads some of that text as instructions rather than as
//! letters: an escape character starts a sequence that clears the screen,
//! renames the window, or colors what follows, and a newline in a file name
//! turns one track of a listing into two lines. Every line this command
//! prints as text goes through [`shown`] first, so a control character
//! appears as the four characters a person can read and one track is always
//! one line. JSON output goes out as it is, because JSON writes a control
//! character as an escape of its own.

use std::path::Path;

/// `text` with every character a terminal reads as an instruction written
/// out as `\xNN`, where `NN` is the character's number in hexadecimal.
///
/// The characters written out are the C0 controls, which are the characters
/// below a space and include the newline and the tab, the delete character,
/// and the C1 controls, which are the characters from `\x80` to `\x9f` and
/// include the one-character forms of the sequences a terminal acts on.
/// Every other character, including every letter of every alphabet, is left
/// as it is.
pub fn shown(text: &str) -> String {
    if !text.chars().any(drives_a_terminal) {
        return text.to_owned();
    }
    let mut written = String::with_capacity(text.len());
    for character in text.chars() {
        if drives_a_terminal(character) {
            written.push_str(&format!("\\x{:02x}", character as u32));
        } else {
            written.push(character);
        }
    }
    written
}

/// A path as [`shown`] writes text, for a line a person reads.
pub fn shown_path(path: &Path) -> String {
    shown(&path.display().to_string())
}

/// Whether a terminal reads this character as an instruction rather than as
/// a letter.
fn drives_a_terminal(character: char) -> bool {
    let code = character as u32;
    code < 0x20 || code == 0x7f || (0x80..=0x9f).contains(&code)
}

/// Prints one line on standard output, as [`println!`] does, with the line
/// passed through [`shown`] so that text from a file cannot drive the
/// terminal. Every text line this command prints goes out this way.
macro_rules! say {
    ($($argument:tt)*) => {
        println!("{}", $crate::text::shown(&format!($($argument)*)))
    };
}

/// Prints one line on standard error, as [`eprintln!`] does, with the line
/// passed through [`shown`] the way [`say`] passes a line to standard
/// output. Progress, warnings, and the `error:` line all go out this way.
macro_rules! note {
    ($($argument:tt)*) => {
        eprintln!("{}", $crate::text::shown(&format!($($argument)*)))
    };
}

pub(crate) use {note, say};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_with_nothing_to_escape_is_left_as_it_is() {
        assert_eq!(shown("Etnica - Alpha.wav"), "Etnica - Alpha.wav");
        assert_eq!(shown("Café Del Mar ☀"), "Café Del Mar ☀");
    }

    #[test]
    fn a_control_character_is_written_out() {
        assert_eq!(shown("a\u{1b}[2Jb"), "a\\x1b[2Jb");
        assert_eq!(shown("one\ntwo"), "one\\x0atwo");
        assert_eq!(shown("bell\u{7}"), "bell\\x07");
        assert_eq!(shown("tab\tstop"), "tab\\x09stop");
        assert_eq!(shown("del\u{7f}"), "del\\x7f");
        assert_eq!(shown("csi\u{9b}31m"), "csi\\x9b31m");
    }
}
