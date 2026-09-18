//! Acceptance tests for the grid editor. A coder agent makes these pass without
//! editing them.

use dermixen_app::{DRAG_PX, GridEditor, TAP_GAP, TAPS_FOR_A_TEMPO, arrow_step};
use dermixen_core::{BeatGrid, Bpm, Edit, Samples, Seconds};

fn grid(first_beat: i64, bpm: f64) -> BeatGrid {
    BeatGrid {
        first_beat: Samples(first_beat),
        bpm: Bpm(bpm),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn nudging_slides_beat_zero_and_leaves_the_tempo_alone() {
    let mut editor = GridEditor::new(2, grid(0, 130.0));
    assert_eq!(editor.track(), 2);
    editor.nudge(Samples(441));
    assert_eq!(editor.grid(), grid(441, 130.0));
    editor.nudge(Samples(-882));
    assert_eq!(
        editor.grid(),
        grid(-441, 130.0),
        "beat zero may fall before the audio"
    );
}

#[test]
fn doubling_and_halving_change_the_tempo_and_keep_beat_zero() {
    let mut editor = GridEditor::new(0, grid(4_410, 130.0));
    editor.double();
    assert_eq!(editor.grid(), grid(4_410, 260.0));
    editor.halve();
    editor.halve();
    assert_eq!(editor.grid(), grid(4_410, 65.0));
}

#[test]
fn a_typed_tempo_must_be_a_positive_finite_number() {
    let mut editor = GridEditor::new(0, grid(0, 130.0));
    assert!(editor.set_bpm(Bpm(140.5)));
    assert_eq!(editor.grid(), grid(0, 140.5));
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(!editor.set_bpm(Bpm(bad)), "{bad} was accepted");
        assert_eq!(editor.grid(), grid(0, 140.5));
    }
}

#[test]
fn four_even_taps_give_the_tempo_and_every_later_tap_refines_it() {
    assert_eq!(TAPS_FOR_A_TEMPO, 4);
    let mut editor = GridEditor::new(0, grid(0, 130.0));
    assert_eq!(editor.tap(Seconds(0.0)), None);
    assert_eq!(editor.tap(Seconds(0.5)), None);
    assert_eq!(editor.tap(Seconds(1.0)), None);
    assert_eq!(editor.taps(), 3);
    assert_eq!(
        editor.grid(),
        grid(0, 130.0),
        "nothing changes before four taps"
    );
    assert_eq!(editor.tap(Seconds(1.5)), Some(Bpm(120.0)));
    assert_eq!(editor.grid(), grid(0, 120.0));
    // A fifth tap, a little late, moves the mean interval to 0.51 seconds.
    let fifth = editor.tap(Seconds(2.04)).unwrap();
    assert!(close(fifth.0, 60.0 / 0.51), "{}", fifth.0);
    assert!(close(editor.grid().bpm.0, 60.0 / 0.51));
    assert_eq!(editor.taps(), 5);
}

#[test]
fn uneven_taps_average_and_a_long_gap_or_a_backwards_tap_starts_over() {
    let mut editor = GridEditor::new(0, grid(0, 130.0));
    for at in [0.0, 0.48, 1.0] {
        assert_eq!(editor.tap(Seconds(at)), None);
    }
    let tempo = editor.tap(Seconds(1.52)).unwrap();
    assert!(close(tempo.0, 60.0 / (1.52 / 3.0)), "{}", tempo.0);

    assert_eq!(TAP_GAP, Seconds(2.0));
    assert_eq!(
        editor.tap(Seconds(1.52 + 2.0 + 0.001)),
        None,
        "a tap after the gap starts over"
    );
    assert_eq!(editor.taps(), 1);
    assert!(
        close(editor.grid().bpm.0, 60.0 / (1.52 / 3.0)),
        "the tempo found before stands"
    );

    for at in [4.0, 4.5, 5.0] {
        editor.tap(Seconds(at));
    }
    assert_eq!(editor.taps(), 4);
    assert_eq!(
        editor.tap(Seconds(4.9)),
        None,
        "a tap before the previous one starts over"
    );
    assert_eq!(editor.taps(), 1);
    assert_eq!(
        editor.tap(Seconds(4.9)),
        None,
        "so does a tap at the same time"
    );
    assert_eq!(editor.taps(), 1);
}

#[test]
fn the_edit_puts_the_corrected_grid_on_the_track() {
    let mut editor = GridEditor::new(3, grid(100, 130.0));
    editor.double();
    editor.nudge(Samples(-100));
    assert_eq!(
        editor.edit(),
        Edit::SetGrid {
            track: 3,
            grid: grid(0, 260.0)
        }
    );
}

// The grid view: the strip of the track's own waveform with the beats over
// it, and the drag that moves the grid by the sample.

use dermixen_app::{GridBeat, GridView, HIGHEST_BPM, LOWEST_BPM, MIN_SAMPLES_PER_PX};
use dermixen_media::Frame;

fn editor_over(length: i64, width: f32) -> GridEditor {
    let mut editor = GridEditor::new(0, grid(1000, 120.0));
    editor.set_width(width);
    editor.set_length(Samples(length));
    editor
}

fn view(start: i64, samples_per_px: f64, width_px: f32) -> GridView {
    GridView {
        start: Samples(start),
        samples_per_px,
        width_px,
    }
}

fn close32(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn the_view_opens_on_the_whole_track_and_never_narrower_than_a_frame_per_pixel() {
    assert_eq!(MIN_SAMPLES_PER_PX, 1.0);
    let editor = editor_over(88_200, 1000.0);
    let v = editor.view();
    assert_eq!(v.start, Samples::ZERO);
    assert!(close(v.samples_per_px, 88.2), "{}", v.samples_per_px);
    assert_eq!(v.width_px, 1000.0);
    assert!(close32(editor.x_of(Samples(44_100)), 500.0));
    assert_eq!(editor.sample_at(500.0), Samples(44_100));
    assert_eq!(
        editor.sample_at(0.5),
        Samples(44),
        "44.1 rounds to the nearest frame"
    );

    // A track shorter than the strip is shown at one frame per pixel, not
    // stretched to fill it.
    let short = editor_over(500, 1000.0);
    assert_eq!(short.view(), view(0, 1.0, 1000.0));

    // A second length keeps the view where it is, within the new bounds.
    let mut editor = editor_over(88_200, 1000.0);
    editor.set_view(view(40_000, 20.0, 1000.0));
    editor.set_length(Samples(50_000));
    assert_eq!(
        editor.view(),
        view(30_000, 20.0, 1000.0),
        "the view ends within the shorter track"
    );

    // Before any length is given the view starts at the first frame at one
    // frame per pixel.
    let mut fresh = GridEditor::new(0, grid(0, 120.0));
    fresh.set_width(300.0);
    assert_eq!(fresh.view(), view(0, 1.0, 300.0));
}

#[test]
fn a_drag_moves_beat_zero_by_the_pixels_times_the_frames_per_pixel_from_the_press() {
    let mut editor = editor_over(200_000, 1000.0);
    editor.set_view(view(0, 4.0, 1000.0));
    assert!(!editor.dragging());
    editor.press(100.0);
    assert!(editor.dragging());
    editor.drag_to(110.4);
    assert_eq!(
        editor.grid(),
        grid(1042, 120.0),
        "10.4 pixels at 4 frames each is 41.6, rounded to 42"
    );
    editor.drag_to(90.0);
    assert_eq!(
        editor.grid(),
        grid(960, 120.0),
        "measured from the press, not from the last move"
    );
    editor.release();
    assert!(!editor.dragging());
    assert_eq!(editor.grid(), grid(960, 120.0));

    // At one frame per pixel a one-pixel move is one frame, once the press
    // has become a drag.
    editor.set_view(view(0, 1.0, 1000.0));
    editor.press(50.0);
    editor.drag_to(53.0);
    assert_eq!(editor.grid(), grid(963, 120.0));
    editor.drag_to(51.0);
    assert_eq!(editor.grid(), grid(961, 120.0));
    editor.drag_to(49.0);
    assert_eq!(editor.grid(), grid(959, 120.0));
    assert_eq!(editor.release(), None, "a drag is not a click");

    // A press let go without a move changes nothing, and a move without a
    // press changes nothing.
    editor.press(300.0);
    editor.release();
    assert_eq!(editor.grid(), grid(959, 120.0));
    editor.drag_to(600.0);
    assert_eq!(editor.grid(), grid(959, 120.0));

    // A drag may take beat zero before the first frame, as a nudge may.
    editor.set_view(view(0, 100.0, 1000.0));
    editor.press(500.0);
    editor.drag_to(480.0);
    editor.release();
    assert_eq!(editor.grid(), grid(-1041, 120.0));
    editor.press(10.0);
    editor.drag_to(f32::NAN);
    assert_eq!(
        editor.grid(),
        grid(-1041, 120.0),
        "a column that is not a number moves nothing"
    );
    editor.release();
}

#[test]
fn a_press_becomes_a_drag_past_the_drag_distance_and_a_click_names_its_frame() {
    assert_eq!(DRAG_PX, 2.0);
    let mut editor = editor_over(200_000, 1000.0);
    editor.set_view(view(0, 4.0, 1000.0));

    // A press that moves less than the drag distance is a click on the
    // frame the pressed column stands for, and the grid stays put.
    editor.press(100.0);
    editor.drag_to(101.5);
    assert_eq!(editor.grid(), grid(1000, 120.0), "1.5 pixels is not a drag");
    editor.drag_to(98.5);
    assert_eq!(editor.grid(), grid(1000, 120.0));
    assert!(editor.dragging(), "the grid is still held");
    assert_eq!(
        editor.release(),
        Some(Samples(400)),
        "column 100 at 4 frames per pixel is frame 400"
    );
    assert!(!editor.dragging());
    assert_eq!(editor.grid(), grid(1000, 120.0));

    // A press that moves the drag distance or more is a drag, measured
    // from the press, and stays one when the pointer comes back.
    editor.press(100.0);
    editor.drag_to(102.0);
    assert_eq!(
        editor.grid(),
        grid(1008, 120.0),
        "two pixels at four frames each"
    );
    editor.drag_to(100.5);
    assert_eq!(
        editor.grid(),
        grid(1002, 120.0),
        "still a drag half a pixel from the press"
    );
    assert_eq!(editor.release(), None);
    assert_eq!(editor.grid(), grid(1002, 120.0));

    // A drag to the left counts the same distance.
    editor.press(100.0);
    editor.drag_to(98.0);
    assert_eq!(editor.grid(), grid(994, 120.0));
    assert_eq!(editor.release(), None);

    // A move to a column that is not a number neither moves the grid nor
    // makes the press a drag.
    editor.press(10.0);
    editor.drag_to(f32::NAN);
    assert_eq!(editor.grid(), grid(994, 120.0));
    assert_eq!(editor.release(), Some(Samples(40)));

    // A release with nothing held gives nothing.
    assert_eq!(editor.release(), None);

    // The click's frame is held within the track: a track shorter than the
    // strip is shown at one frame per pixel, so a click past its end names
    // its last frame.
    let mut short = editor_over(500, 1000.0);
    short.press(700.0);
    assert_eq!(short.release(), Some(Samples(499)));
    short.press(0.0);
    assert_eq!(short.release(), Some(Samples(0)));

    // Before a length is given the click is held only to the first frame.
    let mut unmeasured = GridEditor::new(0, grid(0, 120.0));
    unmeasured.set_width(1000.0);
    unmeasured.press(700.0);
    assert_eq!(unmeasured.release(), Some(Samples(700)));
}

#[test]
fn set_grid_replaces_the_grid_and_lets_go_of_a_press() {
    let mut editor = editor_over(200_000, 1000.0);
    editor.set_view(view(0, 4.0, 1000.0));
    assert_eq!(editor.tap(Seconds(1.0)), None);
    assert_eq!(editor.tap(Seconds(1.5)), None);
    editor.press(100.0);
    editor.set_grid(grid(5000, 130.0));
    assert_eq!(editor.grid(), grid(5000, 130.0));
    assert!(!editor.dragging(), "the press is let go");
    editor.drag_to(200.0);
    assert_eq!(
        editor.grid(),
        grid(5000, 130.0),
        "a move after the press was let go moves nothing"
    );
    assert_eq!(editor.release(), None);
    assert_eq!(editor.taps(), 2, "the taps counted so far are kept");
    assert_eq!(editor.tap(Seconds(2.0)), None);
    assert_eq!(
        editor.tap(Seconds(2.5)),
        Some(Bpm(120.0)),
        "the fourth tap gives a tempo from all four"
    );
    assert_eq!(editor.grid(), grid(5000, 120.0));
    assert_eq!(
        editor.edit(),
        Edit::SetGrid {
            track: 0,
            grid: grid(5000, 120.0)
        }
    );
}

#[test]
fn an_arrow_key_moves_one_ten_or_a_hundred_frames() {
    assert_eq!(arrow_step(false, false), Samples(1));
    assert_eq!(arrow_step(true, false), Samples(10));
    assert_eq!(arrow_step(true, true), Samples(100));
    assert_eq!(
        arrow_step(false, true),
        Samples(1),
        "the command or alt key without shift is the plain step"
    );
}

#[test]
fn beats_in_view_are_at_their_columns_with_the_downbeats_marked() {
    // Beats every 22050 frames from 1000: 23050, 45100, 67150.
    let mut editor = editor_over(200_000, 5000.0);
    editor.set_view(view(20_000, 10.0, 5000.0));
    let scene = editor.scene(&[], None);
    assert_eq!(
        scene.beats,
        vec![
            GridBeat {
                x: 305.0,
                beat: 1,
                downbeat: false
            },
            GridBeat {
                x: 2510.0,
                beat: 2,
                downbeat: false
            },
            GridBeat {
                x: 4715.0,
                beat: 3,
                downbeat: false
            },
        ]
    );

    // Beat zero at 30000 has beat minus one at 7950, which is not a
    // downbeat; beat zero is.
    let mut editor = GridEditor::new(0, grid(30_000, 120.0));
    editor.set_width(5000.0);
    editor.set_length(Samples(200_000));
    editor.set_view(view(0, 10.0, 5000.0));
    let scene = editor.scene(&[], None);
    assert_eq!(
        scene.beats,
        vec![
            GridBeat {
                x: 795.0,
                beat: -1,
                downbeat: false
            },
            GridBeat {
                x: 3000.0,
                beat: 0,
                downbeat: true
            },
        ]
    );

    // A beat at the left edge is in the view while one at the right edge is
    // not.
    editor.set_view(view(7950, 1.0, 22_050.0));
    let scene = editor.scene(&[], None);
    assert_eq!(scene.beats.len(), 1, "{:?}", scene.beats);
    assert_eq!(
        scene.beats[0],
        GridBeat {
            x: 0.0,
            beat: -1,
            downbeat: false
        }
    );
    let mut editor = GridEditor::new(0, grid(-1000, 120.0));
    editor.set_width(1000.0);
    editor.set_length(Samples(200_000));
    editor.set_view(view(87_000, 1.0, 1000.0));
    let scene = editor.scene(&[], None);
    assert_eq!(
        scene.beats,
        vec![GridBeat {
            x: 200.0,
            beat: 4,
            downbeat: true
        }]
    );
}

#[test]
fn zooming_keeps_the_frame_under_the_pointer_and_stays_within_the_track() {
    let mut editor = editor_over(100_000, 1000.0);
    editor.set_view(view(10_000, 10.0, 1000.0));
    assert_eq!(editor.sample_at(200.0), Samples(12_000));
    editor.zoom_by(2.0, 200.0);
    assert_eq!(editor.view(), view(11_000, 5.0, 1000.0));
    assert_eq!(editor.sample_at(200.0), Samples(12_000));
    editor.zoom_by(0.5, 200.0);
    assert_eq!(editor.view(), view(10_000, 10.0, 1000.0));

    // Never narrower than a frame per pixel.
    editor.zoom_by(1000.0, 0.0);
    assert_eq!(editor.view(), view(10_000, 1.0, 1000.0));
    // Never wider than the track, which puts the whole track on the strip.
    editor.zoom_by(0.0001, 0.0);
    assert_eq!(editor.view(), view(0, 100.0, 1000.0));
    // Widening near the end keeps the view within the track.
    editor.set_view(view(95_000, 5.0, 1000.0));
    editor.zoom_by(0.5, 0.0);
    assert_eq!(editor.view(), view(90_000, 10.0, 1000.0));

    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        editor.zoom_by(bad, 100.0);
        assert_eq!(
            editor.view(),
            view(90_000, 10.0, 1000.0),
            "a factor of {bad} changed the view"
        );
    }
    editor.zoom_by(2.0, f32::NAN);
    assert_eq!(editor.view(), view(90_000, 10.0, 1000.0));
}

#[test]
fn scrolling_and_setting_the_view_stay_within_the_track() {
    let mut editor = editor_over(100_000, 1000.0);
    editor.set_view(view(10_000, 10.0, 1000.0));
    editor.scroll_by(100.0);
    assert_eq!(editor.view(), view(11_000, 10.0, 1000.0));
    editor.scroll_by(-5000.0);
    assert_eq!(
        editor.view(),
        view(0, 10.0, 1000.0),
        "never before the first frame"
    );
    editor.scroll_by(1_000_000.0);
    assert_eq!(
        editor.view(),
        view(90_000, 10.0, 1000.0),
        "never past the last frame"
    );
    editor.scroll_by(f32::NAN);
    assert_eq!(editor.view(), view(90_000, 10.0, 1000.0));

    editor.set_view(view(-50, 0.5, 1000.0));
    assert_eq!(editor.view(), view(0, 1.0, 1000.0));
    editor.set_view(view(99_000, 500.0, 1000.0));
    assert_eq!(editor.view(), view(0, 100.0, 1000.0));
    for bad in [0.0, -1.0, f32::NAN] {
        editor.set_view(view(100, 2.0, bad));
        assert_eq!(
            editor.view(),
            view(0, 100.0, 1000.0),
            "a width of {bad} was taken"
        );
    }
    editor.set_view(view(100, f64::NAN, 1000.0));
    assert_eq!(editor.view(), view(0, 100.0, 1000.0));
    editor.set_width(f32::INFINITY);
    assert_eq!(editor.view(), view(0, 100.0, 1000.0));

    // A wider strip at the same frames per pixel keeps its left edge, and
    // the bounds hold the start when the strip would run past the end.
    editor.set_view(view(50_000, 10.0, 1000.0));
    editor.set_width(2000.0);
    assert_eq!(editor.view(), view(50_000, 10.0, 2000.0));
    editor.set_width(8000.0);
    assert_eq!(editor.view(), view(20_000, 10.0, 8000.0));
}

#[test]
fn the_columns_hold_the_peak_of_their_frames_and_ignore_what_is_not_a_number() {
    let audio: Vec<Frame> = vec![
        [0.1, -0.2],
        [0.3, 0.0],
        [-0.5, 0.1],
        [0.0, 0.4],
        [f32::NAN, 0.2],
        [0.6, f32::INFINITY],
        [f32::NAN, f32::NAN],
        [-0.05, 0.0],
        [0.7, 0.0],
        [0.0, -0.8],
    ];
    // Four frames per column over ten frames on a strip two and a half
    // pixels wide: columns cover 0..4, 4..8, and 8..12, the last of which
    // runs past the end.
    let mut editor = editor_over(10, 2.5);
    assert_eq!(editor.view(), view(0, 4.0, 2.5));
    let scene = editor.scene(&audio, None);
    let peaks: Vec<f32> = scene.columns.iter().map(|column| column.peak).collect();
    assert_eq!(
        scene
            .columns
            .iter()
            .map(|column| column.x)
            .collect::<Vec<_>>(),
        vec![0.0, 1.0, 2.0]
    );
    assert_eq!(peaks, vec![0.5, 0.6, 0.8]);

    // Two and a half frames per column: columns cover 0..2, 2..5, 5..7,
    // 7..10.
    editor.set_view(view(0, 2.5, 4.0));
    let scene = editor.scene(&audio, None);
    let peaks: Vec<f32> = scene.columns.iter().map(|column| column.peak).collect();
    assert_eq!(peaks, vec![0.3, 0.5, 0.6, 0.8]);

    // One frame per column from frame 2: the column of a frame whose
    // samples are all not numbers is silent.
    editor.set_view(view(2, 1.0, 8.0));
    let scene = editor.scene(&audio, None);
    let peaks: Vec<f32> = scene.columns.iter().map(|column| column.peak).collect();
    assert_eq!(peaks, vec![0.5, 0.4, 0.2, 0.6, 0.0, 0.05, 0.7, 0.8]);

    // A track shorter than the strip leaves the columns past its end
    // silent.
    let editor = editor_over(10, 12.0);
    assert_eq!(editor.view(), view(0, 1.0, 12.0));
    let scene = editor.scene(&audio, None);
    let peaks: Vec<f32> = scene.columns.iter().map(|column| column.peak).collect();
    assert_eq!(
        peaks,
        vec![0.2, 0.3, 0.5, 0.4, 0.2, 0.6, 0.0, 0.05, 0.7, 0.8, 0.0, 0.0]
    );
}

#[test]
fn the_view_follows_the_playhead_by_turning_the_page() {
    let mut editor = editor_over(100_000, 4000.0);
    editor.set_view(view(0, 1.0, 4000.0));
    editor.follow(Samples(3999));
    assert_eq!(
        editor.view(),
        view(0, 1.0, 4000.0),
        "a frame in the view moves nothing"
    );
    editor.follow(Samples(4000));
    assert_eq!(
        editor.view(),
        view(4000, 1.0, 4000.0),
        "the frame past the right edge starts the next page"
    );
    editor.follow(Samples(4100));
    assert_eq!(editor.view(), view(4000, 1.0, 4000.0));
    editor.follow(Samples(100));
    assert_eq!(
        editor.view(),
        view(100, 1.0, 4000.0),
        "a frame before the view sits at the left edge"
    );
    editor.follow(Samples(99_000));
    assert_eq!(
        editor.view(),
        view(96_000, 1.0, 4000.0),
        "the last page ends at the track's end"
    );

    let scene = editor.scene(&[], Some(Samples(96_500)));
    assert_eq!(scene.playhead, Some(500.0));
    assert_eq!(editor.scene(&[], Some(Samples(50))).playhead, None);
    assert_eq!(
        editor.scene(&[], Some(Samples(100_000))).playhead,
        None,
        "the right edge is outside the view"
    );
    assert_eq!(editor.scene(&[], None).playhead, None);
}

#[test]
fn the_tempo_is_held_between_the_slowest_and_the_fastest_music() {
    assert_eq!(LOWEST_BPM, 20.0);
    assert_eq!(HIGHEST_BPM, 999.0);

    // Doubling stops where the next doubling would pass the ceiling, and
    // halving where the next halving would pass the floor.
    let mut editor = GridEditor::new(0, grid(4_410, 140.0));
    editor.double();
    editor.double();
    assert_eq!(editor.grid(), grid(4_410, 560.0));
    editor.double();
    assert_eq!(
        editor.grid(),
        grid(4_410, 560.0),
        "1120 is above the ceiling"
    );
    let mut editor = GridEditor::new(0, grid(4_410, 140.0));
    editor.halve();
    editor.halve();
    assert_eq!(editor.grid(), grid(4_410, 35.0));
    editor.halve();
    assert_eq!(editor.grid(), grid(4_410, 35.0), "17.5 is below the floor");

    // A typed tempo is held to the same bounds, both ends included.
    let mut editor = GridEditor::new(0, grid(0, 130.0));
    assert!(editor.set_bpm(Bpm(LOWEST_BPM)));
    assert!(editor.set_bpm(Bpm(HIGHEST_BPM)));
    assert!(!editor.set_bpm(Bpm(19.999)));
    assert!(!editor.set_bpm(Bpm(999.001)));
    assert!(!editor.set_bpm(Bpm(1_000_000.0)));
    assert_eq!(editor.grid(), grid(0, HIGHEST_BPM));

    // Taps fifty milliseconds apart say twelve hundred beats per minute,
    // which is refused, and the count goes on.
    let mut editor = GridEditor::new(0, grid(0, 130.0));
    for i in 0..4 {
        assert_eq!(editor.tap(Seconds(i as f64 * 0.05)), None);
    }
    assert_eq!(editor.taps(), 4);
    assert_eq!(editor.grid(), grid(0, 130.0));
    assert_eq!(editor.tap(Seconds(0.2)), None);
    assert_eq!(editor.taps(), 5);

    // The strip draws no beats for a grid outside the bounds, which can
    // arrive with a track even though the editor's own controls refuse it.
    let mut editor = GridEditor::new(0, grid(0, 100_000.0));
    editor.set_width(1000.0);
    editor.set_length(Samples(1_000_000));
    assert!(editor.scene(&[], None).beats.is_empty());
    let mut editor = GridEditor::new(0, grid(0, 10.0));
    editor.set_width(1000.0);
    editor.set_length(Samples(1_000_000));
    assert!(editor.scene(&[], None).beats.is_empty());
    let mut editor = GridEditor::new(0, grid(0, 999.0));
    editor.set_width(1000.0);
    editor.set_length(Samples(1_000_000));
    assert!(!editor.scene(&[], None).beats.is_empty());
}

#[test]
fn a_column_that_is_not_a_number_maps_to_the_left_edge() {
    let mut editor = editor_over(100_000, 1000.0);
    editor.set_view(view(10_000, 10.0, 1000.0));
    assert_eq!(editor.sample_at(f32::NAN), Samples(10_000));
    assert_eq!(editor.sample_at(f32::INFINITY), Samples(10_000));
    assert_eq!(editor.sample_at(f32::NEG_INFINITY), Samples(10_000));
    assert_eq!(editor.sample_at(200.0), Samples(12_000));
}

#[test]
fn each_adjustment_is_taken_back_and_made_again_one_at_a_time() {
    let mut editor = GridEditor::new(0, grid(1000, 120.0));
    assert!(!editor.can_undo());
    assert!(!editor.can_redo());
    assert!(
        !editor.undo(),
        "nothing has been adjusted since the editor opened"
    );
    assert!(!editor.redo());
    assert_eq!(editor.grid(), grid(1000, 120.0));

    editor.nudge(Samples(5));
    editor.double();
    assert!(editor.set_bpm(Bpm(130.0)));
    editor.halve();
    assert_eq!(editor.grid(), grid(1005, 65.0));
    assert!(editor.can_undo());
    assert!(!editor.can_redo());

    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(1005, 130.0),
        "the halving is taken back"
    );
    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(1005, 240.0),
        "the typed tempo is taken back"
    );
    assert!(editor.can_redo());
    assert!(editor.redo());
    assert_eq!(editor.grid(), grid(1005, 130.0));
    assert!(editor.redo());
    assert_eq!(editor.grid(), grid(1005, 65.0));
    assert!(!editor.redo(), "nothing is left to make again");
    assert!(!editor.can_redo());

    assert!(editor.undo());
    assert!(editor.undo());
    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(1005, 120.0),
        "the doubling is taken back"
    );
    assert!(editor.undo());
    assert_eq!(editor.grid(), grid(1000, 120.0), "the nudge is taken back");
    assert!(!editor.can_undo());
    assert!(
        !editor.undo(),
        "the grid the editor opened on is as far back as it goes"
    );
    assert_eq!(editor.grid(), grid(1000, 120.0));

    assert!(editor.redo());
    assert_eq!(editor.grid(), grid(1005, 120.0));
    editor.nudge(Samples(-5));
    assert_eq!(editor.grid(), grid(1000, 120.0));
    assert!(
        !editor.can_redo(),
        "an adjustment after an undo drops what could have been made again"
    );
    assert!(!editor.redo());
    assert!(editor.undo());
    assert_eq!(editor.grid(), grid(1005, 120.0));
    assert_eq!(
        editor.edit(),
        Edit::SetGrid {
            track: 0,
            grid: grid(1005, 120.0)
        }
    );
}

#[test]
fn a_call_that_leaves_the_grid_as_it_was_is_no_adjustment() {
    let mut editor = GridEditor::new(0, grid(0, 120.0));
    editor.nudge(Samples(0));
    assert!(!editor.can_undo(), "a nudge of no frames");
    assert!(!editor.set_bpm(Bpm(0.0)));
    assert!(!editor.can_undo(), "a refused tempo");
    assert!(editor.set_bpm(Bpm(120.0)));
    assert!(!editor.can_undo(), "a tempo the grid already has");

    let mut fast = GridEditor::new(0, grid(0, 960.0));
    fast.double();
    assert_eq!(fast.grid(), grid(0, 960.0));
    assert!(!fast.can_undo(), "a refused doubling");
    let mut slow = GridEditor::new(0, grid(0, 30.0));
    slow.halve();
    assert_eq!(slow.grid(), grid(0, 30.0));
    assert!(!slow.can_undo(), "a refused halving");

    assert_eq!(editor.tap(Seconds(0.0)), None);
    assert_eq!(editor.tap(Seconds(0.4)), None);
    assert_eq!(editor.tap(Seconds(0.8)), None);
    assert!(!editor.can_undo(), "three taps have changed nothing");
    let tapped = editor
        .tap(Seconds(1.2))
        .expect("the fourth tap gives a tempo");
    assert!(close(tapped.0, 150.0), "{tapped:?}");
    assert!(editor.can_undo(), "the fourth tap changed the tempo");
    assert!(editor.undo());
    assert_eq!(editor.grid(), grid(0, 120.0));
    assert_eq!(editor.taps(), 4, "the taps counted so far are kept");
    assert!(editor.can_redo());

    let refined = editor
        .tap(Seconds(1.7))
        .expect("a fifth tap refines the tempo");
    assert!(close(refined.0, 60.0 / 0.425), "{refined:?}");
    assert!(editor.can_undo());
    assert!(
        !editor.can_redo(),
        "the fifth tap dropped the tempo that could have been made again"
    );
    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(0, 120.0),
        "the fifth tap was made on the grid the undo of the fourth left"
    );
}

#[test]
fn a_drag_is_one_adjustment_and_the_documents_grid_drops_them_all() {
    let mut editor = editor_over(200_000, 1000.0);
    editor.set_view(view(0, 4.0, 1000.0));

    editor.press(100.0);
    editor.drag_to(110.4);
    editor.drag_to(120.0);
    assert_eq!(editor.release(), None);
    assert_eq!(editor.grid(), grid(1080, 120.0));
    assert!(editor.can_undo());
    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(1000, 120.0),
        "the whole drag is taken back at once, however many moves it took"
    );
    assert!(!editor.can_undo());
    assert!(editor.redo());
    assert_eq!(editor.grid(), grid(1080, 120.0));

    editor.press(300.0);
    assert_eq!(editor.release(), Some(Samples(1200)), "a click");
    assert!(editor.can_undo());
    assert!(!editor.can_redo());
    editor.press(400.0);
    editor.drag_to(410.0);
    editor.drag_to(400.0);
    assert_eq!(editor.release(), None);
    assert_eq!(editor.grid(), grid(1080, 120.0));
    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(1000, 120.0),
        "a click and a drag that ended where it began were no adjustments, so the undo takes back the first drag"
    );
    assert!(!editor.can_undo());
    assert!(editor.redo());
    assert_eq!(editor.grid(), grid(1080, 120.0));

    editor.press(100.0);
    editor.drag_to(110.0);
    assert_eq!(editor.grid(), grid(1120, 120.0));
    assert!(editor.dragging());
    assert!(editor.undo());
    assert!(!editor.dragging(), "an undo lets go of the press");
    assert_eq!(
        editor.grid(),
        grid(1000, 120.0),
        "the grid the press held goes with the adjustment before it"
    );
    editor.drag_to(200.0);
    assert_eq!(
        editor.grid(),
        grid(1000, 120.0),
        "the let-go press moves nothing"
    );
    assert_eq!(editor.release(), None);
    assert!(editor.redo());
    assert_eq!(editor.grid(), grid(1080, 120.0));

    editor.nudge(Samples(7));
    assert!(editor.undo());
    assert!(editor.can_undo() && editor.can_redo());
    editor.set_grid(grid(5000, 130.0));
    assert!(
        !editor.can_undo() && !editor.can_redo(),
        "the document's grid drops every adjustment"
    );
    assert!(!editor.undo());
    assert!(!editor.redo());
    assert_eq!(editor.grid(), grid(5000, 130.0));
    editor.nudge(Samples(1));
    assert!(editor.undo());
    assert_eq!(
        editor.grid(),
        grid(5000, 130.0),
        "an adjustment made after it is taken back to the document's grid"
    );
}
