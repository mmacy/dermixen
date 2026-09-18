//! The artist, title, and year of a track: read from its tags when it has
//! them, and guessed from its file name when it does not.

use std::path::Path;

use lofty::prelude::{Accessor, ItemKey, TaggedFileExt};
use serde::{Deserialize, Serialize};

/// Where a track's metadata came from, which is how much to trust it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataSource {
    /// Read from the file's own tags. Trusted.
    Tags,
    /// Guessed from the file's name and the folders it sits in. Low confidence:
    /// queries and the user interface treat it differently from real tags.
    Filename,
}

/// The artist, title, and year of a track, and where they came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// The artist, if known.
    pub artist: Option<String>,
    /// The title, if known.
    pub title: Option<String>,
    /// The year of release, if known.
    pub year: Option<u16>,
    /// Whether the year is an estimate rather than a year some source states.
    ///
    /// An estimate comes from the years of a release's other tracks, or of
    /// other work by the same artists. It places a track in an era without
    /// claiming to be the year itself, so a query that needs a year it can
    /// rely on can leave those tracks out.
    #[serde(default)]
    pub year_is_approximate: bool,
    /// Where these came from.
    pub source: MetadataSource,
}

/// Where a track's release identification came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseDataSource {
    /// Matched in the Discogs collection export, which lists the releases the
    /// user owns.
    DiscogsExport,
    /// Looked up through the Discogs API, which searches every release Discogs
    /// knows about and so can match a release the user does not own.
    DiscogsApi,
}

/// What Discogs says about the release a track came from.
///
/// Tags are not a source for any of these. This library's tags name the
/// pressing a file was ripped from, which for most of the catalog is a
/// reissue, so a tag names the wrong release. Every field stays empty until a
/// Discogs match is recorded, and [`Release::data_source`] names which lookup
/// answered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    /// The record label, if known.
    pub label: Option<String>,
    /// The label's catalog number, if known. This is the strongest identifier
    /// for matching a track to a Discogs release, because a label reuses a
    /// title but never a catalog number.
    pub catalog_number: Option<String>,
    /// The album or compilation title, if known.
    pub title: Option<String>,
    /// Which position the track takes on the release, counting from one, if
    /// known.
    pub track_number: Option<u16>,
    /// Which lookup answered, or `None` when no match has been recorded.
    pub data_source: Option<ReleaseDataSource>,
}

/// The reason a file's tags could not be read.
#[derive(Debug, thiserror::Error)]
#[error("cannot read the tags of {path}: {message}")]
pub struct TagError {
    /// The file.
    pub path: std::path::PathBuf,
    /// What went wrong.
    pub message: String,
}

/// Reads the artist, title, and year from a file's tags.
///
/// ID3 tags in MP3 files, Vorbis comments in FLAC files, and iTunes-style
/// tags in MP4 files are all read. Whitespace around a value is dropped, and
/// a value that is empty after that counts as absent. Returns `Ok(None)` when
/// the file has no tags or its tags hold neither an artist nor a title, and
/// an error when the file cannot be opened or its tags cannot be parsed.
pub fn read_tags(path: &Path) -> Result<Option<Metadata>, TagError> {
    let file = lofty::read_from_path(path).map_err(|problem| TagError {
        path: path.to_path_buf(),
        message: problem.to_string(),
    })?;
    // A file may hold more than one kind of tag. The primary one is the kind
    // the format is normally written with, and any other kind is better than
    // nothing.
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return Ok(None);
    };
    let artist = usable(tag.artist().as_deref());
    let title = usable(tag.title().as_deref());
    if artist.is_none() && title.is_none() {
        return Ok(None);
    }
    let year = tag
        .get_string(ItemKey::RecordingDate)
        .or_else(|| tag.get_string(ItemKey::Year))
        .and_then(year_of);
    Ok(Some(Metadata {
        artist,
        title,
        year,
        // Tags state a year rather than estimating one, wrong though that
        // year often is.
        year_is_approximate: false,
        source: MetadataSource::Tags,
    }))
}

/// A tag value with its surrounding whitespace dropped, or `None` when
/// nothing is left of it.
fn usable(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// The year at the front of a recording date.
///
/// Tags write the date as the year on its own, as `1996`, or as a full date
/// in one of several forms, such as `1996-05-02` or `19960502`. All three
/// give 1996. A date that does not begin with four digits, such as
/// `05/02/1996`, gives no year at all, because which four digits are the
/// year is then a guess.
fn year_of(date: &str) -> Option<u16> {
    let digits: String = date
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.len() < 4 {
        return None;
    }
    digits[..4].parse().ok()
}

/// Guesses the artist and title from a file's name and the folders it sits in.
///
/// `docs/library.md` states the rules. The result's source is always
/// [`MetadataSource::Filename`], its title is always present, and its year is
/// never present.
pub fn parse_filename(path: &Path) -> Metadata {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A name written with underscores in place of spaces reads as the same
    // name with spaces, which is the form the rest of the rules expect.
    let name = if stem.contains('_') && !stem.contains(' ') {
        stem.replace('_', " ")
    } else {
        stem.clone()
    };
    let name = without_leading_numbers(&name);

    let mut parts: Vec<String> = name.split(" - ").map(str::to_owned).collect();
    // A number part-way through the name marks where the track's own name
    // starts, so whatever comes before it is an album or a label prefix.
    if let Some(marker) = parts
        .iter()
        .rposition(|part| after_track_number(part).is_some())
    {
        parts.drain(..marker);
        let named = after_track_number(&parts[0]).unwrap_or_default().to_owned();
        if named.trim().is_empty() {
            parts.remove(0);
        } else {
            parts[0] = named;
        }
    }
    if parts.len() > 1 && is_various(&parts[0]) {
        parts.remove(0);
    }

    let (artist, title) = match parts.len() {
        0 => (None, String::new()),
        1 => (artist_from_folders(path), parts.remove(0)),
        _ => {
            let title = parts.pop().unwrap_or_default();
            (Some(parts.remove(0)), title)
        }
    };
    let title = without_trailing_catalog_number(&title);
    // Every name gives a title, so a name that the rules leave nothing of
    // falls back to the file name as it stands.
    let title = if title.is_empty() {
        stem.trim().to_owned()
    } else {
        title
    };
    Metadata {
        artist: usable(artist.as_deref()),
        title: Some(title),
        year: None,
        year_is_approximate: false,
        source: MetadataSource::Filename,
    }
}

/// The name with every leading track number and catalog number dropped.
fn without_leading_numbers(name: &str) -> String {
    let mut rest = name;
    loop {
        if let Some(shorter) = after_track_number(rest) {
            rest = shorter;
            continue;
        }
        if let Some(shorter) = after_catalog_number(rest) {
            rest = shorter;
            continue;
        }
        return rest.to_owned();
    }
}

/// What follows a leading track number, or `None` when the text does not
/// start with one.
///
/// A track number is one to three digits followed by a space, a dot, or a
/// hyphen with a space on each side, or two digits in parentheses followed by
/// a hyphen, which may have spaces around it. Three digits are a disc number
/// and a track number together, as in `203`. Text that is nothing but digits
/// is a track number on its own, and nothing follows it.
fn after_track_number(text: &str) -> Option<&str> {
    let parenthesized = text
        .strip_prefix('(')
        .map(split_digits)
        .filter(|(digits, _rest)| digits.len() == 2)
        .and_then(|(_digits, rest)| rest.strip_prefix(')'))
        .map(str::trim_start)
        .and_then(|rest| rest.strip_prefix('-'));
    if let Some(rest) = parenthesized {
        return Some(rest.trim_start());
    }
    let (digits, rest) = split_digits(text);
    if digits.is_empty() || digits.len() > 3 {
        return None;
    }
    if rest.is_empty() {
        return Some(rest);
    }
    for separator in [" - ", " ", "."] {
        if let Some(rest) = rest.strip_prefix(separator) {
            return Some(rest.trim_start());
        }
    }
    None
}

/// What follows a leading catalog number, or `None` when the text does not
/// start with one.
///
/// A catalog number is two or more capital letters, then digits, then any run
/// of capitals, digits, and hyphens, followed by a space, as in `AFR010` or
/// `KR005-B1`.
fn after_catalog_number(text: &str) -> Option<&str> {
    let letters = text.chars().take_while(char::is_ascii_uppercase).count();
    if letters < 2 {
        return None;
    }
    let (digits, rest) = split_digits(&text[letters..]);
    if digits.is_empty() {
        return None;
    }
    let extra = rest
        .chars()
        .take_while(|letter| {
            letter.is_ascii_uppercase() || letter.is_ascii_digit() || *letter == '-'
        })
        .count();
    rest[extra..].strip_prefix(' ').map(str::trim_start)
}

/// The leading run of digits and whatever follows it.
fn split_digits(text: &str) -> (&str, &str) {
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    text.split_at(digits)
}

/// Whether a name stands for a compilation of several artists rather than for
/// one artist.
fn is_various(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "va" | "various" | "various artists"
    )
}

/// Whether text in square brackets at the end of a title reads as a catalog
/// number: capital letters, digits, hyphens, dots, and underscores, with at
/// least one digit among them, as in `AQUA12-B`.
fn is_catalog_number(text: &str) -> bool {
    !text.is_empty()
        && text.chars().any(|letter| letter.is_ascii_digit())
        && text.chars().all(|letter| {
            letter.is_ascii_uppercase()
                || letter.is_ascii_digit()
                || letter == '-'
                || letter == '.'
                || letter == '_'
        })
}

/// The title with a trailing catalog number in square brackets dropped and
/// its whitespace trimmed.
fn without_trailing_catalog_number(title: &str) -> String {
    let title = title.trim();
    let bracketed = title
        .strip_suffix(']')
        .and_then(|rest| rest.rfind('[').map(|open| (open, &rest[open + 1..])))
        .filter(|(_open, inside)| is_catalog_number(inside));
    match bracketed {
        Some((open, _inside)) => title[..open].trim_end().to_owned(),
        None => title.to_owned(),
    }
}

/// The artist named by the nearest enclosing folder whose name reads
/// `Artist - Album`, which is where a file whose own name gives only a title
/// gets its artist. A folder that names a compilation of several artists
/// gives no artist at all.
fn artist_from_folders(path: &Path) -> Option<String> {
    let mut folder = path.parent();
    while let Some(here) = folder {
        let named = here
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .and_then(|name| {
                name.split_once(" - ")
                    .map(|(artist, _album)| artist.trim().to_owned())
            });
        // A folder such as `CD1` or `wav` has no artist in its name, so the
        // search goes on up.
        if let Some(artist) = named {
            if artist.is_empty() || is_various(&artist) {
                return None;
            }
            return Some(artist);
        }
        folder = here.parent();
    }
    None
}

/// The metadata of a file: its tags when it has usable ones, and otherwise
/// what its name says.
///
/// A file whose tags cannot be read falls back to its name like a file with
/// no tags, because the name is still there to be read and a reader can tell
/// from the source field how much to trust the result.
pub fn metadata_of(path: &Path) -> Metadata {
    match read_tags(path) {
        Ok(Some(metadata)) => metadata,
        Ok(None) | Err(_) => parse_filename(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_gives_its_year_when_it_begins_with_one() {
        assert_eq!(year_of("1996"), Some(1996));
        assert_eq!(year_of("1996-05-02"), Some(1996));
        assert_eq!(year_of("19960502"), Some(1996));
        assert_eq!(year_of(" 1996 "), Some(1996));
        assert_eq!(year_of("05/02/1996"), None);
        assert_eq!(year_of(""), None);
    }

    #[test]
    fn a_track_number_in_parentheses_may_be_spaced_from_its_hyphen() {
        let flush = parse_filename(Path::new("/music/(01)-Foo.mp3"));
        assert_eq!(flush.title.as_deref(), Some("Foo"));
        let spaced = parse_filename(Path::new("/music/(01) - Foo.mp3"));
        assert_eq!(spaced.title.as_deref(), Some("Foo"));
        assert_eq!(spaced.artist, None);
    }
}
