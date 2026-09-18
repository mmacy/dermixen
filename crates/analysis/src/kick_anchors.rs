//! An anchor analyzer that finds the span where a track's kick drum is
//! established and places the anchors at its edges.
//!
//! Dance music of the kind Dermixen is for has one loud, steady kick through
//! the body of the track, a quieter build before it, and a tail after it.
//! The intro anchor belongs on the bar where that kick is established, not
//! on the first sound, so the incoming track arrives with its kick running;
//! the outro anchor belongs sixteen bars before the kick stops, so that
//! with the default eight-bar overlap the outgoing track leaves while its
//! kick still has eight bars to run, and the incoming kick, established at
//! its own intro anchor, is never the only kick in the mix. The one outro
//! label confirmed by ear sits fifteen bars before its kick stops, and the
//! outro labels read from MixMeister project files sit eight or sixteen
//! bars before the kick stops about equally often. The effective beginning
//! and ending are the edges of the kick span, so a quiet build and a
//! trailing tail fall outside it.
//!
//! The kick is read from the low band, thirty to one hundred and thirty
//! hertz, one bar of the given grid at a time. A bar counts as a kick bar
//! when its low-band level is close to the track's body level and the level
//! rises and falls once per beat inside the bar, which separates a kick from
//! a bass drone at the same level. The kick counts as established at the
//! first bar that starts eight kick bars in a row.

use dermixen_core::{Anchors, BEATS_PER_BAR, BeatGrid, Beats, SAMPLE_RATE, Samples};
use dermixen_media::Audio;

use crate::analyzer::AnalysisError;
use crate::dsp::{Band, band_envelopes, decibels};
use crate::structure::{AnchorAnalysis, AnchorAnalyzer, EdgeAnchors, Extent};

/// The low edge of the kick band, in hertz.
const LOW_HZ: f64 = 30.0;

/// The high edge of the kick band, in hertz.
const HIGH_HZ: f64 = 130.0;

/// How many samples the low-band envelope averages over: about one and a
/// half milliseconds, fine enough to see the shape of a kick within a beat.
const HOP: usize = 64;

/// How many level readings per beat the fold that measures the rise and
/// fall of the kick uses.
const FOLD_BINS: usize = 24;

/// A bar this far below the body level, in decibels, still counts as a kick bar.
const LEVEL_MARGIN_DB: f64 = 3.0;

/// The rise and fall within a beat, in decibels, that a kick bar must show.
const MODULATION_MIN_DB: f64 = 4.0;

/// How many kick bars in a row make the kick established.
const ESTABLISHED_BARS: usize = 8;

/// How far before the end of the kick the outro anchor sits: sixteen bars,
/// which leaves the kick running eight bars past the end of the default
/// eight-bar overlap.
const OUTRO_BARS_BEFORE_END: usize = 16;

/// The share of bars whose level defines the body of the track: the level
/// nine in ten bars sit at or below.
const BODY_PERCENTILE: f64 = 0.9;

/// A level step of this many decibels between the bars outside the kick
/// span and the body earns full confidence.
const FULL_CONFIDENCE_STEP_DB: f64 = 12.0;

/// The low-band level of the audio, as one root-mean-square reading per
/// [`HOP`] samples of the mono mix passed through a fourth-order
/// Linkwitz-Riley band-pass from [`LOW_HZ`] to [`HIGH_HZ`].
pub fn low_band_envelope(audio: &Audio) -> Vec<f64> {
    let band = Band {
        low_hz: Some(LOW_HZ),
        high_hz: Some(HIGH_HZ),
    };
    band_envelopes(audio, &[band], HOP).remove(0)
}

/// What one bar of the grid looks like in the low band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// The bar's first beat.
    pub beat: Beats,
    /// The low-band level over the bar, in decibels.
    pub level: f64,
    /// How far the low-band level rises and falls within a beat, in decibels,
    /// read from the bar's four beats folded onto one.
    pub modulation: f64,
}

/// Measures every whole bar of the grid that lies inside the audio, given
/// the low-band envelope from [`low_band_envelope`].
pub fn measure_bars(envelope: &[f64], grid: &BeatGrid) -> Vec<Bar> {
    let hop = HOP as f64;
    let beat_samples = grid.beat_period().0 * f64::from(SAMPLE_RATE);
    let bar_samples = beat_samples * f64::from(BEATS_PER_BAR);
    let first_beat = grid.first_beat.0 as f64;
    let mut bars = Vec::new();
    let mut fold = [0.0f64; FOLD_BINS];
    let mut counts = [0usize; FOLD_BINS];
    let mut index = 0usize;
    loop {
        let start = first_beat + index as f64 * bar_samples;
        let end = start + bar_samples;
        if start < 0.0 {
            index += 1;
            continue;
        }
        let first_hop = (start / hop).ceil() as usize;
        let last_hop = (end / hop).floor() as usize;
        if last_hop > envelope.len() || last_hop <= first_hop {
            break;
        }
        fold.fill(0.0);
        counts.fill(0);
        let mut energy = 0.0;
        for (reading, &value) in envelope[first_hop..last_hop].iter().enumerate() {
            let at = (first_hop + reading) as f64 * hop + hop / 2.0;
            let phase = ((at - start) / beat_samples).fract();
            let bin = ((phase * FOLD_BINS as f64) as usize).min(FOLD_BINS - 1);
            fold[bin] += value;
            counts[bin] += 1;
            energy += value * value;
        }
        let level = decibels((energy / (last_hop - first_hop) as f64).sqrt());
        let mut shape = [0.0f64; FOLD_BINS];
        for bin in 0..FOLD_BINS {
            let mean = if counts[bin] > 0 {
                fold[bin] / counts[bin] as f64
            } else {
                0.0
            };
            shape[bin] = decibels(mean);
        }
        // Smoothing over three neighboring readings, around the circle,
        // keeps one noisy reading from standing in for the whole rise.
        let mut smooth = [0.0f64; FOLD_BINS];
        for bin in 0..FOLD_BINS {
            let before = shape[(bin + FOLD_BINS - 1) % FOLD_BINS];
            let after = shape[(bin + 1) % FOLD_BINS];
            smooth[bin] = (before + shape[bin] + after) / 3.0;
        }
        let top = smooth.iter().copied().fold(f64::MIN, f64::max);
        let bottom = smooth.iter().copied().fold(f64::MAX, f64::min);
        bars.push(Bar {
            beat: Beats((index * BEATS_PER_BAR as usize) as f64),
            level,
            modulation: top - bottom,
        });
        index += 1;
    }
    bars
}

/// The level that a share of the bars sit at or below.
fn percentile(levels: &[f64], share: f64) -> f64 {
    let mut sorted = levels.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = ((sorted.len() - 1) as f64 * share).round() as usize;
    sorted[position]
}

/// The span of bars, as indexes into `bars`, from the first established
/// kick bar to one past the last kick bar of the last established stretch,
/// or `None` when the kick is never established.
fn kick_span(bars: &[Bar], body: f64) -> Option<(usize, usize)> {
    let is_kick: Vec<bool> = bars
        .iter()
        .map(|bar| bar.level >= body - LEVEL_MARGIN_DB && bar.modulation >= MODULATION_MIN_DB)
        .collect();
    let established = |index: usize| {
        index + ESTABLISHED_BARS <= is_kick.len()
            && is_kick[index..index + ESTABLISHED_BARS]
                .iter()
                .all(|&kick| kick)
    };
    let first = (0..bars.len()).find(|&index| established(index))?;
    let last_established = (0..bars.len()).rev().find(|&index| established(index))?;
    let mut end = last_established + ESTABLISHED_BARS;
    while end < bars.len() && is_kick[end] {
        end += 1;
    }
    Some((first, end))
}

/// An anchor analyzer that places the anchors at the edges of the span
/// where the kick is established, described at the top of this module.
///
/// A track whose kick is never established for eight bars in a row, such
/// as an ambient piece or one shorter than eight bars, gets the answer
/// [`EdgeAnchors`] gives, with a confidence of zero.
#[derive(Debug, Clone, Copy, Default)]
pub struct KickAnchors;

impl AnchorAnalyzer for KickAnchors {
    fn name(&self) -> &str {
        "kick"
    }

    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<AnchorAnalysis, AnalysisError> {
        let fallback = EdgeAnchors.analyze(audio, grid)?;
        let envelope = low_band_envelope(audio);
        let bars = measure_bars(&envelope, grid);
        if bars.len() < ESTABLISHED_BARS {
            return Ok(fallback);
        }
        let levels: Vec<f64> = bars.iter().map(|bar| bar.level).collect();
        let body = percentile(&levels, BODY_PERCENTILE);
        let Some((first, end)) = kick_span(&bars, body) else {
            return Ok(fallback);
        };

        let intro = bars[first].beat;
        let kick_end = bars[end - 1].beat + Beats::BAR;
        let outro_bars_back = Beats::from_bars(OUTRO_BARS_BEFORE_END as f64);
        let outro = if kick_end - outro_bars_back > intro {
            kick_end - outro_bars_back
        } else {
            // A kick span shorter than the overlap has no room for it, so
            // the overlap runs from the intro anchor over what there is.
            intro + Beats::BAR
        };
        if outro.0 <= intro.0 {
            return Ok(fallback);
        }

        let mean_level = |range: std::ops::Range<usize>| {
            let picked: Vec<f64> = range
                .filter_map(|index| bars.get(index))
                .map(|bar| bar.level)
                .collect();
            if picked.is_empty() {
                None
            } else {
                Some(picked.iter().sum::<f64>() / picked.len() as f64)
            }
        };
        let step_before = mean_level(first.saturating_sub(4)..first).map(|level| body - level);
        let step_after = mean_level(end..end + 4).map(|level| body - level);
        let steps: Vec<f64> = [step_before, step_after].into_iter().flatten().collect();
        let confidence = if steps.is_empty() {
            0.0
        } else {
            let mean_step = steps.iter().sum::<f64>() / steps.len() as f64;
            (mean_step / FULL_CONFIDENCE_STEP_DB).clamp(0.0, 1.0)
        };

        let extent = Extent {
            begins: grid.position_of(intro).max(Samples::ZERO),
            ends: grid.position_of(kick_end).min(audio.len()),
        };
        Ok(AnchorAnalysis {
            extent,
            anchors: Anchors { intro, outro },
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::{Bpm, Seconds};

    /// Kicks at 120 beats per minute for sixty seconds, with the first
    /// twelve bars (twenty-four seconds) attenuated by twelve decibels and
    /// two seconds of silence at the end: a quiet build, a body, a short tail.
    fn a_track() -> (Audio, BeatGrid) {
        let mut audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(60.0));
        let quiet = Seconds(24.0).to_samples().0 as usize;
        for frame in &mut audio.frames[..quiet] {
            frame[0] *= 0.25;
            frame[1] *= 0.25;
        }
        let tail = Seconds(58.0).to_samples().0 as usize;
        for frame in &mut audio.frames[tail..] {
            *frame = [0.0, 0.0];
        }
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        (audio, grid)
    }

    #[test]
    fn the_intro_anchor_is_where_the_kick_reaches_full_level() {
        let (audio, grid) = a_track();
        let found = KickAnchors.analyze(&audio, &grid).unwrap();
        // The quiet build is twelve bars, so the kick is established at bar
        // twelve, which is beat 48.
        assert_eq!(found.anchors.intro, Beats(48.0));
        // The last kick that fits starts at 57.5 seconds, beat 115, so the
        // kick runs through bar 28 and ends at beat 116; sixteen bars before
        // that is beat 52.
        assert_eq!(found.anchors.outro, Beats(52.0));
        assert_eq!(found.extent.begins, Seconds(24.0).to_samples());
        assert_eq!(found.extent.ends, Seconds(58.0).to_samples());
        assert!(found.confidence > 0.5, "confidence {}", found.confidence);
    }

    #[test]
    fn a_track_with_no_kick_gets_the_edge_answer() {
        let audio = dermixen_testkit::synth::sine(220.0, 0.5, Seconds(30.0));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        let found = KickAnchors.analyze(&audio, &grid).unwrap();
        let edges = EdgeAnchors.analyze(&audio, &grid).unwrap();
        assert_eq!(found, edges);
        assert_eq!(found.confidence, 0.0);
    }
}
