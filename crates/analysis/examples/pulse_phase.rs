//! Prints the low-band level of one track folded onto one beat of a given
//! grid, so the kick's place within the beat can be seen, along with where
//! the pulse analyzer puts its first beat on that grid.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --example pulse_phase -- track.mp3 149.026 1.527321
//! ```

use std::path::PathBuf;

use dermixen_analysis::pulse::{HOP, PHASE_BINS, fold_profile, level_envelopes};
use dermixen_analysis::{BeatAnalyzer, PulseGrid};
use dermixen_core::{BeatGrid, Bpm, SAMPLE_RATE, Seconds};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: pulse_phase <audio> <bpm> <first beat in seconds>");
        std::process::exit(2);
    }
    let bpm: f64 = args[2].parse().expect("the tempo is a number");
    let first_beat: f64 = args[3]
        .parse()
        .expect("the first beat is a number of seconds");
    let decoded = dermixen_media::decode(&PathBuf::from(&args[1])).expect("the audio decodes");
    let (low, _wide) = level_envelopes(&decoded.audio);
    let period = 60.0 / bpm * f64::from(SAMPLE_RATE) / HOP as f64;
    // Folding from the labeled beat zero puts that beat at the start of the profile.
    let start = (first_beat * f64::from(SAMPLE_RATE) / HOP as f64).round() as usize;
    let profile = fold_profile(&low[start.min(low.len())..], period);
    println!(
        "low-band level in decibels across one beat of the given grid, in {PHASE_BINS} steps:"
    );
    for (bin, level) in profile.iter().enumerate() {
        println!(
            "  {:.3} beat  {level:6.1} dB",
            bin as f64 / PHASE_BINS as f64
        );
    }
    let grid = BeatGrid {
        first_beat: Seconds(first_beat).to_samples(),
        bpm: Bpm(bpm),
    };
    match PulseGrid.analyze(&decoded.audio) {
        Ok(found) => {
            println!(
                "pulse: {:.3} beats per minute, first beat at labeled beat {:.3}",
                found.bpm.0,
                grid.beat_at_position(found.beats[0]).0
            );
            let pulse_period = 60.0 / found.bpm.0 * f64::from(SAMPLE_RATE) / HOP as f64;
            let pulse_start = (found.beats[0].0 as f64 / HOP as f64).round() as usize;
            let profile = fold_profile(&low[pulse_start.min(low.len())..], pulse_period);
            println!("low-band level across one beat of the pulse grid, from its first beat:");
            for (bin, level) in profile.iter().enumerate().step_by(4) {
                println!(
                    "  {:.3} beat  {level:6.1} dB",
                    bin as f64 / PHASE_BINS as f64
                );
            }
        }
        Err(error) => println!("pulse failed: {error}"),
    }
}
