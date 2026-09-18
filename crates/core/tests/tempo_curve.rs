//! Acceptance tests for the mix tempo curve and the placement of tracks on it.
//! A coder agent makes these pass without editing them.

use dermixen_core::{
    BeatGrid, Beats, Bpm, PlacedTrack, Samples, Seconds, TempoCurve, TempoCurveError, TempoNode,
};
use proptest::prelude::*;

fn node(at: f64, bpm: f64) -> TempoNode {
    TempoNode {
        at: Beats(at),
        bpm: Bpm(bpm),
    }
}

/// Between one and eight nodes at distinct whole-beat positions, in a random order.
fn node_lists() -> impl Strategy<Value = Vec<TempoNode>> {
    prop::collection::btree_set(-500i64..5000, 1..8)
        .prop_flat_map(|set| {
            let positions: Vec<f64> = set.into_iter().map(|p| p as f64).collect();
            let count = positions.len();
            (
                Just(positions),
                prop::collection::vec(60.0f64..200.0, count),
                any::<u64>(),
            )
        })
        .prop_map(|(positions, bpms, seed)| {
            let mut nodes: Vec<TempoNode> = positions
                .into_iter()
                .zip(bpms)
                .map(|(p, b)| node(p, b))
                .collect();
            let count = nodes.len();
            for i in 0..count {
                let j = (seed as usize).wrapping_mul(31).wrapping_add(i * 17) % count;
                nodes.swap(i, j);
            }
            nodes
        })
}

fn curves() -> impl Strategy<Value = TempoCurve> {
    node_lists().prop_map(|nodes| TempoCurve::new(nodes).unwrap())
}

fn grids() -> impl Strategy<Value = BeatGrid> {
    (0i64..200_000, 60.0f64..200.0).prop_map(|(first, bpm)| BeatGrid {
        first_beat: Samples(first),
        bpm: Bpm(bpm),
    })
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * (1.0 + a.abs().max(b.abs()))
}

#[test]
fn a_curve_needs_at_least_one_node() {
    assert_eq!(TempoCurve::new(vec![]), Err(TempoCurveError::Empty));
}

#[test]
fn tempos_must_be_positive_and_finite() {
    for bad in [0.0, -120.0, f64::NAN, f64::INFINITY] {
        let result = TempoCurve::new(vec![node(0.0, 120.0), node(32.0, bad)]);
        assert!(
            matches!(result, Err(TempoCurveError::InvalidBpm { index: 1, .. })),
            "tempo {bad} was accepted"
        );
    }
    let result = TempoCurve::new(vec![node(f64::NAN, 120.0)]);
    assert_eq!(result, Err(TempoCurveError::InvalidPosition { index: 0 }));
}

#[test]
fn nodes_are_sorted_by_position() {
    let curve =
        TempoCurve::new(vec![node(64.0, 140.0), node(0.0, 120.0), node(32.0, 130.0)]).unwrap();
    assert_eq!(
        curve.nodes(),
        &[node(0.0, 120.0), node(32.0, 130.0), node(64.0, 140.0)]
    );
}

#[test]
fn a_constant_curve_is_plain_multiplication() {
    let curve = TempoCurve::constant(Bpm(120.0));
    assert_eq!(curve.time_at(Beats::ZERO), Seconds::ZERO);
    assert_eq!(curve.time_at(Beats(2.0)), Seconds(1.0));
    assert_eq!(curve.time_at(Beats(-2.0)), Seconds(-1.0));
    assert_eq!(curve.beat_at(Seconds(30.0)), Beats(60.0));
    for beat in [-100.0, 0.0, 17.5, 1e5] {
        assert_eq!(curve.bpm_at_beat(Beats(beat)), Bpm(120.0));
    }
    for time in [-100.0, 0.0, 17.5, 1e5] {
        assert_eq!(curve.bpm_at_time(Seconds(time)), Bpm(120.0));
    }
}

#[test]
fn a_ramp_lasts_the_beat_span_at_the_average_tempo() {
    // Thirty-two beats from 130 to 140 BPM take 32 * 60 / 135 seconds.
    let curve = TempoCurve::new(vec![node(0.0, 130.0), node(32.0, 140.0)]).unwrap();
    let duration = curve.time_at(Beats(32.0)) - curve.time_at(Beats(0.0));
    assert!(close(duration.0, 32.0 * 60.0 / 135.0, 1e-12));
    // The tempo is linear in time, so halfway through the ramp in time it is 135 BPM.
    let halfway = Seconds(curve.time_at(Beats(0.0)).0 + duration.0 / 2.0);
    assert!(close(curve.bpm_at_time(halfway).0, 135.0, 1e-9));
    // Halfway in time is past halfway in beats, because the second half is faster.
    assert!(curve.beat_at(halfway).0 < 16.0);
    // The mapping is exact at the nodes.
    assert!(close(
        curve.beat_at(curve.time_at(Beats(32.0))).0,
        32.0,
        1e-12
    ));
}

#[test]
fn two_nodes_at_one_beat_make_an_instant_change() {
    let curve = TempoCurve::new(vec![
        node(0.0, 120.0),
        node(16.0, 120.0),
        node(16.0, 140.0),
        node(32.0, 140.0),
    ])
    .unwrap();
    let expected = 16.0 * 60.0 / 120.0 + 16.0 * 60.0 / 140.0;
    assert!(close(curve.time_at(Beats(32.0)).0, expected, 1e-12));
    assert_eq!(curve.bpm_at_beat(Beats(16.0)), Bpm(140.0));
    assert_eq!(curve.bpm_at_beat(Beats(15.999)), Bpm(120.0));
}

#[test]
fn nodes_at_one_beat_keep_the_order_they_were_given_in() {
    // The outer nodes are swapped, so the curve ramps from 140 down to 120 over
    // the first sixteen beats, jumps back to 140 at beat 16 because the second
    // of the two nodes there wins, and ramps down to 120 again.
    let curve = TempoCurve::new(vec![
        node(32.0, 120.0),
        node(16.0, 120.0),
        node(16.0, 140.0),
        node(0.0, 140.0),
    ])
    .unwrap();
    assert_eq!(
        curve.nodes(),
        &[
            node(0.0, 140.0),
            node(16.0, 120.0),
            node(16.0, 140.0),
            node(32.0, 120.0)
        ]
    );
    // Two ramps of sixteen beats, each at the average of 140 and 120.
    let expected = 2.0 * 120.0 * 16.0 / (140.0 + 120.0);
    assert!(close(curve.time_at(Beats(32.0)).0, expected, 1e-12));
    assert_eq!(curve.bpm_at_beat(Beats(16.0)), Bpm(140.0));
    assert!(close(curve.bpm_at_beat(Beats(16.001)).0, 140.0, 1e-4));
    assert!(close(curve.bpm_at_beat(Beats(15.999)).0, 120.0, 1e-4));
}

#[test]
fn the_curve_extends_flat_beyond_its_nodes() {
    let curve = TempoCurve::new(vec![node(100.0, 130.0), node(132.0, 140.0)]).unwrap();
    assert_eq!(curve.bpm_at_beat(Beats(-1e6)), Bpm(130.0));
    assert_eq!(curve.bpm_at_beat(Beats(1e6)), Bpm(140.0));
    // One hundred beats at 130 BPM before the first node.
    assert!(close(
        curve.time_at(Beats(100.0)).0,
        100.0 * 60.0 / 130.0,
        1e-12
    ));
    // Every beat after the last node lasts 60 / 140 seconds.
    let a = curve.time_at(Beats(1000.0));
    let b = curve.time_at(Beats(1001.0));
    assert!(close((b - a).0, 60.0 / 140.0, 1e-12));
}

proptest! {
    #[test]
    fn beat_zero_is_time_zero(curve in curves()) {
        prop_assert_eq!(curve.time_at(Beats::ZERO), Seconds::ZERO);
        prop_assert_eq!(curve.beat_at(Seconds::ZERO), Beats::ZERO);
    }

    #[test]

    fn beats_round_trip_through_time(curve in curves(), beat in -1000.0f64..6000.0) {
        let back = curve.beat_at(curve.time_at(Beats(beat)));
        prop_assert!(close(back.0, beat, 1e-9), "{beat} came back as {}", back.0);
    }

    #[test]

    fn time_round_trips_through_beats(curve in curves(), time in -600.0f64..3600.0) {
        let back = curve.time_at(curve.beat_at(Seconds(time)));
        prop_assert!(close(back.0, time, 1e-9), "{time} came back as {}", back.0);
    }

    #[test]

    fn time_increases_with_beats(curve in curves(), a in -1000.0f64..6000.0, b in -1000.0f64..6000.0) {
        prop_assume!(a < b);
        prop_assert!(curve.time_at(Beats(a)) < curve.time_at(Beats(b)));
    }

    #[test]

    fn the_tempo_at_a_node_is_that_node(nodes in node_lists()) {
        let curve = TempoCurve::new(nodes.clone()).unwrap();
        for n in &nodes {
            prop_assert_eq!(curve.bpm_at_beat(n.at), n.bpm);
            prop_assert!(close(curve.bpm_at_time(curve.time_at(n.at)).0, n.bpm.0, 1e-9));
        }
    }

    #[test]

    fn the_tempo_between_nodes_stays_between_them(curve in curves(), fraction in 0.0f64..1.0) {
        for pair in curve.nodes().windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let at = Beats(a.at.0 + (b.at.0 - a.at.0) * fraction);
            let bpm = curve.bpm_at_beat(at).0;
            let (low, high) = (a.bpm.0.min(b.bpm.0), a.bpm.0.max(b.bpm.0));
            prop_assert!(bpm >= low - 1e-9 && bpm <= high + 1e-9);
        }
    }

    #[test]

    fn the_tempo_agrees_whichever_way_it_is_asked(curve in curves(), beat in -1000.0f64..6000.0) {
        let by_beat = curve.bpm_at_beat(Beats(beat));
        let by_time = curve.bpm_at_time(curve.time_at(Beats(beat)));
        prop_assert!(close(by_beat.0, by_time.0, 1e-9));
    }

    #[test]

    fn every_ramp_lasts_its_beat_span_at_the_average_tempo(curve in curves()) {
        for pair in curve.nodes().windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let duration = curve.time_at(b.at) - curve.time_at(a.at);
            let expected = 120.0 * (b.at.0 - a.at.0) / (a.bpm.0 + b.bpm.0);
            prop_assert!(close(duration.0, expected, 1e-9));
        }
    }

    #[test]

    fn a_placed_track_maps_both_ways(curve in curves(), grid in grids(), origin in -500i64..5000, track_time in 0.0f64..600.0) {
        let track = PlacedTrack { origin: Beats(origin as f64), grid, length: Samples(30_000_000) };
        let mix_beat = track.mix_beat_of(Seconds(track_time));
        prop_assert!(close(mix_beat.0, origin as f64 + grid.beat_at(Seconds(track_time)).0, 1e-9));
        prop_assert!(close(track.track_time_at_beat(mix_beat).0, track_time, 1e-9));
        let mix_time = track.mix_time_of(&curve, Seconds(track_time));
        prop_assert!(close(mix_time.0, curve.time_at(mix_beat).0, 1e-9));
        prop_assert!(close(track.track_time_at(&curve, mix_time).0, track_time, 1e-9));
    }

    #[test]

    fn the_rate_is_the_mix_tempo_over_the_original_tempo(curve in curves(), grid in grids(), origin in -500i64..5000, mix_time in -600.0f64..3600.0) {
        let track = PlacedTrack { origin: Beats(origin as f64), grid, length: Samples(30_000_000) };
        let expected = curve.bpm_at_time(Seconds(mix_time)).0 / grid.bpm.0;
        prop_assert!(close(track.rate_at(&curve, Seconds(mix_time)), expected, 1e-9));
    }

    #[test]

    fn a_track_at_its_own_tempo_plays_at_speed_one(grid in grids(), origin in -500i64..5000, mix_time in -600.0f64..3600.0, step in 0.0f64..100.0) {
        let curve = TempoCurve::constant(grid.bpm);
        let track = PlacedTrack { origin: Beats(origin as f64), grid, length: Samples(30_000_000) };
        prop_assert!(close(track.rate_at(&curve, Seconds(mix_time)), 1.0, 1e-12));
        let before = track.track_time_at(&curve, Seconds(mix_time));
        let after = track.track_time_at(&curve, Seconds(mix_time + step));
        prop_assert!(close((after - before).0, step, 1e-9));
    }

    #[test]

    fn a_track_at_double_tempo_plays_twice_as_fast(grid in grids(), origin in -500i64..5000, mix_time in -600.0f64..3600.0, step in 0.0f64..100.0) {
        let curve = TempoCurve::constant(Bpm(grid.bpm.0 * 2.0));
        let track = PlacedTrack { origin: Beats(origin as f64), grid, length: Samples(30_000_000) };
        prop_assert!(close(track.rate_at(&curve, Seconds(mix_time)), 2.0, 1e-12));
        let before = track.track_time_at(&curve, Seconds(mix_time));
        let after = track.track_time_at(&curve, Seconds(mix_time + step));
        prop_assert!(close((after - before).0, 2.0 * step, 1e-9));
    }

    #[test]

    fn a_track_starts_at_its_first_sample_and_ends_at_its_last(curve in curves(), grid in grids(), origin in -500i64..5000, length in 1i64..30_000_000) {
        let track = PlacedTrack { origin: Beats(origin as f64), grid, length: Samples(length) };
        prop_assert!(close(track.start(&curve).0, track.mix_time_of(&curve, Seconds::ZERO).0, 1e-12));
        prop_assert!(close(track.end(&curve).0, track.mix_time_of(&curve, Samples(length).to_seconds()).0, 1e-12));
        prop_assert!(track.start(&curve) < track.end(&curve));
    }

    #[test]

    fn two_tracks_aligned_on_a_beat_share_every_beat(
        curve in curves(),
        grid_a in grids(),
        grid_b in grids(),
        outro_a in 0i64..3000,
        intro_b in 0i64..3000,
        beat in -1000i64..4000,
    ) {
        // Track A's outro anchor is aligned with track B's intro anchor, as the mix layout does.
        let a = PlacedTrack { origin: Beats::ZERO, grid: grid_a, length: Samples(30_000_000) };
        let b = PlacedTrack { origin: Beats((outro_a - intro_b) as f64), grid: grid_b, length: Samples(30_000_000) };
        // Every whole beat of A is heard at the same mix time as some whole beat of B,
        // whatever the curve does, because both tracks follow the one curve.
        let beat_of_a = Beats(beat as f64);
        let beat_of_b = Beats((beat - (outro_a - intro_b)) as f64);
        let time_via_a = a.mix_time_of(&curve, grid_a.time_of(beat_of_a));
        let time_via_b = b.mix_time_of(&curve, grid_b.time_of(beat_of_b));
        prop_assert!(close(time_via_a.0, time_via_b.0, 1e-9), "{} vs {}", time_via_a.0, time_via_b.0);
        // The anchors themselves coincide.
        let anchor_via_a = a.mix_time_of(&curve, grid_a.time_of(Beats(outro_a as f64)));
        let anchor_via_b = b.mix_time_of(&curve, grid_b.time_of(Beats(intro_b as f64)));
        prop_assert!(close(anchor_via_a.0, anchor_via_b.0, 1e-9));
        // At any moment both tracks are stretched to the same mix tempo.
        let mix_time = anchor_via_a;
        let tempo_via_a = a.rate_at(&curve, mix_time) * grid_a.bpm.0;
        let tempo_via_b = b.rate_at(&curve, mix_time) * grid_b.bpm.0;
        prop_assert!(close(tempo_via_a, tempo_via_b, 1e-9));
    }
}
