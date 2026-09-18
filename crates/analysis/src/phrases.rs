//! Where a track's bars and phrases begin and where its arrangement changes.
//!
//! A beat analyzer finds the beats and a beat zero. To place a transition
//! well, the app also needs to know which beat of that grid starts a bar,
//! which bars start the eight-, sixteen-, and thirty-two-bar phrases the
//! arrangement is built from, and where the arrangement changes: the kick
//! starting or stopping, a breakdown beginning or ending. A phrase analyzer
//! answers those three questions for a track whose beats are already known.
//! It trusts the beats and the tempo of the grid it is given, but not the
//! assumption that beat zero starts a bar; it finds the bar lines itself.

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats};
use dermixen_media::Audio;

use crate::analyzer::AnalysisError;

/// The longest phrase length a phrase start is labeled with, in bars.
pub const LONGEST_PHRASE_BARS: u32 = 32;

/// A bar that starts a phrase, and the longest phrase that starts there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhraseStart {
    /// The bar's first beat, a whole beat of the grid the analyzer was given.
    pub at: Beats,
    /// The longest phrase that begins at this bar, in bars: eight, sixteen,
    /// or thirty-two. A bar that starts a thirty-two-bar phrase also starts
    /// a sixteen-bar and an eight-bar one.
    pub bars: u32,
}

/// What a phrase analyzer found in one track.
#[derive(Debug, Clone, PartialEq)]
pub struct PhraseAnalysis {
    /// Which beat of the grid starts a bar, from zero to three: bars begin
    /// at every beat that is this many beats past a whole multiple of four.
    pub downbeat: u32,
    /// Every bar that starts a phrase, in time order, with the longest phrase
    /// that starts there. Every entry is a bar start under `downbeat`.
    pub phrases: Vec<PhraseStart>,
    /// Every bar at which the arrangement changes, in time order, as the
    /// bar's first beat.
    pub sections: Vec<Beats>,
    /// How sure the analyzer is of the phrase structure, from zero to one.
    pub confidence: f64,
}

impl PhraseAnalysis {
    /// The phrase starts of at least the given length, in bars.
    pub fn starts_of_at_least(&self, bars: u32) -> impl Iterator<Item = Beats> + '_ {
        self.phrases
            .iter()
            .filter(move |start| start.bars >= bars)
            .map(|start| start.at)
    }
}

/// Finds a track's bar lines, phrase starts, and section changes on a grid
/// whose beats are already known.
pub trait PhraseAnalyzer {
    /// The name shown on the scoreboard.
    fn name(&self) -> &str;

    /// Analyzes a whole track whose beat grid is already known. The grid's
    /// beats and tempo are trusted; its beat zero is not assumed to start a
    /// bar.
    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<PhraseAnalysis, AnalysisError>;
}

/// The phrase starts that follow from counting bars of a fixed length from
/// a bar, up to the end of the audio: every eighth bar starts an eight-bar
/// phrase, every sixteenth a sixteen-bar one, and every thirty-second a
/// thirty-two-bar one.
pub fn counted_phrases(origin: Beats, audio: &Audio, grid: &BeatGrid) -> Vec<PhraseStart> {
    let end = grid.beat_at_position(audio.len());
    let bar = f64::from(BEATS_PER_BAR);
    let mut phrases = Vec::new();
    let mut count = 0u32;
    loop {
        let at = origin + Beats(f64::from(count) * 8.0 * bar);
        if at.0 >= end.0 {
            break;
        }
        let bars = if count.is_multiple_of(4) {
            32
        } else if count.is_multiple_of(2) {
            16
        } else {
            8
        };
        phrases.push(PhraseStart { at, bars });
        count += 1;
    }
    phrases
}

/// A phrase analyzer that assumes beat zero starts a bar and a thirty-two-bar
/// phrase, and counts phrases from there.
///
/// It reports no section changes and a confidence of zero, because it hears
/// nothing. It is the floor on the phrase scoreboard: a bespoke analyzer
/// that does not beat it has found nothing beyond the count from beat
/// zero.
#[derive(Debug, Clone, Copy, Default)]
pub struct CountedPhrases;

impl PhraseAnalyzer for CountedPhrases {
    fn name(&self) -> &str {
        "counted"
    }

    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<PhraseAnalysis, AnalysisError> {
        if grid.beat_at_position(audio.len()).0 < f64::from(BEATS_PER_BAR) {
            return Err(AnalysisError::TooShort(audio.len().0));
        }
        Ok(PhraseAnalysis {
            downbeat: 0,
            phrases: counted_phrases(Beats::ZERO, audio, grid),
            sections: Vec::new(),
            confidence: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::{Bpm, Samples, Seconds};

    #[test]
    fn counting_from_beat_zero_labels_every_eighth_bar() {
        // Sixty seconds at 120 beats per minute is 120 beats, or 30 bars.
        let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(60.0));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        let found = CountedPhrases.analyze(&audio, &grid).unwrap();
        assert_eq!(found.downbeat, 0);
        assert_eq!(
            found.phrases,
            vec![
                PhraseStart {
                    at: Beats(0.0),
                    bars: 32
                },
                PhraseStart {
                    at: Beats(32.0),
                    bars: 8
                },
                PhraseStart {
                    at: Beats(64.0),
                    bars: 16
                },
                PhraseStart {
                    at: Beats(96.0),
                    bars: 8
                },
            ]
        );
        assert_eq!(found.starts_of_at_least(16).count(), 2);
        assert!(found.sections.is_empty());
        assert_eq!(found.confidence, 0.0);
    }

    #[test]
    fn less_than_a_bar_is_too_short() {
        let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(1.0));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        assert!(matches!(
            CountedPhrases.analyze(&audio, &grid),
            Err(AnalysisError::TooShort(_))
        ));
    }
}
