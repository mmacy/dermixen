//! The phrase scoreboard: how each phrase analyzer's bar lines, phrase
//! starts, and section changes compare with the labeled ones, in one table.
//!
//! Ground truth is the same directory of `name.anchors` annotations the
//! anchor scoreboard reads. The labeled grid's beat zero starts a bar; the
//! `phrase` labels are bars a transition was aligned to; the `section`
//! labels are bars at which the arrangement changes. `docs/ground-truth.md`
//! describes the labels and the metrics.

use std::collections::BTreeMap;
use std::time::Instant;

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, Seconds};

use crate::phrases::{PhraseAnalysis, PhraseAnalyzer};
use crate::scoreboard::{BEAT_TOLERANCE, ScoreboardError};
use crate::structure_scoreboard::{
    AnchorTruth, BAR_TOLERANCE, ConfidenceSplit, Label, SURE_CONFIDENCE, Source, error_in_beats,
};

/// The phrase lengths, in bars, the scoreboard reports a share for.
pub const PHRASE_LENGTHS: [u32; 3] = [8, 16, 32];

/// How the scoreboard hands each analyzer its grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridHandling {
    /// The labeled grid as it is, with beat zero on a bar line.
    AsLabeled,
    /// The labeled grid with beat zero moved later by as many beats as the
    /// track's index in the run, modulo four, so that beat zero starts a bar
    /// on only every fourth track. An analyzer that finds the bar lines in
    /// the audio scores the same either way; one that assumes beat zero
    /// starts a bar is right on a quarter of the tracks.
    Shifted,
}

/// How one analyzer did on one phrase label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhraseLabelScore {
    /// The label's source.
    pub source: Source,
    /// Whether the label lies on one of the analyzer's bar lines.
    pub on_bar: bool,
    /// For each length in [`PHRASE_LENGTHS`], whether the label lies on a
    /// phrase start of at least that length.
    pub on_phrase: [bool; PHRASE_LENGTHS.len()],
    /// The distance in bars from the label to the nearest sixteen-bar
    /// phrase start, positive when the start is later than the label.
    pub bars_to_phrase: f64,
}

/// How one analyzer did on one section label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SectionLabelScore {
    /// The label's source.
    pub source: Source,
    /// The error in beats from the label to the nearest section change the
    /// analyzer found, positive when the change is later, or `None` when
    /// the analyzer found no section changes.
    pub error: Option<f64>,
}

/// How one analyzer did on one track.
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseTrackScore {
    /// The track's name.
    pub name: String,
    /// Whether the analyzer's bar lines are the labeled ones, or `None` if
    /// the analyzer failed.
    pub downbeat: Option<bool>,
    /// The beat of the labeled grid, from zero to three, the analyzer took
    /// as starting a bar, or `None` if it failed.
    pub downbeat_found: Option<u32>,
    /// One score per phrase label, in time order, empty if the analyzer failed.
    pub phrases: Vec<PhraseLabelScore>,
    /// One score per section label, in time order, empty if the analyzer failed.
    pub sections: Vec<SectionLabelScore>,
    /// The confidence the analyzer reported, or `None` if it failed.
    pub confidence: Option<f64>,
    /// How long the analyzer took on this track.
    pub seconds: f64,
}

impl PhraseTrackScore {
    /// Whether the analyzer got the track right: its bar lines are the
    /// labeled ones and every phrase label lies on one of its sixteen-bar
    /// phrase starts. `None` for a track with no phrase labels or no result.
    pub fn hit(&self) -> Option<bool> {
        let downbeat = self.downbeat?;
        if self.phrases.is_empty() {
            return None;
        }
        Some(downbeat && self.phrases.iter().all(|score| score.on_phrase[1]))
    }
}

/// One analyzer's line on the phrase scoreboard, for one source of labels
/// or for all of them together.
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseRow {
    /// The analyzer's name.
    pub analyzer: String,
    /// The source the row is restricted to, or `None` for every source.
    pub source: Option<Source>,
    /// How many tracks the analyzer was run on.
    pub tracks: usize,
    /// How many tracks it failed on.
    pub failures: usize,
    /// The share of tracks whose bar lines are the labeled ones, over the
    /// tracks that got a result. Only the row for every source shows it,
    /// because the grid is the track's, not a label's.
    pub downbeat: Option<f64>,
    /// How many phrase labels were scored.
    pub phrase_labels: usize,
    /// The share of phrase labels on one of the analyzer's bar lines.
    pub on_bar: Option<f64>,
    /// For each length in [`PHRASE_LENGTHS`], the share of phrase labels on
    /// a phrase start of at least that length.
    pub on_phrase: [Option<f64>; PHRASE_LENGTHS.len()],
    /// The median distance in bars from a phrase label to the nearest
    /// sixteen-bar phrase start.
    pub median_bars: Option<f64>,
    /// How many section labels were scored.
    pub section_labels: usize,
    /// The share of section labels within one bar of a section change.
    pub sections_within_bar: Option<f64>,
    /// How the tracks the analyzer was sure and unsure about divide into
    /// hits and misses, as [`PhraseTrackScore::hit`] defines a hit.
    pub confidence_split: ConfidenceSplit,
    /// The mean seconds per track.
    pub seconds_per_track: f64,
}

/// The phrase scoreboard for one run.
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseReport {
    /// How the grids were handed to the analyzers.
    pub handling: GridHandling,
    /// One row per analyzer over every source, then one row per analyzer
    /// and source that has at least one label, in analyzer order.
    pub rows: Vec<PhraseRow>,
    /// Every analyzer's score on every track, keyed by analyzer name, in the
    /// order the ground truth was given.
    pub per_track: BTreeMap<String, Vec<PhraseTrackScore>>,
}

/// The column headings of the table.
const COLUMNS: [&str; 18] = [
    "analyzer",
    "labels",
    "tracks",
    "failures",
    "downbeat %",
    "phrase n",
    "bar %",
    "8 bars %",
    "16 bars %",
    "32 bars %",
    "med bars",
    "section n",
    "bar %",
    "sure hits",
    "sure misses",
    "unsure hits",
    "unsure misses",
    "s/track",
];

/// A share as a percentage with one decimal, or a dash when nothing was scored.
fn percent(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{:.1}", value * 100.0),
        None => "-".to_owned(),
    }
}

/// The median of the values, or `None` if there are none.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    })
}

/// The share of the answers that are true, or `None` if there are none.
fn share(answers: impl Iterator<Item = bool>) -> Option<f64> {
    let mut total = 0usize;
    let mut count = 0usize;
    for answer in answers {
        total += usize::from(answer);
        count += 1;
    }
    (count > 0).then(|| total as f64 / count as f64)
}

impl PhraseReport {
    /// The report as a text table for a terminal: one header line, then one
    /// line per row. The `labels` column says which source the row counts,
    /// or `all`.
    pub fn table(&self) -> String {
        let mut cells: Vec<[String; COLUMNS.len()]> = Vec::with_capacity(self.rows.len() + 1);
        cells.push(COLUMNS.map(str::to_owned));
        for row in &self.rows {
            let line: [String; COLUMNS.len()] = [
                row.analyzer.clone(),
                row.source.map_or("all", Source::name).to_owned(),
                row.tracks.to_string(),
                row.failures.to_string(),
                percent(row.downbeat),
                row.phrase_labels.to_string(),
                percent(row.on_bar),
                percent(row.on_phrase[0]),
                percent(row.on_phrase[1]),
                percent(row.on_phrase[2]),
                match row.median_bars {
                    Some(value) => format!("{value:.1}"),
                    None => "-".to_owned(),
                },
                row.section_labels.to_string(),
                percent(row.sections_within_bar),
                row.confidence_split.sure_hits.to_string(),
                row.confidence_split.sure_misses.to_string(),
                row.confidence_split.unsure_hits.to_string(),
                row.confidence_split.unsure_misses.to_string(),
                format!("{:.3}", row.seconds_per_track),
            ];
            cells.push(line);
        }
        let mut widths = [0usize; COLUMNS.len()];
        for line in &cells {
            for (width, cell) in widths.iter_mut().zip(line) {
                *width = (*width).max(cell.chars().count());
            }
        }
        let mut table = String::new();
        for line in &cells {
            let mut text = String::new();
            for (column, (cell, width)) in line.iter().zip(widths).enumerate() {
                if column > 0 {
                    text.push_str("  ");
                }
                if column < 2 {
                    text.push_str(&format!("{cell:<width$}"));
                } else {
                    text.push_str(&format!("{cell:>width$}"));
                }
            }
            table.push_str(text.trim_end());
            table.push('\n');
        }
        table
    }

    /// Every track's result for one analyzer as text, one line per track:
    /// the name, the beat the analyzer took as the downbeat and whether that
    /// is the labeled one, then each phrase label's distance in bars to the
    /// nearest sixteen-bar phrase start with the longest phrase the label
    /// sits at the start of, each section label's error in beats, and the
    /// confidence. Returns an empty string for an analyzer not in the report.
    pub fn details(&self, analyzer: &str) -> String {
        let Some(scores) = self.per_track.get(analyzer) else {
            return String::new();
        };
        let mut text = String::new();
        for score in scores {
            let Some(confidence) = score.confidence else {
                text.push_str(&format!("{}: failed\n", score.name));
                continue;
            };
            let downbeat = match (score.downbeat_found, score.downbeat) {
                (Some(found), Some(true)) => format!("downbeat at beat {found}, right"),
                (Some(found), _) => format!("downbeat at beat {found}, wrong"),
                (None, _) => "downbeat unknown".to_owned(),
            };
            let phrases: Vec<String> = score
                .phrases
                .iter()
                .map(|label| {
                    let longest = PHRASE_LENGTHS
                        .iter()
                        .zip(label.on_phrase)
                        .filter(|(_, on)| *on)
                        .map(|(bars, _)| *bars)
                        .max();
                    let at = match longest {
                        Some(bars) => format!("starts {bars} bars"),
                        None if label.on_bar => "on a bar".to_owned(),
                        None => "off the bars".to_owned(),
                    };
                    format!(
                        "{:+.1} bars ({}, {})",
                        label.bars_to_phrase,
                        at,
                        label.source.name()
                    )
                })
                .collect();
            let sections: Vec<String> = score
                .sections
                .iter()
                .map(|label| match label.error {
                    Some(error) => format!("{:+.1} beats ({})", error, label.source.name()),
                    None => format!("none found ({})", label.source.name()),
                })
                .collect();
            text.push_str(&format!(
                "{}: {downbeat}, phrases [{}], sections [{}], confidence {confidence:.2}\n",
                score.name,
                phrases.join(", "),
                sections.join(", "),
            ));
        }
        text
    }
}

/// The grid an analyzer is handed for the track at `index`, under the
/// given handling.
pub fn handed_grid(labeled: &BeatGrid, index: usize, handling: GridHandling) -> BeatGrid {
    match handling {
        GridHandling::AsLabeled => *labeled,
        GridHandling::Shifted => {
            let shift = (index % BEATS_PER_BAR as usize) as f64;
            BeatGrid {
                first_beat: labeled.position_of(Beats(shift)),
                bpm: labeled.bpm,
            }
        }
    }
}

/// Whether a time lies within the beat tolerance of any of the times.
fn on_any(at: Seconds, times: &[Seconds]) -> bool {
    times
        .iter()
        .any(|time| (time.0 - at.0).abs() <= BEAT_TOLERANCE.0)
}

/// Scores one analysis of one track against the track's labels. The
/// analysis is in beats of `handed`, the grid the analyzer was given.
fn score_track(
    track: &AnchorTruth,
    handed: &BeatGrid,
    analysis: &PhraseAnalysis,
    seconds: f64,
) -> PhraseTrackScore {
    let bpm = track.grid.bpm;
    let bar = f64::from(BEATS_PER_BAR);
    // The analyzer's downbeat is a beat of the grid it was handed; on the
    // labeled grid that beat is a whole number of beats along, and it
    // starts a bar when that number is a multiple of four.
    let downbeat_time = handed.time_of(Beats(f64::from(analysis.downbeat)));
    let downbeat_on_labeled = track.grid.beat_at(downbeat_time).0;
    let downbeat_found = downbeat_on_labeled.round().rem_euclid(bar) as u32;
    let downbeat = downbeat_found == 0;

    let phrase_times: Vec<(Seconds, u32)> = analysis
        .phrases
        .iter()
        .map(|start| (handed.time_of(start.at), start.bars))
        .collect();
    let sixteen: Vec<Seconds> = phrase_times
        .iter()
        .filter(|(_, bars)| *bars >= 16)
        .map(|(at, _)| *at)
        .collect();
    let phrases = track
        .phrases
        .iter()
        .map(|label| {
            // A label is on a bar line when it is a whole number of bars
            // from the analyzer's downbeat.
            let beats_from_downbeat = (label.at - downbeat_time).beats_at(bpm).0;
            let bars_from_downbeat = beats_from_downbeat / bar;
            let on_bar =
                (bars_from_downbeat - bars_from_downbeat.round()).abs() * bar * bpm.beat_period().0
                    <= BEAT_TOLERANCE.0;
            let mut on_phrase = [false; PHRASE_LENGTHS.len()];
            for (slot, length) in PHRASE_LENGTHS.iter().enumerate() {
                let starts: Vec<Seconds> = phrase_times
                    .iter()
                    .filter(|(_, bars)| bars >= length)
                    .map(|(at, _)| *at)
                    .collect();
                on_phrase[slot] = on_any(label.at, &starts);
            }
            let bars_to_phrase = sixteen
                .iter()
                .map(|start| error_in_beats(*start, label.at, bpm) / bar)
                .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                .unwrap_or(f64::INFINITY);
            PhraseLabelScore {
                source: label.source,
                on_bar,
                on_phrase,
                bars_to_phrase,
            }
        })
        .collect();

    let section_times: Vec<Seconds> = analysis
        .sections
        .iter()
        .map(|at| handed.time_of(*at))
        .collect();
    let sections = track
        .sections
        .iter()
        .map(|label: &Label| SectionLabelScore {
            source: label.source,
            error: section_times
                .iter()
                .map(|at| error_in_beats(*at, label.at, bpm))
                .min_by(|a, b| a.abs().total_cmp(&b.abs())),
        })
        .collect();

    PhraseTrackScore {
        name: track.name.clone(),
        downbeat: Some(downbeat),
        downbeat_found: Some(downbeat_found),
        phrases,
        sections,
        confidence: Some(analysis.confidence),
        seconds,
    }
}

/// Summarizes one analyzer's scores, over every source or over one.
fn summarize(analyzer: &str, source: Option<Source>, scores: &[PhraseTrackScore]) -> PhraseRow {
    let wanted = |label_source: Source| source.is_none_or(|wanted| label_source == wanted);
    let phrases: Vec<&PhraseLabelScore> = scores
        .iter()
        .flat_map(|score| score.phrases.iter())
        .filter(|label| wanted(label.source))
        .collect();
    let sections: Vec<&SectionLabelScore> = scores
        .iter()
        .flat_map(|score| score.sections.iter())
        .filter(|label| wanted(label.source))
        .collect();
    let mut on_phrase = [None; PHRASE_LENGTHS.len()];
    for (slot, entry) in on_phrase.iter_mut().enumerate() {
        *entry = share(phrases.iter().map(|label| label.on_phrase[slot]));
    }
    let distances: Vec<f64> = phrases
        .iter()
        .map(|label| label.bars_to_phrase.abs())
        .filter(|distance| distance.is_finite())
        .collect();
    let mut split = ConfidenceSplit::default();
    if source.is_none() {
        for score in scores {
            let (Some(confidence), Some(hit)) = (score.confidence, score.hit()) else {
                continue;
            };
            let count = match (confidence >= SURE_CONFIDENCE, hit) {
                (true, true) => &mut split.sure_hits,
                (true, false) => &mut split.sure_misses,
                (false, true) => &mut split.unsure_hits,
                (false, false) => &mut split.unsure_misses,
            };
            *count += 1;
        }
    }
    let total: f64 = scores.iter().map(|score| score.seconds).sum();
    PhraseRow {
        analyzer: analyzer.to_owned(),
        source,
        tracks: scores.len(),
        failures: scores
            .iter()
            .filter(|score| score.confidence.is_none())
            .count(),
        downbeat: if source.is_none() {
            share(scores.iter().filter_map(|score| score.downbeat))
        } else {
            None
        },
        phrase_labels: phrases.len(),
        on_bar: share(phrases.iter().map(|label| label.on_bar)),
        on_phrase,
        median_bars: median(&distances),
        section_labels: sections.len(),
        sections_within_bar: share(sections.iter().map(|label| {
            label
                .error
                .is_some_and(|error| error.abs() <= BAR_TOLERANCE)
        })),
        confidence_split: split,
        seconds_per_track: if scores.is_empty() {
            0.0
        } else {
            total / scores.len() as f64
        },
    }
}

/// Runs every phrase analyzer over every ground-truth track and scores it.
///
/// Each audio file is decoded once and handed to every analyzer along with
/// the track's labeled grid, handled as `handling` says. An analyzer's bar
/// lines, phrase starts, and section changes are turned into times through
/// the grid it was handed and compared with the labels in seconds, so the
/// scores do not depend on the handling for an analyzer that reads the bar
/// lines off the audio. An analyzer that returns an error on a track is
/// counted as a failure on that track and the run continues.
pub fn run_phrases(
    truth: &[AnchorTruth],
    analyzers: &[&dyn PhraseAnalyzer],
    handling: GridHandling,
) -> Result<PhraseReport, ScoreboardError> {
    let mut per_track: BTreeMap<String, Vec<PhraseTrackScore>> = BTreeMap::new();
    for (index, track) in truth.iter().enumerate() {
        let decoded =
            dermixen_media::decode(&track.audio).map_err(|error| ScoreboardError::Decode {
                path: track.audio.clone(),
                message: error.to_string(),
            })?;
        let handed = handed_grid(&track.grid, index, handling);
        for analyzer in analyzers {
            let started = Instant::now();
            let result = analyzer.analyze(&decoded.audio, &handed);
            let seconds = started.elapsed().as_secs_f64();
            let score = match result {
                Ok(analysis) => score_track(track, &handed, &analysis, seconds),
                Err(_) => PhraseTrackScore {
                    name: track.name.clone(),
                    downbeat: None,
                    downbeat_found: None,
                    phrases: Vec::new(),
                    sections: Vec::new(),
                    confidence: None,
                    seconds,
                },
            };
            per_track
                .entry(analyzer.name().to_owned())
                .or_default()
                .push(score);
        }
    }

    let mut rows = Vec::new();
    for analyzer in analyzers {
        let scores = per_track
            .get(analyzer.name())
            .map(Vec::as_slice)
            .unwrap_or_default();
        rows.push(summarize(analyzer.name(), None, scores));
        for source in Source::ALL {
            let row = summarize(analyzer.name(), Some(source), scores);
            if row.phrase_labels + row.section_labels > 0 {
                rows.push(row);
            }
        }
    }
    Ok(PhraseReport {
        handling,
        rows,
        per_track,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrases::PhraseStart;
    use dermixen_core::Bpm;
    use std::path::PathBuf;

    /// A track at 120 beats per minute with beat zero at one second, a
    /// phrase label at bar 16 and one at bar 24, and a section label at bar 16.
    fn a_track() -> AnchorTruth {
        let grid = BeatGrid {
            first_beat: Seconds(1.0).to_samples(),
            bpm: Bpm(120.0),
        };
        let label = |bars: f64, source| Label {
            at: grid.time_of(Beats::from_bars(bars)),
            source,
        };
        AnchorTruth {
            name: "a track".to_owned(),
            audio: PathBuf::new(),
            grid,
            begins: None,
            ends: None,
            intro: None,
            outro: None,
            phrases: vec![label(16.0, Source::Ear), label(24.0, Source::MixMeister)],
            sections: vec![label(16.0, Source::Ear)],
        }
    }

    #[test]
    fn a_shifted_grid_moves_beat_zero_by_the_track_index() {
        let labeled = a_track().grid;
        assert_eq!(handed_grid(&labeled, 0, GridHandling::Shifted), labeled);
        let shifted = handed_grid(&labeled, 3, GridHandling::Shifted);
        assert_eq!(shifted.first_beat, Seconds(2.5).to_samples());
        assert_eq!(handed_grid(&labeled, 4, GridHandling::Shifted), labeled);
        assert_eq!(handed_grid(&labeled, 7, GridHandling::AsLabeled), labeled);
    }

    #[test]
    fn labels_are_scored_in_seconds_whatever_grid_was_handed() {
        let track = a_track();
        // The analyzer was handed the grid shifted by three beats, found
        // that the bars start one beat along (which is labeled beat 4, a
        // bar line), and counted sixteen-bar phrases from labeled bar 8,
        // so labeled bar 24 starts a sixteen-bar phrase and bar 16 an
        // eight-bar one.
        let handed = handed_grid(&track.grid, 3, GridHandling::Shifted);
        let bar = |bars: f64| Beats::from_bars(bars) + Beats::ONE;
        let analysis = PhraseAnalysis {
            downbeat: 1,
            phrases: vec![
                PhraseStart {
                    at: bar(7.0),
                    bars: 16,
                },
                PhraseStart {
                    at: bar(15.0),
                    bars: 8,
                },
                PhraseStart {
                    at: bar(23.0),
                    bars: 16,
                },
            ],
            sections: vec![bar(15.0)],
            confidence: 0.8,
        };
        let score = score_track(&track, &handed, &analysis, 0.0);
        assert_eq!(score.downbeat, Some(true));
        assert_eq!(score.downbeat_found, Some(0));
        assert_eq!(score.phrases.len(), 2);
        assert!(score.phrases[0].on_bar);
        assert_eq!(score.phrases[0].on_phrase, [true, false, false]);
        // Bars 8 and 24 both start sixteen-bar phrases, eight bars away.
        assert_eq!(score.phrases[0].bars_to_phrase.abs(), 8.0);
        assert_eq!(score.phrases[1].on_phrase, [true, true, false]);
        assert_eq!(score.phrases[1].bars_to_phrase, 0.0);
        assert_eq!(score.sections[0].error, Some(0.0));
        assert_eq!(score.hit(), Some(false));
    }

    #[test]
    fn a_wrong_downbeat_puts_the_labels_off_the_bars() {
        let track = a_track();
        let handed = track.grid;
        let analysis = PhraseAnalysis {
            downbeat: 2,
            phrases: vec![PhraseStart {
                at: Beats::from_bars(16.0) + Beats(2.0),
                bars: 32,
            }],
            sections: Vec::new(),
            confidence: 0.3,
        };
        let score = score_track(&track, &handed, &analysis, 0.0);
        assert_eq!(score.downbeat, Some(false));
        assert_eq!(score.downbeat_found, Some(2));
        assert!(!score.phrases[0].on_bar);
        assert_eq!(score.phrases[0].on_phrase, [false; 3]);
        assert_eq!(score.sections[0].error, None);
        assert_eq!(score.hit(), Some(false));
        let row = summarize("test", None, &[score]);
        assert_eq!(row.downbeat, Some(0.0));
        assert_eq!(row.on_bar, Some(0.0));
        assert_eq!(row.sections_within_bar, Some(0.0));
        assert_eq!(row.confidence_split.unsure_misses, 1);
    }
}
