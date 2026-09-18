//! Acceptance tests for the playback state: what the transport buttons,
//! the space bar, and a click on the ruler do. A coder agent makes these pass
//! without editing them.

use dermixen_app::{Playback, PlaybackState, TransportOrder};
use dermixen_core::Samples;
use dermixen_engine::{TransportState, TransportStatus};

fn status(state: TransportState, position: i64, length: i64) -> TransportStatus {
    TransportStatus {
        state,
        position: Samples(position),
        length: Samples(length),
        underruns: 0,
        reached: Samples(position),
        restarts: 0,
        last_restart: None,
    }
}

fn start(at: i64) -> TransportOrder {
    TransportOrder::Start { at: Samples(at) }
}

fn seek(at: i64) -> TransportOrder {
    TransportOrder::Seek { at: Samples(at) }
}

const LENGTH: Samples = Samples(10_000);

#[test]
fn playing_starts_at_the_playhead_and_records_the_start_point() {
    let mut playback = Playback::new(Samples(1000));
    assert_eq!(playback.state(), PlaybackState::Stopped);
    assert_eq!(playback.playhead(), Samples(1000));
    assert_eq!(playback.start_point(), Samples(1000));

    assert_eq!(playback.play(LENGTH), vec![start(1000)]);
    assert_eq!(playback.state(), PlaybackState::Playing);
    assert_eq!(playback.start_point(), Samples(1000));
    assert_eq!(
        playback.play(LENGTH),
        vec![],
        "a playing transport is left alone"
    );

    playback.heard(&status(TransportState::Buffering, 1000, 10_000));
    assert_eq!(
        playback.state(),
        PlaybackState::Playing,
        "buffering is shown as playing"
    );
    playback.heard(&status(TransportState::Playing, 2500, 10_000));
    assert_eq!(playback.playhead(), Samples(2500));
    assert_eq!(playback.start_point(), Samples(1000));

    // A playhead at or past the end starts from the beginning.
    for at in [10_000, 20_000] {
        let mut playback = Playback::new(Samples(at));
        assert_eq!(playback.play(LENGTH), vec![start(0)]);
        assert_eq!(playback.start_point(), Samples::ZERO);
        assert_eq!(playback.playhead(), Samples::ZERO);
    }
}

#[test]
fn pausing_holds_and_playing_resumes_without_moving_the_start_point() {
    let mut playback = Playback::new(Samples(1000));
    assert_eq!(playback.pause(), vec![], "nothing to pause");
    playback.play(LENGTH);
    playback.heard(&status(TransportState::Playing, 5000, 10_000));
    assert_eq!(playback.pause(), vec![TransportOrder::Pause]);
    assert_eq!(playback.state(), PlaybackState::Paused);
    playback.heard(&status(TransportState::Paused, 5000, 10_000));
    assert_eq!(playback.playhead(), Samples(5000));
    assert_eq!(playback.pause(), vec![], "already paused");
    assert_eq!(playback.play(LENGTH), vec![TransportOrder::Resume]);
    assert_eq!(playback.state(), PlaybackState::Playing);
    assert_eq!(playback.start_point(), Samples(1000));
}

#[test]
fn stopping_returns_the_playhead_to_the_start_point() {
    let mut playback = Playback::new(Samples(1000));
    assert_eq!(playback.stop(LENGTH), vec![], "nothing to stop");
    playback.play(LENGTH);
    playback.heard(&status(TransportState::Playing, 5000, 10_000));
    assert_eq!(playback.stop(LENGTH), vec![TransportOrder::Stop]);
    assert_eq!(playback.state(), PlaybackState::Stopped);
    assert_eq!(playback.playhead(), Samples(1000));
    assert_eq!(playback.start_point(), Samples(1000));

    // Stopping a pause does the same.
    playback.play(LENGTH);
    playback.pause();
    playback.heard(&status(TransportState::Paused, 7000, 10_000));
    assert_eq!(playback.stop(LENGTH), vec![TransportOrder::Stop]);
    assert_eq!(playback.playhead(), Samples(1000));

    // An edit that left the mix shorter than the start point leaves nowhere
    // to return to but the mix's end.
    playback.play(LENGTH);
    playback.heard(&status(TransportState::Playing, 900, 10_000));
    assert_eq!(playback.stop(Samples(500)), vec![TransportOrder::Stop]);
    assert_eq!(playback.playhead(), Samples(500));
    assert_eq!(playback.start_point(), Samples(500));
}

#[test]
fn the_space_bar_stops_a_playing_transport_and_otherwise_plays() {
    let mut playback = Playback::new(Samples(1000));
    assert_eq!(playback.space(LENGTH), vec![start(1000)]);
    playback.heard(&status(TransportState::Playing, 3000, 10_000));
    assert_eq!(playback.space(LENGTH), vec![TransportOrder::Stop]);
    assert_eq!(playback.state(), PlaybackState::Stopped);
    assert_eq!(playback.playhead(), Samples(1000));

    playback.play(LENGTH);
    playback.heard(&status(TransportState::Buffering, 1000, 10_000));
    assert_eq!(
        playback.space(LENGTH),
        vec![TransportOrder::Stop],
        "buffering counts as playing"
    );

    playback.play(LENGTH);
    playback.pause();
    assert_eq!(
        playback.space(LENGTH),
        vec![TransportOrder::Resume],
        "a pause is resumed, not started over"
    );

    playback.heard(&status(TransportState::Ended, 10_000, 10_000));
    assert_eq!(
        playback.space(LENGTH),
        vec![seek(1000), TransportOrder::Resume]
    );
}

#[test]
fn playing_after_the_end_returns_to_the_start_point() {
    let mut playback = Playback::new(Samples(1000));
    playback.play(LENGTH);
    playback.heard(&status(TransportState::Ended, 10_000, 10_000));
    assert_eq!(playback.state(), PlaybackState::Ended);
    assert_eq!(playback.playhead(), Samples(10_000));
    assert_eq!(
        playback.play(LENGTH),
        vec![seek(1000), TransportOrder::Resume]
    );
    assert_eq!(playback.state(), PlaybackState::Playing);
    assert_eq!(playback.playhead(), Samples(1000));
    assert_eq!(playback.start_point(), Samples(1000));

    // A start point the mix no longer reaches leaves only the beginning.
    playback.heard(&status(TransportState::Ended, 800, 800));
    assert_eq!(
        playback.play(Samples(800)),
        vec![seek(0), TransportOrder::Resume]
    );
    assert_eq!(playback.start_point(), Samples::ZERO);
    assert_eq!(playback.playhead(), Samples::ZERO);

    // Stopping after the end returns to the start point as it always does.
    playback.heard(&status(TransportState::Ended, 800, 800));
    assert_eq!(playback.stop(Samples(800)), vec![TransportOrder::Stop]);
    assert_eq!(playback.state(), PlaybackState::Stopped);
    assert_eq!(playback.playhead(), Samples::ZERO);
}

#[test]
fn a_ruler_click_moves_the_playhead_the_start_point_and_a_running_transport() {
    let mut playback = Playback::new(Samples(1000));
    assert_eq!(
        playback.click_ruler(Samples(3000)),
        vec![],
        "no transport to move"
    );
    assert_eq!(playback.playhead(), Samples(3000));
    assert_eq!(playback.start_point(), Samples(3000));
    assert_eq!(playback.play(LENGTH), vec![start(3000)]);

    playback.heard(&status(TransportState::Playing, 3500, 10_000));
    assert_eq!(playback.click_ruler(Samples(4000)), vec![seek(4000)]);
    assert_eq!(playback.state(), PlaybackState::Playing);
    assert_eq!(playback.playhead(), Samples(4000));
    assert_eq!(playback.start_point(), Samples(4000));

    playback.pause();
    assert_eq!(playback.click_ruler(Samples(4500)), vec![seek(4500)]);
    assert_eq!(
        playback.state(),
        PlaybackState::Paused,
        "a paused transport stays paused where it was moved"
    );
    assert_eq!(playback.start_point(), Samples(4500));

    playback.heard(&status(TransportState::Ended, 10_000, 10_000));
    assert_eq!(playback.click_ruler(Samples(6000)), vec![seek(6000)]);
    assert_eq!(
        playback.state(),
        PlaybackState::Paused,
        "a transport that had played out is paused at the click"
    );
    assert_eq!(playback.playhead(), Samples(6000));
    playback.heard(&status(TransportState::Paused, 6000, 10_000));
    assert_eq!(playback.play(LENGTH), vec![TransportOrder::Resume]);
    playback.heard(&status(TransportState::Playing, 6500, 10_000));
    assert_eq!(playback.stop(LENGTH), vec![TransportOrder::Stop]);
    assert_eq!(playback.playhead(), Samples(6000));
}

#[test]
fn a_start_that_could_not_be_carried_out_leaves_the_transport_stopped() {
    let mut playback = Playback::new(Samples(1000));
    assert_eq!(playback.play(LENGTH), vec![start(1000)]);
    playback.could_not_start();
    assert_eq!(playback.state(), PlaybackState::Stopped);
    assert_eq!(playback.playhead(), Samples(1000));
    assert_eq!(playback.start_point(), Samples(1000));
    assert_eq!(playback.stop(LENGTH), vec![], "nothing is running");
    assert_eq!(
        playback.play(LENGTH),
        vec![start(1000)],
        "playing tries again"
    );
}

#[test]
fn a_failure_is_shown_and_playing_again_starts_fresh() {
    let mut playback = Playback::new(Samples(1000));
    playback.play(LENGTH);
    playback.heard(&status(
        TransportState::Failed("the device was unplugged".to_owned()),
        2000,
        10_000,
    ));
    assert_eq!(
        playback.state(),
        PlaybackState::Failed("the device was unplugged".to_owned())
    );
    assert_eq!(playback.playhead(), Samples(2000));
    assert_eq!(
        playback.pause(),
        vec![],
        "a failed transport takes no pause"
    );
    assert_eq!(
        playback.play(LENGTH),
        vec![TransportOrder::Stop, start(1000)],
        "playing again stops the failed transport, which returns the playhead to the start point, and starts there"
    );
    assert_eq!(playback.state(), PlaybackState::Playing);
    assert_eq!(playback.playhead(), Samples(1000));
    assert_eq!(playback.start_point(), Samples(1000));

    playback.heard(&status(
        TransportState::Failed("gone".to_owned()),
        2000,
        10_000,
    ));
    assert_eq!(
        playback.click_ruler(Samples(2500)),
        vec![],
        "a failed transport is not moved"
    );
    assert_eq!(playback.playhead(), Samples(2500));
    assert_eq!(playback.start_point(), Samples(2500));
    assert_eq!(
        playback.play(LENGTH),
        vec![TransportOrder::Stop, start(2500)]
    );

    playback.heard(&status(
        TransportState::Failed("gone".to_owned()),
        2600,
        10_000,
    ));
    assert_eq!(playback.stop(LENGTH), vec![TransportOrder::Stop]);
    assert_eq!(playback.state(), PlaybackState::Stopped);
    assert_eq!(playback.playhead(), Samples(2500));
}
