//! The correction the timeline writes is read back by the scoreboard's own
//! loader, so the writer in the app crate and the reader in the analysis
//! crate cannot drift apart without this test failing.

use std::path::PathBuf;

use dermixen_analysis::{Source, load_anchor_truth};
use dermixen_app::write_correction;
use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Envelope, EqEnvelopes, Samples, Seconds, Track,
};
use dermixen_media::{WavDepth, write_wav};
use dermixen_testkit::synth;

#[test]
fn a_written_correction_is_read_back_by_the_ground_truth_loader() {
    let dir = tempfile::tempdir().unwrap();
    let track = Track {
        path: PathBuf::from("/music/goa/Etnica - Alpha.mp3"),
        hash: ContentHash([0xab; 32]),
        length: Samples(20 * 44_100),
        grid: BeatGrid {
            first_beat: Samples(22_050),
            bpm: Bpm(136.6689),
        },
        anchors: Anchors {
            intro: Beats(16.0),
            outro: Beats(256.0),
        },
        keylock: true,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    };
    let written = write_correction(dir.path(), &track).unwrap();
    // The link tool puts the audio beside the annotation under the
    // annotation's own name; here a short file stands in for it.
    let audio = written.with_extension("wav");
    write_wav(
        &audio,
        &synth::kicks(Bpm(136.6689), Seconds(0.5), Seconds(2.0)),
        WavDepth::Int16,
    )
    .unwrap();

    let truth = load_anchor_truth(dir.path()).unwrap();
    assert!(truth.without_audio.is_empty(), "{:?}", truth.without_audio);
    assert!(truth.warnings.is_empty(), "{:?}", truth.warnings);
    assert_eq!(truth.tracks.len(), 1);
    let read = &truth.tracks[0];
    assert_eq!(read.name, "Etnica - Alpha-abababab");
    assert_eq!(read.grid.bpm, Bpm(136.6689));
    assert_eq!(read.grid.first_beat, Samples(22_050));
    let intro = read.intro.as_ref().unwrap();
    let outro = read.outro.as_ref().unwrap();
    assert_eq!(intro.source, Source::Ear);
    assert_eq!(outro.source, Source::Ear);
    // Beat 16 at 136.6689 beats per minute is 7.024275 seconds after beat
    // zero at half a second, and beat 256 is 112.388407 seconds after it.
    assert!((intro.at.0 - 7.524275).abs() < 1e-6, "{}", intro.at.0);
    assert!((outro.at.0 - 112.888407).abs() < 1e-6, "{}", outro.at.0);
}
