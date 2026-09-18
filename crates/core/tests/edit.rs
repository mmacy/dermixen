//! Acceptance tests for the edit log: every timeline edit as a value, with
//! undo and redo. A coder agent makes these pass without editing them.
//!
//! Every expectation here follows from the rules the doc comments in
//! `crates/core/src/edit.rs` state and from the nodes `beatmix` is
//! documented to write in `docs/project-file.md`: a fade out at fractions
//! 0, 1/4, 1/2, 3/4, 7/8, and 1 of the span from the outro anchor, a fade in
//! at fractions 0, 1/8, 1/4, 1/2, 3/4, and 1 of the span from the intro
//! anchor, a tempo node at the outgoing outro anchor, and a tempo node at
//! the end of the incoming span.

use std::path::PathBuf;

use dermixen_core::{
    Anchor, Anchors, BeatGrid, Beats, Bpm, ContentHash, Curve, Decibels, Edit, EditError, Envelope,
    EnvelopeNode, EqEnvelopes, History, Mix, Preset, Samples, TempoNode, Track, apply_edit,
    beatmix, blend, clear_incoming, clear_outgoing, fits_between_the_anchors, outgoing_from,
    owner_of, span_of,
};
use proptest::prelude::*;

/// A four-minute track at 130 beats per minute whose grid starts at its
/// first sample, with the anchors given and no nodes.
fn track(name: &str, intro: f64, outro: f64) -> Track {
    Track {
        path: PathBuf::from(name),
        hash: ContentHash([name.len() as u8; 32]),
        length: Samples(240 * 44_100),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(130.0),
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

/// Two tracks joined by an eight-bar beatmix: the first with anchors at
/// beats 16 and 256, the second with anchors at beats 32 and 512. The
/// first track's fade out therefore sits at beats 256, 264, 272, 280, 284,
/// and 288 with a tempo node at 256; the second track's fade in sits at
/// beats 32, 36, 40, 48, 56, and 64 with a tempo node at 64.
fn two_tracks() -> Mix {
    let mut a = track("a", 16.0, 256.0);
    let mut b = track("b", 32.0, 512.0);
    beatmix(&mut a, &mut b, 8);
    Mix { tracks: vec![a, b] }
}

fn node(at: f64, db: f64) -> EnvelopeNode {
    EnvelopeNode {
        at: Beats(at),
        value: Decibels(db),
    }
}

fn tempo(at: f64, bpm: f64) -> TempoNode {
    TempoNode {
        at: Beats(at),
        bpm: Bpm(bpm),
    }
}

/// The beats of an envelope's nodes, in order.
fn beats(envelope: &Envelope) -> Vec<f64> {
    envelope.nodes().iter().map(|n| n.at.0).collect()
}

/// The level of the node at a beat, which must exist.
fn level_at(envelope: &Envelope, at: f64) -> f64 {
    envelope
        .nodes()
        .iter()
        .find(|n| n.at.0 == at)
        .unwrap_or_else(|| panic!("no node at {at} among {:?}", beats(envelope)))
        .value
        .0
}

fn curve_of(track: &Track, curve: Curve) -> &Envelope {
    match curve {
        Curve::Volume => &track.volume,
        Curve::Low => &track.eq.low,
        Curve::Mid => &track.eq.mid,
        Curve::High => &track.eq.high,
    }
}

fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{what}: {actual} is not {expected}"
    );
}

#[test]
fn the_transition_out_of_a_track_begins_a_quarter_beat_before_its_outro_anchor() {
    assert_eq!(
        outgoing_from(Anchors {
            intro: Beats(16.0),
            outro: Beats(256.0)
        }),
        Beats(255.75)
    );
}

#[test]
fn moving_the_outro_anchor_takes_its_fade_and_tempo_node_along() {
    let mut mix = two_tracks();
    let a = &mut mix.tracks[0];
    // A node in the body, one just inside the quarter-beat margin, one just
    // outside it, and one where the moved fade will land.
    for n in [
        node(100.0, -3.0),
        node(255.75, -1.0),
        node(255.5, -2.0),
        node(240.0, -4.0),
    ] {
        a.volume.insert(n).unwrap();
    }
    a.eq.low.insert(node(270.0, -90.0)).unwrap();
    a.eq.low.insert(node(200.0, -6.0)).unwrap();

    apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 0,
            anchor: Anchor::Outro,
            to: Beats(240.0),
        },
    )
    .unwrap();

    let a = &mix.tracks[0];
    assert_eq!(a.anchors.intro, Beats(16.0));
    assert_eq!(a.anchors.outro, Beats(240.0));
    assert_eq!(
        beats(&a.volume),
        vec![
            100.0, 239.75, 240.0, 248.0, 255.5, 256.0, 264.0, 268.0, 272.0
        ]
    );
    assert_eq!(
        level_at(&a.volume, 239.75),
        -1.0,
        "the node in the margin moved"
    );
    assert_eq!(
        level_at(&a.volume, 255.5),
        -2.0,
        "the node outside the margin stayed"
    );
    assert_eq!(
        level_at(&a.volume, 240.0),
        0.0,
        "the fade's first node replaced the node it landed on"
    );
    assert_eq!(level_at(&a.volume, 100.0), -3.0);
    assert_eq!(beats(&a.eq.low), vec![200.0, 254.0]);
    assert_eq!(a.tempo, vec![tempo(240.0, 130.0)]);
    assert_eq!(
        mix.tracks[1],
        two_tracks().tracks[1],
        "the other track is untouched"
    );
}

#[test]
fn moving_the_intro_anchor_takes_its_fade_along_and_leaves_the_body_alone() {
    // The transition into a track reaches from eight bars before the intro
    // anchor to twenty-eight bars after it, which is the blend's extent.
    // Nodes just inside and just outside each end of that reach, and one in
    // the body.
    let mut a = track("a", 16.0, 256.0);
    let mut b = track("b", 40.0, 512.0);
    beatmix(&mut a, &mut b, 8);
    let mut mix = Mix { tracks: vec![a, b] };
    let b = &mut mix.tracks[1];
    for n in [
        node(7.75, -2.0),
        node(8.0, -1.0),
        node(152.0, -3.0),
        node(152.25, -5.0),
        node(200.0, -4.0),
    ] {
        b.volume.insert(n).unwrap();
    }
    b.eq.high.insert(node(50.0, -12.0)).unwrap();
    b.eq.high.insert(node(300.0, -12.0)).unwrap();
    let a_before = mix.tracks[0].clone();

    apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 1,
            anchor: Anchor::Intro,
            to: Beats(48.0),
        },
    )
    .unwrap();

    let b = &mix.tracks[1];
    assert_eq!(b.anchors.intro, Beats(48.0));
    assert_eq!(b.anchors.outro, Beats(512.0));
    assert_eq!(
        beats(&b.volume),
        vec![
            7.75, 16.0, 48.0, 52.0, 56.0, 64.0, 72.0, 80.0, 152.25, 160.0, 200.0
        ]
    );
    assert_eq!(
        level_at(&b.volume, 7.75),
        -2.0,
        "a node before eight bars ahead of the anchor stayed"
    );
    assert_eq!(
        level_at(&b.volume, 16.0),
        -1.0,
        "a node exactly eight bars ahead moved"
    );
    assert_eq!(
        level_at(&b.volume, 160.0),
        -3.0,
        "a node exactly twenty-eight bars after moved"
    );
    assert_eq!(
        level_at(&b.volume, 152.25),
        -5.0,
        "a node past twenty-eight bars stayed"
    );
    assert_eq!(level_at(&b.volume, 200.0), -4.0);
    assert_eq!(
        level_at(&b.volume, 48.0),
        -90.0,
        "the fade in still begins at the anchor"
    );
    assert_eq!(b.tempo, vec![tempo(80.0, 130.0)]);
    assert_eq!(
        beats(&b.eq.high),
        vec![58.0, 300.0],
        "EQ nodes move by the same rule"
    );
    assert_eq!(mix.tracks[0], a_before);
}

#[test]
fn moving_the_intro_anchor_takes_every_node_of_a_blend_along() {
    // The blend's eight rise nodes run from eight bars before the intro
    // anchor to twenty-eight bars after it, and DESIGN.md says the nodes
    // belonging to a transition move with their anchor, so all eight move,
    // with the tempo node, and the rise keeps its shape around the new
    // anchor. A node in the body stays.
    let mut a = track("a", 16.0, 256.0);
    let mut b = track("b", 40.0, 512.0);
    blend(&mut a, &mut b);
    let mut mix = Mix { tracks: vec![a, b] };
    mix.tracks[1].volume.insert(node(300.0, -4.0)).unwrap();
    assert_eq!(
        beats(&mix.tracks[1].volume),
        vec![8.0, 16.0, 28.0, 40.0, 72.0, 104.0, 136.0, 152.0, 300.0]
    );
    let a_before = mix.tracks[0].clone();

    apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 1,
            anchor: Anchor::Intro,
            to: Beats(80.0),
        },
    )
    .unwrap();

    let b = &mix.tracks[1];
    assert_eq!(b.anchors.intro, Beats(80.0));
    assert_eq!(
        beats(&b.volume),
        vec![48.0, 56.0, 68.0, 80.0, 112.0, 144.0, 176.0, 192.0, 300.0]
    );
    for (at, db) in [
        (48.0, -90.0),
        (56.0, -24.0),
        (68.0, -16.0),
        (80.0, -12.0),
        (112.0, -5.5),
        (144.0, -2.5),
        (176.0, -0.5),
        (192.0, 0.0),
        (300.0, -4.0),
    ] {
        assert_eq!(level_at(&b.volume, at), db, "the level at beat {at}");
    }
    assert_eq!(b.tempo, vec![tempo(112.0, 130.0)]);
    assert_eq!(mix.tracks[0], a_before);

    // Moved back, the rise is where the blend wrote it.
    apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 1,
            anchor: Anchor::Intro,
            to: Beats(40.0),
        },
    )
    .unwrap();
    assert_eq!(
        beats(&mix.tracks[1].volume),
        vec![8.0, 16.0, 28.0, 40.0, 72.0, 104.0, 136.0, 152.0, 300.0]
    );
}

#[test]
fn the_intro_window_stops_where_the_transition_out_begins() {
    // A track whose anchors are 24 beats apart: the intro window would reach
    // sixteen bars past the intro anchor, but stops before the quarter beat
    // ahead of the outro anchor, so the transition out stays put.
    let mut mix = Mix {
        tracks: vec![track("e", 16.0, 40.0)],
    };
    let e = &mut mix.tracks[0];
    for n in [node(30.0, -1.0), node(39.75, -2.0), node(50.0, -3.0)] {
        e.volume.insert(n).unwrap();
    }
    apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 0,
            anchor: Anchor::Intro,
            to: Beats(24.0),
        },
    )
    .unwrap();
    assert_eq!(beats(&mix.tracks[0].volume), vec![38.0, 39.75, 50.0]);
    let before = mix.clone();
    apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 0,
            anchor: Anchor::Intro,
            to: Beats(24.0),
        },
    )
    .unwrap();
    assert_eq!(mix, before, "a move to the same beat changes nothing");
}

#[test]
fn the_transition_helpers_clear_by_the_quarter_beat_rule_and_size_each_preset() {
    let mut mix = two_tracks();
    mix.tracks[0].eq.low.insert(node(260.0, -90.0)).unwrap();
    mix.tracks[0].eq.low.insert(node(100.0, -6.0)).unwrap();
    clear_outgoing(&mut mix.tracks[0]);
    assert!(mix.tracks[0].volume.is_empty());
    assert!(mix.tracks[0].tempo.is_empty());
    assert_eq!(beats(&mix.tracks[0].eq.low), vec![100.0]);

    mix.tracks[1].eq.mid.insert(node(20.0, -6.0)).unwrap();
    mix.tracks[1].eq.mid.insert(node(600.0, -6.0)).unwrap();
    clear_incoming(&mut mix.tracks[1]);
    assert!(mix.tracks[1].volume.is_empty());
    assert!(mix.tracks[1].tempo.is_empty());
    assert_eq!(beats(&mix.tracks[1].eq.mid), vec![600.0]);

    assert_eq!(span_of(Preset::Blend), Beats(112.0));
    assert_eq!(span_of(Preset::Beatmix { bars: 8 }), Beats(32.0));
    assert_eq!(span_of(Preset::BassSwap { bars: 3 }), Beats(12.0));
    assert_eq!(span_of(Preset::Cut), Beats(0.25));
    assert_eq!(
        fits_between_the_anchors(&track("f", 16.0, 48.0), Beats(32.0)),
        Ok(())
    );
    assert_eq!(
        fits_between_the_anchors(&track("f", 16.0, 47.0), Beats(32.0)),
        Err(EditError::TransitionDoesNotFit {
            span: Beats(31.0),
            length: Beats(32.0)
        })
    );
}

#[test]
fn an_anchor_must_be_a_whole_beat_and_the_outro_must_follow_the_intro() {
    let mut mix = two_tracks();
    let before = mix.clone();
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::MoveAnchor {
                track: 1,
                anchor: Anchor::Intro,
                to: Beats(40.5)
            }
        ),
        Err(EditError::NotAWholeBeat { beat: Beats(40.5) })
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::MoveAnchor {
                track: 0,
                anchor: Anchor::Outro,
                to: Beats(16.0)
            }
        ),
        Err(EditError::AnchorsOutOfOrder {
            intro: Beats(16.0),
            outro: Beats(16.0)
        })
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::MoveAnchor {
                track: 2,
                anchor: Anchor::Outro,
                to: Beats(16.0)
            }
        ),
        Err(EditError::NoSuchTrack {
            track: 2,
            tracks: 2
        })
    );
    assert_eq!(mix, before, "a refused edit changes nothing");
}

#[test]
fn nodes_are_added_moved_and_removed_on_every_curve() {
    for curve in [Curve::Volume, Curve::Low, Curve::Mid, Curve::High] {
        let mut mix = two_tracks();
        let add = |at: f64, db: f64| Edit::AddNode {
            track: 0,
            curve,
            node: node(at, db),
        };
        apply_edit(&mut mix, &add(100.0, -6.0)).unwrap();
        assert_eq!(level_at(curve_of(&mix.tracks[0], curve), 100.0), -6.0);
        assert_eq!(
            apply_edit(&mut mix, &add(100.0, -12.0)),
            Err(EditError::NodeInTheWay { at: Beats(100.0) })
        );
        assert_eq!(level_at(curve_of(&mix.tracks[0], curve), 100.0), -6.0);
        apply_edit(&mut mix, &add(120.0, -1.0)).unwrap();

        apply_edit(
            &mut mix,
            &Edit::MoveNode {
                track: 0,
                curve,
                from: Beats(100.0),
                to: node(110.0, -3.0),
            },
        )
        .unwrap();
        let envelope = curve_of(&mix.tracks[0], curve);
        assert!(beats(envelope).contains(&110.0) && !beats(envelope).contains(&100.0));
        assert_eq!(level_at(envelope, 110.0), -3.0);
        // The same beat with a new level is a move too.
        apply_edit(
            &mut mix,
            &Edit::MoveNode {
                track: 0,
                curve,
                from: Beats(110.0),
                to: node(110.0, -9.0),
            },
        )
        .unwrap();
        assert_eq!(level_at(curve_of(&mix.tracks[0], curve), 110.0), -9.0);
        assert_eq!(
            apply_edit(
                &mut mix,
                &Edit::MoveNode {
                    track: 0,
                    curve,
                    from: Beats(110.0),
                    to: node(120.0, -9.0),
                }
            ),
            Err(EditError::NodeInTheWay { at: Beats(120.0) })
        );
        assert_eq!(
            apply_edit(
                &mut mix,
                &Edit::MoveNode {
                    track: 0,
                    curve,
                    from: Beats(111.0),
                    to: node(130.0, -9.0),
                }
            ),
            Err(EditError::NoSuchNode { at: Beats(111.0) })
        );

        apply_edit(
            &mut mix,
            &Edit::RemoveNode {
                track: 0,
                curve,
                at: Beats(110.0),
            },
        )
        .unwrap();
        assert!(!beats(curve_of(&mix.tracks[0], curve)).contains(&110.0));
        assert_eq!(
            apply_edit(
                &mut mix,
                &Edit::RemoveNode {
                    track: 0,
                    curve,
                    at: Beats(110.0),
                }
            ),
            Err(EditError::NoSuchNode { at: Beats(110.0) })
        );
        assert_eq!(
            apply_edit(&mut mix, &add(f64::NAN, -6.0)),
            Err(EditError::InvalidNode)
        );
        assert_eq!(
            apply_edit(&mut mix, &add(140.0, f64::INFINITY)),
            Err(EditError::InvalidNode)
        );
        // The other curves and the other track are untouched.
        let mut expected = two_tracks();
        expected.tracks[0] = mix.tracks[0].clone();
        assert_eq!(mix.tracks[1], expected.tracks[1]);
        for other in [Curve::Volume, Curve::Low, Curve::Mid, Curve::High] {
            if other != curve {
                assert_eq!(
                    curve_of(&mix.tracks[0], other),
                    curve_of(&two_tracks().tracks[0], other)
                );
            }
        }
    }
}

#[test]
fn tempo_nodes_are_edited_the_same_way_and_a_tempo_must_be_valid() {
    let mut mix = two_tracks();
    apply_edit(
        &mut mix,
        &Edit::AddTempoNode {
            track: 0,
            node: tempo(100.0, 134.0),
        },
    )
    .unwrap();
    assert_eq!(
        mix.tracks[0].tempo,
        vec![tempo(100.0, 134.0), tempo(256.0, 130.0)]
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::AddTempoNode {
                track: 0,
                node: tempo(256.0, 134.0),
            }
        ),
        Err(EditError::NodeInTheWay { at: Beats(256.0) })
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::AddTempoNode {
                track: 0,
                node: tempo(120.0, 0.0),
            }
        ),
        Err(EditError::InvalidTempo { bpm: Bpm(0.0) })
    );
    apply_edit(
        &mut mix,
        &Edit::MoveTempoNode {
            track: 0,
            from: Beats(100.0),
            to: tempo(110.0, 136.0),
        },
    )
    .unwrap();
    assert_eq!(
        mix.tracks[0].tempo,
        vec![tempo(110.0, 136.0), tempo(256.0, 130.0)]
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::MoveTempoNode {
                track: 0,
                from: Beats(110.0),
                to: tempo(256.0, 136.0),
            }
        ),
        Err(EditError::NodeInTheWay { at: Beats(256.0) })
    );
    apply_edit(
        &mut mix,
        &Edit::RemoveTempoNode {
            track: 0,
            at: Beats(110.0),
        },
    )
    .unwrap();
    assert_eq!(mix.tracks[0].tempo, vec![tempo(256.0, 130.0)]);
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::RemoveTempoNode {
                track: 0,
                at: Beats(110.0),
            }
        ),
        Err(EditError::NoSuchNode { at: Beats(110.0) })
    );
}

#[test]
fn the_master_tempo_control_writes_a_node_on_the_track_under_the_playhead() {
    // The second track's beat zero falls at mix beat 0 + 256 - 32 = 224.
    let mut mix = two_tracks();
    let set = |mix: &mut Mix, mix_beat: f64, bpm: f64| {
        apply_edit(
            mix,
            &Edit::SetTempoAt {
                mix_beat: Beats(mix_beat),
                bpm: Bpm(bpm),
            },
        )
        .unwrap();
    };
    set(&mut mix, 100.0, 135.0);
    assert_eq!(
        mix.tracks[0].tempo,
        vec![tempo(100.0, 135.0), tempo(256.0, 130.0)]
    );
    set(&mut mix, 300.0, 138.0);
    assert_eq!(
        mix.tracks[1].tempo,
        vec![tempo(64.0, 130.0), tempo(76.0, 138.0)]
    );
    set(&mut mix, 224.0, 132.0);
    assert_eq!(
        mix.tracks[1].tempo,
        vec![tempo(0.0, 132.0), tempo(64.0, 130.0), tempo(76.0, 138.0)],
        "a beat at the second track's beat zero belongs to the second track"
    );
    set(&mut mix, 256.0, 140.0);
    assert_eq!(
        mix.tracks[1].tempo,
        vec![
            tempo(0.0, 132.0),
            tempo(32.0, 140.0),
            tempo(64.0, 130.0),
            tempo(76.0, 138.0)
        ],
        "the second track has begun by mix beat 256, so the beat is its own"
    );
    assert_eq!(
        mix.tracks[0].tempo,
        vec![tempo(100.0, 135.0), tempo(256.0, 130.0)],
        "the first track's transition node is left alone"
    );
    set(&mut mix, 100.0, 137.0);
    assert_eq!(
        mix.tracks[0].tempo,
        vec![tempo(100.0, 137.0), tempo(256.0, 130.0)],
        "a node already at the beat is replaced"
    );
    set(&mut mix, -10.0, 128.0);
    assert_eq!(
        mix.tracks[0].tempo,
        vec![
            tempo(-10.0, 128.0),
            tempo(100.0, 137.0),
            tempo(256.0, 130.0)
        ],
        "a beat before every track's beat zero goes on the first track"
    );
    assert_eq!(owner_of(&mix, Beats(256.0)), Some((1, Beats(32.0))));
    assert_eq!(owner_of(&mix, Beats(223.0)), Some((0, Beats(223.0))));
    assert_eq!(owner_of(&mix, Beats(-10.0)), Some((0, Beats(-10.0))));
    assert_eq!(owner_of(&Mix { tracks: Vec::new() }, Beats(0.0)), None);
    let curve = mix.timeline().unwrap().curve;
    assert_close(
        curve.bpm_at_beat(Beats(300.0)).0,
        138.0,
        "the tempo at the node",
    );
    assert_close(
        curve.bpm_at_beat(Beats(100.0)).0,
        137.0,
        "the tempo at the node",
    );
    assert_close(
        curve.bpm_at_beat(Beats(256.0)).0,
        140.0,
        "the later of two nodes at one mix beat is the tempo there",
    );
    let between = curve.bpm_at_beat(Beats(150.0)).0;
    assert!(
        between < 137.0 && between > 132.0,
        "the curve ramps from 137 at beat 100 toward 132 at beat 224, and reads {between} at 150"
    );
    assert_eq!(
        apply_edit(
            &mut Mix { tracks: Vec::new() },
            &Edit::SetTempoAt {
                mix_beat: Beats(0.0),
                bpm: Bpm(130.0),
            }
        ),
        Err(EditError::EmptyMix)
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::SetTempoAt {
                mix_beat: Beats(10.0),
                bpm: Bpm(-1.0),
            }
        ),
        Err(EditError::InvalidTempo { bpm: Bpm(-1.0) })
    );
}

#[test]
fn changing_the_tempo_from_a_beat_pins_the_curve_and_ramps_over_one_beat() {
    // The fixture's curve is flat at 130: the starting node, the first
    // track's node at 256, and the second track's node at its beat 64, which
    // is mix beat 288, all hold 130.
    let mut mix = two_tracks();
    apply_edit(
        &mut mix,
        &Edit::ChangeTempoFrom {
            mix_beat: Beats(100.0),
            bpm: Bpm(125.0),
        },
    )
    .unwrap();
    assert_eq!(
        mix.tracks[0].tempo,
        vec![
            tempo(100.0, 130.0),
            tempo(101.0, 125.0),
            tempo(256.0, 130.0)
        ],
        "a pin holding the curve's tempo, then the new tempo one beat later"
    );
    let curve = mix.timeline().unwrap().curve;
    assert_close(
        curve.bpm_at_beat(Beats(50.0)).0,
        130.0,
        "the curve before the beat is what it was",
    );
    assert_close(curve.bpm_at_beat(Beats(101.0)).0, 125.0, "the new tempo");
    let later = curve.bpm_at_beat(Beats(150.0)).0;
    assert!(
        later > 125.0 && later < 130.0,
        "from the new node the curve ramps toward the next, and reads {later} at 150"
    );

    // A beat the second track owns: mix beat 300 is its beat 76, where the
    // curve holds 130 after its last node.
    apply_edit(
        &mut mix,
        &Edit::ChangeTempoFrom {
            mix_beat: Beats(300.0),
            bpm: Bpm(128.0),
        },
    )
    .unwrap();
    assert_eq!(
        mix.tracks[1].tempo,
        vec![tempo(64.0, 130.0), tempo(76.0, 130.0), tempo(77.0, 128.0)]
    );

    // A beat that already holds a node keeps it as the pin, whatever it
    // holds: mix beat 101 is the first track's node holding 125.
    apply_edit(
        &mut mix,
        &Edit::ChangeTempoFrom {
            mix_beat: Beats(101.0),
            bpm: Bpm(135.0),
        },
    )
    .unwrap();
    assert_eq!(
        mix.tracks[0].tempo,
        vec![
            tempo(100.0, 130.0),
            tempo(101.0, 125.0),
            tempo(102.0, 135.0),
            tempo(256.0, 130.0)
        ]
    );

    assert_eq!(
        apply_edit(
            &mut Mix { tracks: Vec::new() },
            &Edit::ChangeTempoFrom {
                mix_beat: Beats(0.0),
                bpm: Bpm(130.0),
            }
        ),
        Err(EditError::EmptyMix)
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::ChangeTempoFrom {
                mix_beat: Beats(10.0),
                bpm: Bpm(0.0),
            }
        ),
        Err(EditError::InvalidTempo { bpm: Bpm(0.0) })
    );

    // Both nodes are one undoable step.
    let original = two_tracks();
    let mut history = History::new(original.clone());
    history
        .apply(Edit::ChangeTempoFrom {
            mix_beat: Beats(100.0),
            bpm: Bpm(125.0),
        })
        .unwrap();
    assert_eq!(history.mix().tracks[0].tempo.len(), 3);
    assert!(history.undo());
    assert_eq!(history.mix(), &original);
}

#[test]
fn reordering_and_removing_tracks_keeps_every_node_with_its_track() {
    let mut mix = two_tracks();
    let (a, b) = (mix.tracks[0].clone(), mix.tracks[1].clone());
    apply_edit(&mut mix, &Edit::MoveTrack { from: 1, to: 0 }).unwrap();
    assert_eq!(mix.tracks, vec![b.clone(), a.clone()]);
    apply_edit(&mut mix, &Edit::MoveTrack { from: 0, to: 1 }).unwrap();
    assert_eq!(mix.tracks, vec![a.clone(), b.clone()]);
    assert_eq!(
        apply_edit(&mut mix, &Edit::MoveTrack { from: 0, to: 2 }),
        Err(EditError::NoSuchTrack {
            track: 2,
            tracks: 2
        })
    );
    apply_edit(&mut mix, &Edit::RemoveTrack { track: 0 }).unwrap();
    assert_eq!(mix.tracks, vec![b.clone()]);
    assert_eq!(
        apply_edit(&mut mix, &Edit::RemoveTrack { track: 1 }),
        Err(EditError::NoSuchTrack {
            track: 1,
            tracks: 1
        })
    );
    apply_edit(
        &mut mix,
        &Edit::SetKeylock {
            track: 0,
            keylock: false,
        },
    )
    .unwrap();
    assert!(!mix.tracks[0].keylock);
}

#[test]
fn inserting_a_track_in_the_middle_replaces_the_transition_between_its_neighbors() {
    let mut mix = two_tracks();
    // A node in the first track's transition out, one in the second track's
    // transition in, and one in the second track's transition out.
    mix.tracks[0].volume.insert(node(270.0, -3.0)).unwrap();
    mix.tracks[0].eq.low.insert(node(260.0, -90.0)).unwrap();
    mix.tracks[1].volume.insert(node(20.0, -3.0)).unwrap();
    mix.tracks[1].volume.insert(node(600.0, -3.0)).unwrap();
    mix.tracks[1].eq.mid.insert(node(20.0, -6.0)).unwrap();

    apply_edit(
        &mut mix,
        &Edit::InsertTrack {
            at: 1,
            track: Box::new(track("c", 16.0, 256.0)),
            preset: Preset::Beatmix { bars: 8 },
        },
    )
    .unwrap();

    assert_eq!(mix.tracks.len(), 3);
    assert_eq!(mix.tracks[1].path, PathBuf::from("c"));
    let a = &mix.tracks[0];
    assert_eq!(
        beats(&a.volume),
        vec![256.0, 264.0, 272.0, 280.0, 284.0, 288.0],
        "the node in the transition out is gone and the fade is fresh"
    );
    assert_eq!(a.tempo, vec![tempo(256.0, 130.0)]);
    assert!(
        a.eq.low.is_empty(),
        "EQ nodes in the transition out are cleared too"
    );
    let c = &mix.tracks[1];
    assert_eq!(
        beats(&c.volume),
        vec![
            16.0, 20.0, 24.0, 32.0, 40.0, 48.0, 256.0, 264.0, 272.0, 280.0, 284.0, 288.0
        ]
    );
    assert_eq!(c.tempo, vec![tempo(48.0, 130.0), tempo(256.0, 130.0)]);
    let b = &mix.tracks[2];
    assert_eq!(
        beats(&b.volume),
        vec![32.0, 36.0, 40.0, 48.0, 56.0, 64.0, 600.0],
        "the node in the transition in is gone, the fade is fresh, and the node past the outro anchor stays"
    );
    assert_eq!(b.tempo, vec![tempo(64.0, 130.0)]);
    assert!(
        b.eq.mid.is_empty(),
        "EQ nodes in the transition in are cleared too"
    );

    // A track appended at the end is joined only to the track before it.
    apply_edit(
        &mut mix,
        &Edit::InsertTrack {
            at: 3,
            track: Box::new(track("d", 8.0, 128.0)),
            preset: Preset::Cut,
        },
    )
    .unwrap();
    assert_eq!(mix.tracks.len(), 4);
    assert_eq!(beats(&mix.tracks[3].volume), vec![7.75, 8.0]);
    assert_eq!(
        beats(&mix.tracks[2].volume),
        vec![32.0, 36.0, 40.0, 48.0, 56.0, 64.0, 511.75, 512.0],
        "the transition out was cleared, hand-placed node included, before the cut was written"
    );

    let before = mix.clone();
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::InsertTrack {
                at: 5,
                track: Box::new(track("e", 8.0, 128.0)),
                preset: Preset::Cut,
            }
        ),
        Err(EditError::NoSuchPosition {
            position: 5,
            tracks: 4
        })
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::InsertTrack {
                at: 1,
                track: Box::new(track("e", 16.0, 40.0)),
                preset: Preset::Beatmix { bars: 8 },
            }
        ),
        Err(EditError::TransitionDoesNotFit {
            span: Beats(24.0),
            length: Beats(32.0)
        }),
        "the new track would fade in and out at once"
    );
    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::InsertTrack {
                at: 1,
                track: Box::new(track("e", 16.5, 256.0)),
                preset: Preset::Cut,
            }
        ),
        Err(EditError::NotAWholeBeat { beat: Beats(16.5) })
    );
    assert_eq!(mix, before);
}

#[test]
fn changing_the_grid_keeps_anchors_and_nodes_at_their_time_in_the_track() {
    let mut mix = two_tracks();
    mix.tracks[0].volume.insert(node(100.0, -3.0)).unwrap();
    let doubled = BeatGrid {
        first_beat: Samples::ZERO,
        bpm: Bpm(260.0),
    };
    apply_edit(
        &mut mix,
        &Edit::SetGrid {
            track: 0,
            grid: doubled,
        },
    )
    .unwrap();
    let a = &mix.tracks[0];
    assert_eq!(a.grid, doubled);
    assert_eq!(
        a.anchors,
        Anchors {
            intro: Beats(32.0),
            outro: Beats(512.0)
        }
    );
    let expected = [200.0, 512.0, 528.0, 544.0, 560.0, 568.0, 576.0];
    for (actual, expected) in beats(&a.volume).iter().zip(expected) {
        assert_close(*actual, expected, "a node's beat under the doubled grid");
    }
    assert_eq!(a.volume.len(), 7);
    assert_close(a.tempo[0].at.0, 512.0, "the tempo node's beat");
    assert_eq!(
        a.tempo[0].bpm,
        Bpm(130.0),
        "a tempo node's tempo is the mix's, not the grid's"
    );
    assert_eq!(mix.tracks[1], two_tracks().tracks[1]);

    // Sliding beat zero a quarter beat later at the same tempo: every beat
    // number falls by about a quarter, and the anchors round back to whole
    // beats. A beat at 130 beats per minute is 20353.846 samples, so 5088
    // samples is 0.24998 beats.
    let mut mix = two_tracks();
    mix.tracks[0].volume.insert(node(100.0, -3.0)).unwrap();
    let slid = BeatGrid {
        first_beat: Samples(5_088),
        bpm: Bpm(130.0),
    };
    apply_edit(
        &mut mix,
        &Edit::SetGrid {
            track: 0,
            grid: slid,
        },
    )
    .unwrap();
    let a = &mix.tracks[0];
    assert_eq!(
        a.anchors,
        Anchors {
            intro: Beats(16.0),
            outro: Beats(256.0)
        }
    );
    let first = beats(&a.volume)[0];
    assert!(
        (first - 99.75).abs() < 1e-3,
        "the node at beat 100 is now at {first}"
    );
    assert!((a.tempo[0].at.0 - 255.75).abs() < 1e-3);

    assert_eq!(
        apply_edit(
            &mut mix,
            &Edit::SetGrid {
                track: 0,
                grid: BeatGrid {
                    first_beat: Samples::ZERO,
                    bpm: Bpm(0.0),
                },
            }
        ),
        Err(EditError::InvalidTempo { bpm: Bpm(0.0) })
    );
}

#[test]
fn undo_restores_the_document_exactly_and_redo_reapplies_it() {
    let original = two_tracks();
    let mut history = History::new(original.clone());
    assert!(!history.can_undo() && !history.can_redo());
    assert!(!history.undo());
    assert!(!history.redo());

    let edits = vec![
        Edit::MoveAnchor {
            track: 0,
            anchor: Anchor::Outro,
            to: Beats(240.0),
        },
        Edit::AddNode {
            track: 1,
            curve: Curve::Low,
            node: node(100.0, -12.0),
        },
        Edit::SetTempoAt {
            mix_beat: Beats(100.0),
            bpm: Bpm(134.0),
        },
        Edit::SetKeylock {
            track: 1,
            keylock: false,
        },
        Edit::MoveTrack { from: 1, to: 0 },
        // A grid change rounds anchors, so it cannot be undone by applying
        // the old grid again; the history has to restore the document.
        Edit::SetGrid {
            track: 0,
            grid: BeatGrid {
                first_beat: Samples(5_088),
                bpm: Bpm(130.0),
            },
        },
        Edit::InsertTrack {
            at: 1,
            track: Box::new(track("c", 16.0, 256.0)),
            preset: Preset::BassSwap { bars: 8 },
        },
    ];
    let mut states = vec![original.clone()];
    for edit in &edits {
        history.apply(edit.clone()).unwrap();
        states.push(history.mix().clone());
    }
    assert_ne!(history.mix(), &original);
    assert!(history.can_undo() && !history.can_redo());

    // A refused edit records nothing, so it cannot be undone or redone.
    assert_eq!(
        history.apply(Edit::RemoveTrack { track: 9 }),
        Err(EditError::NoSuchTrack {
            track: 9,
            tracks: 3
        })
    );
    assert_eq!(history.mix(), &states[7]);

    for expected in states[..7].iter().rev() {
        assert!(history.undo());
        assert_eq!(history.mix(), expected);
    }
    assert!(!history.undo(), "nothing more to undo");
    assert_eq!(history.mix(), &original);
    assert!(!history.can_undo() && history.can_redo());

    for expected in &states[1..] {
        assert!(history.redo());
        assert_eq!(history.mix(), expected);
    }
    assert!(!history.redo(), "nothing more to redo");

    // A refused edit after an undo discards nothing, and a new edit after
    // an undo discards what could have been redone.
    history.undo();
    history.undo();
    history.undo();
    history.undo();
    assert!(history.can_redo());
    assert!(history.apply(Edit::RemoveTrack { track: 9 }).is_err());
    assert!(history.can_redo());
    assert!(history.redo());
    assert_eq!(history.mix(), &states[4]);
    history.undo();
    history
        .apply(Edit::SetKeylock {
            track: 0,
            keylock: false,
        })
        .unwrap();
    assert!(!history.can_redo());
    assert!(!history.redo());
    let mut expected = states[3].clone();
    expected.tracks[0].keylock = false;
    assert_eq!(history.mix(), &expected);
}

proptest! {
    /// Any sequence of node additions and anchor moves that the history
    /// accepts is undone back to the original document exactly.
    #[test]
        fn any_accepted_sequence_undoes_back_to_the_original(
        steps in prop::collection::vec((0usize..2, 0u8..4, 0.0f64..600.0, -20.0f64..0.0, 0u8..3), 1..40)
    ) {
        let original = two_tracks();
        let mut history = History::new(original.clone());
        let mut applied = 0;
        for (track, curve, at, db, kind) in steps {
            let edit = match kind {
                0 => Edit::AddNode {
                    track,
                    curve: [Curve::Volume, Curve::Low, Curve::Mid, Curve::High][curve as usize],
                    node: node((at * 4.0).round() / 4.0, db),
                },
                1 => Edit::MoveAnchor {
                    track,
                    anchor: Anchor::Outro,
                    to: Beats((at / 4.0).round() * 4.0 + 100.0),
                },
                _ => Edit::MoveAnchor {
                    track,
                    anchor: Anchor::Intro,
                    to: Beats((at / 8.0).round()),
                },
            };
            if history.apply(edit).is_ok() {
                applied += 1;
            }
        }
        for _ in 0..applied {
            prop_assert!(history.undo());
        }
        prop_assert!(!history.undo());
        prop_assert_eq!(history.mix(), &original);
    }
}

/// Every edit here asks for a value outside the limits of a document.
fn out_of_range_edits() -> Vec<Edit> {
    let mut slow = track("c", 16.0, 256.0);
    slow.grid.bpm = Bpm(0.0);
    let mut loud = track("c", 16.0, 256.0);
    loud.gain = Decibels(f64::NEG_INFINITY);
    let mut far = track("c", 16.0, 1e308);
    far.length = Samples(240 * 44_100);
    far.anchors.intro = Beats(-1e308);
    let mut long = track("c", 16.0, 256.0);
    long.length = Samples(238_140_001);
    let grid = |first_beat: i64, bpm: f64| BeatGrid {
        first_beat: Samples(first_beat),
        bpm: Bpm(bpm),
    };
    let insert = |track: Track| Edit::InsertTrack {
        at: 2,
        track: Box::new(track),
        preset: Preset::Cut,
    };
    let mut edits = vec![
        insert(slow),
        insert(loud),
        insert(far),
        insert(long),
        Edit::SetGrid {
            track: 0,
            grid: grid(0, 1e12),
        },
        Edit::SetGrid {
            track: 0,
            grid: grid(0, 19.999),
        },
        Edit::SetGrid {
            track: 1,
            grid: grid(i64::MIN, 130.0),
        },
        Edit::SetGrid {
            track: 1,
            grid: grid(238_140_001, 130.0),
        },
        Edit::AddNode {
            track: 0,
            curve: Curve::Volume,
            node: node(8.0, 24.5),
        },
        Edit::AddNode {
            track: 0,
            curve: Curve::Low,
            node: node(1e18, 0.0),
        },
        Edit::MoveNode {
            track: 0,
            curve: Curve::Volume,
            from: Beats(256.0),
            to: node(-10_000_001.0, 0.0),
        },
        Edit::MoveNode {
            track: 0,
            curve: Curve::Volume,
            from: Beats(256.0),
            to: node(256.0, -1e6),
        },
        Edit::AddTempoNode {
            track: 0,
            node: tempo(1e18, 130.0),
        },
        Edit::MoveTempoNode {
            track: 0,
            from: Beats(256.0),
            to: tempo(-1e300, 130.0),
        },
    ];
    for to in [1e308, -1e308, 10_000_001.0, -10_000_001.0, 1e18] {
        for anchor in [Anchor::Intro, Anchor::Outro] {
            for track in [0, 1] {
                edits.push(Edit::MoveAnchor {
                    track,
                    anchor,
                    to: Beats(to),
                });
            }
        }
    }
    for bpm in [1e-300, 1e-6, 19.999, 999.001, 1e12, 1e308] {
        edits.push(Edit::AddTempoNode {
            track: 1,
            node: tempo(100.0, bpm),
        });
        edits.push(Edit::MoveTempoNode {
            track: 0,
            from: Beats(256.0),
            to: tempo(256.0, bpm),
        });
        edits.push(Edit::SetTempoAt {
            mix_beat: Beats(100.0),
            bpm: Bpm(bpm),
        });
        edits.push(Edit::ChangeTempoFrom {
            mix_beat: Beats(100.0),
            bpm: Bpm(bpm),
        });
    }
    for mix_beat in [1e308, -1e308, 1e18, f64::INFINITY, f64::NAN] {
        edits.push(Edit::SetTempoAt {
            mix_beat: Beats(mix_beat),
            bpm: Bpm(140.0),
        });
        edits.push(Edit::ChangeTempoFrom {
            mix_beat: Beats(mix_beat),
            bpm: Bpm(140.0),
        });
    }
    edits
}

#[test]
fn an_edit_outside_the_limits_of_a_document_is_refused_and_changes_nothing() {
    for edit in out_of_range_edits() {
        let mut mix = two_tracks();
        let outcome = apply_edit(&mut mix, &edit);
        assert!(outcome.is_err(), "{edit:?} was accepted");
        assert_eq!(
            mix,
            two_tracks(),
            "{edit:?} was refused and changed the mix"
        );
    }
}

#[test]
fn two_anchor_moves_cannot_leave_a_mix_that_has_no_layout() {
    // Each of these anchors is a finite whole beat, and their difference is
    // not a finite number, so the layout of the two together would panic.
    let mut mix = two_tracks();
    let first = apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 0,
            anchor: Anchor::Outro,
            to: Beats(1e308),
        },
    );
    let second = apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 1,
            anchor: Anchor::Intro,
            to: Beats(-1e308),
        },
    );
    assert!(matches!(first, Err(EditError::OutOfRange(_))), "{first:?}");
    assert!(
        matches!(second, Err(EditError::OutOfRange(_))),
        "{second:?}"
    );
    assert_eq!(mix, two_tracks());
    assert_eq!(mix.check(), Ok(()));
}

#[test]
fn an_edit_that_makes_the_mix_longer_than_a_day_is_refused() {
    // 300,000 beats at 130 beats per minute is more than 38 hours, and every
    // number in the edit is within its own limit.
    let mut mix = two_tracks();
    let outcome = apply_edit(
        &mut mix,
        &Edit::MoveAnchor {
            track: 0,
            anchor: Anchor::Outro,
            to: Beats(300_000.0),
        },
    );
    match outcome {
        Err(EditError::OutOfRange(problem)) => {
            assert!(problem.message.contains("24 hours"), "{problem}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(mix, two_tracks());
}

fn edge_beats() -> impl Strategy<Value = f64> {
    prop::sample::select(vec![
        -10_000_000.0,
        -9_999_999.0,
        -300_000.0,
        -64.0,
        0.0,
        16.0,
        32.0,
        256.0,
        512.0,
        100_000.0,
        9_999_999.0,
        10_000_000.0,
    ])
}

fn presets() -> impl Strategy<Value = Preset> {
    prop::sample::select(vec![
        Preset::Blend,
        Preset::Cut,
        Preset::Beatmix { bars: 8 },
        Preset::BassSwap { bars: 8 },
        Preset::Beatmix { bars: u32::MAX },
        Preset::BassSwap { bars: 2_500_000 },
    ])
}

fn edits_at_the_limits() -> impl Strategy<Value = Edit> {
    let anchors = prop::sample::select(vec![Anchor::Intro, Anchor::Outro]);
    let bpms = prop::sample::select(vec![20.0, 130.0, 999.0]);
    prop_oneof![
        (0usize..2, anchors, edge_beats()).prop_map(|(track, anchor, to)| Edit::MoveAnchor {
            track,
            anchor,
            to: Beats(to)
        }),
        (
            0usize..3,
            edge_beats(),
            edge_beats(),
            presets(),
            bpms.clone()
        )
            .prop_map(|(at, intro, outro, preset, bpm)| {
                let mut new = track("c", intro, outro);
                new.grid.bpm = Bpm(bpm);
                Edit::InsertTrack {
                    at,
                    track: Box::new(new),
                    preset,
                }
            }),
        (edge_beats(), bpms.clone()).prop_map(|(at, bpm)| Edit::SetTempoAt {
            mix_beat: Beats(at),
            bpm: Bpm(bpm)
        }),
        (edge_beats(), bpms.clone()).prop_map(|(at, bpm)| Edit::ChangeTempoFrom {
            mix_beat: Beats(at),
            bpm: Bpm(bpm)
        }),
        (0usize..2, edge_beats(), bpms.clone()).prop_map(|(track, at, bpm)| Edit::AddTempoNode {
            track,
            node: tempo(at, bpm)
        }),
        (0usize..2, -238_140_000i64..=238_140_000, bpms).prop_map(|(track, first_beat, bpm)| {
            Edit::SetGrid {
                track,
                grid: BeatGrid {
                    first_beat: Samples(first_beat),
                    bpm: Bpm(bpm),
                },
            }
        }),
        (0usize..2, 0usize..2).prop_map(|(from, to)| Edit::MoveTrack { from, to }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn edits_at_the_limits_never_panic_and_never_leave_a_mix_the_reader_refuses(
        edits in prop::collection::vec(edits_at_the_limits(), 1..6),
    ) {
        let outcome = std::panic::catch_unwind(|| {
            let mut mix = two_tracks();
            for edit in &edits {
                let before = mix.clone();
                match apply_edit(&mut mix, edit) {
                    Ok(()) => {
                        if let Err(problem) = mix.check() {
                            return Err(format!("{edit:?} left a refused mix: {problem}"));
                        }
                        if mix.timeline().is_none() && !mix.tracks.is_empty() {
                            return Err(format!("{edit:?} left a mix with no layout"));
                        }
                    }
                    Err(_) if mix != before => {
                        return Err(format!("{edit:?} was refused and changed the mix"));
                    }
                    Err(_) => {}
                }
            }
            Ok(())
        });
        match outcome {
            Ok(result) => prop_assert_eq!(result, Ok(())),
            Err(_) => prop_assert!(false, "panicked on {:?}", edits),
        }
    }
}
