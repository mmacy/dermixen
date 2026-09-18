//! Runs every phrase analyzer over a ground-truth directory and prints the
//! phrase scoreboard twice, once with the labeled grids as they are and
//! once with each grid's beat zero moved by up to three beats, then each
//! analyzer's result on every track from the first run.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --example phrases -- ~/.cache/dermixen/anchors
//! ```
//!
//! The directory holds `name.anchors` annotation files with audio beside
//! them; `docs/ground-truth.md` describes the labels and the metrics, and
//! `tools/anchor_truth.py` builds such a directory.

use std::path::PathBuf;

use dermixen_analysis::{
    CountedPhrases, GridHandling, PhraseAnalyzer, ShiftPhrases, load_anchor_truth, run_phrases,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: phrases <ground-truth directory>");
        std::process::exit(2);
    }
    let dir = PathBuf::from(&args[1]);
    let truth = match load_anchor_truth(&dir) {
        Ok(truth) => truth,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    for path in &truth.without_audio {
        eprintln!("no audio beside {}", path.display());
    }
    for warning in &truth.warnings {
        eprintln!("{warning}");
    }
    let counted = CountedPhrases;
    let shifts = ShiftPhrases;
    let analyzers: Vec<&dyn PhraseAnalyzer> = vec![&counted, &shifts];
    let mut details = String::new();
    for handling in [GridHandling::AsLabeled, GridHandling::Shifted] {
        match run_phrases(&truth.tracks, &analyzers, handling) {
            Ok(report) => {
                match handling {
                    GridHandling::AsLabeled => println!("grids as labeled:"),
                    GridHandling::Shifted => {
                        println!("\ngrids with beat zero moved by up to three beats:");
                    }
                }
                print!("{}", report.table());
                if handling == GridHandling::AsLabeled {
                    for analyzer in &analyzers {
                        details.push_str(&format!("\n{}:\n", analyzer.name()));
                        details.push_str(&report.details(analyzer.name()));
                    }
                }
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
    print!("{details}");
}
