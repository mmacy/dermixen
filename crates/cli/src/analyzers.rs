//! Choosing the analyzers a command runs over an audio file.
//!
//! The library crate's [`AnalyzerSet`] is the one home for which analyzers
//! a scan runs, so that the window and the command agree. What a command
//! adds is the tempo a person may type in place of grid analysis, which
//! replaces the grid analyzer with one that answers with the typed grid.

use dermixen_analysis::{AnalysisError, BeatAnalysis, BeatAnalyzer};
use dermixen_core::{BeatGrid, Bpm, Seconds};
use dermixen_library::{AnalyzerSet, Analyzers};
use dermixen_media::Audio;

/// The tempo options that both `analyze` and `mix add` accept.
///
/// A tempo given here replaces grid analysis altogether, and the time of the
/// first beat is read alongside it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Given {
    /// The tempo in beats per minute, or `None` to analyze the file.
    pub bpm: Option<f64>,
    /// The time of the first beat in seconds, or `None` for the start of the file.
    pub first_beat: Option<f64>,
}

impl Given {
    /// The beat grid these options name, or `None` when no tempo was given.
    ///
    /// A tempo that is not a positive finite number and a first beat that is
    /// not a finite number are both refused, each naming the option it came
    /// from.
    pub fn grid(&self) -> Result<Option<BeatGrid>, String> {
        let Some(bpm) = self.bpm else {
            return Ok(None);
        };
        let bpm = Bpm(bpm);
        if !bpm.is_valid() {
            return Err(format!(
                "--bpm must be a positive finite number of beats per minute, not {}",
                bpm.0
            ));
        }
        let first_beat = Seconds(self.first_beat.unwrap_or(0.0));
        if !first_beat.0.is_finite() {
            return Err(format!(
                "--first-beat must be a finite number of seconds, not {}",
                first_beat.0
            ));
        }
        Ok(Some(BeatGrid {
            first_beat: first_beat.to_samples(),
            bpm,
        }))
    }
}

/// A beat analyzer that answers with the grid a person typed rather than
/// looking at the audio.
///
/// It stands in for the grid analyzer so that a file whose tempo is given
/// goes through exactly the same analysis as any other file: the key and the
/// anchors are still found, and the anchors are placed on the given grid.
struct GivenTempo(BeatGrid);

impl BeatAnalyzer for GivenTempo {
    fn name(&self) -> &str {
        "given"
    }

    fn analyze(&self, _audio: &Audio) -> Result<BeatAnalysis, AnalysisError> {
        Ok(BeatAnalysis {
            bpm: self.0.bpm,
            beats: vec![self.0.first_beat],
            confidence: 1.0,
        })
    }
}

/// The analyzers one command will run, which the library crate's set owns
/// so that the borrowed [`Analyzers`] handed to a scan points into them.
pub struct Chosen(AnalyzerSet);

impl Chosen {
    /// The analyzers for a file whose tempo may have been given.
    ///
    /// With a tempo, the grid is that tempo and no grid analysis runs.
    /// Without one, the built-in set finds the grid. The anchors, the
    /// phrases, and the key come from the built-in set either way.
    pub fn new(given: &Given) -> Result<Chosen, String> {
        Ok(Chosen(match given.grid()? {
            Some(grid) => AnalyzerSet::with_beats(Box::new(GivenTempo(grid))),
            None => AnalyzerSet::built_in(),
        }))
    }

    /// These analyzers as the library crate takes them.
    pub fn as_library(&self) -> Analyzers<'_> {
        self.0.as_analyzers()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::Samples;

    #[test]
    fn a_given_tempo_becomes_a_grid_with_the_first_beat_where_it_was_put() {
        let grid = Given {
            bpm: Some(130.0),
            first_beat: Some(0.5),
        }
        .grid()
        .unwrap()
        .unwrap();
        assert_eq!(grid.bpm, Bpm(130.0));
        assert_eq!(grid.first_beat, Samples(22_050));
    }

    #[test]
    fn no_tempo_names_no_grid() {
        assert!(Given::default().grid().unwrap().is_none());
    }

    #[test]
    fn a_tempo_that_is_not_a_positive_number_is_refused_by_name() {
        for bpm in [0.0, -1.0, f64::NAN] {
            let message = Given {
                bpm: Some(bpm),
                first_beat: None,
            }
            .grid()
            .unwrap_err();
            assert!(message.contains("--bpm"), "{message}");
        }
        let message = Given {
            bpm: Some(130.0),
            first_beat: Some(f64::INFINITY),
        }
        .grid()
        .unwrap_err();
        assert!(message.contains("--first-beat"), "{message}");
    }

    #[test]
    fn the_stand_in_analyzer_reports_the_given_grid_as_certain() {
        let grid = BeatGrid {
            first_beat: Samples(11_025),
            bpm: Bpm(138.0),
        };
        let analyzer = GivenTempo(grid);
        assert_eq!(analyzer.name(), "given");
        let found = analyzer.analyze(&Audio::new()).unwrap();
        assert_eq!(found.bpm, Bpm(138.0));
        assert_eq!(found.beats, vec![Samples(11_025)]);
        assert_eq!(found.confidence, 1.0);
    }
}
