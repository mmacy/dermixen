//! Where a track's music effectively begins and ends, and the intro and
//! outro anchors that follow from it.
//!
//! Lead-in silence, a quiet build before the first kick, and a trailing tail
//! all fall outside the span between the effective beginning and the
//! effective ending. An anchor analyzer finds that span and then places each
//! anchor on a beat at a musically sensible point inside it, which is where a
//! track's anchors start from when it is added to a mix.

use dermixen_core::{Anchors, BeatGrid, Samples};
use dermixen_media::Audio;
use serde::{Deserialize, Serialize};

use crate::analyzer::AnalysisError;

/// The span of a track that holds its music: from the effective beginning
/// to the effective ending, in samples from the first sample of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extent {
    /// The first sample of the music. Everything before it is lead-in.
    #[serde(rename = "begins_sample")]
    pub begins: Samples,
    /// The sample after the last of the music. Everything from it on is tail.
    #[serde(rename = "ends_sample")]
    pub ends: Samples,
}

/// What an anchor analyzer found in one track.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorAnalysis {
    /// Where the music effectively begins and ends.
    pub extent: Extent,
    /// The anchors for the default eight-bar overlap. Both are whole beats
    /// of the grid the analyzer was given, and the outro anchor is a later
    /// beat than the intro anchor.
    pub anchors: Anchors,
    /// How sure the analyzer is, from zero to one.
    pub confidence: f64,
}

/// Finds where a track's music begins and ends and where its anchors go.
pub trait AnchorAnalyzer {
    /// The name shown on the scoreboard.
    fn name(&self) -> &str;

    /// Analyzes a whole track whose beat grid is already known.
    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<AnchorAnalysis, AnalysisError>;
}

/// The level below which a sample counts as silence for [`EdgeAnchors`],
/// as an amplitude: one thousandth of full scale, which is sixty decibels down.
const EDGE_THRESHOLD: f32 = 0.001;

/// An anchor analyzer that trims leading and trailing silence and nothing more.
///
/// The music begins at the first sample louder than sixty decibels below
/// full scale and ends after the last such sample. The intro anchor is the
/// whole beat nearest the beginning. The outro anchor is the last
/// beat that starts a bar and leaves eight whole bars before the ending, or,
/// when the track is too short for that, the last whole beat before the
/// ending. The confidence is always zero, because this analyzer knows
/// nothing about the music. It exists so the library and the command line
/// have anchors before the bespoke analyzer lands, and so the trait is
/// proven swappable.
#[derive(Debug, Clone, Copy, Default)]
pub struct EdgeAnchors;

impl AnchorAnalyzer for EdgeAnchors {
    fn name(&self) -> &str {
        "edges"
    }

    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<AnchorAnalysis, AnalysisError> {
        let audible =
            |frame: &[f32; 2]| frame[0].abs() > EDGE_THRESHOLD || frame[1].abs() > EDGE_THRESHOLD;
        let Some(first) = audio.frames.iter().position(audible) else {
            return Err(AnalysisError::Failed(
                "the audio is silent throughout, so it has no beginning or ending".to_owned(),
            ));
        };
        let last = audio
            .frames
            .iter()
            .rposition(audible)
            .expect("a first audible frame means there is a last one");
        let extent = Extent {
            begins: Samples(first as i64),
            ends: Samples(last as i64 + 1),
        };

        let intro = grid.beat_at_position(extent.begins).round();
        let ending = grid.beat_at_position(extent.ends).0;
        let bar = f64::from(dermixen_core::BEATS_PER_BAR);
        let eight_bars = 8.0 * bar;
        let outro = if ending - eight_bars > intro.0 {
            dermixen_core::Beats(((ending - eight_bars) / bar).floor() * bar)
        } else {
            dermixen_core::Beats(ending.ceil() - 1.0)
        };
        if outro.0 <= intro.0 {
            return Err(AnalysisError::TooShort(audio.len().0));
        }
        Ok(AnchorAnalysis {
            extent,
            anchors: Anchors { intro, outro },
            confidence: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::{Beats, Bpm, Seconds};

    /// Kicks at 120 beats per minute, so a beat is half a second and a bar is
    /// two seconds, with the first kick a second in and silence after the
    /// kicks stop.
    fn a_track() -> (Audio, BeatGrid) {
        let mut audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds(1.0), Seconds(30.0));
        for frame in &mut audio.frames[44_100 * 25..] {
            *frame = [0.0, 0.0];
        }
        let grid = BeatGrid {
            first_beat: Seconds(1.0).to_samples(),
            bpm: Bpm(120.0),
        };
        (audio, grid)
    }

    #[test]
    fn the_edges_are_where_the_sound_starts_and_stops() {
        let (audio, grid) = a_track();
        let found = EdgeAnchors.analyze(&audio, &grid).unwrap();
        // A kick starts from zero and rises within its first millisecond.
        assert!(found.extent.begins >= Samples(44_100));
        assert!(found.extent.begins < Samples(44_100 + 44));
        // The last kick that fits starts at 24.5 seconds and rings for about
        // fifty milliseconds, and the silence begins at twenty-five.
        assert!(found.extent.ends > Seconds(24.5).to_samples());
        assert!(found.extent.ends <= Seconds(25.0).to_samples());
        assert_eq!(found.anchors.intro, Beats::ZERO);
        // The ending is beat 47 and a fraction; eight bars before that is
        // beat 15 and a fraction, and the bar before that starts at beat 12.
        assert_eq!(found.anchors.outro, Beats(12.0));
        assert_eq!(found.confidence, 0.0);
    }

    #[test]
    fn a_short_track_gets_its_outro_anchor_on_its_last_whole_beat() {
        let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(4.2));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        let found = EdgeAnchors.analyze(&audio, &grid).unwrap();
        assert_eq!(found.anchors.intro, Beats::ZERO);
        // The ending is a little past beat 8, so the last whole beat before it is 8.
        assert_eq!(found.anchors.outro, Beats(8.0));
    }

    #[test]
    fn silence_has_no_edges() {
        let audio = dermixen_testkit::synth::silence(Seconds(5.0));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        assert!(matches!(
            EdgeAnchors.analyze(&audio, &grid),
            Err(AnalysisError::Failed(_))
        ));
    }
}
