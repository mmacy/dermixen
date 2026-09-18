//! Acceptance tests for the waveform overview. A coder agent makes these pass
//! without editing them.

use dermixen_core::{Bpm, Samples, Seconds};
use dermixen_media::{Audio, Overview, Peak};
use dermixen_testkit::synth;

fn peak(low: f32, high: f32) -> Peak {
    Peak { low, high }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.002
}

#[test]
fn silence_is_flat() {
    let overview = Overview::of(&synth::silence(Seconds(1.0)), Samples(441));
    assert_eq!(overview.bucket(), Samples(441));
    assert_eq!(overview.length(), Samples(44_100));
    assert_eq!(overview.peaks().len(), 100);
    assert!(overview.peaks().iter().all(|p| *p == peak(0.0, 0.0)));
}

#[test]
fn a_sine_peaks_at_its_amplitude_in_every_bucket_that_holds_a_cycle() {
    // A tenth of a second at 440 hertz holds forty-four cycles, so every
    // bucket reaches within a sample of the crest and the trough.
    let overview = Overview::of(&synth::sine(440.0, 0.5, Seconds(1.0)), Samples(4_410));
    assert_eq!(overview.peaks().len(), 10);
    for (index, p) in overview.peaks().iter().enumerate() {
        assert!(close(p.high, 0.5), "bucket {index} peaks at {}", p.high);
        assert!(close(p.low, -0.5), "bucket {index} dips to {}", p.low);
    }
}

#[test]
fn the_last_bucket_holds_the_remainder() {
    let mut audio = synth::silence(Seconds(1_000.0 / 44_100.0));
    audio.frames.truncate(1_000);
    assert_eq!(audio.len(), Samples(1_000));
    audio.frames[950] = [0.6, -0.2];
    let overview = Overview::of(&audio, Samples(300));
    assert_eq!(overview.length(), Samples(1_000));
    assert_eq!(
        overview.peaks().len(),
        4,
        "three whole buckets and one of a hundred frames"
    );
    assert_eq!(overview.peaks()[..3], [peak(0.0, 0.0); 3]);
    assert_eq!(overview.peaks()[3], peak(-0.2, 0.6));
}

#[test]
fn a_bucket_of_one_frame_is_the_extremes_of_that_frame() {
    let audio = Audio {
        frames: vec![[0.25, -0.5], [0.1, 0.9], [-0.7, -0.6]],
    };
    let overview = Overview::of(&audio, Samples(1));
    assert_eq!(
        overview.peaks(),
        [peak(-0.5, 0.25), peak(0.1, 0.9), peak(-0.7, -0.6)]
    );
    let same = Overview::of(&audio, Samples(0));
    assert_eq!(
        same.bucket(),
        Samples(1),
        "a bucket below one frame is one frame"
    );
    assert_eq!(same.peaks(), overview.peaks());
}

#[test]
fn an_empty_track_has_no_peaks() {
    let overview = Overview::of(&Audio::new(), Samples(100));
    assert_eq!(overview.length(), Samples::ZERO);
    assert!(overview.peaks().is_empty());
    assert_eq!(
        overview.peak_over(Samples::ZERO..Samples(50)),
        peak(0.0, 0.0)
    );
}

#[test]
fn the_peak_over_a_span_is_the_extreme_of_the_buckets_it_touches() {
    let mut audio = synth::silence(Seconds(1.0));
    audio.frames.truncate(1_000);
    audio.frames[250] = [0.7, 0.7];
    audio.frames[251] = [-0.3, 0.0];
    audio.frames[999] = [0.0, 0.4];
    let overview = Overview::of(&audio, Samples(100));
    assert_eq!(overview.peaks().len(), 10);
    assert_eq!(overview.peak_over(Samples(0)..Samples(100)), peak(0.0, 0.0));
    assert_eq!(
        overview.peak_over(Samples(200)..Samples(300)),
        peak(-0.3, 0.7)
    );
    // A span narrower than a bucket takes the whole bucket's peak.
    assert_eq!(
        overview.peak_over(Samples(210)..Samples(211)),
        peak(-0.3, 0.7)
    );
    // A span across a bucket boundary touches both buckets.
    assert_eq!(
        overview.peak_over(Samples(299)..Samples(301)),
        peak(-0.3, 0.7)
    );
    assert_eq!(
        overview.peak_over(Samples(300)..Samples(1_000)),
        peak(0.0, 0.4)
    );
    // An empty span, a span past the end, and a span before the start are silence.
    assert_eq!(
        overview.peak_over(Samples(250)..Samples(250)),
        peak(0.0, 0.0)
    );
    assert_eq!(
        overview.peak_over(Samples(1_000)..Samples(2_000)),
        peak(0.0, 0.0)
    );
    assert_eq!(
        overview.peak_over(Samples(-100)..Samples(0)),
        peak(0.0, 0.0)
    );
    // The part of a span outside the audio is ignored, not an error.
    assert_eq!(
        overview.peak_over(Samples(-100)..Samples(300)),
        peak(-0.3, 0.7)
    );
    assert_eq!(
        overview.peak_over(Samples(900)..Samples(5_000)),
        peak(0.0, 0.4)
    );
}

#[test]
fn kicks_show_as_a_row_of_peaks_that_decay_between_them() {
    // Kicks every half second, each a tenth of a second long: the bucket a
    // kick lands in is loud, and the bucket just before the next kick holds
    // only the silence between kicks.
    let overview = Overview::of(
        &synth::kicks(Bpm(120.0), Seconds::ZERO, Seconds(4.0)),
        Samples(4_410),
    );
    assert_eq!(overview.peaks().len(), 40);
    for kick in (0..40).step_by(5) {
        let loud = overview.peaks()[kick].high;
        let quiet = overview.peaks()[kick + 4].high;
        assert!(loud > 0.3, "bucket {kick} peaks at only {loud}");
        assert!(
            quiet < loud,
            "bucket {} at {quiet} is not quieter than the kick at {loud}",
            kick + 4
        );
    }
}
