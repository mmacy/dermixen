//! Plays two seconds of a 440 hertz tone through the machine's own audio
//! device, which is the one part of a preview that no automated test can
//! reach.
//!
//! The acceptance tests in `tests/preview.rs` hold the preview to the
//! render's own frames, but they pull those frames through a capturing
//! output that stands in for a device. This example is how a person checks
//! the rest: that a real device takes the engine's 44.1 kHz stereo stream,
//! and that what comes out of the speakers is the tone the engine sent. Run
//! it, and a steady tone should sound for two seconds, followed by a line
//! saying how many frames the device pulled and how many times it asked for
//! frames the render had not made yet.
//!
//! ```text
//! cargo run -p dermixen-engine --features playback --example preview_tone
//! ```

#[cfg(feature = "playback")]
fn main() {
    use std::path::PathBuf;

    use dermixen_core::{
        Anchors, BeatGrid, Beats, Bpm, ContentHash, Envelope, EqEnvelopes, Mix, SAMPLE_RATE,
        Samples, Seconds, Track,
    };
    use dermixen_engine::{CpalOutput, Resampler, Source, TimeStretcher, play};
    use dermixen_testkit::synth;

    let tone = synth::sine(440.0, 0.3, Seconds(2.0));
    let track = Track {
        path: PathBuf::from("a 440 hertz tone"),
        hash: ContentHash([0; 32]),
        length: tone.len(),
        // The tone has no beats in it, so the grid and the anchors are only
        // there to place the one track on the timeline at its own tempo.
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        },
        anchors: Anchors {
            intro: Beats::ZERO,
            outro: Beats(4.0),
        },
        keylock: false,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    };
    let mix = Mix {
        tracks: vec![track],
    };

    let mut output = match CpalOutput::open(None) {
        Ok(output) => output,
        Err(message) => {
            eprintln!("the audio device could not be opened: {message}");
            std::process::exit(1);
        }
    };

    println!("playing two seconds of a 440 hertz tone through the default output device");
    let played = play(
        &mix,
        Samples::ZERO..Samples(i64::MAX),
        &mut |_, _| Ok(Box::new(tone.clone()) as Box<dyn Source>),
        &mut |_| Box::new(Resampler::new()) as Box<dyn TimeStretcher>,
        &mut output,
        // A lookahead as long as the tone itself, so the whole tone sits in
        // the feed before the device begins to play it.
        Samples(2 * i64::from(SAMPLE_RATE)),
        &mut |_| {},
    );
    match played {
        Ok(report) => println!(
            "played {} frames, {:.3} seconds, with {} underruns",
            report.played.0,
            report.played.to_seconds().0,
            report.underruns
        ),
        Err(error) => {
            eprintln!("the tone could not be played: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "playback"))]
fn main() {
    eprintln!(
        "this example needs the audio output, which is behind the playback feature: \
         cargo run -p dermixen-engine --features playback --example preview_tone"
    );
    std::process::exit(1);
}
