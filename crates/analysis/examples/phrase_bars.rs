//! Prints what the shift phrase analyzer measures in one track: the beat it
//! takes as the downbeat, the phase it settles on for each phrase length,
//! then one line per bar with the bar's first beat, its time, the level of
//! each band in decibels, the low band's rise and fall within a beat, and
//! the one-bar and four-bar shifts at that bar, marking the bars it
//! reports as phrase starts and section changes.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --example phrase_bars -- track.mp3 136.6689 0.863555
//! ```
//!
//! The second and third arguments are the grid: the tempo in beats per
//! minute and the time of beat zero in seconds.

use std::path::PathBuf;

use dermixen_analysis::shift_phrases::{ShiftPhrases, measure};
use dermixen_analysis::{PhraseAnalyzer, PhraseStart};
use dermixen_core::{BeatGrid, Bpm, Seconds};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: phrase_bars <audio> <bpm> <first beat in seconds>");
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
    let found = ShiftPhrases
        .analyze(&decoded.audio, &grid)
        .expect("the track is long enough");
    let measured = measure(&decoded.audio, &grid).expect("the track is long enough");
    println!(
        "downbeat at beat {} of the grid; confidence {:.2}",
        found.downbeat, found.confidence
    );
    println!("phase sums by bar, for eight-bar phrases:");
    for (phase, sum) in measured.eight_bar_sums.iter().enumerate() {
        println!("  bar {phase}: {sum:8.1} dB");
    }
    for (index, bar) in measured.bars.iter().enumerate() {
        let beat = measured.first_bar_beat + (index * 4) as i64;
        let start = found
            .phrases
            .iter()
            .find(|start: &&PhraseStart| start.at.0 as i64 == beat)
            .map_or(String::new(), |start| {
                format!("  starts {} bars", start.bars)
            });
        let section = if found.sections.iter().any(|at| at.0 as i64 == beat) {
            "  section"
        } else {
            ""
        };
        println!(
            "beat {beat:6}  {:8.2} s  low {:6.1}  mid {:6.1}  high {:6.1}  modulation {:5.1}  shift {:5.1}  four-bar shift {:5.1}{start}{section}",
            grid.time_of(dermixen_core::Beats(beat as f64)).0,
            bar[0],
            bar[1],
            bar[2],
            bar[3],
            measured.one_bar_shift[index],
            measured.four_bar_shift[index],
        );
    }
}
