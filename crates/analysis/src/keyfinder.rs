//! The libkeyfinder key analyzer, a baseline from an established library.

use dermixen_media::Audio;

use crate::analyzer::{AnalysisError, Key, KeyAnalysis, KeyAnalyzer, Mode, PitchClass};

/// The twelve pitch classes in semitone order from C, so that a count of
/// semitones above C, which is how the libkeyfinder wrapper names a tonic, is
/// a position in this list.
const TONICS: [PitchClass; 12] = [
    PitchClass::C,
    PitchClass::Cs,
    PitchClass::D,
    PitchClass::Ds,
    PitchClass::E,
    PitchClass::F,
    PitchClass::Fs,
    PitchClass::G,
    PitchClass::Gs,
    PitchClass::A,
    PitchClass::As,
    PitchClass::B,
];

/// The libkeyfinder key detector.
///
/// Available only with the `keyfinder` feature, which builds the vendored
/// C++ library. Its row on the key scoreboard is the floor any bespoke key
/// analyzer must beat. The audio is mixed down to one channel and handed to
/// libkeyfinder whole; the key it answers with is reported as found, and the
/// confidence is the margin by which its best key beat its second best,
/// scaled into the range from zero to one.
///
/// The margin is scaled by dividing it by the best key's own score, so a
/// confidence of zero means the top two keys tied and a confidence of one
/// means the runner-up scored nothing at all. Read it as a ranking rather
/// than as a probability: every key's expected spread of pitches overlaps
/// heavily with every other's, so even a plain progression that sits squarely
/// in one key wins by only a few percent.
///
/// Audio too short to fill one of libkeyfinder's analysis frames, which is a
/// little under four seconds, is reported as too short to analyze. Handed such
/// a fragment libkeyfinder pads it with silence and names a key anyway, and
/// that key is read mostly from the padding, so this analyzer turns the
/// fragment away rather than passing the answer on.
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyfinderKey;

impl KeyAnalyzer for KeyfinderKey {
    fn name(&self) -> &str {
        "keyfinder"
    }

    fn analyze(&self, audio: &Audio) -> Result<KeyAnalysis, AnalysisError> {
        // libkeyfinder pads audio shorter than one analysis frame with silence
        // and names a key from the padded frame, so the length is checked here
        // rather than left to libkeyfinder to wave through. Empty audio falls
        // into the same check.
        if audio.frames.len() < keyfinder_sys::frame_samples(dermixen_core::SAMPLE_RATE) {
            return Err(AnalysisError::TooShort(audio.len().0));
        }

        let mono: Vec<f32> = audio
            .frames
            .iter()
            .map(|frame| 0.5 * (frame[0] + frame[1]))
            .collect();

        let found = keyfinder_sys::key_of_audio(&mono, dermixen_core::SAMPLE_RATE)
            .map_err(|failure| AnalysisError::Failed(failure.to_string()))?;

        // libkeyfinder answers with one of the twelve tonics, so the position
        // it gives is always inside the list, but reading it out this way
        // means a number from outside the list becomes a plain failure rather
        // than a panic.
        let tonic = *TONICS.get(usize::from(found.key.tonic)).ok_or_else(|| {
            AnalysisError::Failed(format!(
                "libkeyfinder named a tonic {} semitones above C, and there are only twelve",
                found.key.tonic
            ))
        })?;
        let mode = match found.key.mode {
            keyfinder_sys::Mode::Major => Mode::Major,
            keyfinder_sys::Mode::Minor => Mode::Minor,
        };

        Ok(KeyAnalysis {
            key: Key { tonic, mode },
            confidence: found.confidence,
        })
    }
}
