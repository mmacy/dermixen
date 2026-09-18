//! Acceptance tests for the default transition. A coder agent makes these pass without editing them.

use std::path::PathBuf;

use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Envelope, EnvelopeNode, EqEnvelopes,
    Samples, TempoNode, Track, beatmix,
};

fn track(bpm: f64, intro: f64, outro: f64) -> Track {
    Track {
        path: PathBuf::from("synthetic"),
        hash: ContentHash([0; 32]),
        length: Samples(10_000_000),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(bpm),
        },
        anchors: Anchors {
            intro: Beats(intro),
            outro: Beats(outro),
        },
        keylock: true,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    }
}

fn db(amplitude: f64) -> f64 {
    20.0 * amplitude.log10()
}

fn levels(envelope: &Envelope) -> Vec<(f64, f64)> {
    envelope
        .nodes()
        .iter()
        .map(|n| (n.at.0, n.value.0))
        .collect()
}

fn close_pairs(got: &[(f64, f64)], want: &[(f64, f64)]) -> bool {
    got.len() == want.len()
        && got
            .iter()
            .zip(want)
            .all(|(g, w)| (g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9)
}

#[test]
fn an_eight_bar_beatmix_writes_the_ramp_and_both_fades() {
    let mut a = track(130.0, 0.0, 64.0);
    let mut b = track(140.0, 16.0, 80.0);
    beatmix(&mut a, &mut b, 8);

    assert_eq!(
        a.tempo,
        vec![TempoNode {
            at: Beats(64.0),
            bpm: Bpm(130.0)
        }]
    );
    assert_eq!(
        b.tempo,
        vec![TempoNode {
            at: Beats(48.0),
            bpm: Bpm(140.0)
        }]
    );

    let fade_out = [
        (64.0, 0.0),
        (72.0, db(0.75)),
        (80.0, db(0.5)),
        (88.0, db(0.25)),
        (92.0, db(0.125)),
        (96.0, Decibels::SILENCE.0),
    ];
    assert!(
        close_pairs(&levels(&a.volume), &fade_out),
        "{:?}",
        levels(&a.volume)
    );
    let fade_in = [
        (16.0, Decibels::SILENCE.0),
        (20.0, db(0.125)),
        (24.0, db(0.25)),
        (32.0, db(0.5)),
        (40.0, db(0.75)),
        (48.0, 0.0),
    ];
    assert!(
        close_pairs(&levels(&b.volume), &fade_in),
        "{:?}",
        levels(&b.volume)
    );

    assert!(a.eq.low.is_empty() && a.eq.mid.is_empty() && a.eq.high.is_empty());
    assert!(b.eq.low.is_empty() && b.eq.mid.is_empty() && b.eq.high.is_empty());
    assert_eq!(
        a.anchors,
        Anchors {
            intro: Beats(0.0),
            outro: Beats(64.0)
        }
    );
    assert_eq!(
        b.anchors,
        Anchors {
            intro: Beats(16.0),
            outro: Beats(80.0)
        }
    );
}

#[test]
fn the_span_follows_the_bar_count() {
    let mut a = track(130.0, 0.0, 100.0);
    let mut b = track(131.0, 8.0, 200.0);
    beatmix(&mut a, &mut b, 4);
    let out = levels(&a.volume);
    assert_eq!(out.len(), 6);
    assert_eq!(out[0].0, 100.0);
    assert_eq!(out[5].0, 116.0);
    let inn = levels(&b.volume);
    assert_eq!(inn[0].0, 8.0);
    assert_eq!(inn[5].0, 24.0);
    assert_eq!(b.tempo[0].at, Beats(24.0));
}

#[test]
fn existing_nodes_elsewhere_are_kept_and_nodes_at_the_same_beat_are_replaced() {
    let mut a = track(130.0, 0.0, 64.0);
    a.volume
        .insert(EnvelopeNode {
            at: Beats(10.0),
            value: Decibels(-3.0),
        })
        .unwrap();
    a.volume
        .insert(EnvelopeNode {
            at: Beats(80.0),
            value: Decibels(-40.0),
        })
        .unwrap();
    a.tempo.push(TempoNode {
        at: Beats(0.0),
        bpm: Bpm(128.0),
    });
    let mut b = track(140.0, 16.0, 80.0);
    b.eq.low
        .insert(EnvelopeNode {
            at: Beats(16.0),
            value: Decibels::SILENCE,
        })
        .unwrap();
    beatmix(&mut a, &mut b, 8);

    let out = levels(&a.volume);
    assert_eq!(out.len(), 7);
    assert_eq!(out[0], (10.0, -3.0));
    let at_80 = out.iter().find(|n| n.0 == 80.0).unwrap();
    assert!(
        (at_80.1 - db(0.5)).abs() < 1e-9,
        "the node at beat 80 was not replaced"
    );
    assert_eq!(a.tempo.len(), 2);
    assert_eq!(a.tempo[0].at, Beats(0.0));
    assert_eq!(
        a.tempo[1],
        TempoNode {
            at: Beats(64.0),
            bpm: Bpm(130.0)
        }
    );
    assert_eq!(b.eq.low.len(), 1);
}

#[test]
fn applying_it_twice_changes_nothing_more() {
    let mut a = track(130.0, 0.0, 64.0);
    let mut b = track(140.0, 16.0, 80.0);
    beatmix(&mut a, &mut b, 8);
    let (once_a, once_b) = (a.clone(), b.clone());
    beatmix(&mut a, &mut b, 8);
    assert_eq!(a.volume, once_a.volume);
    assert_eq!(b.volume, once_b.volume);
    assert_eq!(a.tempo.len(), 1);
    assert_eq!(b.tempo.len(), 1);
}

mod presets {
    //! Acceptance tests for the transition presets. A coder agent makes these
    //! pass without editing them.

    use super::*;
    use dermixen_core::{
        BLEND_LEAD_BARS, BLEND_RISE_BARS, DEFAULT_BARS, Mix, Preset, apply, blend, outro_for,
        span_of,
    };

    /// A track at 120 beats per minute with beat zero on its first sample,
    /// so that a beat is exactly half a second and the track's last sample
    /// falls on the beat `end`.
    fn timed_track(intro: f64, outro: f64, end: f64) -> Track {
        let mut track = track(120.0, intro, outro);
        track.length = Samples((end * 22_050.0).round() as i64);
        track
    }

    #[test]
    fn a_blend_rises_over_thirty_six_bars_and_eases_the_outgoing_track_to_its_end() {
        assert_eq!(BLEND_LEAD_BARS, 8);
        assert_eq!(BLEND_RISE_BARS, 28);
        let mut a = timed_track(64.0, 896.0, 1000.0);
        let mut b = timed_track(32.0, 800.0, 900.0);
        blend(&mut a, &mut b);

        // The tempo nodes are the ones an eight-bar beatmix writes.
        assert_eq!(
            a.tempo,
            vec![TempoNode {
                at: Beats(896.0),
                bpm: Bpm(120.0)
            }]
        );
        assert_eq!(
            b.tempo,
            vec![TempoNode {
                at: Beats(64.0),
                bpm: Bpm(120.0)
            }]
        );

        // The outgoing track eases from full at its outro anchor to -7 dB at
        // its last sample, 104 beats later, with nodes a quarter of the way
        // apart.
        assert_eq!(
            levels(&a.volume),
            vec![
                (896.0, 0.0),
                (922.0, -1.0),
                (948.0, -2.5),
                (974.0, -4.5),
                (1000.0, -7.0)
            ]
        );
        // The incoming track rises from silence eight bars before its intro
        // anchor to -12 dB at the anchor and full level twenty-eight bars
        // after it.
        assert_eq!(
            levels(&b.volume),
            vec![
                (0.0, Decibels::SILENCE.0),
                (8.0, -24.0),
                (20.0, -16.0),
                (32.0, -12.0),
                (64.0, -5.5),
                (96.0, -2.5),
                (128.0, -0.5),
                (144.0, 0.0)
            ]
        );
        assert!(a.eq == EqEnvelopes::default() && b.eq == EqEnvelopes::default());

        // The preset by name writes the same nodes.
        let (mut c, mut d) = (
            timed_track(64.0, 896.0, 1000.0),
            timed_track(32.0, 800.0, 900.0),
        );
        apply(Preset::Blend, &mut c, &mut d);
        assert_eq!(a, c);
        assert_eq!(b, d);
    }

    #[test]
    fn a_blend_rounds_its_middle_nodes_to_beats_and_ends_on_the_last_sample() {
        // A tail of 117.8 beats: the quarter points fall at 925.45, 954.9,
        // and 984.35, and the last node is on the fractional last beat.
        let mut a = timed_track(64.0, 896.0, 1013.8);
        let mut b = timed_track(32.0, 800.0, 900.0);
        blend(&mut a, &mut b);
        let got = levels(&a.volume);
        let want = [
            (896.0, 0.0),
            (925.0, -1.0),
            (955.0, -2.5),
            (984.0, -4.5),
            (1013.8, -7.0),
        ];
        assert!(close_pairs(&got, &want), "{got:?}");
    }

    #[test]
    fn a_blend_out_of_a_short_tail_writes_no_middle_node_and_keeps_full_level_to_the_anchor() {
        // A tail under four beats has no whole beat of its own for each of
        // the three middle nodes, so the ease is two nodes: full level at
        // the anchor and -7 dB at the last sample. The track is at full
        // level everywhere before its anchor, since the envelope's value
        // before its first node is that node's value.
        for (outro, end) in [(130.0, 130.6), (129.0, 130.6), (127.0, 130.9)] {
            let mut a = timed_track(0.0, outro, end);
            let mut b = timed_track(32.0, 800.0, 900.0);
            blend(&mut a, &mut b);
            let got = levels(&a.volume);
            assert!(
                close_pairs(&got, &[(outro, 0.0), (end, -7.0)]),
                "outro {outro}, end {end}: {got:?}"
            );
            assert_eq!(a.volume.value_at(Beats(10.0)), Decibels::UNITY);
            assert_eq!(a.volume.value_at(Beats(outro)), Decibels::UNITY);
        }
        // Four beats is enough: each middle node lands on its own whole
        // beat strictly between the anchor and the last sample.
        let mut a = timed_track(0.0, 126.0, 130.0);
        let mut b = timed_track(32.0, 800.0, 900.0);
        blend(&mut a, &mut b);
        let got = levels(&a.volume);
        assert!(
            close_pairs(
                &got,
                &[
                    (126.0, 0.0),
                    (127.0, -1.0),
                    (128.0, -2.5),
                    (129.0, -4.5),
                    (130.0, -7.0)
                ]
            ),
            "{got:?}"
        );
    }

    #[test]
    fn a_blend_out_of_a_track_whose_outro_anchor_is_at_or_past_its_end_writes_one_node() {
        for end in [896.0, 880.0] {
            let mut a = timed_track(64.0, 896.0, end);
            let mut b = timed_track(32.0, 800.0, 900.0);
            blend(&mut a, &mut b);
            assert_eq!(levels(&a.volume), vec![(896.0, 0.0)], "end at {end}");
            assert_eq!(b.volume.len(), 8);
        }
    }

    #[test]
    fn a_blend_keeps_other_nodes_and_replaces_only_its_own_beats() {
        let mut a = timed_track(64.0, 896.0, 1000.0);
        let mut b = timed_track(32.0, 800.0, 900.0);
        // A node in the body of each track, and one on a beat the blend
        // writes.
        a.volume
            .insert(EnvelopeNode {
                at: Beats(400.0),
                value: Decibels(-3.0),
            })
            .unwrap();
        a.volume
            .insert(EnvelopeNode {
                at: Beats(948.0),
                value: Decibels(-20.0),
            })
            .unwrap();
        b.volume
            .insert(EnvelopeNode {
                at: Beats(500.0),
                value: Decibels(-1.0),
            })
            .unwrap();
        b.volume
            .insert(EnvelopeNode {
                at: Beats(64.0),
                value: Decibels(-40.0),
            })
            .unwrap();
        blend(&mut a, &mut b);
        assert_eq!(a.volume.value_at(Beats(400.0)), Decibels(-3.0));
        assert_eq!(a.volume.value_at(Beats(948.0)), Decibels(-2.5));
        assert_eq!(a.volume.len(), 6);
        assert_eq!(b.volume.value_at(Beats(500.0)), Decibels(-1.0));
        assert_eq!(b.volume.value_at(Beats(64.0)), Decibels(-5.5));
        assert_eq!(b.volume.len(), 9);

        let (once_a, once_b) = (a.clone(), b.clone());
        blend(&mut a, &mut b);
        assert_eq!(a, once_a);
        assert_eq!(b, once_b);
    }

    #[test]
    fn a_blend_needs_its_rise_between_the_anchors_of_the_outgoing_track() {
        assert_eq!(span_of(Preset::Blend), Beats(112.0));
        // The track before is the one whose anchors the rise must fit
        // between: an intro-to-outro span of 111 beats is refused and one
        // of 112 is not.
        let mut mix = Mix {
            tracks: vec![timed_track(64.0, 175.0, 300.0)],
        };
        let edit = dermixen_core::Edit::InsertTrack {
            at: 1,
            track: Box::new(timed_track(32.0, 800.0, 900.0)),
            preset: Preset::Blend,
        };
        assert_eq!(
            dermixen_core::apply_edit(&mut mix, &edit),
            Err(dermixen_core::EditError::TransitionDoesNotFit {
                span: Beats(111.0),
                length: Beats(112.0)
            })
        );
        mix.tracks[0].anchors.outro = Beats(176.0);
        assert_eq!(dermixen_core::apply_edit(&mut mix, &edit), Ok(()));
        assert_eq!(mix.tracks.len(), 2);
        assert_eq!(mix.tracks[1].volume.len(), 8);
    }

    #[test]
    fn presets_are_known_by_name() {
        assert_eq!(
            Preset::from_name("beatmix", 8),
            Some(Preset::Beatmix { bars: 8 })
        );
        assert_eq!(
            Preset::from_name("bass-swap", 16),
            Some(Preset::BassSwap { bars: 16 })
        );
        assert_eq!(Preset::from_name("cut", 8), Some(Preset::Cut));
        assert_eq!(Preset::from_name("cut", 3), Some(Preset::Cut));
        assert_eq!(Preset::from_name("blend", 8), Some(Preset::Blend));
        assert_eq!(Preset::from_name("blend", 3), Some(Preset::Blend));
        assert_eq!(Preset::from_name("Beatmix", 8), None);
        assert_eq!(Preset::from_name("fade", 8), None);
        assert_eq!(Preset::from_name("", 8), None);
        for preset in [
            Preset::Blend,
            Preset::Beatmix { bars: 4 },
            Preset::BassSwap { bars: 4 },
            Preset::Cut,
        ] {
            assert_eq!(
                Preset::from_name(preset.name(), 4),
                Some(preset),
                "{preset:?}"
            );
        }
        assert_eq!(DEFAULT_BARS, 8);
    }

    #[test]
    fn the_beatmix_preset_writes_what_beatmix_writes() {
        let (mut a, mut b) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        beatmix(&mut a, &mut b, 8);
        let (mut c, mut d) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        apply(Preset::Beatmix { bars: 8 }, &mut c, &mut d);
        assert_eq!(a, c);
        assert_eq!(b, d);
        let (mut c, mut d) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        apply(Preset::Beatmix { bars: 2 }, &mut c, &mut d);
        assert_eq!(c.volume.nodes().last().unwrap().at, Beats(904.0));
        assert_eq!(
            d.tempo,
            vec![TempoNode {
                at: Beats(40.0),
                bpm: Bpm(140.0)
            }]
        );
    }

    #[test]
    fn a_cut_switches_tracks_and_tempo_at_the_shared_beat() {
        let (mut a, mut b) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        apply(Preset::Cut, &mut a, &mut b);
        assert_eq!(
            a.tempo,
            vec![TempoNode {
                at: Beats(896.0),
                bpm: Bpm(138.0)
            }]
        );
        assert_eq!(
            b.tempo,
            vec![TempoNode {
                at: Beats(32.0),
                bpm: Bpm(140.0)
            }]
        );
        assert_eq!(
            levels(&a.volume),
            vec![(895.75, 0.0), (896.0, Decibels::SILENCE.0)]
        );
        assert_eq!(
            levels(&b.volume),
            vec![(31.75, Decibels::SILENCE.0), (32.0, 0.0)]
        );
        assert!(a.eq == EqEnvelopes::default() && b.eq == EqEnvelopes::default());

        // On the timeline the tempo is the outgoing track's until the shared
        // beat and the incoming track's from it.
        let mix = Mix { tracks: vec![a, b] };
        let timeline = mix.timeline().unwrap();
        assert_eq!(timeline.curve.bpm_at_beat(Beats(895.9)), Bpm(138.0));
        assert_eq!(timeline.curve.bpm_at_beat(Beats(896.0)), Bpm(140.0));
        assert_eq!(timeline.curve.bpm_at_beat(Beats(1000.0)), Bpm(140.0));
    }

    #[test]
    fn a_bass_swap_is_a_beatmix_with_the_lows_handed_over_at_the_middle() {
        let (mut a, mut b) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        // A low node far from the transition must survive it.
        b.eq.low
            .insert(EnvelopeNode {
                at: Beats(400.0),
                value: Decibels(-6.0),
            })
            .unwrap();
        let (mut c, mut d) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        beatmix(&mut c, &mut d, 8);
        apply(Preset::BassSwap { bars: 8 }, &mut a, &mut b);

        assert_eq!(a.tempo, c.tempo);
        assert_eq!(b.tempo, d.tempo);
        assert_eq!(a.volume, c.volume);
        assert_eq!(b.volume, d.volume);
        // Eight bars is thirty-two beats, so the swap is sixteen beats in.
        assert_eq!(
            levels(&a.eq.low),
            vec![(911.0, 0.0), (912.0, Decibels::SILENCE.0)]
        );
        assert_eq!(
            levels(&b.eq.low),
            vec![(47.0, Decibels::SILENCE.0), (48.0, 0.0), (400.0, -6.0)]
        );
        assert!(a.eq.mid.is_empty() && a.eq.high.is_empty());
        assert!(b.eq.mid.is_empty() && b.eq.high.is_empty());

        // The incoming track's lows are silent before the swap and full at it;
        // past it the envelope ramps toward the node the test placed at beat 400.
        assert_eq!(b.eq.low.value_at(Beats(32.0)), Decibels::SILENCE);
        assert_eq!(b.eq.low.value_at(Beats(48.0)), Decibels::UNITY);
        assert_eq!(a.eq.low.value_at(Beats(900.0)), Decibels::UNITY);
        assert_eq!(a.eq.low.value_at(Beats(920.0)), Decibels::SILENCE);
    }

    #[test]
    fn a_shorter_bass_swap_swaps_at_its_own_middle() {
        let (mut a, mut b) = (track(138.0, 64.0, 896.0), track(140.0, 32.0, 800.0));
        apply(Preset::BassSwap { bars: 2 }, &mut a, &mut b);
        assert_eq!(
            levels(&a.eq.low),
            vec![(899.0, 0.0), (900.0, Decibels::SILENCE.0)]
        );
        assert_eq!(
            levels(&b.eq.low),
            vec![(35.0, Decibels::SILENCE.0), (36.0, 0.0)]
        );
    }

    #[test]
    fn the_outro_anchor_moves_to_keep_the_end_of_the_overlap() {
        let analyzed = Anchors {
            intro: Beats(64.0),
            outro: Beats(896.0),
        };
        assert_eq!(outro_for(analyzed, 8), Beats(896.0));
        assert_eq!(outro_for(analyzed, 16), Beats(864.0));
        assert_eq!(outro_for(analyzed, 32), Beats(800.0));
        assert_eq!(outro_for(analyzed, 4), Beats(912.0));
        assert_eq!(outro_for(analyzed, 1), Beats(924.0));
    }
}
