//! A beat analyzer for music rendered from a grid: one tempo for the whole
//! track, read from how the track repeats itself, and a beat zero on a
//! downbeat.
//!
//! The music Dermixen is for was made on a sequencer, so its beats sit on
//! one lattice from start to finish. That makes the tempo a property of the
//! whole file rather than something to track moment by moment: the lag at
//! which the track's onsets best line up with themselves is the beat period,
//! the same lag measured across sixty-four beats gives the period to a few
//! microseconds, and the kick's phase against that lattice, read in every
//! window of the track, advances in a straight line whose slope corrects the
//! period once more and whose intercept fixes beat zero. The beat instant is
//! the kick's attack, which is the broadband rise just before the low band
//! peaks. The downbeat is the beat of each bar at which the sound changes
//! most, since arrangements change on bar lines.

use dermixen_core::{BEATS_PER_BAR, Bpm, SAMPLE_RATE, Seconds};
use dermixen_media::Audio;

use crate::analyzer::{AnalysisError, BeatAnalysis, BeatAnalyzer};
use crate::dsp::{Band, band_envelopes, decibels};

/// How many samples one onset reading covers: about six milliseconds.
pub const HOP: usize = 256;

/// The slowest tempo considered, in beats per minute.
const MIN_BPM: f64 = 60.0;

/// The fastest tempo considered, in beats per minute.
const MAX_BPM: f64 = 200.0;

/// How far either side of the expected lag each refinement stage looks, in
/// beats: less than half a beat, so the peak one beat over is never taken.
const REFINE_WINDOW_BEATS: f64 = 0.4;

/// How well the low band must correlate at a lag, as a share of its best
/// correlation among the lag's half, the lag, and its double, for that lag
/// to be taken as the kick's period.
const KICK_LAG_SHARE: f64 = 0.75;

/// The tempo the prior is centered on, in beats per minute.
const PRIOR_BPM: f64 = 130.0;

/// The width of the prior, in octaves at one standard deviation.
const PRIOR_WIDTH_OCTAVES: f64 = 1.0;

/// How long each window is, in seconds, when the kick's phase is read
/// across the track to fit the lattice.
const WINDOW_SECONDS: f64 = 20.0;

/// How far the folded low-band peak must stand above the fold's median
/// level, in decibels, for a window to count as having a clear kick.
const CLEAR_KICK_DB: f64 = 6.0;

/// How many windows with a clear kick the lattice fit needs before it
/// corrects the period from the drift of the kick's phase.
const SLOPE_WINDOWS: usize = 8;

/// How many phase bins per beat the fold that finds the beat instant uses.
pub const PHASE_BINS: usize = 64;

/// The bands whose per-beat levels reveal where the arrangement changes.
const DOWNBEAT_BANDS: [Band; 3] = [
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

/// The onset strength of the track, one reading per [`HOP`] samples: how
/// much the low-band level and the broadband level each rise from one
/// reading to the next, in decibels, with falls counted as nothing.
///
/// Returns two signals: the low band's rises alone, and both bands' rises
/// with the low band counted twice, because the kick, which lives in the
/// low band, is the steadiest thing in this music.
pub fn onset_strength(audio: &Audio) -> (Vec<f64>, Vec<f64>) {
    let (low, wide) = level_envelopes(audio);
    onsets_of(&low, &wide)
}

/// The low-band and broadband level envelopes, one reading per [`HOP`] samples.
pub fn level_envelopes(audio: &Audio) -> (Vec<f64>, Vec<f64>) {
    let bands = [
        Band {
            low_hz: Some(30.0),
            high_hz: Some(130.0),
        },
        Band {
            low_hz: None,
            high_hz: None,
        },
    ];
    let mut envelopes = band_envelopes(audio, &bands, HOP);
    let wide = envelopes.pop().expect("two bands were asked for");
    let low = envelopes.pop().expect("two bands were asked for");
    (low, wide)
}

/// The onset strengths, as [`onset_strength`] describes, from the level envelopes.
fn onsets_of(low: &[f64], wide: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut low_onsets = vec![0.0; low.len()];
    let mut onsets = vec![0.0; low.len()];
    for i in 1..low.len() {
        let low_rise = (decibels(low[i]) - decibels(low[i - 1])).max(0.0);
        let wide_rise = (decibels(wide[i]) - decibels(wide[i - 1])).max(0.0);
        low_onsets[i] = low_rise;
        onsets[i] = 2.0 * low_rise + wide_rise;
    }
    (low_onsets, onsets)
}

/// The autocorrelation of a signal at one lag, normalized by the number
/// of pairs so that different lags compare.
pub fn autocorrelation(signal: &[f64], lag: usize) -> f64 {
    if lag >= signal.len() {
        return 0.0;
    }
    let pairs = signal.len() - lag;
    let mut sum = 0.0;
    for i in 0..pairs {
        sum += signal[i] * signal[i + lag];
    }
    sum / pairs as f64
}

/// The position of the peak of a curve sampled at three consecutive
/// points, as an offset from the middle one between minus one and one.
fn parabolic_peak(before: f64, at: f64, after: f64) -> f64 {
    let denominator = before - 2.0 * at + after;
    if denominator.abs() < 1e-12 {
        0.0
    } else {
        (0.5 * (before - after) / denominator).clamp(-1.0, 1.0)
    }
}

/// How likely a tempo is before the track is heard: a bell over the
/// logarithm of the tempo, centered on [`PRIOR_BPM`] and one octave wide
/// at one standard deviation, so a tempo an octave from the center is
/// weighed at about one seventh. The analyzer uses the prior to choose
/// between two lags in the ratio of small whole numbers when the track's
/// own correlations rate them alike.
fn prior(lag: f64) -> f64 {
    let bpm = 60.0 * f64::from(SAMPLE_RATE) / (lag * HOP as f64);
    let octaves = (bpm / PRIOR_BPM).log2();
    (-0.5 * (octaves / PRIOR_WIDTH_OCTAVES).powi(2)).exp()
}

/// The beat period in onset readings: the lag in the tempo range that
/// scores best when each lag is credited with its doubles and weighed by
/// the prior, moved to its half or its double where the low band says the
/// kick repeats at that lag instead, then measured again across four,
/// sixteen, and sixty-four beats, as far as the track allows, so the
/// period is known to a small fraction of a reading.
///
/// Returns the period and a confidence from zero to one: the coarse
/// score's contrast, which is the best score against the mean score over
/// the range, scaled by how clearly the low band settled the octave.
fn beat_period(low_onsets: &[f64], onsets: &[f64]) -> Option<(f64, f64)> {
    let readings_per_second = f64::from(SAMPLE_RATE) / HOP as f64;
    let min_lag = (60.0 / MAX_BPM * readings_per_second).floor() as usize;
    let max_lag = (60.0 / MIN_BPM * readings_per_second).ceil() as usize;
    if onsets.len() < 4 * max_lag + 2 {
        return None;
    }
    let lags: Vec<f64> = (0..=4 * max_lag + 1)
        .map(|lag| autocorrelation(onsets, lag))
        .collect();
    // Crediting each lag with its doubles keeps a lag with no rhythmic
    // meaning, whose doubles correlate no better, from beating the beat.
    let score =
        |lag: usize| (lags[lag] + 0.5 * lags[2 * lag] + 0.25 * lags[4 * lag]) * prior(lag as f64);
    let mut best = min_lag;
    let mut total = 0.0;
    for lag in min_lag..=max_lag {
        total += score(lag);
        if score(lag) > score(best) {
            best = lag;
        }
    }
    let mean = total / (max_lag - min_lag + 1) as f64;
    // The best score at twice the mean score over the range earns full
    // contrast; a peak that barely clears the mean earns none.
    let contrast = if mean > 0.0 {
        (score(best) / mean - 1.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // A lag of two beats scores about as well as one beat, because the
    // doubles of both correlate, and a snare on the second and fourth
    // beats can make two beats score better. The analyzer therefore reads
    // the octave off the low band, where the kick repeats once per beat:
    // the beat is the shortest of the half lag, the lag, and the double
    // lag at which the low band correlates nearly as well as at the
    // strongest of the three.
    let low_score = |lag: usize| autocorrelation(low_onsets, lag) * prior(lag as f64);
    let candidates: Vec<usize> = [best / 2, best, 2 * best]
        .into_iter()
        .filter(|&lag| (min_lag..=max_lag).contains(&lag))
        .collect();
    let strongest = candidates
        .iter()
        .map(|&lag| low_score(lag))
        .fold(f64::MIN, f64::max);
    if let Some(&shortest) = candidates
        .iter()
        .find(|&&lag| low_score(lag) >= KICK_LAG_SHARE * strongest)
    {
        best = shortest;
    }
    // The octave choice is only as sure as its margins: how far above the
    // share the chosen lag's low-band correlation sits, and how far below
    // it every shorter candidate's sits. A choice made right at the share
    // earns nothing, and one where the chosen lag is the strongest and no
    // shorter lag comes close earns everything.
    let ratio = |lag: usize| low_score(lag) / strongest;
    let mut certainty = if strongest > 0.0 {
        ((ratio(best) - KICK_LAG_SHARE) / (1.0 - KICK_LAG_SHARE)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    for &lag in candidates.iter().filter(|&&lag| lag < best) {
        let margin = ((KICK_LAG_SHARE - ratio(lag)) / KICK_LAG_SHARE).clamp(0.0, 1.0);
        certainty = certainty.min(margin);
    }
    let confidence = contrast * certainty;
    let coarse = best as f64 + parabolic_peak(lags[best - 1], lags[best], lags[best + 1]);

    // Measuring the lag across more beats divides the error by that many
    // beats, in stages, because each stage may only look less than half a
    // beat either side of where the previous one says the peak is: the
    // peaks one beat over correlate just as well. A stage runs only when
    // the lag leaves at least half the track to correlate against.
    let mut period = coarse;
    for beats in [4usize, 16, 64] {
        let center = beats as f64 * period;
        if center > onsets.len() as f64 / 2.0 {
            break;
        }
        let half_window = REFINE_WINDOW_BEATS * period;
        let low = (center - half_window).floor().max(1.0) as usize;
        let high = (center + half_window).ceil() as usize;
        let mut best_long = low;
        let mut best_value = f64::MIN;
        for lag in low..=high {
            let value = autocorrelation(onsets, lag);
            if value > best_value {
                best_value = value;
                best_long = lag;
            }
        }
        let refined = best_long as f64
            + parabolic_peak(
                autocorrelation(onsets, best_long - 1),
                best_value,
                autocorrelation(onsets, best_long + 1),
            );
        period = refined / beats as f64;
    }
    Some((period, confidence))
}

/// The rough phase of the kick, in readings from the start of the track:
/// the peak of the low-band level folded onto one beat at the given
/// period. The kick is the loudest thing in the low band, so the fold's
/// peak is the kick, even where a bass line between the beats rises as
/// sharply as the kick does. [`fit_lattice`] refines this.
fn beat_phase(low_levels: &[f64], period: f64) -> f64 {
    let profile = fold_profile(low_levels, period);
    let peak = (0..PHASE_BINS)
        .max_by(|&a, &b| profile[a].total_cmp(&profile[b]))
        .unwrap_or(0);
    ((peak as f64 + 0.5) / PHASE_BINS as f64) * period
}

/// The level of a signal folded onto one beat of the given period, in
/// decibels, as [`PHASE_BINS`] readings from the start of the beat, with
/// the fold starting at the first reading of the signal.
pub fn fold_profile(levels: &[f64], period: f64) -> [f64; PHASE_BINS] {
    let mut fold = [0.0f64; PHASE_BINS];
    let mut counts = [0usize; PHASE_BINS];
    for (i, &value) in levels.iter().enumerate() {
        let phase = (i as f64 / period).fract();
        let bin = ((phase * PHASE_BINS as f64) as usize).min(PHASE_BINS - 1);
        fold[bin] += value;
        counts[bin] += 1;
    }
    let mut profile = [0.0f64; PHASE_BINS];
    for bin in 0..PHASE_BINS {
        let mean = if counts[bin] > 0 {
            fold[bin] / counts[bin] as f64
        } else {
            0.0
        };
        profile[bin] = decibels(mean);
    }
    profile
}

/// The lattice fitted to the kick across the track: the period and the
/// phase, in readings, of the first low-band peak.
///
/// The fold over the whole track gives a period and a rough phase. The
/// kick's phase against that lattice is then read in each window of
/// [`WINDOW_SECONDS`] that has a clear kick, and a straight line through
/// those readings gives the correction to the period, from its slope, and
/// the phase at the start of the track, from its intercept. The line is
/// fitted with medians rather than least squares, so a section edit that
/// shifts the beats in part of a track moves a few readings without
/// bending the line.
fn fit_lattice(low_levels: &[f64], period: f64, phase: f64) -> (f64, f64) {
    let window = (WINDOW_SECONDS * f64::from(SAMPLE_RATE) / HOP as f64) as usize;
    let mut readings: Vec<(f64, f64)> = Vec::new();
    let mut start = 0usize;
    while start + window <= low_levels.len() {
        let profile = fold_profile(&low_levels[start..start + window], period);
        let peak = (0..PHASE_BINS)
            .max_by(|&a, &b| profile[a].total_cmp(&profile[b]))
            .unwrap_or(0);
        let mut sorted = profile;
        sorted.sort_by(f64::total_cmp);
        let contrast = profile[peak] - sorted[PHASE_BINS / 2];
        if contrast >= CLEAR_KICK_DB {
            // The window's fold starts at its first reading, so its peak
            // phase is relative to that reading; the kick's phase against
            // the lattice is that plus how far the reading sits past the
            // lattice beat before it.
            let peak_phase = (peak as f64 + 0.5) / PHASE_BINS as f64;
            let lattice_phase = ((start as f64 - phase) / period).fract();
            let at = (start as f64 + window as f64 / 2.0 - phase) / period;
            readings.push((at, (peak_phase + lattice_phase).rem_euclid(1.0)));
        }
        start += window;
    }
    if readings.len() < 3 {
        return (period, phase);
    }
    // Unwrapping keeps each reading within half a beat of the one before.
    let mut unwrapped: Vec<(f64, f64)> = Vec::with_capacity(readings.len());
    let mut previous = readings[0].1;
    for &(at, value) in &readings {
        let mut value = value;
        while value - previous > 0.5 {
            value -= 1.0;
        }
        while previous - value > 0.5 {
            value += 1.0;
        }
        unwrapped.push((at, value));
        previous = value;
    }
    let mut slopes: Vec<f64> = Vec::new();
    for i in 0..unwrapped.len() {
        for j in i + 1..unwrapped.len() {
            let (at_i, value_i) = unwrapped[i];
            let (at_j, value_j) = unwrapped[j];
            if at_j > at_i {
                slopes.push((value_j - value_i) / (at_j - at_i));
            }
        }
    }
    slopes.sort_by(f64::total_cmp);
    // Each reading is quantized to one phase bin, so the slope between two
    // of them is only trustworthy once there are enough windows for the
    // quantization to average out; before that the period from the long
    // lags stands.
    let slope = if readings.len() >= SLOPE_WINDOWS {
        slopes[slopes.len() / 2]
    } else {
        0.0
    };
    let mut intercepts: Vec<f64> = unwrapped
        .iter()
        .map(|&(at, value)| value - slope * at)
        .collect();
    intercepts.sort_by(f64::total_cmp);
    let intercept = intercepts[intercepts.len() / 2];
    // A kick that drifts later by `slope` beats per beat means the true
    // period is longer by that fraction; the peak sits `intercept` beats
    // after the rough lattice at the start.
    let fitted_period = period * (1.0 + slope);
    let fitted_phase = (phase + intercept * period).rem_euclid(fitted_period);
    (fitted_period, fitted_phase)
}

/// How far before the low-band peak the kick starts, in readings: the
/// steepest rise of the broadband level, folded onto one beat, within the
/// quarter beat before the peak. A kick's click arrives first and its low
/// bloom peaks tens of milliseconds later, and the click is the beat.
fn attack_lead(wide_levels: &[f64], period: f64, peak_phase: f64) -> f64 {
    // Folding from the peak puts the peak at the start of the profile, so
    // the quarter beat before it is the end of the profile.
    let start = peak_phase.round().max(0.0) as usize;
    if start >= wide_levels.len() {
        return 0.0;
    }
    let profile = fold_profile(&wide_levels[start..], period);
    let mut steepest = f64::MIN;
    let mut lead_bins = 0usize;
    for back in 0..=PHASE_BINS / 4 {
        let bin = (PHASE_BINS - back) % PHASE_BINS;
        let before = (bin + PHASE_BINS - 1) % PHASE_BINS;
        let rise = profile[bin] - profile[before];
        if rise > steepest {
            steepest = rise;
            lead_bins = back;
        }
    }
    lead_bins as f64 / PHASE_BINS as f64 * period
}

/// Which of the four beats of a bar the first beat is, as the offset from
/// zero to three at which the sound changes most from one beat to the next,
/// summed over every bar, given the level of each band in each beat.
fn downbeat_offset(levels: &[Vec<f64>]) -> usize {
    let beats = levels.first().map_or(0, Vec::len);
    let bar = BEATS_PER_BAR as usize;
    let mut novelty = vec![0.0f64; bar];
    for beat in 1..beats {
        let change: f64 = levels
            .iter()
            .map(|band| (band[beat] - band[beat - 1]).abs())
            .sum();
        novelty[beat % bar] += change;
    }
    (0..bar)
        .max_by(|&a, &b| novelty[a].total_cmp(&novelty[b]))
        .unwrap_or(0)
}

/// The level of each band in each beat, in decibels, for beats starting at
/// `phase` readings and spaced `period` readings apart.
fn beat_levels(audio: &Audio, phase: f64, period: f64) -> Vec<Vec<f64>> {
    let envelopes = band_envelopes(audio, &DOWNBEAT_BANDS, HOP);
    let readings = envelopes[0].len();
    let mut levels: Vec<Vec<f64>> = DOWNBEAT_BANDS.iter().map(|_| Vec::new()).collect();
    let mut start = phase;
    while start + period <= readings as f64 {
        let first = start.round() as usize;
        let last = (start + period).round() as usize;
        for (band, envelope) in envelopes.iter().enumerate() {
            let energy: f64 = envelope[first..last].iter().map(|v| v * v).sum();
            levels[band].push(decibels((energy / (last - first) as f64).sqrt()));
        }
        start += period;
    }
    levels
}

/// The beat analyzer described at the top of this module.
#[derive(Debug, Clone, Copy, Default)]
pub struct PulseGrid;

impl BeatAnalyzer for PulseGrid {
    fn name(&self) -> &str {
        "pulse"
    }

    fn analyze(&self, audio: &Audio) -> Result<BeatAnalysis, AnalysisError> {
        let (low_levels, wide_levels) = level_envelopes(audio);
        let (low_onsets, onsets) = onsets_of(&low_levels, &wide_levels);
        let Some((period, confidence)) = beat_period(&low_onsets, &onsets) else {
            return Err(AnalysisError::TooShort(audio.len().0));
        };
        if period <= 0.0 || !period.is_finite() || onsets.iter().all(|&value| value == 0.0) {
            return Err(AnalysisError::Failed(
                "no onsets to read a tempo from".to_owned(),
            ));
        }
        let rough_phase = beat_phase(&low_levels, period);
        let (period, peak_phase) = fit_lattice(&low_levels, period, rough_phase);
        let phase = (peak_phase - attack_lead(&wide_levels, period, peak_phase)).rem_euclid(period);
        let levels = beat_levels(audio, phase, period);
        let offset = downbeat_offset(&levels);

        let bpm = Bpm(60.0 * f64::from(SAMPLE_RATE) / (period * HOP as f64));
        let mut beats = Vec::new();
        let mut at = phase + offset as f64 * period;
        let end = audio.frames.len() as f64 / HOP as f64;
        while at < end {
            beats.push(Seconds(at * HOP as f64 / f64::from(SAMPLE_RATE)).to_samples());
            at += period;
        }
        if beats.len() < 2 {
            return Err(AnalysisError::TooShort(audio.len().0));
        }
        Ok(BeatAnalysis {
            bpm,
            beats,
            confidence,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kick_track_gets_its_tempo_to_a_hundredth_of_a_percent() {
        for bpm in [118.0f64, 132.5, 145.0] {
            let audio = dermixen_testkit::synth::kicks(Bpm(bpm), Seconds(0.1), Seconds(60.0));
            let found = PulseGrid.analyze(&audio).unwrap();
            assert!(
                (found.bpm.0 - bpm).abs() < bpm * 0.0001,
                "at {bpm} beats per minute the analyzer said {}",
                found.bpm.0
            );
            // Every beat lands within twenty milliseconds of a kick, which is
            // the width of a kick's attack.
            let period = 60.0 / bpm;
            for beat in &found.beats {
                let time = beat.to_seconds().0 - 0.1;
                let distance = (time / period - (time / period).round()).abs() * period;
                assert!(distance < 0.020, "beat at {time} is {distance} off");
            }
        }
    }

    #[test]
    fn silence_has_no_tempo() {
        let audio = dermixen_testkit::synth::silence(Seconds(30.0));
        assert!(PulseGrid.analyze(&audio).is_err());
    }

    #[test]
    fn a_short_clip_is_too_short() {
        let audio = dermixen_testkit::synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(2.0));
        assert_eq!(
            PulseGrid.analyze(&audio),
            Err(AnalysisError::TooShort(audio.len().0))
        );
    }
}
