//! Prints how well one track's onsets line up with themselves at fractions
//! and multiples of a given beat period, which is what the pulse analyzer
//! chooses its tempo from.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --example pulse_lags -- track.mp3 136.669
//! ```

use std::path::PathBuf;

use dermixen_analysis::pulse::{HOP, autocorrelation, onset_strength};
use dermixen_core::SAMPLE_RATE;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: pulse_lags <audio> <bpm>");
        std::process::exit(2);
    }
    let bpm: f64 = args[2].parse().expect("the tempo is a number");
    let decoded = dermixen_media::decode(&PathBuf::from(&args[1])).expect("the audio decodes");
    let (low_onsets, onsets) = onset_strength(&decoded.audio);
    let period = 60.0 / bpm * f64::from(SAMPLE_RATE) / HOP as f64;
    let score = |lag: usize| {
        autocorrelation(&onsets, lag)
            + 0.5 * autocorrelation(&onsets, 2 * lag)
            + 0.25 * autocorrelation(&onsets, 4 * lag)
    };
    for beats in [0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 8.0] {
        let lag = (beats * period).round() as usize;
        println!(
            "{beats:5.2} beats = {lag:4} readings: correlation {:8.3}  score {:8.3}  low band {:8.3}",
            autocorrelation(&onsets, lag),
            score(lag),
            autocorrelation(&low_onsets, lag)
        );
    }
    // The tempo range the analyzer searches: sixty to two hundred beats per minute.
    let readings_per_second = f64::from(SAMPLE_RATE) / HOP as f64;
    let min_lag = (60.0 / 200.0 * readings_per_second).floor() as usize;
    let max_lag = readings_per_second.ceil() as usize;
    let best = (min_lag..=max_lag)
        .max_by(|&a, &b| score(a).total_cmp(&score(b)))
        .unwrap();
    println!(
        "best lag in the tempo range: {best} readings = {:.3} beats per minute, score {:.3}",
        60.0 * f64::from(SAMPLE_RATE) / (best as f64 * HOP as f64),
        score(best)
    );
}
