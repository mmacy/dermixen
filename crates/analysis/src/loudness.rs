//! Loudness measurement by EBU R 128, through the `ebur128` crate, which
//! is what volume leveling reads to bring every track to one level.
//!
//! `DESIGN.md` names EBU R 128 under "Feature kernel" for leveling quiet
//! masters and loud masters to one perceived level. The measurement is the
//! standard's integrated loudness over the whole track, with its gates, and
//! the true peak of the track, which the leveling rule in
//! [`dermixen_core::leveling`] needs so that a quiet track is never raised
//! into clipping.

use dermixen_core::{Decibels, Lufs, SAMPLE_RATE};
use dermixen_media::Audio;
use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};

/// The number of channels every decoded track has. `dermixen_media`'s crate
/// documentation states that every buffer it returns is stereo.
const CHANNELS: u32 = 2;

/// What loudness analysis found in one track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Loudness {
    /// The integrated loudness of the whole track, gated as EBU R 128 gates
    /// it: blocks below minus seventy LUFS are left out, and then blocks more
    /// than ten loudness units below the mean of the rest are left out too.
    #[serde(rename = "integrated_lufs")]
    pub integrated: Lufs,
    /// The highest true peak in either channel, in decibels relative to full
    /// scale, found by oversampling four times so that a peak between two
    /// samples counts.
    #[serde(rename = "true_peak_db")]
    pub true_peak: Decibels,
}

/// Measures the integrated loudness and the true peak of a track.
///
/// The audio is stereo at the internal sample rate, as every decoded track
/// is. `None` means there is nothing to measure: audio shorter than the four
/// hundred milliseconds one measurement block spans, or audio that never
/// rises above the absolute gate of minus seventy LUFS, which is silence for
/// this purpose. A track with a loudness always has a true peak.
pub fn measure_loudness(audio: &Audio) -> Option<Loudness> {
    let mut meter = EbuR128::new(CHANNELS, SAMPLE_RATE, Mode::I | Mode::TRUE_PEAK)
        .expect("stereo audio at the fixed sample rate is always a valid EBU R 128 configuration");

    // `Frame` is `[f32; 2]`, left then right, which is already the layout
    // the meter reads, so this feeds the track's own buffer without a copy.
    meter
        .add_frames_f32(audio.frames.as_flattened())
        .expect("the flattened buffer always has one sample per channel per frame");

    // `loudness_global` gates as EBU R 128 specifies. It drops any block
    // quieter than the absolute gate of minus seventy LUFS. It then drops
    // any remaining block more than ten loudness units quieter than the
    // mean of what is left. The result is negative infinity for silence and
    // for audio shorter than the four hundred millisecond gating block,
    // since neither ever produces a block that clears the absolute gate.
    // The result is also non-finite when a sample is NaN or infinite.
    let integrated = meter.loudness_global().expect(
        "the meter is created with Mode::I, so the integrated loudness is always available",
    );
    if !integrated.is_finite() {
        return None;
    }

    // `true_peak` returns a linear amplitude, oversampled four times because
    // the sample rate is under 96 kHz, and the highest true peak in either
    // channel is the one the leveling rule needs.
    let true_peak_linear = (0..CHANNELS)
        .map(|channel| {
            meter.true_peak(channel).expect(
                "the meter is created with Mode::TRUE_PEAK, so every channel has a true peak",
            )
        })
        .fold(0.0_f64, f64::max);

    Some(Loudness {
        integrated: Lufs(integrated),
        true_peak: Decibels::from_linear(true_peak_linear),
    })
}
