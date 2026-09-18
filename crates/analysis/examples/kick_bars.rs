//! Prints what the kick analyzer measures in every bar of one track: the
//! bar's first beat, its time, its low-band level in decibels, and how far
//! that level rises and falls within a beat.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --example kick_bars -- track.mp3 136.6689 0.863555
//! ```
//!
//! The second and third arguments are the grid: the tempo in beats per
//! minute and the time of beat zero in seconds.

use std::path::PathBuf;

use dermixen_analysis::kick_anchors::{low_band_envelope, measure_bars};
use dermixen_core::{BeatGrid, Bpm, Seconds};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: kick_bars <audio> <bpm> <first beat in seconds>");
        std::process::exit(2);
    }
    let bpm: f64 = args[2].parse().expect("the tempo is a number");
    let first_beat: f64 = args[3]
        .parse()
        .expect("the first beat is a number of seconds");
    let grid = BeatGrid {
        first_beat: Seconds(first_beat).to_samples(),
        bpm: Bpm(bpm),
    };
    let decoded = dermixen_media::decode(&PathBuf::from(&args[1])).expect("the audio decodes");
    let envelope = low_band_envelope(&decoded.audio);
    for bar in measure_bars(&envelope, &grid) {
        println!(
            "beat {:6.0}  {:8.2} s  level {:6.1} dB  modulation {:5.1} dB",
            bar.beat.0,
            grid.time_of(bar.beat).0,
            bar.level,
            bar.modulation
        );
    }
}
