//! Acceptance tests for the beat grid. A coder agent makes these pass without editing them.

use dermixen_core::{BEATS_PER_BAR, BeatGrid, Beats, Bpm, Samples, Seconds};
use proptest::prelude::*;

fn grids() -> impl Strategy<Value = BeatGrid> {
    (0i64..2_000_000, 60.0f64..200.0).prop_map(|(first, bpm)| BeatGrid {
        first_beat: Samples(first),
        bpm: Bpm(bpm),
    })
}

proptest! {
    #[test]
    fn beat_zero_is_the_first_beat(grid in grids()) {
        prop_assert_eq!(grid.position_of(Beats::ZERO), grid.first_beat);
        prop_assert_eq!(grid.time_of(Beats::ZERO), grid.first_beat.to_seconds());
        prop_assert!(grid.beat_at(grid.first_beat.to_seconds()).0.abs() < 1e-9);
    }

    #[test]

    fn beat_index_round_trips_through_time(grid in grids(), beat in -1000.0f64..10_000.0) {
        let back = grid.beat_at(grid.time_of(Beats(beat)));
        prop_assert!((back.0 - beat).abs() < 1e-6, "{beat} came back as {}", back.0);
    }

    #[test]

    fn time_round_trips_through_beat_index(grid in grids(), time in -60.0f64..600.0) {
        let back = grid.time_of(grid.beat_at(Seconds(time)));
        prop_assert!((back.0 - time).abs() < 1e-9, "{time} came back as {}", back.0);
    }

    #[test]

    fn consecutive_beats_are_one_period_apart(grid in grids(), beat in -1000.0f64..10_000.0) {
        let a = grid.time_of(Beats(beat));
        let b = grid.time_of(Beats(beat + 1.0));
        prop_assert!(((b - a).0 - grid.beat_period().0).abs() < 1e-9);
    }

    #[test]

    fn later_beats_come_later(grid in grids(), a in -1000.0f64..10_000.0, b in -1000.0f64..10_000.0) {
        prop_assume!(a < b);
        prop_assert!(grid.time_of(Beats(a)) < grid.time_of(Beats(b)));
        prop_assert!(grid.position_of(Beats(a)) <= grid.position_of(Beats(b)));
    }

    #[test]

    fn positions_agree_with_times(grid in grids(), beat in -1000.0f64..10_000.0, position in -3_000_000i64..30_000_000) {
        prop_assert_eq!(grid.position_of(Beats(beat)), grid.time_of(Beats(beat)).to_samples());
        let position = Samples(position);
        let via_time = grid.beat_at(position.to_seconds());
        prop_assert!((grid.beat_at_position(position).0 - via_time.0).abs() < 1e-9);
    }

    #[test]

    fn nearest_beat_is_whole_and_within_half_a_beat(grid in grids(), time in -60.0f64..600.0) {
        let nearest = grid.nearest_beat(Seconds(time));
        prop_assert!(nearest.is_whole());
        let exact = grid.beat_at(Seconds(time));
        prop_assert!((nearest.0 - exact.0).abs() <= 0.5 + 1e-9);
    }

    #[test]

    fn nearest_bar_is_a_whole_bar_and_within_half_a_bar(grid in grids(), time in -60.0f64..600.0) {
        let nearest = grid.nearest_bar(Seconds(time));
        prop_assert!(nearest.is_whole());
        prop_assert_eq!(nearest.0 % f64::from(BEATS_PER_BAR), 0.0);
        let exact = grid.beat_at(Seconds(time));
        prop_assert!((nearest.0 - exact.0).abs() <= f64::from(BEATS_PER_BAR) / 2.0 + 1e-9);
    }
}

#[test]
fn a_known_grid_at_138_bpm() {
    // The first beat is half a second in, so beat four is at 0.5 + 4 * 60 / 138 seconds.
    let grid = BeatGrid {
        first_beat: Seconds(0.5).to_samples(),
        bpm: Bpm(138.0),
    };
    let period = 60.0 / 138.0;
    let beat_four = 0.5 + 4.0 * period;
    assert!((grid.time_of(Beats(4.0)).0 - beat_four).abs() < 1e-12);
    assert_eq!(
        grid.position_of(Beats(4.0)),
        Samples((beat_four * 44_100.0).round() as i64)
    );
    assert!(grid.beat_at(Seconds(0.5)).0.abs() < 1e-12);
    // Half a second before the first beat is minus 1.15 beats at this tempo.
    assert!((grid.beat_at(Seconds(0.0)).0 - (-0.5 / period)).abs() < 1e-12);
    assert_eq!(grid.nearest_beat(Seconds(0.5 + 3.4 * period)), Beats(3.0));
    assert_eq!(grid.nearest_beat(Seconds(0.5 - 0.6 * period)), Beats(-1.0));
    assert_eq!(grid.nearest_bar(Seconds(0.5 + 5.0 * period)), Beats(4.0));
    assert_eq!(grid.nearest_bar(Seconds(0.5 + 6.5 * period)), Beats(8.0));
    assert_eq!(grid.nearest_bar(Seconds(0.5 - 1.0 * period)), Beats(0.0));
}
