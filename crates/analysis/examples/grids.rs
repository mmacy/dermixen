//! Runs the beat analyzers over an anchor ground-truth directory and prints
//! how each one's tempo and beat zero compare with the labeled grids, then
//! every track's result.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --features aubio --example grids -- ~/.cache/dermixen/anchors
//! ```
//!
//! The aubio row appears when the crate is built with the `aubio` feature.

use std::path::PathBuf;

use dermixen_analysis::{BeatAnalyzer, PulseGrid, load_anchor_truth, run_grids};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: grids <ground-truth directory>");
        std::process::exit(2);
    }
    let truth = match load_anchor_truth(&PathBuf::from(&args[1])) {
        Ok(truth) => truth,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    for warning in &truth.warnings {
        eprintln!("{warning}");
    }
    let pulse = PulseGrid;
    #[cfg(feature = "aubio")]
    let aubio = dermixen_analysis::AubioBeats;
    let analyzers: Vec<&dyn BeatAnalyzer> = vec![
        &pulse,
        #[cfg(feature = "aubio")]
        &aubio,
    ];
    match run_grids(&truth.tracks, &analyzers) {
        Ok(report) => {
            print!("{}", report.table());
            for analyzer in &analyzers {
                println!("\n{}:", analyzer.name());
                print!("{}", report.details(analyzer.name()));
            }
        }
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
