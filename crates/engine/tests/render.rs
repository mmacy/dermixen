//! Acceptance tests for the offline render. A coder agent makes these pass without editing them.
//!
//! Every mix here is built from synthetic audio, so each expectation is a
//! number that follows from the document rather than from a recording.

use std::path::{Path, PathBuf};

use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Envelope, EnvelopeNode, EqEnvelopes, Mix,
    Samples, Seconds, TempoNode, Track, beatmix,
};
use dermixen_engine::{RenderError, Resampler, TimeStretcher, render};
use dermixen_media::{Audio, Frame};
use dermixen_testkit::spectrum::{band_level_db, dominant_frequency};
use dermixen_testkit::synth;

fn envelope(nodes: &[(f64, f64)]) -> Envelope {
    Envelope::from_nodes(
        nodes
            .iter()
            .map(|(beat, db)| EnvelopeNode {
                at: Beats(*beat),
                value: Decibels(*db),
            })
            .collect(),
    )
    .unwrap()
}

/// A track whose grid starts at its first sample.
fn track(audio: &Audio, bpm: f64, intro: f64, outro: f64, tempo: &[(f64, f64)]) -> Track {
    Track {
        path: PathBuf::from("synthetic"),
        hash: ContentHash([0; 32]),
        length: audio.len(),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(bpm),
        },
        anchors: Anchors {
            intro: Beats(intro),
            outro: Beats(outro),
        },
        keylock: false,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: tempo
            .iter()
            .map(|(beat, bpm)| TempoNode {
                at: Beats(*beat),
                bpm: Bpm(*bpm),
            })
            .collect(),
    }
}

fn resamplers() -> impl FnMut(&Track) -> Box<dyn TimeStretcher> {
    |_| Box::new(Resampler::new())
}

fn rms(frames: &[Frame]) -> f64 {
    let sum: f64 = frames
        .iter()
        .map(|f| f64::from(f[0]) * f64::from(f[0]))
        .sum();
    (sum / frames.len() as f64).sqrt()
}

fn max_abs(frames: &[Frame]) -> f32 {
    frames
        .iter()
        .map(|f| f[0].abs().max(f[1].abs()))
        .fold(0.0, f32::max)
}

fn close(a: f64, b: f64, relative: f64) -> bool {
    (a - b).abs() <= relative * b.abs()
}

/// The frames of a span of beats of a track that plays at its own tempo from time zero.
fn beat_span(bpm: f64, from_beat: f64, to_beat: f64) -> std::ops::Range<usize> {
    let period = 60.0 / bpm * 44_100.0;
    (from_beat * period).round() as usize..(to_beat * period).round() as usize
}

#[test]
fn an_empty_mix_renders_to_nothing() {
    let out = render(&Mix::new(), &[], &mut resamplers()).unwrap();
    assert!(out.is_empty());
}

#[test]
fn sources_are_checked_against_the_document() {
    let audio = synth::sine(440.0, 0.5, Seconds(1.0));
    let mix = Mix {
        tracks: vec![track(&audio, 130.0, 0.0, 16.0, &[])],
    };
    assert_eq!(
        render(&mix, &[], &mut resamplers()),
        Err(RenderError::SourceCount {
            tracks: 1,
            sources: 0
        })
    );
    let short = synth::sine(440.0, 0.5, Seconds(0.5));
    assert_eq!(
        render(&mix, &[short], &mut resamplers()),
        Err(RenderError::SourceLength {
            index: 0,
            expected: 44_100,
            got: 22_050
        })
    );
}

#[test]
fn a_track_at_its_own_tempo_plays_through_unchanged_in_level_and_pitch() {
    let audio = synth::sine(440.0, 0.5, Seconds(3.0));
    let mix = Mix {
        tracks: vec![track(&audio, 130.0, 0.0, 16.0, &[])],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert!(
        (out.len().0 - audio.len().0).abs() <= 1,
        "length {}",
        out.len().0
    );
    let middle = &out.frames[22_050..110_250];
    assert!(close(
        rms(middle),
        rms(&audio.frames[22_050..110_250]),
        0.02
    ));
    let pitch = dominant_frequency(middle, 0);
    assert!((pitch - 440.0).abs() < 1.0, "pitch {pitch}");
    // The EQ's crossover network rings briefly when a tone starts in one
    // sample, so the peak is measured once that has died away.
    assert!(max_abs(middle) < 0.55);
}

#[test]
fn the_volume_envelope_scales_the_track() {
    let audio = synth::sine(440.0, 0.5, Seconds(3.0));
    let mut quiet = track(&audio, 130.0, 0.0, 16.0, &[]);
    quiet.volume = envelope(&[(0.0, -6.0206)]);
    let mix = Mix {
        tracks: vec![quiet],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    let expected = rms(&audio.frames[22_050..110_250]) / 2.0;
    assert!(close(rms(&out.frames[22_050..110_250]), expected, 0.02));

    let mut silent = track(&audio, 130.0, 0.0, 16.0, &[]);
    silent.volume = envelope(&[(0.0, -90.0)]);
    let mix = Mix {
        tracks: vec![silent],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert!(max_abs(&out.frames) < 1e-6);
}

#[test]
fn the_gain_scales_the_track_on_top_of_its_volume_envelope() {
    let audio = synth::sine(440.0, 0.5, Seconds(3.0));
    let middle = 22_050..110_250;
    let source_level = rms(&audio.frames[middle.clone()]);

    // Six decibels down is half.
    let mut leveled = track(&audio, 130.0, 0.0, 16.0, &[]);
    leveled.gain = Decibels(-6.0206);
    let mix = Mix {
        tracks: vec![leveled],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert!(close(
        rms(&out.frames[middle.clone()]),
        source_level / 2.0,
        0.02
    ));

    // The gain and the envelope add: six down and six down is a quarter.
    let mut both = track(&audio, 130.0, 0.0, 16.0, &[]);
    both.gain = Decibels(-6.0206);
    both.volume = envelope(&[(0.0, -6.0206)]);
    let mix = Mix { tracks: vec![both] };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert!(close(
        rms(&out.frames[middle.clone()]),
        source_level / 4.0,
        0.02
    ));

    // A gain may raise a track: six up is twice.
    let mut raised = track(&audio, 130.0, 0.0, 16.0, &[]);
    raised.gain = Decibels(6.0206);
    let mix = Mix {
        tracks: vec![raised],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert!(close(rms(&out.frames[middle]), source_level * 2.0, 0.02));
}

#[test]
fn the_volume_envelope_alone_decides_silence() {
    // A track faded to the floor contributes nothing however much gain it
    // has, which is the same rule the transport uses to tell a silent track
    // from a sounding one.
    let audio = synth::sine(440.0, 0.5, Seconds(3.0));
    let mut silent = track(&audio, 130.0, 0.0, 16.0, &[]);
    silent.volume = envelope(&[(0.0, -90.0)]);
    silent.gain = Decibels(20.0);
    let mix = Mix {
        tracks: vec![silent],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert!(max_abs(&out.frames) < 1e-6);
}

#[test]
fn a_fade_follows_the_envelope_beat_by_beat() {
    let bpm = 130.0;
    let audio = synth::sine(440.0, 0.5, Seconds(30.0));
    let mut fading = track(&audio, bpm, 0.0, 16.0, &[]);
    fading.volume = envelope(&[(16.0, 0.0), (48.0, -90.0)]);
    let mix = Mix {
        tracks: vec![fading],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    let full = rms(&audio.frames[beat_span(bpm, 4.0, 12.0)]);
    assert!(close(
        rms(&out.frames[beat_span(bpm, 4.0, 12.0)]),
        full,
        0.02
    ));
    // Halfway through the fade the level is minus forty-five decibels.
    let halfway = rms(&out.frames[beat_span(bpm, 31.5, 32.5)]);
    let halfway_db = 20.0 * (halfway / full).log10();
    assert!(
        (halfway_db + 45.0).abs() < 2.0,
        "halfway level {halfway_db} dB"
    );
    // A quarter of the way it is about minus twenty-two and a half.
    let quarter = rms(&out.frames[beat_span(bpm, 23.5, 24.5)]);
    let quarter_db = 20.0 * (quarter / full).log10();
    assert!(
        (quarter_db + 22.5).abs() < 2.0,
        "quarter level {quarter_db} dB"
    );
    // Past the end of the fade there is nothing.
    assert!(max_abs(&out.frames[beat_span(bpm, 50.0, 60.0)]) < 1e-6);
}

#[test]
fn eq_envelopes_shape_each_band() {
    let bass = synth::sine(100.0, 0.5, Seconds(4.0));
    let flat = Mix {
        tracks: vec![track(&bass, 130.0, 0.0, 16.0, &[])],
    };
    let reference = render(&flat, std::slice::from_ref(&bass), &mut resamplers()).unwrap();
    let mut killed = track(&bass, 130.0, 0.0, 16.0, &[]);
    killed.eq.low = envelope(&[(0.0, -90.0)]);
    let mix = Mix {
        tracks: vec![killed],
    };
    let out = render(&mix, std::slice::from_ref(&bass), &mut resamplers()).unwrap();
    let drop = band_level_db(&out.frames, 0, 90.0, 110.0)
        - band_level_db(&reference.frames, 0, 90.0, 110.0);
    assert!(drop < -40.0, "the 100 Hz tone only dropped by {drop} dB");

    let tone = synth::sine(1000.0, 0.5, Seconds(4.0));
    let flat = Mix {
        tracks: vec![track(&tone, 130.0, 0.0, 16.0, &[])],
    };
    let reference = render(&flat, std::slice::from_ref(&tone), &mut resamplers()).unwrap();
    let mut killed = track(&tone, 130.0, 0.0, 16.0, &[]);
    killed.eq.low = envelope(&[(0.0, -90.0)]);
    killed.eq.high = envelope(&[(0.0, -90.0)]);
    let mix = Mix {
        tracks: vec![killed],
    };
    let out = render(&mix, std::slice::from_ref(&tone), &mut resamplers()).unwrap();
    let change = band_level_db(&out.frames, 0, 950.0, 1050.0)
        - band_level_db(&reference.frames, 0, 950.0, 1050.0);
    assert!(change.abs() < 1.0, "the 1 kHz tone changed by {change} dB");
}

#[test]
fn a_track_is_stretched_to_the_mix_tempo() {
    let audio = synth::sine(440.0, 0.5, Seconds(3.0));
    // The curve holds 143 BPM from the start, ten percent above the track's own 130.
    let mix = Mix {
        tracks: vec![track(&audio, 130.0, 0.0, 16.0, &[(0.0, 143.0)])],
    };
    let out = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    let expected = (3.0_f64 / 1.1 * 44_100.0).round() as i64;
    assert!(
        (out.len().0 - expected).abs() <= 90,
        "length {} expected {expected}",
        out.len().0
    );
    let middle = &out.frames[20_000..100_000];
    let pitch = dominant_frequency(middle, 0);
    assert!(
        (pitch - 484.0).abs() < 2.0,
        "pitch {pitch}, expected 484 from a resampler"
    );
    assert!(close(
        rms(middle),
        rms(&audio.frames[20_000..100_000]),
        0.05
    ));
}

#[test]
fn rendering_is_deterministic() {
    let audio = synth::white_noise(9, 0.3, Seconds(2.0));
    let mut noisy = track(&audio, 130.0, 0.0, 16.0, &[(0.0, 137.0)]);
    noisy.volume = envelope(&[(0.0, 0.0), (32.0, -20.0)]);
    noisy.eq.high = envelope(&[(0.0, -6.0)]);
    let mix = Mix {
        tracks: vec![noisy],
    };
    let first = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    let second = render(&mix, std::slice::from_ref(&audio), &mut resamplers()).unwrap();
    assert_eq!(first, second);
}

/// Two kick tracks at different tempos, joined by the product's own default
/// transition.
///
/// Track A at 130 BPM has its outro anchor at `outro_a`; track B at 140 BPM
/// has its intro anchor at `intro_b`, so B's beat zero is mix beat
/// `outro_a - intro_b` and the two tracks overlap from there until A ends.
/// `beatmix` writes the tempo ramp and the two fades across `bars` bars from
/// the aligned anchors, so what renders here is exactly the transition the
/// mix commands write.
fn two_kicks(seconds: f64, outro_a: f64, intro_b: f64, bars: u32) -> (Mix, Vec<Audio>) {
    let a = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(seconds));
    let b = synth::kicks(Bpm(140.0), Seconds::ZERO, Seconds(seconds));
    let mut track_a = track(&a, 130.0, 0.0, outro_a, &[]);
    // Nothing follows track B, so its outro anchor only needs to be a
    // sensible beat inside the track: its last whole beat.
    let outro_b = (seconds * 140.0 / 60.0).floor();
    let mut track_b = track(&b, 140.0, intro_b, outro_b, &[]);
    beatmix(&mut track_a, &mut track_b, bars);
    (
        Mix {
            tracks: vec![track_a, track_b],
        },
        vec![a, b],
    )
}

/// The same mix with every track but one held at silence, so that one can be
/// measured on its own without moving anything on the timeline.
fn solo(mix: &Mix, keep: usize) -> Mix {
    let mut solo = mix.clone();
    for (i, track) in solo.tracks.iter_mut().enumerate() {
        if i != keep {
            track.volume = envelope(&[(0.0, -90.0)]);
        }
    }
    solo
}

/// The loudest frame within `half_window` frames either side of `centre`.
fn peak_near(frames: &[Frame], centre: i64, half_window: i64) -> i64 {
    let from = (centre - half_window).max(0) as usize;
    let to = ((centre + half_window) as usize).min(frames.len());
    from as i64
        + frames[from..to]
            .iter()
            .enumerate()
            .max_by(|x, y| x.1[0].abs().total_cmp(&y.1[0].abs()))
            .map(|(i, _)| i as i64)
            .unwrap()
}

#[test]
fn two_kick_tracks_line_up_across_the_overlap() {
    // Track B's beat zero is mix beat 48, and the tempo ramps over mix beats
    // 64 to 96 while A fades out and B fades in.
    let (mix, sources) = two_kicks(40.0, 64.0, 16.0, 8);
    let timeline = mix.timeline().unwrap();
    let curve = &timeline.curve;
    let out = render(&mix, &sources, &mut resamplers()).unwrap();
    let start = timeline.start();
    let expected_len = (timeline.end() - start).to_samples().0;
    assert!(
        (out.len().0 - expected_len).abs() <= 1,
        "length {}",
        out.len().0
    );

    // Where a kick's loudest sample falls relative to its beat, measured on the
    // first track alone, so that whatever the EQ does to a kick's shape is
    // taken into account.
    let only_a = render(&solo(&mix, 0), &sources, &mut resamplers()).unwrap();
    let only_b = render(&solo(&mix, 1), &sources, &mut resamplers()).unwrap();
    let beat_four = (curve.time_at(Beats(4.0)) - start).to_samples().0;
    let offset = peak_near(&only_a.frames, beat_four, 4_000) - beat_four;
    assert!((0..=1_000).contains(&offset), "kick peak offset {offset}");

    // Every mix beat from the second to the last full beat of track B has a
    // kick within a millisecond of where the curve says the beat is, and
    // where both tracks are audible (B from mix beat 64, when its fade in
    // leaves the silence floor, and A until it ends near mix beat 86) each
    // track's own kick, rendered alone on the same timeline, lands within a
    // millisecond of the other's.
    let last_beat = 48.0 + (40.0_f64 * 140.0 / 60.0).floor() - 1.0;
    let mut checked = 0;
    let mut beat = 1.0;
    while beat <= last_beat {
        let expected = (curve.time_at(Beats(beat)) - start).to_samples().0 + offset;
        let peak = peak_near(&out.frames, expected, 8_000);
        assert!(
            (peak - expected).abs() <= 44,
            "beat {beat}: kick at {peak}, expected {expected}, off by {} frames",
            peak - expected
        );
        if (66.0..=85.0).contains(&beat) {
            let kick_a = peak_near(&only_a.frames, expected, 8_000);
            let kick_b = peak_near(&only_b.frames, expected, 8_000);
            assert!(
                (kick_a - kick_b).abs() <= 44,
                "beat {beat}: track A's kick at {kick_a} and track B's at {kick_b} are {} frames apart",
                (kick_a - kick_b).abs()
            );
            assert!(
                (kick_b - expected).abs() <= 44,
                "beat {beat}: track B's kick at {kick_b}, expected {expected}"
            );
        }
        checked += 1;
        beat += 1.0;
    }
    assert!(checked > 130);
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/golden/two-kicks.wav")
}

#[test]
fn the_golden_render_has_not_drifted() {
    // Six-second tracks with the transition early enough to fall inside them:
    // B's beat zero is mix beat 4, and the tempo ramps over mix beats 8 to 16
    // while A fades out and B fades in.
    let (mix, sources) = two_kicks(6.0, 8.0, 4.0, 2);
    let out = render(&mix, &sources, &mut resamplers()).unwrap();
    let path = golden_path();
    let to_i16 = |v: f32| (f64::from(v) * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
    if std::env::var_os("DERMIXEN_UPDATE_GOLDEN").is_some() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for frame in &out.frames {
            writer.write_sample(to_i16(frame[0])).unwrap();
            writer.write_sample(to_i16(frame[1])).unwrap();
        }
        writer.finalize().unwrap();
        eprintln!("wrote {}", path.display());
        return;
    }
    let mut reader = hound::WavReader::open(&path).unwrap_or_else(|e| {
        panic!(
            "no golden render at {}: {e}. Render it with DERMIXEN_UPDATE_GOLDEN=1, listen, and commit it.",
            path.display()
        )
    });
    let golden: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    assert_eq!(
        golden.len(),
        out.frames.len() * 2,
        "the render is a different length"
    );
    let mut worst = 0i32;
    for (frame, pair) in out.frames.iter().zip(golden.chunks(2)) {
        worst = worst.max((i32::from(to_i16(frame[0])) - i32::from(pair[0])).abs());
        worst = worst.max((i32::from(to_i16(frame[1])) - i32::from(pair[1])).abs());
    }
    assert!(
        worst <= 4,
        "the render differs from the golden file by up to {worst} steps of 32768"
    );
}

/// The pitch-preserving path: keylock is on by default, so the first render a
/// person hears goes through Signalsmith, and it must hold the same promises
/// the resampler does. These compile only with the `signalsmith` feature.
#[cfg(feature = "signalsmith")]
mod keylock {
    use super::*;
    use dermixen_engine::SignalsmithStretcher;

    fn keylocked() -> impl FnMut(&Track) -> Box<dyn TimeStretcher> {
        |_| Box::new(SignalsmithStretcher::new())
    }

    #[test]
    fn keylock_keeps_the_two_tracks_together_across_the_overlap() {
        let (mix, sources) = two_kicks(40.0, 64.0, 16.0, 8);
        let timeline = mix.timeline().unwrap();
        let curve = &timeline.curve;
        let start = timeline.start();
        let only_a = render(&solo(&mix, 0), &sources, &mut keylocked()).unwrap();
        let only_b = render(&solo(&mix, 1), &sources, &mut keylocked()).unwrap();
        let beat_four = (curve.time_at(Beats(4.0)) - start).to_samples().0;
        let offset = peak_near(&only_a.frames, beat_four, 4_000) - beat_four;
        // Where both tracks are audible, each track's kick, rendered alone on
        // the same timeline through the pitch-preserving stretcher, lands
        // within a millisecond of the other's.
        let mut beat = 66.0;
        while beat <= 85.0 {
            let expected = (curve.time_at(Beats(beat)) - start).to_samples().0 + offset;
            let kick_a = peak_near(&only_a.frames, expected, 8_000);
            let kick_b = peak_near(&only_b.frames, expected, 8_000);
            assert!(
                (kick_a - kick_b).abs() <= 44,
                "beat {beat}: track A's kick at {kick_a} and track B's at {kick_b} are {} frames apart",
                (kick_a - kick_b).abs()
            );
            beat += 1.0;
        }
    }

    #[test]
    fn keylock_rendering_is_deterministic() {
        let (mix, sources) = two_kicks(12.0, 8.0, 4.0, 2);
        let first = render(&mix, &sources, &mut keylocked()).unwrap();
        let second = render(&mix, &sources, &mut keylocked()).unwrap();
        assert_eq!(first, second);
    }
}

mod streaming {
    //! Acceptance tests for the streaming render. A coder agent makes these pass
    //! without editing them.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use dermixen_engine::{BLOCK_FRAMES, Progress, Source, render_to};

    /// A source that keeps count of how many of its kind are alive, so a
    /// test can see when the render lets go of a track.
    struct Counted {
        audio: Audio,
        live: Rc<Cell<usize>>,
    }

    impl Counted {
        fn boxed(audio: Audio, live: &Rc<Cell<usize>>) -> Box<dyn Source> {
            live.set(live.get() + 1);
            Box::new(Counted {
                audio,
                live: Rc::clone(live),
            })
        }
    }

    impl Source for Counted {
        fn frames(&self) -> &[Frame] {
            &self.audio.frames
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }

    /// Collects every block the render writes.
    fn collect(mix: &Mix, sources: &[Audio]) -> (Vec<Frame>, Vec<usize>, Vec<Progress>, i64) {
        let mut frames: Vec<Frame> = Vec::new();
        let mut blocks: Vec<usize> = Vec::new();
        let mut reports: Vec<Progress> = Vec::new();
        let total = render_to(
            mix,
            &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
            &mut resamplers(),
            &mut |block| {
                frames.extend_from_slice(block);
                blocks.push(block.len());
                Ok(())
            },
            &mut |progress| reports.push(progress),
        )
        .unwrap();
        (frames, blocks, reports, total.0)
    }

    #[test]
    fn the_stream_is_the_same_audio_as_the_buffered_render() {
        let (mix, sources) = two_kicks(20.0, 32.0, 8.0, 4);
        let buffered = render(&mix, &sources, &mut resamplers()).unwrap();
        let (frames, _, _, total) = collect(&mix, &sources);
        assert_eq!(total, buffered.len().0);
        assert_eq!(frames, buffered.frames);
    }

    #[test]
    fn blocks_arrive_in_order_and_progress_counts_them() {
        let (mix, sources) = two_kicks(20.0, 32.0, 8.0, 4);
        let (frames, blocks, reports, total) = collect(&mix, &sources);
        assert_eq!(frames.len() as i64, total);
        assert!(
            total % BLOCK_FRAMES as i64 != 0,
            "choose a mix that does not divide evenly"
        );
        let full = blocks.len() - 1;
        assert!(
            blocks[..full].iter().all(|len| *len == BLOCK_FRAMES),
            "{blocks:?}"
        );
        assert_eq!(blocks[full] as i64, total % BLOCK_FRAMES as i64);
        assert_eq!(reports.len(), blocks.len());
        let mut written = 0;
        for (report, len) in reports.iter().zip(&blocks) {
            written += *len as i64;
            assert_eq!(report.written, Samples(written));
            assert_eq!(report.total, Samples(total));
        }
    }

    #[test]
    fn tracks_are_loaded_when_they_are_first_heard_and_dropped_when_done() {
        // Four twenty-second tracks in a chain. Each hands over to the next
        // at its beat 40, about 18.5 seconds in, so consecutive tracks overlap
        // by a second and a half and no track overlaps the one after next.
        let audio = synth::kicks(Bpm(130.0), Seconds::ZERO, Seconds(20.0));
        let mut tracks: Vec<Track> = (0..4)
            .map(|_| track(&audio, 130.0, 0.0, 40.0, &[]))
            .collect();
        for i in 0..3 {
            let (before, after) = tracks.split_at_mut(i + 1);
            beatmix(&mut before[i], &mut after[0], 1);
        }
        let mix = Mix { tracks };
        let timeline = mix.timeline().unwrap();
        let opening = timeline.start();
        let entries: Vec<i64> = timeline
            .tracks
            .iter()
            .map(|placed| (placed.start(&timeline.curve) - opening).to_samples().0)
            .collect();

        let live = Rc::new(Cell::new(0usize));
        let loads: Rc<RefCell<Vec<(usize, i64, usize)>>> = Rc::new(RefCell::new(Vec::new()));
        let written = Rc::new(Cell::new(0i64));
        let total = {
            let loads = Rc::clone(&loads);
            let live_at_load = Rc::clone(&live);
            let written_at_load = Rc::clone(&written);
            let mut load = move |index: usize, _: &Track| {
                let source = Counted::boxed(audio.clone(), &live_at_load);
                loads
                    .borrow_mut()
                    .push((index, written_at_load.get(), live_at_load.get()));
                Ok(source)
            };
            let written_by_sink = Rc::clone(&written);
            render_to(
                &mix,
                &mut load,
                &mut resamplers(),
                &mut |block| {
                    written_by_sink.set(written_by_sink.get() + block.len() as i64);
                    Ok(())
                },
                &mut |_| {},
            )
            .unwrap()
        };
        assert_eq!(total.0, written.get());
        assert_eq!(live.get(), 0, "every source is dropped by the end");

        let loads = loads.borrow();
        let order: Vec<usize> = loads.iter().map(|(index, _, _)| *index).collect();
        assert_eq!(
            order,
            vec![0, 1, 2, 3],
            "each track is loaded once, in playlist order"
        );
        for (index, written_before, live_after) in loads.iter() {
            let entry = entries[*index];
            // The track is loaded for the block its first frame falls in:
            // no earlier than the block before it, and never after it.
            assert!(
                *written_before + BLOCK_FRAMES as i64 >= entry && *written_before <= entry,
                "track {index} loaded after {written_before} frames, entering at {entry}"
            );
            assert!(
                *live_after <= 2,
                "track {index}: {live_after} sources alive at once, but only neighbors overlap"
            );
        }
        // The third track enters after the first has ended, so by then the
        // first source is gone.
        assert_eq!(loads[2].2, 2);
    }

    #[test]
    fn an_empty_mix_streams_nothing() {
        let mut loaded = 0;
        let mut written = 0;
        let total = render_to(
            &Mix::new(),
            &mut |_, _| {
                loaded += 1;
                Ok(Box::new(Audio::new()) as Box<dyn Source>)
            },
            &mut resamplers(),
            &mut |block| {
                written += block.len();
                Ok(())
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(total, Samples::ZERO);
        assert_eq!((loaded, written), (0, 0));
    }

    #[test]
    fn a_loader_or_sink_that_fails_stops_the_render_and_says_which() {
        let (mix, sources) = two_kicks(20.0, 32.0, 8.0, 4);
        let problem = render_to(
            &mix,
            &mut |index, _| {
                if index == 1 {
                    Err("the disk went away".to_owned())
                } else {
                    Ok(Box::new(sources[index].clone()) as Box<dyn Source>)
                }
            },
            &mut resamplers(),
            &mut |_| Ok(()),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(
            problem,
            RenderError::Load {
                index: 1,
                message: "the disk went away".to_owned()
            }
        );

        let mut blocks = 0;
        let problem = render_to(
            &mix,
            &mut |index, _| Ok(Box::new(sources[index].clone()) as Box<dyn Source>),
            &mut resamplers(),
            &mut |_| {
                blocks += 1;
                if blocks == 3 {
                    Err("no space left".to_owned())
                } else {
                    Ok(())
                }
            },
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(problem, RenderError::Write("no space left".to_owned()));
        assert_eq!(blocks, 3, "nothing is written after the sink fails");
    }

    #[test]
    fn a_source_of_the_wrong_length_is_refused_when_it_is_loaded() {
        let (mix, sources) = two_kicks(20.0, 32.0, 8.0, 4);
        let short = Audio {
            frames: sources[1].frames[..1000].to_vec(),
        };
        let problem = render_to(
            &mix,
            &mut |index, _| {
                Ok(Box::new(if index == 1 {
                    short.clone()
                } else {
                    sources[index].clone()
                }) as Box<dyn Source>)
            },
            &mut resamplers(),
            &mut |_| Ok(()),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(
            problem,
            RenderError::SourceLength {
                index: 1,
                expected: sources[1].len().0,
                got: 1000
            }
        );
    }
}

/// Runs a render on a thread and panics when it has not answered in five
/// seconds, so that a render that never ends fails the test.
fn render_within_five_seconds(mix: Mix, sources: Vec<Audio>) -> Result<Audio, RenderError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || sender.send(render(&mix, &sources, &mut resamplers())));
    receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the render did not return within five seconds")
}

/// A change that takes one number of a mix out of its range.
type Damage = Box<dyn Fn(&mut Mix)>;

#[test]
#[ignore = "engine-guards"]
fn a_mix_no_document_may_hold_is_refused_before_anything_is_rendered() {
    let audio = synth::sine(440.0, 0.5, Seconds(2.0));
    let cases: Vec<(&str, Damage)> = vec![
        (
            "tracks[0].tempo[0].bpm",
            Box::new(|mix| mix.tracks[0].tempo[0].bpm = Bpm(1e-300)),
        ),
        (
            "tracks[0].tempo[0].bpm",
            Box::new(|mix| mix.tracks[0].tempo[0].bpm = Bpm(1e12)),
        ),
        (
            "tracks[1].grid.bpm",
            Box::new(|mix| mix.tracks[1].grid.bpm = Bpm(1e-6)),
        ),
        (
            "tracks[0].anchors.outro_beat",
            Box::new(|mix| mix.tracks[0].anchors.outro = Beats(1e18)),
        ),
        (
            "tracks[1].gain_db",
            Box::new(|mix| mix.tracks[1].gain = Decibels(f64::NAN)),
        ),
    ];
    for (field, damage) in cases {
        let mut mix = Mix {
            tracks: vec![
                track(&audio, 120.0, 0.0, 2.0, &[(0.0, 120.0)]),
                track(&audio, 120.0, 0.0, 4.0, &[]),
            ],
        };
        damage(&mut mix);
        match render_within_five_seconds(mix, vec![audio.clone(), audio.clone()]) {
            Err(RenderError::Document(problem)) => assert_eq!(problem.field, field),
            other => panic!("{field}: {:?}", other.map(|audio| audio.len())),
        }
    }
}

#[test]
#[ignore = "engine-guards"]
fn a_source_sample_that_is_not_a_number_does_not_reach_the_mix() {
    let clean = synth::sine(440.0, 0.5, Seconds(2.0));
    let mut hostile = clean.clone();
    hostile.frames[10_000] = [f32::NAN, f32::INFINITY];
    hostile.frames[20_000] = [f32::NEG_INFINITY, 1e30];
    // Both tracks sound together from the first frame to the last.
    let mix = Mix {
        tracks: vec![
            track(&hostile, 120.0, 0.0, 0.0, &[]),
            track(&clean, 120.0, 0.0, 4.0, &[]),
        ],
    };
    let rendered = render(&mix, &[hostile, clean.clone()], &mut resamplers()).unwrap();
    assert!(
        rendered
            .frames
            .iter()
            .all(|frame| frame[0].is_finite() && frame[1].is_finite()),
        "a frame of the mix is not a finite number"
    );
    // The second track is heard through the whole overlap, the damaged frames included.
    for at in [9_000, 10_000, 20_000, 30_000] {
        let window = &rendered.frames[at..at + 4_410];
        assert!(rms(window) > 0.2, "the mix is silent near frame {at}");
    }
}
