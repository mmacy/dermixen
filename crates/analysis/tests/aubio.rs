//! Acceptance tests for the aubio baseline. A coder agent makes these pass without editing them.
//!
//! These compile only with the `aubio` feature:
//!
//! ```text
//! cargo test -p dermixen-analysis --test aubio --features aubio -- --include-ignored
//! ```

#![cfg(feature = "aubio")]

use dermixen_analysis::{AubioBeats, BeatAnalyzer, beat_f_measure, tempo_accuracy1};
use dermixen_core::{Bpm, Seconds};
use dermixen_testkit::synth;

#[test]
fn aubio_finds_the_tempo_and_beats_of_a_kick_track() {
    let analyzer = AubioBeats;
    assert_eq!(analyzer.name(), "aubio");
    for bpm in [120.0, 130.0, 140.0] {
        let audio = synth::kicks(Bpm(bpm), Seconds(0.1), Seconds(20.0));
        let result = analyzer.analyze(&audio).unwrap();
        assert!(
            tempo_accuracy1(result.bpm, Bpm(bpm)),
            "at {bpm} BPM aubio said {}",
            result.bpm.0
        );
        let reference: Vec<Seconds> = (0..)
            .map(|n| Seconds(0.1 + f64::from(n) * 60.0 / bpm))
            .take_while(|t| t.0 < 20.0)
            .collect();
        let estimate: Vec<Seconds> = result.beats.iter().map(|s| s.to_seconds()).collect();
        // The first beat or two may be missed while the tracker settles.
        let f = beat_f_measure(&estimate[..], &reference[2..], Seconds(0.07));
        assert!(f > 0.85, "at {bpm} BPM the beat F-measure is {f}");
        assert!((0.0..=1.0).contains(&result.confidence));
    }
}

#[test]
fn aubio_does_not_panic_on_silence_or_a_tone() {
    let analyzer = AubioBeats;
    let _ = analyzer.analyze(&synth::silence(Seconds(5.0)));
    let _ = analyzer.analyze(&synth::sine(440.0, 0.5, Seconds(5.0)));
    let _ = analyzer.analyze(&synth::silence(Seconds(0.01)));
}
