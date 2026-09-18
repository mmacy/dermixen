//! Runs every anchor analyzer over a ground-truth directory and prints the
//! anchor scoreboard, then each analyzer's error on every track. Beside
//! the `edges` and `kick` analyzers it runs two experiments, `kick+8` and
//! `kick+16`, which move the `kick` anchors to the nearest eight-bar and
//! sixteen-bar phrase start the `shifts` phrase analyzer finds, so the
//! effect of snapping anchors to phrases can be read off the table.
//!
//! ```text
//! cargo run --release -p dermixen-analysis --example anchors -- ~/.cache/dermixen/anchors
//! ```
//!
//! The directory holds `name.anchors` annotation files with audio beside
//! them; `docs/ground-truth.md` describes the format and
//! `tools/anchor_truth.py` builds such a directory.

use std::path::PathBuf;

use dermixen_analysis::{
    AnchorAnalyzer, EdgeAnchors, KickAnchors, PhrasedAnchors, ShiftPhrases, load_anchor_truth,
    run_anchors,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: anchors <ground-truth directory>");
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
    let edges = EdgeAnchors;
    let kick = KickAnchors;
    let eight = PhrasedAnchors {
        name: "kick+8".to_owned(),
        anchors: KickAnchors,
        phrases: ShiftPhrases,
        bars: 8,
    };
    let sixteen = PhrasedAnchors {
        name: "kick+16".to_owned(),
        anchors: KickAnchors,
        phrases: ShiftPhrases,
        bars: 16,
    };
    let analyzers: Vec<&dyn AnchorAnalyzer> = vec![&edges, &kick, &eight, &sixteen];
    match run_anchors(&truth.tracks, &analyzers) {
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
