//! Runs the beat analyzers over a ground-truth directory and prints the
//! beat scoreboard.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --features aubio --example tempo -- --giantsteps ~/.cache/dermixen/giantsteps/tempo
//! cargo run --release -p dermixen-analysis --features aubio --example tempo -- ~/.cache/dermixen/anchors
//! ```
//!
//! With `--giantsteps` the directory is a GiantSteps dataset checkout;
//! otherwise it is a directory in Dermixen's own layout. The aubio row
//! appears when the crate is built with the `aubio` feature. An optional
//! second number limits the run to that many tracks.

use std::path::PathBuf;

use dermixen_analysis::{BeatAnalyzer, PulseGrid, load_directory, load_giantsteps, run};

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let giantsteps = args.first().is_some_and(|arg| arg == "--giantsteps");
    if giantsteps {
        args.remove(0);
    }
    if args.is_empty() || args.len() > 2 {
        eprintln!("usage: tempo [--giantsteps] <directory> [track limit]");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[0]);
    let truth = if giantsteps {
        load_giantsteps(&dir)
    } else {
        load_directory(&dir)
    };
    let mut truth = match truth {
        Ok(truth) => truth,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    if let Some(limit) = args.get(1) {
        let limit: usize = limit.parse().expect("the track limit is a number");
        truth.truncate(limit);
    }
    let pulse = PulseGrid;
    #[cfg(feature = "aubio")]
    let aubio = dermixen_analysis::AubioBeats;
    let analyzers: Vec<&dyn BeatAnalyzer> = vec![
        &pulse,
        #[cfg(feature = "aubio")]
        &aubio,
    ];
    match run(&truth, &analyzers) {
        Ok(report) => {
            print!("{}", report.table());
            for row in &report.rows {
                println!("\n{}:", row.analyzer);
                for track in &row.per_track {
                    let bpm = track
                        .bpm
                        .map_or("failed".to_owned(), |bpm| format!("{:.3}", bpm.0));
                    let reference = truth
                        .iter()
                        .find(|entry| entry.name == track.name)
                        .and_then(|entry| entry.bpm)
                        .map_or("-".to_owned(), |bpm| format!("{:.3}", bpm.0));
                    println!(
                        "{}: {bpm} against {reference}, within four percent: {}, confidence {}",
                        track.name,
                        track.accuracy1.map_or("-".to_owned(), |ok| ok.to_string()),
                        track
                            .confidence
                            .map_or("-".to_owned(), |confidence| format!("{confidence:.2}"))
                    );
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
