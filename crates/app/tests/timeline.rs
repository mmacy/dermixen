//! Acceptance tests for the timeline view-model. A coder agent makes these pass
//! without editing them.
//!
//! The fixture is two tracks at 120 beats per minute whose grids start at
//! their first sample, so a beat is half a second and a bar two seconds:
//! the first track is 200 seconds long with anchors at beats 16 and 256,
//! the second 300 seconds long with anchors at beats 32 and 512, joined by
//! an eight-bar beatmix. The second track's beat zero therefore falls at mix
//! beat 256 - 32 = 224, which is 112 seconds, and the mix is 412 seconds
//! long. The default view is a thousand pixels over the first hundred
//! seconds, ten pixels per second, with lanes a hundred pixels tall.

use std::path::PathBuf;

use dermixen_app::{
    DEFAULT_TEMPO, EDGE_PX, GridEditor, HIT_PX, MAX_LANE_PX, MIN_BAR_PX, MIN_LANE_PX, Point,
    REACH_MARGIN, Selection, TEMPO_MARGIN, Timeline, View, WheelGesture, ZOOM_WHEEL_PX,
    wheel_gesture, write_correction,
};
use dermixen_core::{
    Anchor, Anchors, BeatGrid, Beats, Bpm, ContentHash, Curve, Decibels, Envelope, EqEnvelopes,
    Mix, Samples, Seconds, Track, beatmix,
};
use dermixen_library::{PhraseRecord, PhraseStartRecord};
use dermixen_media::{Audio, Overview};

fn track(name: &str, byte: u8, seconds: i64, intro: f64, outro: f64) -> Track {
    Track {
        path: PathBuf::from(name),
        hash: ContentHash([byte; 32]),
        length: Samples(seconds * 44_100),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
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

fn two_tracks() -> Mix {
    let mut a = track("a", 1, 200, 16.0, 256.0);
    let mut b = track("b", 2, 300, 32.0, 512.0);
    beatmix(&mut a, &mut b, 8);
    Mix { tracks: vec![a, b] }
}

fn view(from: f64, to: f64) -> View {
    View {
        width_px: 1000.0,
        lane_height_px: 100.0,
        from: Seconds(from),
        to: Seconds(to),
    }
}

fn timeline() -> Timeline {
    let mut timeline = Timeline::new(two_tracks());
    timeline.set_view(view(0.0, 100.0));
    timeline
}

fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn close64(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// The beats of a track's volume nodes.
fn volume_beats(mix: &Mix, track: usize) -> Vec<f64> {
    mix.tracks[track]
        .volume
        .nodes()
        .iter()
        .map(|n| n.at.0)
        .collect()
}

#[test]
fn a_lane_shows_the_gain_leveling_wrote_for_its_track() {
    let mut mix = two_tracks();
    mix.tracks[1].gain = Decibels(-2.5);
    let mut timeline = Timeline::new(mix);
    timeline.set_view(view(0.0, 100.0));
    let scene = timeline.scene();
    assert_eq!(scene.lanes[0].gain, Decibels::UNITY);
    assert_eq!(scene.lanes[1].gain, Decibels(-2.5));
}

#[test]
fn lanes_follow_the_placement_and_the_view_maps_time_to_pixels() {
    let timeline = timeline();
    let scene = timeline.scene();
    assert_eq!(scene.length, Seconds(412.0));
    assert_eq!(scene.lanes.len(), 2);
    let a = &scene.lanes[0];
    assert_eq!((a.track, a.name.as_str(), a.top), (0, "a", 0.0));
    assert!(close(a.start_x, 0.0) && close(a.end_x, 2000.0), "{a:?}");
    assert_eq!(a.bpm, Bpm(120.0));
    assert!(a.keylock);
    let b = &scene.lanes[1];
    assert_eq!((b.track, b.name.as_str(), b.top), (1, "b", 100.0));
    assert!(close(b.start_x, 1120.0) && close(b.end_x, 4120.0), "{b:?}");
    assert_eq!(scene.tempo.top, 200.0);
    assert!(a.waveform.is_empty(), "no overview has been given");
    assert_eq!(
        a.curve.len(),
        1001,
        "one point per whole column from 0 to 1000"
    );
    assert!(
        close(a.curve[10].x, 10.0) && close(a.curve[10].y, 0.0),
        "unity at the top"
    );
    assert!(
        b.curve.is_empty(),
        "the second track has no column within the view"
    );

    assert!(close(timeline.x_of(Seconds(50.0)), 500.0));
    assert_eq!(timeline.time_at(250.0), Seconds(25.0));
    assert!(close(timeline.x_of(Seconds(-5.0)), -50.0));
}

#[test]
fn the_view_scrolls_zooms_and_stays_within_bounds() {
    let mut timeline = timeline();
    timeline.scroll_by(100.0);
    assert_eq!(timeline.view(), view(10.0, 110.0));
    timeline.zoom_by(2.0, 500.0);
    assert_eq!(
        timeline.view(),
        view(35.0, 85.0),
        "the time at column 500 stays put"
    );
    timeline.scroll_by(-10_000.0);
    assert_eq!(timeline.view(), view(0.0, 50.0), "never before the mix");
    timeline.zoom_by(0.0001, 0.0);
    assert_eq!(
        timeline.view(),
        view(0.0, 472.0),
        "never wider than the mix plus a minute"
    );
    timeline.zoom_by(1.0e9, 0.0);
    assert_eq!(
        timeline.view(),
        view(0.0, 1.0),
        "never narrower than a second"
    );
    timeline.set_view(view(30.0, 20.0));
    assert_eq!(
        timeline.view(),
        view(0.0, 1.0),
        "a backwards view is ignored"
    );
}

#[test]
fn anchors_and_nodes_are_drawn_where_their_beats_fall() {
    let mut timeline = timeline();
    timeline.set_view(view(100.0, 140.0));
    let scene = timeline.scene();
    let a = &scene.lanes[0];
    assert_eq!(
        a.anchors.len(),
        1,
        "only the outro anchor is within the view"
    );
    assert_eq!(a.anchors[0].anchor, Anchor::Outro);
    assert_eq!(a.anchors[0].at, Beats(256.0));
    assert!(
        close(a.anchors[0].x, 700.0),
        "beat 256 is 128 seconds, 28 in at 25 pixels a second"
    );
    let node = |at: f64| {
        a.nodes
            .iter()
            .find(|n| n.at == Beats(at))
            .unwrap_or_else(|| panic!("no node at {at} in {:?}", a.nodes))
    };
    assert!(close(node(256.0).at_px.x, 700.0) && close(node(256.0).at_px.y, 0.0));
    assert!(close(node(264.0).at_px.x, 800.0) && close(node(264.0).at_px.y, 25.0));
    assert!(close(node(272.0).at_px.x, 900.0) && close(node(272.0).at_px.y, 50.0));
    assert!(close(node(280.0).at_px.x, 1000.0) && close(node(280.0).at_px.y, 75.0));
    assert!(
        a.nodes.iter().all(|n| n.at != Beats(284.0)),
        "beat 284 is 142 seconds, past the view"
    );
    let b = &scene.lanes[1];
    assert!(
        close(b.start_x, 300.0),
        "the second track starts at 112 seconds"
    );
    assert_eq!(b.anchors.len(), 1);
    assert_eq!(b.anchors[0].anchor, Anchor::Intro);
    assert!(
        close(b.anchors[0].x, 700.0),
        "the intro anchor sits on the outro anchor"
    );
    let node = |at: f64| b.nodes.iter().find(|n| n.at == Beats(at)).unwrap();
    assert!(close(node(32.0).at_px.x, 700.0) && close(node(32.0).at_px.y, 200.0));
    assert!(close(node(36.0).at_px.x, 750.0) && close(node(36.0).at_px.y, 187.5));
    assert!(close(node(48.0).at_px.x, 900.0) && close(node(48.0).at_px.y, 150.0));
    assert!(close64(node(48.0).level.0, 20.0 * 0.5f64.log10()));
    assert!(
        scene
            .lanes
            .iter()
            .all(|lane| lane.nodes.iter().all(|n| !n.selected))
    );
}

#[test]
fn the_waveform_follows_the_overview() {
    let mut timeline = timeline();
    let mut audio = Audio {
        frames: vec![[0.0, 0.0]; 2 * 44_100],
    };
    audio.frames[44_100] = [0.9, 0.9];
    audio.frames[44_200] = [-0.3, -0.3];
    timeline.set_overview(ContentHash([1; 32]), Overview::of(&audio, Samples(441)));
    let scene = timeline.scene();
    let a = &scene.lanes[0];
    assert_eq!(
        a.waveform.len(),
        1001,
        "one column per whole pixel from 0 to 1000"
    );
    let column = |x: f32| a.waveform.iter().find(|c| c.x == x).unwrap();
    assert!(close(column(10.0).top, 5.0), "{:?}", column(10.0));
    assert!(close(column(10.0).bottom, 65.0), "{:?}", column(10.0));
    assert!(close(column(500.0).top, 50.0) && close(column(500.0).bottom, 50.0));
    assert!(scene.lanes[1].waveform.is_empty());
}

#[test]
fn a_press_on_the_curve_adds_a_node_on_the_nearest_beat_at_the_lines_level() {
    let mut timeline = timeline();
    assert_eq!(HIT_PX, 6.0);
    assert_eq!(timeline.hit(point(503.0, 3.0)), Selection::Track(0));
    assert_eq!(
        timeline.selection(),
        Selection::Nothing,
        "asking changes nothing"
    );
    timeline.press(point(503.0, 3.0));
    assert_eq!(
        timeline.selection(),
        Selection::Node {
            track: 0,
            curve: Curve::Volume,
            at: Beats(101.0)
        },
        "50.3 seconds is beat 100.6, so beat 101"
    );
    assert!(volume_beats(timeline.mix(), 0).contains(&101.0));
    assert_eq!(
        timeline.mix().tracks[0].volume.value_at(Beats(101.0)),
        Decibels::UNITY
    );
    assert!(timeline.can_undo());
    assert!(timeline.take_document_change());
    timeline.release();
    assert!(
        !timeline.take_document_change(),
        "a release without a move commits nothing"
    );
    let marks = &timeline.scene().lanes[0].nodes;
    let mark = marks.iter().find(|n| n.at == Beats(101.0)).unwrap();
    assert!(close(mark.at_px.x, 505.0) && close(mark.at_px.y, 0.0));
    assert!(mark.selected);
    assert_eq!(
        timeline.hit(point(508.0, 4.0)),
        Selection::Node {
            track: 0,
            curve: Curve::Volume,
            at: Beats(101.0)
        }
    );

    timeline.press(point(503.0, 20.0));
    assert_eq!(
        timeline.selection(),
        Selection::Track(0),
        "twenty pixels below the line is the lane, not the curve"
    );
    timeline.release();
    assert!(
        volume_beats(timeline.mix(), 0).len() == 7,
        "no node was added"
    );

    timeline.press(point(500.0, 150.0));
    assert_eq!(
        timeline.selection(),
        Selection::Track(1),
        "the second track's audio has not started at 50 seconds, so the press selects it"
    );
    timeline.release();
    assert_eq!(volume_beats(timeline.mix(), 1).len(), 6);
}

#[test]
fn dragging_a_node_previews_it_and_commits_one_move_on_release() {
    let dir = tempfile::tempdir().unwrap();
    let mut timeline = timeline();
    timeline.set_corrections_dir(dir.path().to_path_buf());
    timeline.press(point(503.0, 3.0));
    timeline.release();
    timeline.take_document_change();

    timeline.press(point(505.0, 0.0));
    assert_eq!(
        timeline.selection(),
        Selection::Node {
            track: 0,
            curve: Curve::Volume,
            at: Beats(101.0)
        }
    );
    timeline.drag_to(point(605.0, 50.0));
    let preview = timeline.scene();
    let mark = preview.lanes[0]
        .nodes
        .iter()
        .find(|n| n.selected)
        .expect("the dragged node is drawn");
    assert!(
        close(mark.at_px.x, 605.0) && close(mark.at_px.y, 50.0),
        "{mark:?}"
    );
    assert_eq!(mark.at, Beats(121.0));
    assert!(
        volume_beats(timeline.mix(), 0).contains(&101.0),
        "the document holds the node where it was until release"
    );
    assert!(!timeline.take_document_change());

    timeline.release();
    let beats = volume_beats(timeline.mix(), 0);
    assert!(
        beats.contains(&121.0) && !beats.contains(&101.0),
        "{beats:?}"
    );
    let level = timeline.mix().tracks[0].volume.value_at(Beats(121.0));
    assert!(
        close64(level.0, 20.0 * 0.5f64.log10()),
        "half amplitude is {} decibels",
        level.0
    );
    assert!(timeline.take_document_change());
    assert_eq!(
        timeline.selection(),
        Selection::Node {
            track: 0,
            curve: Curve::Volume,
            at: Beats(121.0)
        }
    );
    assert!(timeline.undo());
    assert!(volume_beats(timeline.mix(), 0).contains(&101.0));
    assert_eq!(timeline.selection(), Selection::Nothing);
    assert!(timeline.redo());
    assert!(volume_beats(timeline.mix(), 0).contains(&121.0));

    // The bottom of the lane is silence, and a node dragged onto another is
    // refused, so the document stays as it was.
    timeline.press(point(605.0, 50.0));
    timeline.drag_to(point(605.0, 100.0));
    timeline.release();
    assert_eq!(
        timeline.mix().tracks[0].volume.value_at(Beats(121.0)),
        Decibels::SILENCE
    );
    timeline.press(point(605.0, 100.0));
    timeline.drag_to(point(1280.0, 100.0));
    timeline.release();
    assert!(
        volume_beats(timeline.mix(), 0).contains(&121.0),
        "beat 256 already holds the fade's first node, so the move was refused"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "node moves write no correction"
    );
}

#[test]
fn a_press_that_adds_and_drags_is_two_undoable_steps() {
    let mut timeline = timeline();
    timeline.press(point(503.0, 3.0));
    timeline.drag_to(point(605.0, 50.0));
    timeline.release();
    let beats = volume_beats(timeline.mix(), 0);
    assert!(
        beats.contains(&121.0) && !beats.contains(&101.0),
        "{beats:?}"
    );
    assert!(timeline.undo());
    let beats = volume_beats(timeline.mix(), 0);
    assert!(
        beats.contains(&101.0) && !beats.contains(&121.0),
        "{beats:?}"
    );
    assert!(timeline.undo());
    assert_eq!(volume_beats(timeline.mix(), 0).len(), 6);
    assert!(!timeline.undo());
}

#[test]
fn dragging_an_anchor_moves_its_transition_and_writes_a_correction() {
    let dir = tempfile::tempdir().unwrap();
    let mut timeline = timeline();
    timeline.set_corrections_dir(dir.path().to_path_buf());
    timeline.set_view(view(100.0, 140.0));
    timeline.press(point(700.0, 50.0));
    assert_eq!(
        timeline.selection(),
        Selection::Anchor {
            track: 0,
            anchor: Anchor::Outro
        },
        "fifty pixels below the fade's first node, the anchor line is what the press reaches"
    );
    timeline.drag_to(point(650.0, 50.0));
    let preview = timeline.scene();
    assert!(close(preview.lanes[0].anchors[0].x, 650.0));
    assert!(preview.lanes[0].anchors[0].selected);
    assert!(
        close(preview.lanes[1].start_x, 250.0),
        "the second track moves with the anchor in the preview"
    );
    assert_eq!(timeline.mix().tracks[0].anchors.outro, Beats(256.0));
    timeline.release();
    assert_eq!(timeline.mix().tracks[0].anchors.outro, Beats(252.0));
    assert_eq!(
        volume_beats(timeline.mix(), 0),
        vec![252.0, 260.0, 268.0, 276.0, 280.0, 284.0]
    );
    assert!(close(timeline.scene().lanes[1].start_x, 250.0));
    assert!(timeline.take_document_change());
    let written = std::fs::read_to_string(dir.path().join("a-01010101.anchors")).unwrap();
    for line in [
        "file a",
        "bpm 120",
        "first_beat 0.000000",
        "intro 8.000000 ear",
        "outro 126.000000 ear",
    ] {
        assert!(
            written.lines().any(|l| l == line),
            "no line {line:?} in\n{written}"
        );
    }
    assert_eq!(timeline.take_correction_failure(), None);

    timeline.set_corrections_dir(dir.path().join("missing").join("folder"));
    timeline.press(point(650.0, 50.0));
    timeline.drag_to(point(600.0, 50.0));
    timeline.release();
    assert_eq!(
        timeline.mix().tracks[0].anchors.outro,
        Beats(248.0),
        "the edit stands"
    );
    assert!(
        timeline.take_correction_failure().is_some(),
        "the failure is reported"
    );
    assert_eq!(timeline.take_correction_failure(), None);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "one file, for the one track whose anchor moved"
    );
}

#[test]
fn a_grid_correction_goes_through_the_timeline_and_is_written_as_ground_truth() {
    let dir = tempfile::tempdir().unwrap();
    let mut timeline = timeline();
    timeline.set_corrections_dir(dir.path().to_path_buf());
    let mut editor = GridEditor::new(0, timeline.mix().tracks[0].grid);
    editor.double();
    timeline.apply(editor.edit()).unwrap();
    let a = &timeline.mix().tracks[0];
    assert_eq!(a.grid.bpm, Bpm(240.0));
    assert_eq!(
        a.anchors,
        Anchors {
            intro: Beats(32.0),
            outro: Beats(512.0)
        },
        "the anchors keep their time"
    );
    assert!(timeline.take_document_change());
    let written = std::fs::read_to_string(dir.path().join("a-01010101.anchors")).unwrap();
    for line in ["bpm 240", "intro 8.000000 ear", "outro 128.000000 ear"] {
        assert!(
            written.lines().any(|l| l == line),
            "no line {line:?} in\n{written}"
        );
    }
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &two_tracks());
    assert!(
        timeline
            .apply(dermixen_core::Edit::RemoveTrack { track: 9 })
            .is_err()
    );
    assert_eq!(timeline.mix(), &two_tracks());
}

fn grid_at(first_beat: i64) -> BeatGrid {
    BeatGrid {
        first_beat: Samples(first_beat),
        bpm: Bpm(120.0),
    }
}

fn first_beat(timeline: &Timeline, track: usize) -> Samples {
    timeline.mix().tracks[track].grid.first_beat
}

#[test]
fn consecutive_grid_changes_from_the_editor_are_one_undoable_step() {
    let dir = tempfile::tempdir().unwrap();
    let mut timeline = timeline();
    timeline.set_corrections_dir(dir.path().to_path_buf());
    timeline.select_track(1);
    let before = two_tracks();
    assert!(!timeline.can_undo());

    // Two changes, one step. Each is written as a correction as it is
    // made, and the anchors are placed from the grid the correction began
    // on: 0.3 beats is 6,615 frames, and the intro anchor at beat 16 of
    // the first grid, eight seconds in, is beat 15.4 of a grid 0.6 beats
    // later, so beat 15, where placing it from the grid one step earlier
    // would have rounded it to 16 twice over.
    timeline.set_grid(0, grid_at(6_615)).unwrap();
    let written = std::fs::read_to_string(dir.path().join("a-01010101.anchors")).unwrap();
    assert!(
        written.lines().any(|l| l == "first_beat 0.150000"),
        "{written}"
    );
    timeline.set_grid(0, grid_at(13_230)).unwrap();
    let written = std::fs::read_to_string(dir.path().join("a-01010101.anchors")).unwrap();
    assert!(
        written.lines().any(|l| l == "first_beat 0.300000"),
        "{written}"
    );
    assert_eq!(first_beat(&timeline, 0), Samples(13_230));
    assert_eq!(
        timeline.mix().tracks[0].anchors,
        Anchors {
            intro: Beats(15.0),
            outro: Beats(255.0)
        }
    );
    assert_eq!(timeline.mix().tracks[1], before.tracks[1]);
    assert_eq!(
        timeline.selection(),
        Selection::Track(1),
        "the selection is not touched"
    );
    assert!(timeline.take_document_change());
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);
    assert!(!timeline.can_undo(), "the two changes were one step");
    assert!(timeline.redo());
    assert_eq!(first_beat(&timeline, 0), Samples(13_230));

    // A redo ends the step, so the next change starts another.
    timeline.set_grid(0, grid_at(0)).unwrap();
    assert!(timeline.undo());
    assert_eq!(first_beat(&timeline, 0), Samples(13_230));
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);
    assert!(!timeline.can_undo());

    // Any other edit ends the step.
    timeline.set_grid(0, grid_at(100)).unwrap();
    timeline
        .apply(dermixen_core::Edit::SetKeylock {
            track: 0,
            keylock: false,
        })
        .unwrap();
    timeline.set_grid(0, grid_at(200)).unwrap();
    assert!(timeline.undo());
    assert_eq!(first_beat(&timeline, 0), Samples(100));
    assert!(!timeline.mix().tracks[0].keylock);
    assert!(timeline.undo());
    assert!(timeline.mix().tracks[0].keylock);
    assert_eq!(first_beat(&timeline, 0), Samples(100));
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);

    // A change to another track ends the step and starts one for that
    // track.
    timeline.set_grid(0, grid_at(100)).unwrap();
    timeline.set_grid(1, grid_at(100)).unwrap();
    timeline.set_grid(1, grid_at(200)).unwrap();
    assert_eq!(first_beat(&timeline, 1), Samples(200));
    assert!(timeline.undo());
    assert_eq!(first_beat(&timeline, 1), Samples(0));
    assert_eq!(first_beat(&timeline, 0), Samples(100));
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);

    // Ending the correction by hand does the same, and ending one that is
    // not in progress changes nothing.
    timeline.end_grid_correction();
    timeline.set_grid(0, grid_at(100)).unwrap();
    timeline.end_grid_correction();
    timeline.end_grid_correction();
    timeline.set_grid(0, grid_at(200)).unwrap();
    assert!(timeline.undo());
    assert_eq!(first_beat(&timeline, 0), Samples(100));
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);

    // A refused grid changes nothing, and the step goes on.
    timeline.set_grid(0, grid_at(100)).unwrap();
    assert!(
        timeline
            .set_grid(
                0,
                BeatGrid {
                    first_beat: Samples(200),
                    bpm: Bpm(f64::NAN),
                }
            )
            .is_err()
    );
    assert!(timeline.set_grid(9, grid_at(200)).is_err());
    assert_eq!(first_beat(&timeline, 0), Samples(100));
    timeline.set_grid(0, grid_at(300)).unwrap();
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);
    assert!(!timeline.can_undo());

    // An edit the document refuses through `apply` changes nothing either,
    // so the step goes on past it.
    timeline.set_grid(0, grid_at(100)).unwrap();
    assert!(
        timeline
            .apply(dermixen_core::Edit::RemoveTrack { track: 9 })
            .is_err()
    );
    timeline.set_grid(0, grid_at(200)).unwrap();
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &before);
    assert!(
        !timeline.can_undo(),
        "the refused edit did not end the step"
    );
}

#[test]
fn the_tempo_lane_shows_the_curve_and_edits_its_nodes() {
    let mut timeline = timeline();
    assert_eq!(TEMPO_MARGIN, Bpm(2.0));
    let scene = timeline.scene();
    assert_eq!(
        (scene.tempo.low, scene.tempo.high),
        (Bpm(118.0), Bpm(122.0))
    );
    assert_eq!(scene.tempo.curve.len(), 1001);
    assert!(
        close(scene.tempo.curve[500].y, 250.0),
        "120 sits halfway down the lane"
    );
    assert!(
        scene.tempo.nodes.is_empty(),
        "the nodes at 128 and 144 seconds are past the view"
    );

    timeline.press(point(502.0, 250.0));
    assert_eq!(
        timeline.selection(),
        Selection::TempoNode {
            track: 0,
            at: Beats(100.0)
        },
        "50.2 seconds is mix beat 100.4, so beat 100"
    );
    assert_eq!(
        timeline.mix().tracks[0].tempo[0],
        dermixen_core::TempoNode {
            at: Beats(100.0),
            bpm: Bpm(120.0)
        }
    );
    // The range is computed at the press and kept for the whole drag, so
    // the top of the lane is 122 however far the node was dragged before.
    timeline.drag_to(point(500.0, 225.0));
    timeline.drag_to(point(500.0, 200.0));
    timeline.release();
    assert_eq!(
        timeline.mix().tracks[0].tempo[0].bpm,
        Bpm(122.0),
        "the top of the lane"
    );
    let scene = timeline.scene();
    assert_eq!(
        (scene.tempo.low, scene.tempo.high),
        (Bpm(118.0), Bpm(124.0))
    );
    let mark = scene
        .tempo
        .nodes
        .iter()
        .find(|n| n.at == Beats(100.0))
        .unwrap();
    // The curve now ramps from 120 at mix beat 0 to 122 at mix beat 100, so
    // beat 100 falls at 120 * 100 / 242 = 49.5868 seconds, column 495.87.
    assert!(
        close(mark.at_px.x, 495.87) && close(mark.at_px.y, 233.3333),
        "{mark:?}"
    );
    assert!(mark.selected);
    timeline.set_selected_tempo(Bpm(121.0));
    assert_eq!(timeline.mix().tracks[0].tempo[0].bpm, Bpm(121.0));
    timeline.set_selected_tempo(Bpm(0.0));
    assert_eq!(
        timeline.mix().tracks[0].tempo[0].bpm,
        Bpm(121.0),
        "an invalid tempo is ignored"
    );

    timeline.delete_selection();
    assert!(
        timeline.mix().tracks[0]
            .tempo
            .iter()
            .all(|n| n.at != Beats(100.0))
    );
    assert_eq!(timeline.selection(), Selection::Nothing);

    let empty = Timeline::new(Mix { tracks: Vec::new() });
    let scene = empty.scene();
    assert!(scene.lanes.is_empty());
    assert_eq!(scene.tempo.top, 0.0);
    assert_eq!(
        (scene.tempo.low, scene.tempo.high),
        (Bpm(DEFAULT_TEMPO.0 - 2.0), Bpm(DEFAULT_TEMPO.0 + 2.0))
    );
    assert_eq!(scene.length, Seconds::ZERO);
}

#[test]
fn the_master_control_pins_the_curve_and_leaves_the_selection_alone() {
    // A node at mix beat 100 set to 121 makes the curve ramp from 120 at
    // beat 0 to 121 at beat 100. Thirty seconds along that ramp is mix beat
    // 60.15, and a tenth of a second later is about beat 60.35, so the
    // control pins the curve at beat 61 with the tempo the ramp has there,
    // about 120.61, and writes 125 at beat 62. The tempo at the playhead,
    // which lies before the pin, is what it was.
    let mut timeline = timeline();
    timeline.press(point(502.0, 250.0));
    timeline.release();
    timeline.set_selected_tempo(Bpm(121.0));
    assert_eq!(
        timeline.mix().tracks[0].tempo[0],
        dermixen_core::TempoNode {
            at: Beats(100.0),
            bpm: Bpm(121.0)
        }
    );
    timeline.set_playhead(Samples(30 * 44_100));
    let before = timeline.master_tempo().0;
    timeline.set_master_tempo(Bpm(125.0));
    let nodes = timeline.mix().tracks[0].tempo.clone();
    assert!(
        nodes.contains(&dermixen_core::TempoNode {
            at: Beats(62.0),
            bpm: Bpm(125.0)
        }),
        "{nodes:?}"
    );
    let pin = nodes.iter().find(|n| n.at == Beats(61.0)).unwrap();
    assert!(
        (pin.bpm.0 - 120.61).abs() < 0.05,
        "the pin holds {}",
        pin.bpm.0
    );
    let shown = timeline.master_tempo().0;
    assert!(
        (shown - before).abs() < 1e-6,
        "the tempo at the playhead was {before} and reads {shown}"
    );
    assert_eq!(
        timeline.selection(),
        Selection::TempoNode {
            track: 0,
            at: Beats(100.0)
        },
        "the control leaves the selection alone"
    );
    // The first track's nodes are now the pin at 61, the excursion at 62,
    // the node at 100, and the transition's at 256.
    assert_eq!(nodes.len(), 4);
    assert!(timeline.undo(), "the two nodes are one step");
    assert_eq!(timeline.mix().tracks[0].tempo.len(), 2);
    assert!(timeline.redo());
    assert_eq!(timeline.mix().tracks[0].tempo.len(), 4);
    // The redo cleared the selection. The lane's range is now 118 to 127,
    // so the node at beat 100 holding 121 is drawn at row 266.67, and the
    // curve's ramps put beat 100 at about 49.45 seconds, column 494.5.
    timeline.press(point(494.5, 266.7));
    assert_eq!(
        timeline.selection(),
        Selection::TempoNode {
            track: 0,
            at: Beats(100.0)
        }
    );
    timeline.release();
    timeline.delete_selection();
    assert!(
        timeline.mix().tracks[0]
            .tempo
            .iter()
            .all(|n| n.at != Beats(100.0))
    );
    assert_eq!(timeline.selection(), Selection::Nothing);
}

#[test]
fn the_master_control_writes_past_what_the_render_has_reached() {
    // With the curve flat at 120, a render that has reached thirty and a
    // half seconds, beat 61, plus the margin of a tenth of a second, beat
    // 61.2, puts the pin at 62 and the new tempo at 63.
    assert_eq!(REACH_MARGIN, Samples(4_410));
    let mut reached = timeline();
    reached.set_playhead(Samples(30 * 44_100));
    reached.set_reach(Samples(30 * 44_100 + 22_050));
    reached.set_master_tempo(Bpm(125.0));
    assert_eq!(
        reached.mix().tracks[0].tempo,
        vec![
            dermixen_core::TempoNode {
                at: Beats(62.0),
                bpm: Bpm(120.0)
            },
            dermixen_core::TempoNode {
                at: Beats(63.0),
                bpm: Bpm(125.0)
            },
            dermixen_core::TempoNode {
                at: Beats(256.0),
                bpm: Bpm(120.0)
            },
        ]
    );
    // A reach before the playhead is ignored in favor of the playhead, which
    // at thirty and a half seconds is beat 61, so the pin goes to 62.
    let mut behind = timeline();
    behind.set_playhead(Samples(30 * 44_100 + 22_050));
    behind.set_reach(Samples(30 * 44_100));
    behind.set_master_tempo(Bpm(125.0));
    assert_eq!(behind.mix().tracks[0].tempo[0].at, Beats(62.0));
    // Without a reach the playhead alone decides: thirty seconds is beat 60,
    // a tenth of a second later is beat 60.2, so the pin goes to 61.
    let mut alone = timeline();
    alone.set_playhead(Samples(30 * 44_100));
    alone.set_master_tempo(Bpm(125.0));
    assert_eq!(alone.mix().tracks[0].tempo[0].at, Beats(61.0));
}

#[test]
fn phrases_sections_and_bar_lines_come_from_the_record() {
    let mut timeline = timeline();
    let bars_before: Vec<f32> = timeline.scene().lanes[0].bars.clone();
    assert!(
        close(bars_before[0], 0.0) && close(bars_before[1], 20.0),
        "bars from beat zero without a record"
    );
    timeline.set_phrases(
        ContentHash([1; 32]),
        PhraseRecord {
            analyzer: "shifts".to_owned(),
            confidence: 0.3,
            downbeat: 1,
            starts: vec![
                PhraseStartRecord {
                    beat: Beats(1.0),
                    bars: 32,
                },
                PhraseStartRecord {
                    beat: Beats(33.0),
                    bars: 8,
                },
                PhraseStartRecord {
                    beat: Beats(201.0),
                    bars: 16,
                },
            ],
            sections: vec![Beats(65.0)],
        },
    );
    let scene = timeline.scene();
    let a = &scene.lanes[0];
    assert_eq!(a.phrases.len(), 2, "beat 201 is past the view");
    assert!(close(a.phrases[0].x, 5.0) && a.phrases[0].bars == 32);
    assert!(close(a.phrases[1].x, 165.0) && a.phrases[1].bars == 8);
    assert_eq!(a.sections.len(), 1);
    assert!(close(a.sections[0], 325.0));
    assert_eq!(
        a.bars.len(),
        50,
        "beats 1, 5, ..., 197 within a hundred seconds"
    );
    assert!(close(a.bars[0], 5.0) && close(a.bars[1], 25.0));
    assert!(scene.lanes[1].phrases.is_empty());

    assert_eq!(MIN_BAR_PX, 16.0);
    timeline.set_view(view(0.0, 1000.0));
    let scene = timeline.scene();
    assert!(
        scene.lanes[0].bars.is_empty(),
        "a two-pixel bar is not drawn"
    );
    assert_eq!(
        scene.lanes[0].phrases.len(),
        3,
        "phrases are drawn at every zoom"
    );
}

#[test]
fn the_playhead_the_ruler_and_the_playlist_commands() {
    let mut timeline = timeline();
    assert_eq!(timeline.scene().playhead_x, Some(0.0));
    timeline.set_playhead(Samples(30 * 44_100));
    assert_eq!(timeline.playhead(), Samples(30 * 44_100));
    assert!(close(timeline.scene().playhead_x.unwrap(), 300.0));
    timeline.set_playhead(Samples(200 * 44_100));
    assert_eq!(timeline.scene().playhead_x, None, "past the view");

    assert_eq!(timeline.take_seek(), None);
    timeline.click_ruler(250.0);
    assert_eq!(timeline.take_seek(), Some(Samples(25 * 44_100)));
    assert_eq!(timeline.take_seek(), None);

    assert!(!timeline.take_document_change());
    timeline.set_keylock(0, false);
    assert!(!timeline.mix().tracks[0].keylock);
    assert!(!timeline.scene().lanes[0].keylock);
    timeline.move_track(1, 0);
    assert_eq!(timeline.mix().tracks[0].path, PathBuf::from("b"));
    assert_eq!(timeline.scene().lanes[0].name, "b");
    assert!(timeline.take_document_change());
    assert!(timeline.undo());
    assert!(timeline.undo());
    assert_eq!(timeline.mix(), &two_tracks());
    assert!(
        timeline.take_document_change(),
        "an undo is a change the transport must hear"
    );

    timeline.press(point(500.0, 150.0));
    assert_eq!(timeline.selection(), Selection::Track(1));
    timeline.release();
    timeline.delete_selection();
    assert_eq!(timeline.mix().tracks.len(), 1);
    assert_eq!(timeline.selection(), Selection::Nothing);
    timeline.delete_selection();
    assert_eq!(
        timeline.mix().tracks.len(),
        1,
        "nothing selected is nothing to remove"
    );
    timeline.press(point(5000.0, 5000.0));
    assert_eq!(
        timeline.selection(),
        Selection::Nothing,
        "a press outside every lane changes nothing"
    );
}

#[test]
fn selecting_another_curve_shows_its_nodes_and_clears_a_node_selection() {
    let mut timeline = timeline();
    timeline.press(point(503.0, 3.0));
    timeline.release();
    timeline.select_curve(Curve::Low);
    assert_eq!(timeline.curve(), Curve::Low);
    assert_eq!(timeline.selection(), Selection::Nothing);
    let scene = timeline.scene();
    assert!(scene.lanes[0].nodes.is_empty(), "the low band has no nodes");
    assert!(
        close(scene.lanes[0].curve[300].y, 0.0),
        "an empty envelope is unity"
    );
    assert_eq!(timeline.hit(point(300.0, 50.0)), Selection::Track(0));
    timeline.press(point(300.0, 50.0));
    assert_eq!(
        timeline.selection(),
        Selection::Node {
            track: 0,
            curve: Curve::Low,
            at: Beats(60.0)
        },
        "a press anywhere in the audio places the first node of an empty curve"
    );
    timeline.release();
    assert_eq!(timeline.mix().tracks[0].eq.low.len(), 1);
    let level = timeline.mix().tracks[0].eq.low.value_at(Beats(60.0));
    assert!(
        close64(level.0, 20.0 * 0.5f64.log10()),
        "at the row's level, {}",
        level.0
    );
    // The low band holds its one node's level, half amplitude, from beat 60
    // on, so its line runs along row 50 there; a press thirty pixels above
    // it is off the line.
    timeline.press(point(700.0, 20.0));
    assert_eq!(
        timeline.selection(),
        Selection::Track(0),
        "with a node on the curve, a press off the line selects the track"
    );
    timeline.release();
    timeline.select_track(1);
    assert_eq!(timeline.selection(), Selection::Track(1));
    assert_eq!(
        timeline.mix().tracks[0].volume.len(),
        7,
        "the volume curve is untouched"
    );
}

#[test]
fn a_correction_is_written_in_the_ground_truth_format() {
    let dir = tempfile::tempdir().unwrap();
    let mut track = track("Etnica - Alpha.mp3", 1, 200, 16.0, 256.0);
    track.path = PathBuf::from("/music/goa/Etnica - Alpha.mp3");
    track.grid = BeatGrid {
        first_beat: Samples(22_050),
        bpm: Bpm(136.6689),
    };
    let path = write_correction(dir.path(), &track).unwrap();
    // The file is named after the audio file's stem and the first eight
    // hexadecimal digits of its content hash, so two tracks that share a
    // file name in different folders keep separate corrections.
    assert_eq!(path, dir.path().join("Etnica - Alpha-01010101.anchors"));
    let written = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = written.lines().collect();
    assert!(lines[0].starts_with('#'), "{written}");
    // A beat at 136.6689 beats per minute is 0.439017 seconds, so beat 16
    // is 7.024275 seconds after beat zero and beat 256 is 112.388407.
    assert_eq!(
        &lines[1..],
        [
            "file /music/goa/Etnica - Alpha.mp3",
            "bpm 136.6689",
            "first_beat 0.500000",
            "intro 7.524275 ear",
            "outro 112.888407 ear",
        ]
    );
    let again = write_correction(dir.path(), &track).unwrap();
    assert_eq!(again, path, "an earlier file is replaced");
}

// Lane heights. A person drags a lane's bottom edge to make its waveform
// taller for frame-level work, as `docs/window.md` describes under "The
// mouse". A lane that has not been resized is the view's `lane_height_px`
// tall.

#[test]
fn lanes_are_the_default_height_until_one_is_given_its_own() {
    let mut timeline = timeline();
    assert_eq!(MIN_LANE_PX, 44.0);
    assert_eq!(MAX_LANE_PX, 600.0);
    assert_eq!(timeline.lane_height(0), 100.0);
    assert_eq!(timeline.lane_height(1), 100.0);
    assert_eq!(timeline.lane_height(2), 100.0, "the tempo lane is lane 2");
    assert_eq!(timeline.lanes_height(), 300.0);

    timeline.set_lane_height(0, 250.0);
    assert_eq!(timeline.lane_height(0), 250.0);
    assert_eq!(timeline.lane_height(1), 100.0);
    assert_eq!(timeline.lanes_height(), 450.0);
    let scene = timeline.scene();
    assert_eq!((scene.lanes[0].top, scene.lanes[0].height), (0.0, 250.0));
    assert_eq!((scene.lanes[1].top, scene.lanes[1].height), (250.0, 100.0));
    assert_eq!((scene.tempo.top, scene.tempo.height), (350.0, 100.0));

    timeline.set_lane_height(1, 10.0);
    assert_eq!(
        timeline.lane_height(1),
        MIN_LANE_PX,
        "never shorter than the minimum"
    );
    timeline.set_lane_height(2, 5000.0);
    assert_eq!(
        timeline.lane_height(2),
        MAX_LANE_PX,
        "never taller than the maximum"
    );
    assert_eq!(timeline.lanes_height(), 250.0 + 44.0 + 600.0);

    timeline.set_lane_height(3, 100.0);
    assert_eq!(timeline.lanes_height(), 894.0, "there is no lane 3");
    timeline.set_lane_height(0, f32::NAN);
    timeline.set_lane_height(0, f32::INFINITY);
    assert_eq!(
        timeline.lane_height(0),
        250.0,
        "a height that is not a number is ignored"
    );
}

#[test]
fn a_lane_with_its_own_height_keeps_it_when_the_view_changes() {
    let mut timeline = timeline();
    timeline.set_lane_height(0, 250.0);
    timeline.set_view(View {
        width_px: 1000.0,
        lane_height_px: 80.0,
        from: Seconds(0.0),
        to: Seconds(100.0),
    });
    assert_eq!(
        timeline.lane_height(0),
        250.0,
        "a lane with its own height keeps it"
    );
    assert_eq!(timeline.lane_height(1), 80.0, "the others follow the view");
    assert_eq!(timeline.lane_height(2), 80.0);
    assert_eq!(timeline.lanes_height(), 410.0);
}

#[test]
fn a_lane_with_its_own_height_draws_its_contents_to_that_height() {
    let mut timeline = timeline();
    let mut audio = Audio {
        frames: vec![[0.0, 0.0]; 2 * 44_100],
    };
    audio.frames[44_100] = [0.9, 0.9];
    audio.frames[44_200] = [-0.3, -0.3];
    timeline.set_overview(ContentHash([1; 32]), Overview::of(&audio, Samples(441)));
    timeline.set_lane_height(0, 200.0);
    let scene = timeline.scene();
    let a = &scene.lanes[0];
    let column = |x: f32| a.waveform.iter().find(|c| c.x == x).unwrap();
    assert!(close(column(10.0).top, 10.0), "{:?}", column(10.0));
    assert!(close(column(10.0).bottom, 130.0), "{:?}", column(10.0));
    assert!(close(column(500.0).top, 100.0) && close(column(500.0).bottom, 100.0));

    timeline.set_lane_height(2, 200.0);
    let scene = timeline.scene();
    assert_eq!((scene.tempo.top, scene.tempo.height), (300.0, 200.0));
    assert!(
        close(scene.tempo.curve[500].y, 400.0),
        "120 sits halfway down a lane from 300 to 500"
    );
    timeline.press(point(502.0, 400.0));
    assert_eq!(
        timeline.selection(),
        Selection::TempoNode {
            track: 0,
            at: Beats(100.0)
        },
        "the tempo lane's own height places its nodes"
    );
    timeline.release();

    timeline.set_view(view(100.0, 140.0));
    let scene = timeline.scene();
    let a = &scene.lanes[0];
    let node = |at: f64| a.nodes.iter().find(|n| n.at == Beats(at)).unwrap();
    assert!(close(node(256.0).at_px.x, 700.0) && close(node(256.0).at_px.y, 0.0));
    assert!(close(node(264.0).at_px.y, 50.0), "{:?}", node(264.0));
    assert!(close(node(272.0).at_px.y, 100.0), "{:?}", node(272.0));
    assert!(close(node(280.0).at_px.y, 150.0), "{:?}", node(280.0));
    assert!(close(a.anchors[0].x, 700.0));
    let b = &scene.lanes[1];
    assert_eq!((b.top, b.height), (200.0, 100.0));
    let node = |at: f64| b.nodes.iter().find(|n| n.at == Beats(at)).unwrap();
    assert!(close(node(32.0).at_px.y, 300.0), "{:?}", node(32.0));
    assert!(close(node(36.0).at_px.y, 287.5), "{:?}", node(36.0));
    assert!(close(node(48.0).at_px.y, 250.0), "{:?}", node(48.0));
    assert_eq!(scene.tempo.top, 300.0);
}

#[test]
fn a_press_in_a_resized_lane_finds_its_nodes_at_their_rows() {
    let mut timeline = timeline();
    timeline.set_view(view(100.0, 140.0));
    timeline.set_lane_height(0, 200.0);
    assert_eq!(
        timeline.hit(point(800.0, 50.0)),
        Selection::Node {
            track: 0,
            curve: Curve::Volume,
            at: Beats(264.0)
        }
    );
    assert_eq!(
        timeline.hit(point(750.0, 287.5)),
        Selection::Node {
            track: 1,
            curve: Curve::Volume,
            at: Beats(36.0)
        }
    );
    assert_eq!(
        timeline.hit(point(500.0, 250.0)),
        Selection::Track(1),
        "row 250 is in the second track's lane, not the tempo lane"
    );

    timeline.press(point(800.0, 50.0));
    timeline.drag_to(point(800.0, 150.0));
    let scene = timeline.scene();
    let held = scene.lanes[0]
        .nodes
        .iter()
        .find(|n| n.at == Beats(264.0))
        .unwrap();
    assert!(close(held.at_px.y, 150.0), "{held:?}");
    assert!(
        close64(held.level.0, 20.0 * 0.25f64.log10()),
        "row 150 of a lane 200 tall is a quarter of unity: {held:?}"
    );
    timeline.release();
    let node = timeline.mix().tracks[0]
        .volume
        .nodes()
        .iter()
        .find(|n| n.at == Beats(264.0))
        .copied()
        .unwrap();
    assert!(close64(node.value.0, 20.0 * 0.25f64.log10()), "{node:?}");
}

#[test]
fn a_press_near_a_lanes_bottom_edge_takes_hold_of_the_edge_and_a_drag_resizes_it() {
    let mut timeline = timeline();
    assert_eq!(EDGE_PX, 4.0);
    assert_eq!(timeline.edge_at(point(500.0, 100.0)), Some(0));
    assert_eq!(timeline.edge_at(point(500.0, 96.0)), Some(0));
    assert_eq!(timeline.edge_at(point(500.0, 104.0)), Some(0));
    assert_eq!(timeline.edge_at(point(500.0, 95.0)), None);
    assert_eq!(timeline.edge_at(point(500.0, 105.0)), None);
    assert_eq!(timeline.edge_at(point(500.0, 50.0)), None);
    assert_eq!(
        timeline.edge_at(point(500.0, 2.0)),
        None,
        "the top of the first lane is no edge"
    );
    assert_eq!(timeline.edge_at(point(500.0, 200.0)), Some(1));
    assert_eq!(
        timeline.edge_at(point(500.0, 300.0)),
        Some(2),
        "the tempo lane's bottom edge"
    );
    assert_eq!(timeline.edge_at(point(500.0, 304.0)), Some(2));
    assert_eq!(timeline.edge_at(point(500.0, 305.0)), None);
    assert_eq!(timeline.edge_at(point(500.0, f32::NAN)), None);
    assert_eq!(
        timeline.hit(point(500.0, 100.0)),
        Selection::Nothing,
        "a press on an edge selects nothing"
    );

    timeline.select_track(1);
    timeline.press(point(500.0, 100.0));
    assert_eq!(
        timeline.selection(),
        Selection::Track(1),
        "the selection stands"
    );
    timeline.drag_to(point(500.0, 180.0));
    assert_eq!(timeline.lane_height(0), 180.0);
    assert_eq!(
        timeline.scene().lanes[1].top,
        180.0,
        "the lanes below move as the drag goes"
    );
    assert_eq!(timeline.selection(), Selection::Track(1));
    timeline.drag_to(point(500.0, 20.0));
    assert_eq!(timeline.lane_height(0), MIN_LANE_PX);
    timeline.drag_to(point(500.0, 2000.0));
    assert_eq!(timeline.lane_height(0), MAX_LANE_PX);
    timeline.drag_to(point(500.0, 250.0));
    timeline.release();
    assert_eq!(
        timeline.lane_height(0),
        250.0,
        "the height stays after the release"
    );
    assert!(!timeline.can_undo(), "a resize is not an edit");
    assert_eq!(timeline.mix(), &two_tracks());
    timeline.release();
    assert_eq!(timeline.lane_height(0), 250.0);

    assert_eq!(
        timeline.edge_at(point(500.0, 350.0)),
        Some(1),
        "the second lane's edge moved down"
    );
    timeline.press(point(500.0, 352.0));
    timeline.drag_to(point(500.0, 410.0));
    timeline.release();
    assert_eq!(
        timeline.lane_height(1),
        160.0,
        "measured from the lane's own top"
    );

    assert_eq!(timeline.edge_at(point(500.0, 510.0)), Some(2));
    timeline.press(point(500.0, 510.0));
    timeline.drag_to(point(500.0, 560.0));
    assert_eq!(
        timeline.lane_height(2),
        150.0,
        "the tempo lane's top is at 410 under lanes of 250 and 160"
    );
    timeline.drag_to(point(500.0, 100.0));
    timeline.release();
    assert_eq!(timeline.lane_height(2), MIN_LANE_PX);
    assert_eq!(timeline.lanes_height(), 250.0 + 160.0 + 44.0);

    timeline.press(point(500.0, 700.0));
    timeline.drag_to(point(500.0, 720.0));
    timeline.release();
    assert_eq!(
        timeline.lanes_height(),
        454.0,
        "a press below every lane does nothing"
    );
    assert!(!timeline.can_undo());
}

#[test]
fn a_node_or_an_anchor_near_an_edge_comes_before_the_edge() {
    let mut timeline = timeline();
    assert_eq!(
        timeline.edge_at(point(80.0, 98.0)),
        Some(0),
        "the edge is there whatever else is"
    );
    assert_eq!(
        timeline.hit(point(80.0, 98.0)),
        Selection::Anchor {
            track: 0,
            anchor: Anchor::Intro
        },
        "the intro anchor's column is 80"
    );
    timeline.press(point(80.0, 98.0));
    timeline.drag_to(point(90.0, 98.0));
    timeline.release();
    assert_eq!(
        timeline.mix().tracks[0].anchors.intro,
        Beats(18.0),
        "the anchor moved"
    );
    assert_eq!(timeline.lane_height(0), 100.0, "the lane did not");
    assert!(timeline.can_undo());
}

#[test]
fn a_node_on_a_lanes_bottom_edge_comes_before_the_edge() {
    let mut timeline = timeline();
    timeline.set_view(view(100.0, 140.0));
    assert_eq!(
        timeline.hit(point(700.0, 202.0)),
        Selection::Node {
            track: 1,
            curve: Curve::Volume,
            at: Beats(32.0)
        },
        "a node at silence is drawn on row 200, its lane's bottom edge, and is taken from across the edge"
    );
    timeline.press(point(700.0, 202.0));
    timeline.drag_to(point(700.0, 150.0));
    timeline.release();
    assert_eq!(timeline.lane_height(1), 100.0);
    assert!(close64(
        timeline.mix().tracks[1].volume.value_at(Beats(32.0)).0,
        20.0 * 0.5f64.log10()
    ));
}

#[test]
fn a_lanes_height_follows_its_track_through_a_move_a_removal_and_an_undo() {
    let mut timeline = timeline();
    timeline.set_lane_height(0, 200.0);
    timeline.set_lane_height(2, 300.0);
    timeline.move_track(0, 1);
    assert_eq!(timeline.mix().tracks[1].hash, ContentHash([1; 32]));
    assert_eq!(timeline.lane_height(0), 100.0);
    assert_eq!(
        timeline.lane_height(1),
        200.0,
        "the height went with the track"
    );
    assert_eq!(timeline.lane_height(2), 300.0);
    let scene = timeline.scene();
    assert_eq!((scene.lanes[1].top, scene.lanes[1].height), (100.0, 200.0));
    assert_eq!(scene.tempo.top, 300.0);
    assert!(timeline.undo());
    assert_eq!(timeline.lane_height(0), 200.0);
    assert_eq!(timeline.lane_height(1), 100.0);

    timeline.select_track(0);
    timeline.delete_selection();
    assert_eq!(timeline.mix().tracks.len(), 1);
    assert_eq!(
        timeline.lane_height(0),
        100.0,
        "the remaining track keeps its default"
    );
    assert_eq!(
        timeline.lane_height(1),
        300.0,
        "the tempo lane is lane 1 now and keeps its height"
    );
    assert_eq!(timeline.lanes_height(), 400.0);
    assert!(timeline.undo());
    assert_eq!(
        timeline.lane_height(0),
        200.0,
        "the track comes back with its height"
    );
    assert_eq!(timeline.lane_height(2), 300.0);
    assert_eq!(timeline.lanes_height(), 600.0);
}

#[test]
fn a_double_click_on_a_lanes_bottom_edge_maximizes_or_minimizes_the_lane() {
    let mut timeline = timeline();
    let before = timeline.mix().clone();
    timeline.select_track(1);
    assert!(
        !timeline.double_click(point(500.0, 50.0)),
        "the middle of a lane is no edge"
    );
    assert_eq!(timeline.lane_height(0), 100.0);

    assert!(timeline.double_click(point(500.0, 100.0)));
    assert_eq!(
        timeline.lane_height(0),
        MAX_LANE_PX,
        "a lane below the maximum goes to it"
    );
    assert_eq!(
        timeline.lane_height(1),
        100.0,
        "the other lanes keep their height"
    );
    assert_eq!(
        timeline.scene().lanes[1].top,
        MAX_LANE_PX,
        "the lanes below move down"
    );
    assert_eq!(
        timeline.selection(),
        Selection::Track(1),
        "the selection stands"
    );
    assert_eq!(*timeline.mix(), before, "a resize changes no mix");
    assert!(
        !timeline.can_undo(),
        "a resize adds nothing for undo to walk back"
    );

    assert!(
        timeline.double_click(point(500.0, MAX_LANE_PX + 4.0)),
        "the edge is within four pixels of where the taller lane ends"
    );
    assert_eq!(
        timeline.lane_height(0),
        MIN_LANE_PX,
        "a lane at the maximum goes to the minimum"
    );
    assert!(timeline.double_click(point(500.0, MIN_LANE_PX)));
    assert_eq!(
        timeline.lane_height(0),
        MAX_LANE_PX,
        "and from the minimum back to the maximum"
    );

    assert!(
        timeline.double_click(point(500.0, 800.0)),
        "the tempo lane's bottom edge, at 600 + 100 + 100"
    );
    assert_eq!(timeline.lane_height(2), MAX_LANE_PX, "the tempo lane too");

    assert!(timeline.double_click(point(500.0, 700.0)));
    assert_eq!(timeline.lane_height(1), MAX_LANE_PX);
    assert!(timeline.double_click(point(500.0, 1200.0)));
    assert_eq!(timeline.lane_height(1), MIN_LANE_PX);
    assert_eq!(
        timeline.lanes_height(),
        MAX_LANE_PX + MIN_LANE_PX + MAX_LANE_PX
    );

    assert!(!timeline.double_click(point(500.0, f32::NAN)));
    assert!(
        !timeline.double_click(point(500.0, 1000.0)),
        "a point between the edges at 644 and 1244 is no edge"
    );
    assert_eq!(timeline.lane_height(0), MAX_LANE_PX);
    assert_eq!(timeline.lane_height(1), MIN_LANE_PX);

    timeline.move_track(0, 1);
    assert_eq!(
        timeline.lane_height(1),
        MAX_LANE_PX,
        "the height follows its track through a move"
    );
    assert_eq!(timeline.lane_height(0), MIN_LANE_PX);

    // A node, an anchor, or a tempo node within HIT_PX of the point comes
    // before the edge, as it does for a press, so a double-click on a node
    // at silence, which is drawn on its lane's bottom row, resizes nothing.
    let mut timeline = self::timeline();
    timeline.set_view(view(100.0, 140.0));
    assert!(matches!(
        timeline.hit(point(700.0, 202.0)),
        Selection::Node { track: 1, .. }
    ));
    assert!(!timeline.double_click(point(700.0, 202.0)));
    assert_eq!(timeline.lane_height(1), 100.0, "the node comes first");
    assert!(timeline.double_click(point(500.0, 202.0)));
    assert_eq!(
        timeline.lane_height(1),
        MAX_LANE_PX,
        "the same edge away from the node resizes"
    );
}

#[test]
fn the_wheel_scrolls_the_lanes_and_a_zoom_key_zooms_or_scrolls_in_time() {
    assert_eq!(ZOOM_WHEEL_PX, 200.0);

    // No zoom key: the lanes take the vertical movement as the wheel gave
    // it, and a sideways movement moves the view later when the fingers go
    // left, which is the direction the content moves.
    assert_eq!(
        wheel_gesture(0.0, -30.0, false, false),
        WheelGesture::Scroll {
            lanes_px: -30.0,
            time_px: 0.0
        }
    );
    assert_eq!(
        wheel_gesture(-12.0, 0.0, false, false),
        WheelGesture::Scroll {
            lanes_px: 0.0,
            time_px: 12.0
        }
    );
    assert_eq!(
        wheel_gesture(-12.0, 5.0, false, false),
        WheelGesture::Scroll {
            lanes_px: 5.0,
            time_px: 12.0
        }
    );
    assert_eq!(
        wheel_gesture(-12.0, 5.0, false, true),
        WheelGesture::Scroll {
            lanes_px: 5.0,
            time_px: 12.0
        },
        "shift on its own changes nothing"
    );

    // A zoom key without shift: e to the movement over ZOOM_WHEEL_PX, so a
    // wheel that goes up narrows the view.
    let factor = |gesture| match gesture {
        WheelGesture::Zoom { factor } => factor,
        other => panic!("not a zoom: {other:?}"),
    };
    assert!(close64(
        factor(wheel_gesture(0.0, 200.0, true, false)),
        std::f64::consts::E
    ));
    assert!(close64(
        factor(wheel_gesture(50.0, -250.0, true, false)),
        1.0 / std::f64::consts::E
    ));
    assert!(close64(factor(wheel_gesture(0.0, 0.0, true, false)), 1.0));

    // A zoom key with shift: the view moves in time by both movements
    // together, later when the wheel goes down.
    assert_eq!(
        wheel_gesture(-12.0, -8.0, true, true),
        WheelGesture::Time { time_px: 20.0 }
    );
    assert_eq!(
        wheel_gesture(0.0, 0.0, true, true),
        WheelGesture::Time { time_px: 0.0 }
    );

    // A movement that is not a number counts as zero.
    assert_eq!(
        wheel_gesture(f32::NAN, -30.0, false, false),
        WheelGesture::Scroll {
            lanes_px: -30.0,
            time_px: 0.0
        }
    );
    assert_eq!(
        wheel_gesture(f32::INFINITY, f32::NAN, true, true),
        WheelGesture::Time { time_px: 0.0 }
    );
    assert!(close64(
        factor(wheel_gesture(f32::NAN, f32::NEG_INFINITY, true, false)),
        1.0
    ));

    // The timeline takes the pixels as it always has: ten pixels per
    // second in the default view.
    let mut timeline = timeline();
    match wheel_gesture(-100.0, 0.0, false, false) {
        WheelGesture::Scroll { time_px, .. } => timeline.scroll_by(time_px),
        other => panic!("not a scroll: {other:?}"),
    }
    assert!(
        close64(timeline.view().from.0, 10.0),
        "{:?}",
        timeline.view()
    );
    match wheel_gesture(0.0, -50.0, true, true) {
        WheelGesture::Time { time_px } => timeline.scroll_by(time_px),
        other => panic!("not a scroll in time: {other:?}"),
    }
    assert!(
        close64(timeline.view().from.0, 15.0),
        "{:?}",
        timeline.view()
    );
}
