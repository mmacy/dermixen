//! An anchor analyzer that moves another analyzer's anchors onto the
//! nearest phrase starts a phrase analyzer finds.
//!
//! It exists to measure one question on the anchor scoreboard: whether
//! snapping the `kick` analyzer's intro and outro anchors to a phrase start
//! brings them closer to the labeled positions. It is not wired
//! into the library or the command line; the anchor scoreboard's `anchors`
//! example runs it beside `kick` so the two rows can be compared.

use dermixen_core::{Anchors, BeatGrid, Beats};
use dermixen_media::Audio;

use crate::analyzer::AnalysisError;
use crate::phrases::PhraseAnalyzer;
use crate::structure::{AnchorAnalysis, AnchorAnalyzer};

/// An anchor analyzer whose anchors are another analyzer's anchors moved
/// to the nearest phrase start of at least a given length. The extent and
/// the confidence are the inner anchor analyzer's.
pub struct PhrasedAnchors<A, P> {
    /// The name shown on the scoreboard.
    pub name: String,
    /// The analyzer whose anchors are moved.
    pub anchors: A,
    /// The analyzer whose phrase starts the anchors are moved onto.
    pub phrases: P,
    /// The shortest phrase, in bars, whose start an anchor may move onto.
    pub bars: u32,
}

impl<A: AnchorAnalyzer, P: PhraseAnalyzer> AnchorAnalyzer for PhrasedAnchors<A, P> {
    fn name(&self) -> &str {
        &self.name
    }

    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<AnchorAnalysis, AnalysisError> {
        let inner = self.anchors.analyze(audio, grid)?;
        let phrases = self.phrases.analyze(audio, grid)?;
        let starts: Vec<Beats> = phrases.starts_of_at_least(self.bars).collect();
        let nearest = |anchor: Beats| {
            starts
                .iter()
                .copied()
                .min_by(|a, b| (a.0 - anchor.0).abs().total_cmp(&(b.0 - anchor.0).abs()))
                .unwrap_or(anchor)
        };
        let intro = nearest(inner.anchors.intro);
        let outro = nearest(inner.anchors.outro);
        if outro.0 <= intro.0 {
            // Both anchors landed on the same phrase start, or crossed, so
            // the inner analyzer's anchors stand.
            return Ok(inner);
        }
        Ok(AnchorAnalysis {
            extent: inner.extent,
            anchors: Anchors { intro, outro },
            confidence: inner.confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phrases::{PhraseAnalysis, PhraseStart};
    use crate::structure::Extent;
    use dermixen_core::{Bpm, Samples, Seconds};

    /// An anchor analyzer that always answers the same thing.
    struct Fixed(Anchors);

    impl AnchorAnalyzer for Fixed {
        fn name(&self) -> &str {
            "fixed"
        }

        fn analyze(&self, audio: &Audio, _: &BeatGrid) -> Result<AnchorAnalysis, AnalysisError> {
            Ok(AnchorAnalysis {
                extent: Extent {
                    begins: Samples::ZERO,
                    ends: audio.len(),
                },
                anchors: self.0,
                confidence: 0.5,
            })
        }
    }

    /// A phrase analyzer with sixteen-bar phrases at bars 10 and 26 and an
    /// eight-bar one at bar 18.
    struct Fixtures;

    impl PhraseAnalyzer for Fixtures {
        fn name(&self) -> &str {
            "fixtures"
        }

        fn analyze(&self, _: &Audio, _: &BeatGrid) -> Result<PhraseAnalysis, AnalysisError> {
            Ok(PhraseAnalysis {
                downbeat: 0,
                phrases: vec![
                    PhraseStart {
                        at: Beats::from_bars(10.0),
                        bars: 16,
                    },
                    PhraseStart {
                        at: Beats::from_bars(18.0),
                        bars: 8,
                    },
                    PhraseStart {
                        at: Beats::from_bars(26.0),
                        bars: 16,
                    },
                ],
                sections: Vec::new(),
                confidence: 1.0,
            })
        }
    }

    fn a_track() -> (Audio, BeatGrid) {
        let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(70.0));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        (audio, grid)
    }

    #[test]
    fn anchors_move_to_the_nearest_phrase_start_of_the_asked_length() {
        let (audio, grid) = a_track();
        let inner = Fixed(Anchors {
            intro: Beats::from_bars(17.0),
            outro: Beats::from_bars(25.0),
        });
        let eight = PhrasedAnchors {
            name: "eight".to_owned(),
            anchors: inner,
            phrases: Fixtures,
            bars: 8,
        };
        let found = eight.analyze(&audio, &grid).unwrap();
        assert_eq!(found.anchors.intro, Beats::from_bars(18.0));
        assert_eq!(found.anchors.outro, Beats::from_bars(26.0));
        assert_eq!(found.confidence, 0.5);

        let sixteen = PhrasedAnchors {
            name: "sixteen".to_owned(),
            anchors: eight.anchors,
            phrases: Fixtures,
            bars: 16,
        };
        let found = sixteen.analyze(&audio, &grid).unwrap();
        assert_eq!(found.anchors.intro, Beats::from_bars(10.0));
        assert_eq!(found.anchors.outro, Beats::from_bars(26.0));
    }

    #[test]
    fn anchors_that_would_meet_stay_where_they_were() {
        let (audio, grid) = a_track();
        let inner = Fixed(Anchors {
            intro: Beats::from_bars(17.0),
            outro: Beats::from_bars(19.0),
        });
        let snapped = PhrasedAnchors {
            name: "eight".to_owned(),
            anchors: inner,
            phrases: Fixtures,
            bars: 8,
        };
        let found = snapped.analyze(&audio, &grid).unwrap();
        assert_eq!(found.anchors.intro, Beats::from_bars(17.0));
        assert_eq!(found.anchors.outro, Beats::from_bars(19.0));
    }
}
