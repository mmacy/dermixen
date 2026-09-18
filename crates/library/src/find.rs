//! Fuzzy text search over the library, for resolving a line of a tracklisting to a file.

use crate::index::TrackRecord;

/// One candidate for a search, with how well it matched.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// The track.
    pub record: TrackRecord,
    /// How well the text matched, from zero to one. `docs/library.md`
    /// states the formula and what the thresholds mean.
    pub score: f64,
}

/// The share of the score that comes from the text's words being found in
/// the track. The rest comes from the track's words being found in the text,
/// which is what makes a line that names artist and title beat a line that
/// names the title alone.
const TEXT_SHARE: f64 = 0.75;

/// The shortest word that is still recognized with one letter wrong, missing,
/// or extra. Below this length a single letter is too much of the word to
/// forgive.
const SHORTEST_FUZZY_WORD: usize = 5;

/// Ranks the records that match some free text, such as a line from a
/// tracklisting, best first.
///
/// The text and each record's artist and title are compared as words, without
/// regard to case or punctuation, forgiving small misspellings, and a leading
/// track number or timestamp on the text is ignored. The score is three
/// quarters the share of the text's words found in the record plus one
/// quarter the share of the record's words found in the text, so a line that
/// names a track exactly scores one, a line that names only its title scores
/// high but below a line that names artist and title, and a line that names
/// nothing in the record scores zero. Records that score zero are left out,
/// the rest are sorted by descending score and then ascending path, and at
/// most `limit` are returned.
pub fn find(records: &[TrackRecord], text: &str, limit: usize) -> Vec<Match> {
    let wanted = words_of_text(text);
    if wanted.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut matches: Vec<Match> = records
        .iter()
        .filter_map(|record| {
            let held = words_of_record(record);
            if held.is_empty() {
                return None;
            }
            let found = share(&wanted, &held);
            let named = share(&held, &wanted);
            let score = TEXT_SHARE * found + (1.0 - TEXT_SHARE) * named;
            (score > 0.0).then(|| Match {
                record: record.clone(),
                score,
            })
        })
        .collect();
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.record.path.cmp(&right.record.path))
    });
    matches.truncate(limit);
    matches
}

/// The share of `these` words that are found among `those`, from zero to one.
fn share(these: &[String], those: &[String]) -> f64 {
    let found = these
        .iter()
        .filter(|word| those.iter().any(|other| alike(word, other)))
        .count();
    found as f64 / these.len() as f64
}

/// The words of a line of text, with a leading track number or timestamp
/// dropped.
fn words_of_text(text: &str) -> Vec<String> {
    let mut tokens = text.split_whitespace().peekable();
    if tokens.peek().is_some_and(|token| is_a_number(token)) {
        tokens.next();
    }
    tokens.filter_map(word_of).collect()
}

/// The words of a record: its artist and its title together.
fn words_of_record(record: &TrackRecord) -> Vec<String> {
    let named = [
        record.metadata.artist.as_deref(),
        record.metadata.title.as_deref(),
    ];
    named
        .into_iter()
        .flatten()
        .flat_map(|value| value.split_whitespace().filter_map(word_of))
        .collect()
}

/// One token as a word to compare: its letters and digits, in lower case, or
/// nothing when the token was punctuation alone, such as the hyphen between
/// an artist and a title.
fn word_of(token: &str) -> Option<String> {
    let word: String = token
        .chars()
        .filter(|letter| letter.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    (!word.is_empty()).then_some(word)
}

/// Whether a token is a track number or a timestamp, such as `03.`, `[12:34]`,
/// or `1:02:34`.
fn is_a_number(token: &str) -> bool {
    let digits = token.chars().filter(char::is_ascii_digit).count();
    let punctuation = |letter: char| ":.-[]()".contains(letter);
    digits > 0
        && token
            .chars()
            .all(|letter| letter.is_ascii_digit() || punctuation(letter))
}

/// Whether two words are the same word, forgiving one letter wrong, missing,
/// or extra in a word long enough to spare it.
fn alike(one: &str, other: &str) -> bool {
    if one == other {
        return true;
    }
    let longest = one.chars().count().max(other.chars().count());
    longest >= SHORTEST_FUZZY_WORD && one_letter_apart(one, other)
}

/// Whether one word becomes the other by changing, removing, or adding one
/// letter.
fn one_letter_apart(one: &str, other: &str) -> bool {
    let one: Vec<char> = one.chars().collect();
    let other: Vec<char> = other.chars().collect();
    let (shorter, longer) = if one.len() <= other.len() {
        (&one, &other)
    } else {
        (&other, &one)
    };
    match longer.len() - shorter.len() {
        // The same length: one letter may differ.
        0 => {
            shorter
                .iter()
                .zip(longer.iter())
                .filter(|(left, right)| left != right)
                .count()
                == 1
        }
        // One letter longer: the shorter word must be what is left of the
        // longer one after passing over a single letter.
        1 => {
            let mut matched = 0;
            let mut passed = false;
            for letter in longer {
                if matched < shorter.len() && shorter[matched] == *letter {
                    matched += 1;
                } else if passed {
                    return false;
                } else {
                    passed = true;
                }
            }
            matched == shorter.len()
        }
        _ => false,
    }
}
