//! A phrase analyzer that reads the bar lines and the phrase structure off
//! the shifts in level between one bar and the next.
//!
//! Dance music of the kind Dermixen is for is arranged in phrases of eight,
//! sixteen, and thirty-two bars, and the arrangement changes at their
//! starts: an element enters or leaves, the kick starts or stops, a
//! breakdown begins. Each such change shows as a shift in the level of one
//! or more bands that holds for bars afterwards. The analyzer measures the
//! low, middle, and high bands and the rise and fall of the low band within
//! a beat, one beat at a time on the grid it is given, and then:
//!
//! - takes the downbeat to be the beat of the bar at which the levels of
//!   the bar from that beat differ most from the levels of the bar before
//!   it, summed over the whole track, since arrangements change on bar
//!   lines and a comparison over whole bars is blind to the pattern within
//!   a bar;
//! - scores every bar by how much the levels shift there, comparing the bar
//!   with the one before and the four bars from it with the four before;
//! - takes as section changes the bars where the kick starts or stops and
//!   the bars where the four-bar shift stands out from its neighbors;
//! - counts the phrases from the section changes: every section change
//!   starts a phrase, and phrases run eight bars apart from there until the
//!   next section change, with every second one a sixteen-bar phrase and
//!   every fourth a thirty-two-bar one, so that after a breakdown of an
//!   odd length the analyzer counts the phrases from the bar the breakdown
//!   ends on, which is where a listener counts them from.
//!
//! The confidence is the share of those phrase starts that also lie on the
//! one eight-bar phase that fits the whole track best, scaled by how
//! clearly that phase stood out: the analyzer reports a high confidence
//! for a track whose sections all sit on one grid and a low one for a
//! track with sections of odd lengths.

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, SAMPLE_RATE};
use dermixen_media::Audio;

use crate::analyzer::AnalysisError;
use crate::dsp::{Band, band_envelopes, decibels};
use crate::phrases::{PhraseAnalysis, PhraseAnalyzer, PhraseStart};

/// How many samples one level reading covers: about six milliseconds.
const HOP: usize = 256;

/// The bands whose levels are measured in every beat.
const BANDS: [Band; 3] = [
    Band {
        low_hz: Some(30.0),
        high_hz: Some(130.0),
    },
    Band {
        low_hz: Some(130.0),
        high_hz: Some(2000.0),
    },
    Band {
        low_hz: Some(2000.0),
        high_hz: None,
    },
];

/// The level, in decibels, below which a band counts as silent, so that
/// the near-silence between sounds does not swing the measurements.
const FLOOR_DB: f64 = -80.0;

/// How far before each beat of the grid its window of readings starts, as
/// a share of a beat. On a quarter of the labeled grids the
/// beat lines sit on the kick's peak rather than its attack, so the attack
/// leads the beat line by up to a tenth of a beat; starting the window an
/// eighth of a beat early keeps each kick's attack, which is the sharpest
/// change in the track, inside the window of its own beat rather than the
/// beat before it.
const BEAT_LEAD: f64 = 0.125;

/// How many bars the longer comparison spans on each side of a bar.
const CONTEXT_BARS: usize = 4;

/// How much the four-bar shift counts against the one-bar shift when the
/// bars are scored, because a change that holds for bars is a phrase
/// boundary and a change that lasts one bar is a fill.
const CONTEXT_WEIGHT: f64 = 2.0;

/// The phrase lengths settled in turn, in bars, shortest first.
const PHRASE_BARS: [usize; 3] = [8, 16, 32];

/// The analyzer reports full confidence in a winning phase whose score is
/// this many times the average phase's score.
const FULL_CONFIDENCE_RATIO: f64 = 2.0;

/// A bar this far below the body level, in decibels, still counts as a
/// kick bar when the kick's start and stop are found.
const KICK_LEVEL_MARGIN_DB: f64 = 3.0;

/// The rise and fall within a beat, in decibels, that a kick bar must show.
const KICK_MODULATION_MIN_DB: f64 = 4.0;

/// The share of bars whose low-band level defines the body of the track.
const BODY_PERCENTILE: f64 = 0.9;

/// How many bars in a row the kick must run or rest for its start or stop
/// to count as a section change.
const KICK_HOLD_BARS: usize = 4;

/// A four-bar shift of at least this many decibels, summed over the
/// features, that stands above its neighbors counts as a section change.
const SECTION_SHIFT_DB: f64 = 9.0;

/// What one beat or one bar of the grid looks like: the level of the low,
/// middle, and high bands in decibels, then how far the low band rises and
/// falls within a beat, in decibels.
pub type Features = [f64; 4];

/// Everything the analyzer measures in one track, for the analyzer itself
/// and for the example that prints it bar by bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    /// The grid beat at which the first measured bar starts.
    pub first_bar_beat: i64,
    /// The features of every whole bar from that beat.
    pub bars: Vec<Features>,
    /// How much the levels shift at each bar from the bar before it.
    pub one_bar_shift: Vec<f64>,
    /// How much the mean levels of the four bars from each bar differ from
    /// the four before it.
    pub four_bar_shift: Vec<f64>,
    /// The sum of the shift scores at each bar of every eight.
    pub eight_bar_sums: Vec<f64>,
}

/// The features of every whole beat of the grid that lies inside the
/// audio, and the grid beat of the first of them.
fn beat_features(audio: &Audio, grid: &BeatGrid) -> (Vec<Features>, i64) {
    let envelopes = band_envelopes(audio, &BANDS, HOP);
    let readings = envelopes[0].len();
    let period = grid.beat_period().0 * f64::from(SAMPLE_RATE) / HOP as f64;
    let first_beat = grid.first_beat.0 as f64 / HOP as f64;
    // Beats before beat zero count too, at negative indexes, as long as
    // they start inside the audio.
    let first_index = ((-first_beat) / period).ceil() as i64;
    let mut features = Vec::new();
    let mut index = first_index;
    loop {
        // The first beat's window is cut off at the start of the audio.
        let start = (first_beat + (index as f64 - BEAT_LEAD) * period).max(0.0);
        let end = first_beat + (index as f64 + 1.0 - BEAT_LEAD) * period;
        let first = start.round() as usize;
        let last = end.round() as usize;
        if last > readings || last <= first {
            break;
        }
        let mut beat = [0.0f64; 4];
        for (band, envelope) in envelopes.iter().enumerate() {
            let energy: f64 = envelope[first..last].iter().map(|v| v * v).sum();
            beat[band] = decibels((energy / (last - first) as f64).sqrt()).max(FLOOR_DB);
        }
        // The low band's rise and fall within the beat, read from a
        // three-reading smoothing so one noisy reading does not stand in
        // for the whole rise.
        let low = &envelopes[0][first..last];
        let mut top = f64::MIN;
        let mut bottom = f64::MAX;
        for i in 0..low.len() {
            let before = low[i.saturating_sub(1)];
            let after = low[(i + 1).min(low.len() - 1)];
            let smooth = decibels((before + low[i] + after) / 3.0).max(FLOOR_DB);
            top = top.max(smooth);
            bottom = bottom.min(smooth);
        }
        beat[3] = top - bottom;
        features.push(beat);
        index += 1;
    }
    (features, first_index)
}

/// The mean of the features over a run of beats or bars.
fn mean_features(run: &[Features]) -> Features {
    let mut mean = [0.0f64; 4];
    for entry in run {
        for (slot, value) in mean.iter_mut().zip(entry) {
            *slot += value / run.len() as f64;
        }
    }
    mean
}

/// The distance between two feature vectors: the sum of the absolute
/// differences of the three band levels, in decibels. The rise and fall
/// within a beat is left out, because it describes the kick rather than
/// the arrangement.
fn distance(a: &Features, b: &Features) -> f64 {
    (0..3).map(|band| (a[band] - b[band]).abs()).sum()
}

/// Which of the four beats of a bar the first measured beat is: the
/// offset from zero to three that cuts the beats into whole bars whose
/// levels differ most from one bar to the next, as the sum over the track
/// of the squared differences between each bar and the bar before it.
/// Whole bars are blind to any pattern that repeats every bar, such as a
/// hat on the off-beats, so only changes in the arrangement count, and
/// those fall on bar lines. Squaring the differences is what tells the
/// offsets apart: a change that falls on a bar line shows as one large
/// difference, and the same change cut in two by a wrong offset shows as
/// two smaller ones whose squares add up to less.
fn downbeat_offset(beats: &[Features]) -> usize {
    let bar = BEATS_PER_BAR as usize;
    let change_at: Vec<f64> = (0..bar)
        .map(|offset| {
            let bars = bar_features(beats, offset);
            bars.windows(2)
                .map(|pair| {
                    (0..3)
                        .map(|band| (pair[1][band] - pair[0][band]).powi(2))
                        .sum::<f64>()
                })
                .sum()
        })
        .collect();
    (0..bar)
        .max_by(|&a, &b| change_at[a].total_cmp(&change_at[b]))
        .unwrap_or(0)
}

/// The mean features of each whole bar starting at `offset` beats.
fn bar_features(beats: &[Features], offset: usize) -> Vec<Features> {
    beats[offset..]
        .as_chunks::<{ BEATS_PER_BAR as usize }>()
        .0
        .iter()
        .map(|bar| mean_features(bar))
        .collect()
}

/// How much the levels shift at each bar: the distance from the bar before
/// it, and the distance between the mean of the four bars from it and the
/// mean of the four before it. The first bar gets no shift.
fn shifts(bars: &[Features]) -> (Vec<f64>, Vec<f64>) {
    let mut one = vec![0.0; bars.len()];
    let mut context = vec![0.0; bars.len()];
    for index in 1..bars.len() {
        one[index] = distance(&bars[index], &bars[index - 1]);
        let before = &bars[index.saturating_sub(CONTEXT_BARS)..index];
        let after = &bars[index..(index + CONTEXT_BARS).min(bars.len())];
        context[index] = distance(&mean_features(after), &mean_features(before));
    }
    (one, context)
}

/// The sum of the scores at every bar that is `phase` bars into a period.
fn phase_sum(scores: &[f64], period: usize, phase: usize) -> f64 {
    scores
        .iter()
        .enumerate()
        .filter(|(index, _)| index % period == phase)
        .map(|(_, score)| score)
        .sum()
}

/// The phase at which the scores add up highest among the candidates,
/// with the winner's sum and the runner-up's.
fn best_phase(scores: &[f64], period: usize, candidates: &[usize]) -> (usize, f64, f64) {
    let mut ranked: Vec<(usize, f64)> = candidates
        .iter()
        .map(|&phase| (phase, phase_sum(scores, period, phase)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let (best, best_score) = ranked[0];
    let runner_up = ranked.get(1).map_or(0.0, |entry| entry.1);
    (best, best_score, runner_up)
}

/// The level that a share of the values sit at or below.
fn percentile(values: &[f64], share: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = ((sorted.len() - 1) as f64 * share).round() as usize;
    sorted[position]
}

/// The bars at which the kick starts or stops, as bar indexes: a bar
/// counts as a kick bar when its low band sits near the body level and
/// rises and falls within a beat, and a start or stop counts only when
/// the new state holds for [`KICK_HOLD_BARS`] bars.
fn kick_changes(bars: &[Features]) -> Vec<usize> {
    if bars.len() < KICK_HOLD_BARS {
        return Vec::new();
    }
    let levels: Vec<f64> = bars.iter().map(|bar| bar[0]).collect();
    let body = percentile(&levels, BODY_PERCENTILE);
    let is_kick: Vec<bool> = bars
        .iter()
        .map(|bar| bar[0] >= body - KICK_LEVEL_MARGIN_DB && bar[3] >= KICK_MODULATION_MIN_DB)
        .collect();
    let holds = |index: usize, state: bool| {
        index + KICK_HOLD_BARS <= is_kick.len()
            && is_kick[index..index + KICK_HOLD_BARS]
                .iter()
                .all(|&kick| kick == state)
    };
    let mut changes = Vec::new();
    let mut state = is_kick[0];
    for (index, &kick) in is_kick.iter().enumerate().skip(1) {
        if kick != state && holds(index, kick) {
            state = kick;
            changes.push(index);
        }
    }
    changes
}

/// Measures a track on the given grid: the downbeat, the bars from it,
/// and the shifts at every bar. Returns `None` for a track shorter than
/// two bars.
pub fn measure(audio: &Audio, grid: &BeatGrid) -> Option<Measurement> {
    let bar = BEATS_PER_BAR as usize;
    let (beats, first_index) = beat_features(audio, grid);
    if beats.len() < 2 * bar {
        return None;
    }
    let offset = downbeat_offset(&beats);
    let bars = bar_features(&beats, offset);
    if bars.len() < 2 {
        return None;
    }
    let (one_bar_shift, four_bar_shift) = shifts(&bars);
    let scores: Vec<f64> = one_bar_shift
        .iter()
        .zip(&four_bar_shift)
        .map(|(one, context)| one + CONTEXT_WEIGHT * context)
        .collect();
    let eight_bar_sums = (0..PHRASE_BARS[0])
        .map(|phase| phase_sum(&scores, PHRASE_BARS[0], phase))
        .collect();
    Some(Measurement {
        first_bar_beat: first_index + offset as i64,
        bars,
        one_bar_shift,
        four_bar_shift,
        eight_bar_sums,
    })
}

/// The phrase analyzer described at the top of this module.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShiftPhrases;

/// The bar indexes at which the arrangement changes: where the kick starts
/// or stops, and where the four-bar shift reaches [`SECTION_SHIFT_DB`] and
/// stands above every four-bar shift within four bars of it.
fn section_changes(measured: &Measurement) -> Vec<usize> {
    let context = &measured.four_bar_shift;
    let mut sections: Vec<usize> = kick_changes(&measured.bars);
    for index in 1..measured.bars.len() {
        let from = index.saturating_sub(CONTEXT_BARS);
        let to = (index + CONTEXT_BARS + 1).min(measured.bars.len());
        let neighbors = context[from..to].iter().copied().fold(f64::MIN, f64::max);
        if context[index] >= SECTION_SHIFT_DB && context[index] >= neighbors {
            sections.push(index);
        }
    }
    sections.sort_unstable();
    sections.dedup();
    sections
}

/// The phrase starts, as bar indexes with the longest phrase starting at
/// each, when one phase serves the whole track: the eight-bar phase is
/// chosen among all eight bars, and each longer phase among the bars that
/// agree with the phase before it. Also returns the confidence: how far
/// the winning eight-bar phase stands above the average phase, scaled by
/// how clearly the sixteen-bar choice won.
fn one_phase_phrases(scores: &[f64]) -> (Vec<(usize, u32)>, f64) {
    let mut phases = [0usize; PHRASE_BARS.len()];
    let mut confidence = 0.0;
    let mut candidates: Vec<usize> = (0..PHRASE_BARS[0]).collect();
    for (level, &period) in PHRASE_BARS.iter().enumerate() {
        let (best, best_score, runner_up) = best_phase(scores, period, &candidates);
        phases[level] = best;
        if level == 0 {
            let mean = scores.iter().sum::<f64>() / period as f64;
            confidence = if mean > 0.0 {
                ((best_score / mean - 1.0) / (FULL_CONFIDENCE_RATIO - 1.0)).clamp(0.0, 1.0)
            } else {
                0.0
            };
        } else if level == 1 {
            let margin = if best_score > 0.0 {
                1.0 - runner_up / best_score
            } else {
                0.0
            };
            confidence *= margin.clamp(0.0, 1.0);
        }
        candidates = vec![best, best + period];
    }
    let phrases = (0..scores.len())
        .filter(|index| index % PHRASE_BARS[0] == phases[0])
        .map(|index| {
            let longest = PHRASE_BARS
                .iter()
                .zip(phases)
                .filter(|(period, phase)| index % **period == *phase)
                .map(|(period, _)| *period as u32)
                .max()
                .unwrap_or(PHRASE_BARS[0] as u32);
            (index, longest)
        })
        .collect();
    (phrases, confidence)
}

/// The phrase starts, as bar indexes with the longest phrase starting at
/// each, when phrases are counted from the section changes: every section
/// change starts a thirty-two-bar phrase, and phrases run eight bars apart
/// from there until the next section change, with every second one a
/// sixteen-bar phrase and every fourth a thirty-two-bar one. Before the
/// first section change the phrases are counted backwards from it. With
/// no section changes at all the phase is fitted to the whole track
/// instead. Also returns the confidence: the share of the counted phrase
/// starts that agree with the phase fitted to the whole track.
fn section_counted_phrases(scores: &[f64], sections: &[usize]) -> (Vec<(usize, u32)>, f64) {
    if sections.is_empty() {
        return one_phase_phrases(scores);
    }
    let step = PHRASE_BARS[0];
    let longest = |count: usize| -> u32 {
        PHRASE_BARS
            .iter()
            .filter(|period| (count * step).is_multiple_of(**period))
            .map(|period| *period as u32)
            .max()
            .unwrap_or(step as u32)
    };
    let mut phrases: Vec<(usize, u32)> = Vec::new();
    let first = sections[0];
    let mut count = 1;
    while first >= count * step {
        phrases.push((first - count * step, longest(count)));
        count += 1;
    }
    phrases.reverse();
    for (position, &start) in sections.iter().enumerate() {
        let end = sections.get(position + 1).copied().unwrap_or(scores.len());
        let mut count = 0;
        while start + count * step < end {
            phrases.push((start + count * step, longest(count)));
            count += 1;
        }
    }
    // A track whose sections all sit on one eight-bar grid is regular, and
    // the count from its section changes agrees with the phase fitted to
    // the whole track; a track with sections of odd lengths is not, and
    // the two disagree. The confidence is the share of the counted phrase
    // starts that lie on the fitted eight-bar phase, scaled by how clearly
    // that phase stood out.
    let (fitted, fitted_confidence) = one_phase_phrases(scores);
    let on_fitted = phrases
        .iter()
        .filter(|(index, _)| fitted.iter().any(|(bar, _)| bar == index))
        .count();
    let agreement = on_fitted as f64 / phrases.len().max(1) as f64;
    (phrases, agreement * fitted_confidence.max(0.5))
}

impl PhraseAnalyzer for ShiftPhrases {
    fn name(&self) -> &str {
        "shifts"
    }

    fn analyze(&self, audio: &Audio, grid: &BeatGrid) -> Result<PhraseAnalysis, AnalysisError> {
        let bar = BEATS_PER_BAR as usize;
        let Some(measured) = measure(audio, grid) else {
            return Err(AnalysisError::TooShort(audio.len().0));
        };
        let downbeat = measured.first_bar_beat.rem_euclid(bar as i64) as u32;
        let scores: Vec<f64> = measured
            .one_bar_shift
            .iter()
            .zip(&measured.four_bar_shift)
            .map(|(one, context)| one + CONTEXT_WEIGHT * context)
            .collect();
        let sections = section_changes(&measured);
        let (phrases, confidence) = section_counted_phrases(&scores, &sections);
        let beat_of =
            |bar_index: usize| Beats((measured.first_bar_beat + (bar_index * bar) as i64) as f64);
        Ok(PhraseAnalysis {
            downbeat,
            phrases: phrases
                .into_iter()
                .map(|(index, bars)| PhraseStart {
                    at: beat_of(index),
                    bars,
                })
                .collect(),
            sections: sections.into_iter().map(beat_of).collect(),
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dermixen_core::{Bpm, Samples, Seconds};

    /// Kicks at 120 beats per minute for 128 seconds, which is 64 bars,
    /// with the first sixteen bars quiet, a hat twenty milliseconds after
    /// beats two and four of every bar from bar 32, and the kick dropped
    /// from bars 48 to 55: a build, a body, a change at bar 32, and a
    /// breakdown, all on a sixteen-bar structure whose phrases start at
    /// bar zero.
    fn a_track() -> Audio {
        let bpm = Bpm(120.0);
        let mut audio = dermixen_testkit::synth::kicks(bpm, Seconds::ZERO, Seconds(128.0));
        let beat = Seconds(0.5).to_samples().0 as usize;
        let bar = 4 * beat;
        for frame in &mut audio.frames[..16 * bar] {
            frame[0] *= 0.25;
            frame[1] *= 0.25;
        }
        for frame in &mut audio.frames[48 * bar..56 * bar] {
            *frame = [0.0, 0.0];
        }
        let hat = dermixen_testkit::synth::white_noise(7, 0.2, Seconds(0.05));
        let late = Seconds(0.02).to_samples().0 as usize;
        for bar_index in 32..64 {
            for beat_index in [1usize, 3] {
                let start = bar_index * bar + beat_index * beat + late;
                for (offset, sample) in hat.frames.iter().enumerate() {
                    audio.frames[start + offset][0] += sample[0];
                    audio.frames[start + offset][1] += sample[1];
                }
            }
        }
        audio
    }

    #[test]
    fn the_structure_is_read_off_the_audio_whatever_beat_the_grid_starts_on() {
        let audio = a_track();
        for shift in 0..4 {
            let grid = BeatGrid {
                first_beat: Seconds(0.5 * shift as f64).to_samples(),
                bpm: Bpm(120.0),
            };
            let found = ShiftPhrases.analyze(&audio, &grid).unwrap();
            // Bars start at time zero, which is beat `-shift` of this grid.
            assert_eq!(found.downbeat, (4 - shift) % 4, "shift {shift}");
            let seconds_of = |beat: Beats| grid.time_of(beat).0;
            let sixteen: Vec<f64> = found.starts_of_at_least(16).map(seconds_of).collect();
            // Counted from the section changes, the kick's return at bar
            // 56 (112 seconds) starts a phrase as well as bars 0, 16, 32,
            // and 48 do.
            assert_eq!(sixteen, vec![0.0, 32.0, 64.0, 96.0, 112.0], "shift {shift}");
            let sections: Vec<f64> = found.sections.iter().copied().map(seconds_of).collect();
            assert!(sections.contains(&96.0), "shift {shift}: {sections:?}");
            assert!(sections.contains(&112.0), "shift {shift}: {sections:?}");
            let eight: Vec<f64> = found.starts_of_at_least(8).map(seconds_of).collect();
            assert_eq!(
                eight,
                vec![0.0, 16.0, 32.0, 48.0, 64.0, 80.0, 96.0, 112.0],
                "shift {shift}"
            );
            // Every section change of this track sits on the eight-bar grid
            // that fits the whole track, so the count from the sections
            // agrees with that grid entirely.
            assert!(
                found.confidence >= 0.5,
                "shift {shift}: {}",
                found.confidence
            );
        }
    }

    #[test]
    fn a_short_clip_is_too_short() {
        let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(3.0));
        let grid = BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        };
        assert!(matches!(
            ShiftPhrases.analyze(&audio, &grid),
            Err(AnalysisError::TooShort(_))
        ));
    }
}
