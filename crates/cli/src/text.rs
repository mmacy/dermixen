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
//!
//! Every write to standard output and standard error goes through this
//! module for a second reason: a reader such as `head` closes the pipe when
//! it has read enough, and a write after that fails. That is the reader's
//! choice rather than a failure of the command, so the command stops writing
//! on that stream, says nothing about it, and goes on to its end.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

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

/// Whether the reader of standard output has closed the pipe, and the same
/// for standard error.
///
/// A person who writes `dermixen library query | head` reads the first lines
/// and closes the pipe, which is their choice and not a failure of the
/// command. The first write after that fails with a broken pipe, and every
/// later write would fail the same way, so the flag is set and the command
/// prints nothing more on that stream. The command goes on to its end, so a
/// file it is writing is finished or removed as it would have been, and it
/// ends with the code it would have ended with.
static READER_GONE: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

/// Which stream a write goes to, which is also its place in [`READER_GONE`].
const OUT: usize = 0;
const ERR: usize = 1;

/// Writes `text` and a newline to standard output, or to standard error when
/// `stream` is [`ERR`], and gives up on that stream for good once its reader
/// has closed the pipe.
///
/// A write that fails for any other reason, such as a full disk under a
/// redirection, is a failure this command cannot report on the stream it was
/// reporting on, so it panics as [`println!`] does and the hook in `main.rs`
/// turns it into the one line and the exit code `docs/cli.md` gives a defect.
fn write_line(stream: usize, text: &str) {
    write_to(stream, format_args!("{text}\n"));
}

/// Writes `text` to the stream as it stands, without a newline of its own.
fn write_to(stream: usize, text: std::fmt::Arguments<'_>) {
    if READER_GONE[stream].load(Ordering::Relaxed) {
        return;
    }
    let written = if stream == ERR {
        std::io::stderr().lock().write_fmt(text)
    } else {
        std::io::stdout().lock().write_fmt(text)
    };
    if let Err(problem) = written {
        if problem.kind() == std::io::ErrorKind::BrokenPipe {
            READER_GONE[stream].store(true, Ordering::Relaxed);
            return;
        }
        panic!("cannot write the command's output: {problem}");
    }
}

/// Writes a block of text that holds newlines of its own, such as the
/// scoreboard's tables, escaping each of its lines as [`shown`] escapes one
/// and leaving the newlines between them.
pub fn block(text: &str) {
    let lines: Vec<String> = text.split('\n').map(shown).collect();
    write_to(OUT, format_args!("{}", lines.join("\n")));
}

/// Prints one line on standard output, as [`println!`] does, with the line
/// passed through [`shown`] so that text from a file cannot drive the
/// terminal. Every text line this command prints goes out this way.
macro_rules! say {
    ($($argument:tt)*) => {
        $crate::text::print_line($crate::text::Stream::Out, &format!($($argument)*))
    };
}

/// Prints one line on standard error, as [`eprintln!`] does, with the line
/// passed through [`shown`] the way [`say`] passes a line to standard
/// output. Progress, warnings, and the `error:` line all go out this way.
macro_rules! note {
    ($($argument:tt)*) => {
        $crate::text::print_line($crate::text::Stream::Err, &format!($($argument)*))
    };
}

/// The stream a line goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// Standard output, which holds the result of a command.
    Out,
    /// Standard error, which holds progress, warnings, and the `error:` line.
    Err,
}

/// Prints one escaped line on `stream`, which is what [`say`] and [`note`]
/// call. `print_json` in `analyze.rs` prints its one document through
/// [`block`] instead, because JSON escapes a control character itself.
pub fn print_line(stream: Stream, text: &str) {
    let stream = match stream {
        Stream::Out => OUT,
        Stream::Err => ERR,
    };
    write_line(stream, &shown(text));
}

/// Prints text on standard output as it stands, with a newline after it,
/// which is how the one JSON document a command prints goes out.
pub fn print_json_text(text: &str) {
    write_line(OUT, text);
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
