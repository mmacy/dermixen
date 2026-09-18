//! The analyzer traits and the results they produce.
//!
//! Every analyzer, whether a bound baseline library or bespoke work, is one
//! implementation of these traits, so the scoreboard can run them side by
//! side and the library can swap one for another without other changes.

use std::fmt;
use std::str::FromStr;

use dermixen_core::{Bpm, Samples};
use dermixen_media::Audio;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The reason an analysis could not be completed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisError {
    /// The audio was too short for the analyzer to say anything.
    #[error("the audio is too short to analyze: {0} samples")]
    TooShort(i64),
    /// The analyzer failed for a reason of its own.
    #[error("{0}")]
    Failed(String),
}

/// What a beat analyzer found in one track.
#[derive(Debug, Clone, PartialEq)]
pub struct BeatAnalysis {
    /// The tempo of the whole track.
    pub bpm: Bpm,
    /// Every beat the analyzer located, in order.
    pub beats: Vec<Samples>,
    /// How sure the analyzer is, from zero to one.
    pub confidence: f64,
}

/// Finds the tempo and the beats of a track.
pub trait BeatAnalyzer {
    /// The name shown on the scoreboard.
    fn name(&self) -> &str;

    /// Analyzes a whole track.
    fn analyze(&self, audio: &Audio) -> Result<BeatAnalysis, AnalysisError>;
}

/// The twelve pitch classes, declared in chromatic order from C so that the
/// distance in semitones between two of them is the difference of their
/// positions here. The Camelot mapping and the key score rely on that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum PitchClass {
    C,
    Cs,
    D,
    Ds,
    E,
    F,
    Fs,
    G,
    Gs,
    A,
    As,
    B,
}

/// Major or minor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum Mode {
    Major,
    Minor,
}

/// A musical key: a tonic and a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    /// The tonic.
    pub tonic: PitchClass,
    /// Major or minor.
    pub mode: Mode,
}

impl PitchClass {
    /// The name of the pitch class, written with a sharp where it is not a
    /// natural: `C`, `C#`, `D`, and so on.
    pub fn name(self) -> &'static str {
        match self {
            PitchClass::C => "C",
            PitchClass::Cs => "C#",
            PitchClass::D => "D",
            PitchClass::Ds => "D#",
            PitchClass::E => "E",
            PitchClass::F => "F",
            PitchClass::Fs => "F#",
            PitchClass::G => "G",
            PitchClass::Gs => "G#",
            PitchClass::A => "A",
            PitchClass::As => "A#",
            PitchClass::B => "B",
        }
    }
}

impl fmt::Display for Key {
    /// Writes the key as its tonic and mode, as in `A minor` or `F# major`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.mode {
            Mode::Major => "major",
            Mode::Minor => "minor",
        };
        write!(f, "{} {mode}", self.tonic.name())
    }
}

/// The reason a text could not be read as a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a key is a tonic and a mode, such as A minor or F# major, got {0:?}")]
pub struct KeyParseError(pub String);

impl FromStr for Key {
    type Err = KeyParseError;

    /// Reads a key in any of the forms the ground-truth annotations use,
    /// such as `A minor`, `F# major`, `Bb min`, `Fm`, or a bare `A` for major.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        crate::scoreboard::parse_key(text).ok_or_else(|| KeyParseError(text.to_owned()))
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// What a key analyzer found in one track.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyAnalysis {
    /// The key of the whole track.
    pub key: Key,
    /// How sure the analyzer is, from zero to one.
    pub confidence: f64,
}

/// Finds the musical key of a track.
pub trait KeyAnalyzer {
    /// The name shown on the scoreboard.
    fn name(&self) -> &str;

    /// Analyzes a whole track.
    fn analyze(&self, audio: &Audio) -> Result<KeyAnalysis, AnalysisError>;
}

/// A beat analyzer that reports one fixed tempo with beats spaced evenly from
/// the first sample, and no confidence at all.
///
/// It exists so the trait has an implementation to run the scoreboard and
/// the library against before a real analyzer is bound.
#[derive(Debug, Clone, Copy)]
pub struct FixedTempo(pub Bpm);

impl BeatAnalyzer for FixedTempo {
    fn name(&self) -> &str {
        "fixed tempo"
    }

    fn analyze(&self, audio: &Audio) -> Result<BeatAnalysis, AnalysisError> {
        if audio.is_empty() {
            return Err(AnalysisError::TooShort(0));
        }
        let period = self.0.beat_period().0 * f64::from(dermixen_core::SAMPLE_RATE);
        let beats = (0..)
            .map(|i| Samples((f64::from(i) * period).round() as i64))
            .take_while(|beat| *beat < audio.len())
            .collect();
        Ok(BeatAnalysis {
            bpm: self.0,
            beats,
            confidence: 0.0,
        })
    }
}

/// A key analyzer that always answers C major with no confidence.
///
/// It exists so the trait has an implementation before a real one is bound.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnknownKey;

impl KeyAnalyzer for UnknownKey {
    fn name(&self) -> &str {
        "unknown key"
    }

    fn analyze(&self, audio: &Audio) -> Result<KeyAnalysis, AnalysisError> {
        if audio.is_empty() {
            return Err(AnalysisError::TooShort(0));
        }
        Ok(KeyAnalysis {
            key: Key {
                tonic: PitchClass::C,
                mode: Mode::Major,
            },
            confidence: 0.0,
        })
    }
}

/// How much audio aubio's beat tracker looks at in one step, in samples.
///
/// This is aubio's own default. A window covers about twenty-three
/// milliseconds at the sample rate Dermixen works at, and its size sets how
/// finely the tracker can separate one frequency from another when it measures
/// how much the spectrum changed. How finely the tracker can separate one
/// moment from another is set by [`AUBIO_HOP`] instead.
#[cfg(feature = "aubio")]
const AUBIO_WINDOW: usize = 1024;

/// How far aubio's beat tracker moves between steps, in samples.
///
/// aubio announces a beat only during the step that beat falls in, and it
/// places every beat at the start of a step, so the longer the step the more
/// beats the tracker passes over without announcing. At aubio's own default of
/// 512 samples it announces well under half the beats of a steady
/// four-on-the-floor track. At 128 samples, just under three milliseconds, it
/// announces nearly all of those beats and puts each one within a few
/// milliseconds of the drum hit.
#[cfg(feature = "aubio")]
const AUBIO_HOP: usize = 128;

/// The aubio beat tracker, a baseline from an established library.
///
/// Available only with the `aubio` feature, which builds the vendored C
/// library. Its row on the scoreboard is the floor any bespoke analyzer must
/// beat.
///
/// The audio is mixed down to one channel, because aubio works on one channel,
/// and then pushed through aubio's tempo tracker from the first sample to the
/// last. Every beat aubio announces along the way becomes one entry in
/// [`BeatAnalysis::beats`]. The tempo of the whole track comes from those
/// beats, as the time from the first to the last divided by the number of beats
/// between them. The confidence is aubio's own figure clamped into the range
/// from zero to one, because aubio's figure has no upper limit of its own.
#[cfg(feature = "aubio")]
#[derive(Debug, Default, Clone, Copy)]
pub struct AubioBeats;

#[cfg(feature = "aubio")]
impl BeatAnalyzer for AubioBeats {
    fn name(&self) -> &str {
        "aubio"
    }

    fn analyze(&self, audio: &Audio) -> Result<BeatAnalysis, AnalysisError> {
        if audio.frames.len() < AUBIO_HOP {
            return Err(AnalysisError::TooShort(audio.len().0));
        }
        let mut tracker =
            aubio_sys::Tempo::new(AUBIO_WINDOW, AUBIO_HOP, dermixen_core::SAMPLE_RATE).ok_or_else(
                || AnalysisError::Failed("aubio would not start a beat tracker".to_owned()),
            )?;

        let mut block = [0.0f32; AUBIO_HOP];
        let mut beats: Vec<Samples> = Vec::new();
        for frames in audio.frames.chunks(AUBIO_HOP) {
            for (sample, frame) in block.iter_mut().zip(frames) {
                *sample = 0.5 * (frame[0] + frame[1]);
            }
            // The last block of a track is rarely a whole hop. Padding it with
            // silence keeps aubio reading only samples that exist.
            block[frames.len()..].fill(0.0);
            let Some(at) = tracker.feed(&block) else {
                continue;
            };
            // The position arrives as an unsigned count of samples, which
            // aubio builds by adding a signed correction to a running total. An
            // unsigned count cannot hold a position before the start of the
            // track, so a correction that reached back past the start would
            // arrive not as a negative number but as a count near four thousand
            // million. Checking the position against the length of the track is
            // what keeps a number of that shape out of the beat list.
            if at >= audio.frames.len() {
                continue;
            }
            // aubio hands back the beat it heard most recently, which is the
            // same beat again if two blocks in a row announce one. Letting the
            // same position into the list twice would hand `tempo_of` a gap of
            // no length at all.
            let at = Samples(at as i64);
            if beats.last().is_none_or(|last| *last < at) {
                beats.push(at);
            }
        }

        // Silence, and anything else with no pulse in it, comes out here: aubio
        // announces no beats, so there is nothing to read a tempo from.
        let bpm = tempo_of(&beats).ok_or_else(|| {
            AnalysisError::Failed("aubio found too few beats to give a tempo".to_owned())
        })?;
        Ok(BeatAnalysis {
            bpm,
            beats,
            confidence: tracker.confidence(),
        })
    }
}

/// The tempo of a whole track worked out from the beats found in it, which must
/// be in increasing order. Fewer than two beats give no answer at all.
///
/// The gap from one beat to the next is a tempo on its own, but those gaps
/// wobble, and aubio's wobble does not average out. aubio spaces the beats
/// inside each of its own analysis windows a few tenths of a percent too close
/// together and makes the time up in one longer gap at each window boundary.
/// Reading the tempo off the typical gap therefore runs consistently fast,
/// which is enough to walk a beat grid off the drums over a six-minute track.
///
/// Counting instead is exact. The typical gap is used only to say how many
/// beats each gap spans, one gap at a time so that a beat the tracker missed is
/// counted as the two or three beats it really covers, and the tempo is the
/// time from the first beat to the last divided by the beats between them.
#[cfg(feature = "aubio")]
fn tempo_of(beats: &[Samples]) -> Option<Bpm> {
    let gap_of = |pair: &[Samples]| (pair[1] - pair[0]).to_seconds().0;
    let mut gaps: Vec<f64> = beats
        .windows(2)
        .map(gap_of)
        .filter(|gap| *gap > 0.0)
        .collect();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_by(f64::total_cmp);
    let typical = gaps[gaps.len() / 2];

    // Every gap covers at least one beat, however small the tracker made it.
    let counted: f64 = beats
        .windows(2)
        .map(|pair| (gap_of(pair) / typical).round().max(1.0))
        .sum();
    let span = (beats[beats.len() - 1] - beats[0]).to_seconds().0;
    if span <= 0.0 {
        return None;
    }
    Some(Bpm(counted * 60.0 / span))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_second() -> Audio {
        Audio {
            frames: vec![[0.0, 0.0]; 44_100],
        }
    }

    #[test]
    fn the_fixed_tempo_analyzer_spaces_beats_evenly() {
        let analyzer: Box<dyn BeatAnalyzer> = Box::new(FixedTempo(Bpm(120.0)));
        assert_eq!(analyzer.name(), "fixed tempo");
        let result = analyzer.analyze(&one_second()).unwrap();
        assert_eq!(result.bpm, Bpm(120.0));
        assert_eq!(result.beats, vec![Samples(0), Samples(22_050)]);
        assert_eq!(result.confidence, 0.0);
        assert_eq!(
            analyzer.analyze(&Audio::new()),
            Err(AnalysisError::TooShort(0))
        );
    }

    #[test]
    fn a_key_is_written_and_read_as_its_tonic_and_mode() {
        let key = Key {
            tonic: PitchClass::Fs,
            mode: Mode::Minor,
        };
        assert_eq!(key.to_string(), "F# minor");
        assert_eq!("F# minor".parse::<Key>().unwrap(), key);
        assert_eq!("Gb min".parse::<Key>().unwrap(), key);
        assert!("H major".parse::<Key>().is_err());
        assert_eq!(serde_json::to_string(&key).unwrap(), "\"F# minor\"");
        assert_eq!(
            serde_json::from_str::<Key>("\"A\"").unwrap().to_string(),
            "A major"
        );
    }

    #[test]
    fn the_unknown_key_analyzer_answers_c_major() {
        let analyzer: Box<dyn KeyAnalyzer> = Box::new(UnknownKey);
        let result = analyzer.analyze(&one_second()).unwrap();
        assert_eq!(
            result.key,
            Key {
                tonic: PitchClass::C,
                mode: Mode::Major
            }
        );
        assert_eq!(result.confidence, 0.0);
    }
}

#[cfg(all(test, feature = "aubio"))]
mod aubio_tests {
    use super::*;

    #[test]
    fn the_whole_track_tempo_counts_the_beats_across_the_span() {
        // Half a second between beats is one hundred and twenty beats per
        // minute, and the one double-length gap stands for a missed beat.
        let beats = [
            Samples(0),
            Samples(22_050),
            Samples(44_100),
            Samples(88_200),
            Samples(110_250),
        ];
        assert_eq!(tempo_of(&beats), Some(Bpm(120.0)));
        assert_eq!(tempo_of(&beats[..1]), None);
        assert_eq!(tempo_of(&[]), None);

        // Gaps a fifth of a percent too short, with the time made up in one
        // longer gap, is the shape of aubio's own wobble. Reading the tempo off
        // the typical gap would answer 120.24; counting answers 120.
        let period = 60.0 / 120.0 * 44_100.0;
        let mut wobbly = vec![Samples(0)];
        for beat in 1..40 {
            let at = f64::from(beat) * period;
            let early = if beat % 10 == 0 { 0.0 } else { period * 0.002 };
            wobbly.push(Samples((at - early).round() as i64));
        }
        let found = tempo_of(&wobbly).unwrap();
        assert!((found.0 - 120.0).abs() < 0.01, "counted {}", found.0);
    }

    #[test]
    fn audio_shorter_than_one_block_is_too_short() {
        let audio = Audio {
            frames: vec![[0.0, 0.0]; 64],
        };
        assert_eq!(AubioBeats.analyze(&audio), Err(AnalysisError::TooShort(64)));
        assert_eq!(
            AubioBeats.analyze(&Audio::new()),
            Err(AnalysisError::TooShort(0))
        );
    }

    #[test]
    fn a_kick_track_gets_a_tempo_far_closer_than_the_scoreboard_asks_for() {
        // The scoreboard passes a tempo within four percent. Counting the beats
        // across the whole track, rather than reading the tempo off the typical
        // gap between two of them, lands inside a tenth of a percent on
        // material this clean, and a change that loses that accuracy should
        // show up here rather than on a six-minute mix.
        for bpm in [118.0f64, 132.5, 145.0] {
            let audio = dermixen_testkit::synth::kicks(
                Bpm(bpm),
                dermixen_core::Seconds(0.1),
                dermixen_core::Seconds(40.0),
            );
            let found = AubioBeats.analyze(&audio).unwrap().bpm;
            assert!(
                (found.0 - bpm).abs() < bpm * 0.001,
                "at {bpm} beats per minute aubio said {}",
                found.0
            );
        }
    }

    #[test]
    fn silence_has_no_tempo() {
        let audio = Audio {
            frames: vec![[0.0, 0.0]; 44_100 * 10],
        };
        let Err(AnalysisError::Failed(reason)) = AubioBeats.analyze(&audio) else {
            panic!("ten seconds of silence should hold no tempo");
        };
        assert!(reason.contains("too few beats"), "{reason}");
    }
}
