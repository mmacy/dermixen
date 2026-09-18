//! Acceptance tests for the volume leveling rule. A coder agent makes these pass
//! without editing them.
//!
//! Every expectation here is arithmetic on the two constants: the target
//! loudness and the true peak ceiling.

use dermixen_core::{Decibels, Lufs, TARGET_LOUDNESS, TRUE_PEAK_CEILING, leveling_gain};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn the_target_and_the_ceiling_are_the_numbers_the_documents_give() {
    assert_eq!(TARGET_LOUDNESS, Lufs(-14.0));
    assert_eq!(TRUE_PEAK_CEILING, Decibels(-1.0));
}

#[test]
fn a_loudness_and_a_gain_add_up_and_two_loudnesses_differ_by_a_gain() {
    assert_eq!(Lufs(-20.0) + Decibels(6.0), Lufs(-14.0));
    assert_eq!(Lufs(-14.0) - Lufs(-20.0), Decibels(6.0));
}

#[test]
fn a_loud_track_is_turned_down_to_the_target() {
    // Minus eight LUFS with peaks at minus 0.2 dBTP: minus six brings it to
    // the target, and turning a track down never lifts a peak.
    let gain = leveling_gain(Lufs(-8.0), Decibels(-0.2));
    assert!(close(gain.0, -6.0), "{gain:?}");
}

#[test]
fn a_quiet_track_with_room_is_turned_up_to_the_target() {
    // Minus twenty LUFS with peaks at minus eight dBTP: plus six reaches the
    // target and leaves the peaks at minus two, under the ceiling.
    let gain = leveling_gain(Lufs(-20.0), Decibels(-8.0));
    assert!(close(gain.0, 6.0), "{gain:?}");
}

#[test]
fn a_quiet_track_with_sharp_peaks_stops_at_the_ceiling() {
    // Minus twenty LUFS with peaks at minus three dBTP: the target asks for
    // plus six, but plus two already puts the peaks at the ceiling.
    let gain = leveling_gain(Lufs(-20.0), Decibels(-3.0));
    assert!(close(gain.0, 2.0), "{gain:?}");
    // Peaks already over the ceiling are brought down to it even though the
    // track is quiet.
    let gain = leveling_gain(Lufs(-20.0), Decibels(0.5));
    assert!(close(gain.0, -1.5), "{gain:?}");
}

#[test]
fn a_track_at_the_target_with_peaks_at_the_ceiling_is_left_alone() {
    assert_eq!(leveling_gain(Lufs(-14.0), Decibels(-1.0)), Decibels::UNITY);
}
