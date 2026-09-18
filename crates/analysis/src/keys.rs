//! The key half of the scoreboard: how a key estimate is scored against an
//! annotation, and the report that puts every key analyzer in one table.

use std::time::Instant;

use crate::analyzer::{Key, KeyAnalyzer, Mode};
use crate::scoreboard::{
    GroundTruth, ScoreboardError, decode_error, mean, percent, render_table, share,
};

/// How much credit an estimated key earns against the annotated key.
///
/// The same key earns one point. A key a perfect fifth away in either
/// direction with the same mode, which is one step around the Camelot wheel,
/// earns half a point. The relative major or minor, which shares the Camelot
/// number, earns three tenths. The parallel major or minor, the same tonic in
/// the other mode, earns two tenths. Anything else earns nothing. These are
/// the weights the music information retrieval community uses, except that
/// the community's `mir_eval` program credits only the fifth above the
/// annotated key, so a weighted score from here can sit slightly above one
/// computed there on the same estimates.
pub fn key_score(estimate: Key, reference: Key) -> f64 {
    if estimate == reference {
        return 1.0;
    }
    let apart = (estimate.tonic as i32 - reference.tonic as i32).rem_euclid(12);
    if estimate.mode == reference.mode && (apart == 5 || apart == 7) {
        return 0.5;
    }
    if estimate.mode != reference.mode {
        if estimate.tonic == reference.tonic {
            return 0.2;
        }
        // The relative major sits three semitones above its relative minor,
        // so the relative key of the reference is three semitones up from
        // its tonic when the reference is minor, or three semitones down
        // when the reference is major.
        let relative = if reference.mode == Mode::Minor {
            (reference.tonic as i32 + 3).rem_euclid(12)
        } else {
            (reference.tonic as i32 - 3).rem_euclid(12)
        };
        if estimate.tonic as i32 == relative {
            return 0.3;
        }
    }
    0.0
}

/// How one key analyzer did on one track.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyTrackScore {
    /// The track's name.
    pub name: String,
    /// The key the analyzer reported, or `None` if it failed.
    pub key: Option<Key>,
    /// The credit earned by [`key_score`], or `None` without a key annotation or a result.
    pub score: Option<f64>,
    /// How long the analyzer took on this track.
    pub seconds: f64,
}

/// One key analyzer's line on the scoreboard.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyRow {
    /// The analyzer's name.
    pub analyzer: String,
    /// How many tracks it was run on.
    pub tracks: usize,
    /// How many tracks it failed on.
    pub failures: usize,
    /// The share of the key-annotated tracks the analyzer answered on whose key was exactly right, from zero to one, or `None` if there were none. A track the analyzer failed on counts toward the failures and not here.
    pub exact: Option<f64>,
    /// The mean of [`key_score`] over the key-annotated tracks the analyzer answered on, or `None` if there were none.
    pub weighted: Option<f64>,
    /// The mean seconds per track.
    pub seconds_per_track: f64,
    /// The score on every track, in the order the ground truth was given.
    pub per_track: Vec<KeyTrackScore>,
}

/// The key scoreboard for one run.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyReport {
    /// One row per analyzer, in the order the analyzers were given.
    pub rows: Vec<KeyRow>,
}

/// The column headings of the key table, in order.
const KEY_COLUMNS: [&str; 6] = [
    "analyzer",
    "tracks",
    "failures",
    "exact %",
    "weighted %",
    "s/track",
];

impl KeyReport {
    /// The report as a text table for a terminal, in the same shape as the
    /// beat scoreboard's table: one header line, then one line per analyzer
    /// with its name, track count, failures, the exact and weighted scores as
    /// percentages, and seconds per track. A score with no annotated tracks
    /// is shown as `-`.
    pub fn table(&self) -> String {
        let mut cells: Vec<[String; KEY_COLUMNS.len()]> = Vec::with_capacity(self.rows.len() + 1);
        cells.push(KEY_COLUMNS.map(str::to_owned));
        for row in &self.rows {
            cells.push([
                row.analyzer.clone(),
                row.tracks.to_string(),
                row.failures.to_string(),
                percent(row.exact),
                percent(row.weighted),
                format!("{:.3}", row.seconds_per_track),
            ]);
        }
        render_table(&cells)
    }
}

/// Runs every key analyzer over every ground-truth track and scores it.
///
/// Each audio file is decoded once and handed to every analyzer. An analyzer
/// that returns an error on a track is counted as a failure on that track
/// and the run continues. Tracks with no key annotation are still run, so
/// the time per track is measured, but they contribute no score.
pub fn run_keys(
    truth: &[GroundTruth],
    analyzers: &[&dyn KeyAnalyzer],
) -> Result<KeyReport, ScoreboardError> {
    let mut rows: Vec<KeyRow> = analyzers
        .iter()
        .map(|analyzer| KeyRow {
            analyzer: analyzer.name().to_owned(),
            tracks: truth.len(),
            failures: 0,
            exact: None,
            weighted: None,
            seconds_per_track: 0.0,
            per_track: Vec::with_capacity(truth.len()),
        })
        .collect();

    for track in truth {
        let decoded = dermixen_media::decode(&track.audio)
            .map_err(|error| decode_error(&track.audio, error.to_string()))?;
        for (row, analyzer) in rows.iter_mut().zip(analyzers) {
            let started = Instant::now();
            let result = analyzer.analyze(&decoded.audio);
            let seconds = started.elapsed().as_secs_f64();
            let score = match result {
                Ok(analysis) => KeyTrackScore {
                    name: track.name.clone(),
                    key: Some(analysis.key),
                    score: track
                        .key
                        .map(|reference| key_score(analysis.key, reference)),
                    seconds,
                },
                Err(_) => {
                    row.failures += 1;
                    KeyTrackScore {
                        name: track.name.clone(),
                        key: None,
                        score: None,
                        seconds,
                    }
                }
            };
            row.per_track.push(score);
        }
    }

    for row in &mut rows {
        // `key_score` returns exactly one only for the annotated key itself,
        // so comparing the score with one is an exact test.
        row.exact = share(
            row.per_track
                .iter()
                .filter_map(|track| track.score.map(|score| score == 1.0)),
        );
        row.weighted = mean(row.per_track.iter().filter_map(|track| track.score));
        let total: f64 = row.per_track.iter().map(|track| track.seconds).sum();
        row.seconds_per_track = if row.tracks == 0 {
            0.0
        } else {
            total / row.tracks as f64
        };
    }
    Ok(KeyReport { rows })
}
