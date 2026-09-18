//! Decodes one audio file the way the app does and writes the result as a
//! 32-bit float WAV. Other programs, such as the Python probes used to
//! check the analyzers, then measure exactly the samples the analyzers see,
//! with the same decoder handling of MP3 padding and delay.
//!
//! ```text
//! cargo run -p dermixen-analysis --example decode -- input.mp3 output.wav
//! ```

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: decode <input audio> <output wav>");
        std::process::exit(2);
    }
    let input = PathBuf::from(&args[1]);
    let output = PathBuf::from(&args[2]);
    let decoded = match dermixen_media::decode(&input) {
        Ok(decoded) => decoded,
        Err(error) => {
            eprintln!("cannot decode {}: {error}", input.display());
            std::process::exit(1);
        }
    };
    if let Err(error) =
        dermixen_media::write_wav(&output, &decoded.audio, dermixen_media::WavDepth::Float32)
    {
        eprintln!("cannot write {}: {error}", output.display());
        std::process::exit(1);
    }
    println!("{} frames", decoded.audio.frames.len());
}
