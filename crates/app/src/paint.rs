//! Painting the timeline: the scene the view-model describes, drawn with
//! lines, rectangles, and text.
//!
//! Nothing here decides anything about the mix. Every position comes from
//! [`Scene`], whose pixel columns and rows are counted from the top left of
//! the timeline's own area, so painting is a matter of adding that area's
//! corner to each of them.

use std::collections::HashMap;

use dermixen_app::{GridScene, Lane, Scene, Selection, View};
use dermixen_core::{Anchor, Curve, Seconds};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2};

/// The color behind a track lane in an even position of the playlist.
const LANE_EVEN: Color32 = Color32::from_rgb(28, 30, 36);

/// The color behind a track lane in an odd position of the playlist.
const LANE_ODD: Color32 = Color32::from_rgb(33, 35, 42);

/// The color behind the selected track's lane.
const LANE_SELECTED: Color32 = Color32::from_rgb(44, 48, 60);

/// The color behind the stretch of a lane the track's audio covers.
const AUDIO_SPAN: Color32 = Color32::from_rgb(38, 42, 52);

/// The color of a waveform.
const WAVE: Color32 = Color32::from_rgb(96, 132, 176);

/// The color of a bar line.
const BAR: Color32 = Color32::from_rgb(58, 62, 72);

/// The color of a phrase start.
const PHRASE: Color32 = Color32::from_rgb(120, 176, 120);

/// The color of a section change.
const SECTION: Color32 = Color32::from_rgb(200, 160, 80);

/// The color of the intro anchor.
const INTRO: Color32 = Color32::from_rgb(120, 200, 240);

/// The color of the outro anchor.
const OUTRO: Color32 = Color32::from_rgb(240, 150, 120);

/// The color of the playhead.
const PLAYHEAD: Color32 = Color32::from_rgb(250, 250, 250);

/// The color of a label.
const LABEL: Color32 = Color32::from_rgb(210, 214, 222);

/// The color of a label that says something secondary.
const FAINT: Color32 = Color32::from_rgb(140, 146, 158);

/// The color of the tempo curve.
const TEMPO: Color32 = Color32::from_rgb(230, 200, 120);

/// The color of a node that is not selected.
const NODE: Color32 = Color32::from_rgb(240, 240, 240);

/// The color of the selected node.
const NODE_SELECTED: Color32 = Color32::from_rgb(255, 210, 60);

/// The radius a node is drawn at.
const NODE_RADIUS: f32 = 4.0;

/// The size of an ordinary label.
const TEXT: f32 = 12.0;

/// The size of a label beside a mark on a lane.
const SMALL_TEXT: f32 = 10.0;

/// The color the selected curve is drawn in.
fn curve_color(curve: Curve) -> Color32 {
    match curve {
        Curve::Volume => Color32::from_rgb(150, 220, 150),
        Curve::Low => Color32::from_rgb(230, 140, 140),
        Curve::Mid => Color32::from_rgb(200, 180, 240),
        Curve::High => Color32::from_rgb(140, 210, 230),
    }
}

/// What a curve is called in the window.
pub fn curve_name(curve: Curve) -> &'static str {
    match curve {
        Curve::Volume => "Volume",
        Curve::Low => "Low",
        Curve::Mid => "Mid",
        Curve::High => "High",
    }
}

/// A time as minutes, seconds, and tenths, which is how every time in the
/// window is written. The library panel writes the length column with the
/// same function, so a length in the table and a time on the ruler are
/// written the same way.
pub use dermixen_app::library::time_text;

/// A time as minutes and seconds, which is short enough to label a ruler
/// tick with.
fn tick_text(at: Seconds) -> String {
    let total = at.0.max(0.0);
    let minutes = (total / 60.0).floor();
    let seconds = (total - minutes * 60.0).round();
    // A second that rounds up to sixty belongs to the next minute.
    if seconds >= 60.0 {
        return format!("{}:00", minutes + 1.0);
    }
    format!("{minutes}:{seconds:02.0}")
}

/// The spacing of ruler ticks, in seconds, from the closest together to the
/// furthest apart. The ruler takes the first that leaves its ticks far
/// enough apart to label.
const TICK_STEPS: [f64; 12] = [
    1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0,
];

/// How many pixels apart ruler ticks must be for their labels to be read.
const TICK_GAP_PX: f64 = 72.0;

/// Paints the ruler above the lanes: a tick and a time for every round
/// number of seconds that fits.
pub fn ruler(painter: &Painter, rect: Rect, view: View) {
    painter.rect_filled(rect, 0.0, LANE_ODD);
    let span = view.to.0 - view.from.0;
    if !(span.is_finite() && span > 0.0) {
        return;
    }
    let per_pixel = span / f64::from(view.width_px);
    let step = TICK_STEPS
        .iter()
        .copied()
        .find(|step| step / per_pixel >= TICK_GAP_PX)
        .unwrap_or(*TICK_STEPS.last().expect("the steps are not empty"));

    let first = (view.from.0 / step).floor() * step;
    let mut at = first;
    while at <= view.to.0 {
        if at >= 0.0 {
            let x = rect.left() + ((at - view.from.0) / per_pixel) as f32;
            painter.add(Shape::line_segment(
                [pos2(x, rect.bottom() - 6.0), pos2(x, rect.bottom())],
                Stroke::new(1.0, FAINT),
            ));
            painter.text(
                pos2(x + 3.0, rect.top() + 2.0),
                Align2::LEFT_TOP,
                tick_text(Seconds(at)),
                FontId::proportional(SMALL_TEXT),
                FAINT,
            );
        }
        at += step;
    }
}

/// What a lane says in place of a waveform when the window has no audio for
/// its track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoFile {
    /// There is no file at the path the mix document names for the track.
    Missing,
    /// A file is at the path the mix document names, and its content hash is
    /// not the one the document names for the track.
    Changed,
    /// A file is at the path the mix document names, and the window could
    /// not read it.
    Unreadable,
}

impl NoFile {
    /// What the lane's label says about the file, ready to be added to the
    /// end of the label.
    fn label(self) -> &'static str {
        match self {
            NoFile::Missing => "  ·  the file is missing",
            NoFile::Changed => "  ·  the file has changed",
            NoFile::Unreadable => "  ·  the file cannot be read",
        }
    }
}

/// Paints everything the scene describes into `rect`, whose top left corner
/// is the scene's origin. Every lane names its own top and its own height,
/// so a lane a person has made taller is painted taller and the lanes under
/// it are painted further down.
///
/// `no_file` names the playlist position of every track the window has no
/// audio for, and what is wrong with its file, which the lane of that track
/// says in place of a waveform.
pub fn scene(
    painter: &Painter,
    rect: Rect,
    scene: &Scene,
    curve: Curve,
    selection: Selection,
    no_file: &HashMap<usize, NoFile>,
) {
    let selected_track = match selection {
        Selection::Track(track) => Some(track),
        Selection::Node { track, .. }
        | Selection::Anchor { track, .. }
        | Selection::TempoNode { track, .. } => Some(track),
        Selection::Nothing => None,
    };
    for lane in &scene.lanes {
        paint_lane(
            painter,
            rect,
            lane,
            curve,
            selected_track == Some(lane.track),
            no_file.get(&lane.track).copied(),
        );
    }
    paint_tempo(painter, rect, scene);
    if let Some(x) = scene.playhead_x {
        painter.add(Shape::line_segment(
            [
                pos2(rect.left() + x, rect.top()),
                pos2(rect.left() + x, rect.bottom()),
            ],
            Stroke::new(1.5, PLAYHEAD),
        ));
    }
}

/// Paints one track's lane: its background, its waveform, its bar lines,
/// phrase starts and section changes, the selected curve and its nodes, its
/// anchors, and its label.
///
/// The label names the track's place in the playlist, its file name, its
/// original tempo, whether keylock is on, and the gain that volume leveling
/// wrote for it. A lane whose track has a `no_file` note says what is wrong
/// with the file there, since the window cannot draw a waveform for a file
/// it has not read.
fn paint_lane(
    painter: &Painter,
    rect: Rect,
    lane: &Lane,
    curve: Curve,
    selected: bool,
    no_file: Option<NoFile>,
) {
    let top = rect.top() + lane.top;
    let bottom = top + lane.height;
    let area = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom));
    let background = if selected {
        LANE_SELECTED
    } else if lane.track.is_multiple_of(2) {
        LANE_EVEN
    } else {
        LANE_ODD
    };
    painter.rect_filled(area, 0.0, background);

    // The stretch the track's audio covers, so a gap between two songs is
    // plain to see.
    let from = (rect.left() + lane.start_x).max(rect.left());
    let to = (rect.left() + lane.end_x).min(rect.right());
    if to > from {
        painter.rect_filled(
            Rect::from_min_max(pos2(from, top + 1.0), pos2(to, bottom - 1.0)),
            0.0,
            AUDIO_SPAN,
        );
    }

    let mut shapes = Vec::new();
    for column in &lane.bars {
        shapes.push(Shape::line_segment(
            [
                pos2(rect.left() + column, top),
                pos2(rect.left() + column, bottom),
            ],
            Stroke::new(1.0, BAR),
        ));
    }
    for wave in &lane.waveform {
        shapes.push(Shape::line_segment(
            [
                pos2(rect.left() + wave.x, rect.top() + wave.top),
                pos2(rect.left() + wave.x, rect.top() + wave.bottom),
            ],
            Stroke::new(1.0, WAVE),
        ));
    }
    for column in &lane.sections {
        shapes.push(Shape::line_segment(
            [
                pos2(rect.left() + column, top),
                pos2(rect.left() + column, bottom),
            ],
            Stroke::new(1.0, SECTION),
        ));
    }
    for phrase in &lane.phrases {
        // A tick at the foot of the lane, drawn taller the longer the phrase
        // that starts there, so the sixteen and thirty-two bar boundaries a
        // transition lands on stand out from the shorter ones.
        let tall = match phrase.bars {
            0..=8 => 8.0,
            9..=16 => 12.0,
            _ => 16.0,
        };
        shapes.push(Shape::line_segment(
            [
                pos2(rect.left() + phrase.x, bottom - tall),
                pos2(rect.left() + phrase.x, bottom),
            ],
            Stroke::new(2.0, PHRASE),
        ));
    }
    painter.extend(shapes);

    if lane.curve.len() > 1 {
        painter.add(Shape::line(
            lane.curve
                .iter()
                .map(|point| pos2(rect.left() + point.x, rect.top() + point.y))
                .collect(),
            Stroke::new(1.5, curve_color(curve)),
        ));
    }
    for node in &lane.nodes {
        let at = pos2(rect.left() + node.at_px.x, rect.top() + node.at_px.y);
        painter.circle_filled(
            at,
            NODE_RADIUS,
            if node.selected { NODE_SELECTED } else { NODE },
        );
        if node.selected {
            painter.text(
                pos2(at.x + 6.0, at.y - 14.0),
                Align2::LEFT_TOP,
                format!("{:.1} dB", node.level.0),
                FontId::proportional(SMALL_TEXT),
                NODE_SELECTED,
            );
        }
    }

    for anchor in &lane.anchors {
        let x = rect.left() + anchor.x;
        let color = match anchor.anchor {
            Anchor::Intro => INTRO,
            Anchor::Outro => OUTRO,
        };
        painter.add(Shape::line_segment(
            [pos2(x, top), pos2(x, bottom)],
            Stroke::new(if anchor.selected { 3.0 } else { 2.0 }, color),
        ));
        let name = match anchor.anchor {
            Anchor::Intro => "in",
            Anchor::Outro => "out",
        };
        painter.text(
            pos2(x + 3.0, bottom - 14.0),
            Align2::LEFT_TOP,
            name,
            FontId::proportional(SMALL_TEXT),
            color,
        );
    }

    let keylock = if lane.keylock {
        "Keylock on"
    } else {
        "Keylock off"
    };
    let file = match no_file {
        Some(note) => note.label(),
        None => "",
    };
    painter.text(
        pos2(rect.left() + 6.0, top + 3.0),
        Align2::LEFT_TOP,
        format!(
            "{}. {}  ·  {:.1} BPM  ·  {keylock}  ·  {:+.1} dB{file}",
            lane.track + 1,
            lane.name,
            lane.bpm.0,
            lane.gain.0
        ),
        FontId::proportional(TEXT),
        if no_file.is_some() { SECTION } else { LABEL },
    );
    painter.add(Shape::line_segment(
        [pos2(rect.left(), bottom), pos2(rect.right(), bottom)],
        Stroke::new(1.0, Color32::from_rgb(18, 20, 24)),
    ));
}

/// Paints the tempo lane: the mix tempo curve, its nodes, and the range of
/// tempo the lane covers.
fn paint_tempo(painter: &Painter, rect: Rect, scene: &Scene) {
    let top = rect.top() + scene.tempo.top;
    let bottom = top + scene.tempo.height;
    let area = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom));
    painter.rect_filled(area, 0.0, Color32::from_rgb(22, 24, 30));

    if scene.tempo.curve.len() > 1 {
        painter.add(Shape::line(
            scene
                .tempo
                .curve
                .iter()
                .map(|point| pos2(rect.left() + point.x, rect.top() + point.y))
                .collect(),
            Stroke::new(1.5, TEMPO),
        ));
    }
    for node in &scene.tempo.nodes {
        let at = pos2(rect.left() + node.at_px.x, rect.top() + node.at_px.y);
        painter.circle_filled(
            at,
            NODE_RADIUS,
            if node.selected { NODE_SELECTED } else { TEMPO },
        );
        painter.text(
            pos2(at.x + 6.0, at.y - 14.0),
            Align2::LEFT_TOP,
            format!("{:.1}", node.bpm.0),
            FontId::proportional(SMALL_TEXT),
            if node.selected { NODE_SELECTED } else { TEMPO },
        );
    }
    painter.text(
        pos2(rect.left() + 6.0, top + 3.0),
        Align2::LEFT_TOP,
        format!(
            "Mix tempo  ·  {:.1} to {:.1} BPM",
            scene.tempo.low.0, scene.tempo.high.0
        ),
        FontId::proportional(TEXT),
        LABEL,
    );
}

/// The color behind the grid editor's waveform strip.
const STRIP: Color32 = Color32::from_rgb(20, 22, 28);

/// The color of a beat line over the grid editor's waveform.
const BEAT: Color32 = Color32::from_rgb(150, 158, 172);

/// The color of a beat line that starts a bar, which is brighter than the
/// other three beats of the bar so the bars can be seen as well as heard.
const DOWNBEAT: Color32 = Color32::from_rgb(255, 210, 60);

/// Paints the grid editor's strip: the track's own waveform with the grid's
/// beats drawn over it and the audition's playhead among them.
///
/// Every column and every beat comes from [`GridScene`], counted from the
/// left edge of `rect`, so painting is a matter of adding that edge to each
/// of them. The waveform is drawn as a bar from the middle of the strip
/// outward in both directions, so a quiet passage is a thin band and a loud
/// one fills the strip.
pub fn grid_strip(painter: &Painter, rect: Rect, scene: &GridScene) {
    painter.rect_filled(rect, 0.0, STRIP);
    let middle = rect.center().y;
    let reach = rect.height() / 2.0 - 2.0;

    let mut shapes = Vec::new();
    for column in &scene.columns {
        if column.peak <= 0.0 {
            continue;
        }
        let half = column.peak * reach;
        let x = rect.left() + column.x;
        shapes.push(Shape::line_segment(
            [pos2(x, middle - half), pos2(x, middle + half)],
            Stroke::new(1.0, WAVE),
        ));
    }
    for beat in &scene.beats {
        let x = rect.left() + beat.x;
        let (color, width) = if beat.downbeat {
            (DOWNBEAT, 1.5)
        } else {
            (BEAT, 1.0)
        };
        shapes.push(Shape::line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(width, color),
        ));
    }
    painter.extend(shapes);

    if let Some(x) = scene.playhead {
        let x = rect.left() + x;
        painter.add(Shape::line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5, PLAYHEAD),
        ));
    }
}

/// Where a point in the window falls in the scene's own pixels.
pub fn point_in(rect: Rect, at: Pos2) -> dermixen_app::Point {
    dermixen_app::Point {
        x: at.x - rect.left(),
        y: at.y - rect.top(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use dermixen_app::Timeline;
    use dermixen_core::{
        Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Envelope, EqEnvelopes, Mix, Samples,
        Track, beatmix,
    };

    /// A two-minute track at 120 beats per minute whose gain is `gain`.
    fn track(name: &str, byte: u8, gain: Decibels) -> Track {
        Track {
            path: PathBuf::from(name),
            hash: ContentHash([byte; 32]),
            length: Samples(120 * 44_100),
            grid: BeatGrid {
                first_beat: Samples::ZERO,
                bpm: Bpm(120.0),
            },
            anchors: Anchors {
                intro: Beats(16.0),
                outro: Beats(200.0),
            },
            keylock: true,
            gain,
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        }
    }

    /// The text of every label [`scene`] drew for a mix of two tracks, the
    /// first turned down by six decibels and the second up by two and a
    /// half, with the note in `no_file` on the lane at each position it
    /// names.
    ///
    /// The shapes come back from a real `egui` context, so this is the text
    /// a person reads on the lanes rather than a string the test built.
    fn labels(no_file: &HashMap<usize, NoFile>) -> Vec<String> {
        let mut first = track("kalifornia.mp3", 1, Decibels(-6.0));
        let mut second = track("mahadeva.mp3", 2, Decibels(2.5));
        beatmix(&mut first, &mut second, 8);
        let mut timeline = Timeline::new(Mix {
            tracks: vec![first, second],
        });
        timeline.set_view(View {
            width_px: 800.0,
            lane_height_px: 80.0,
            from: Seconds(0.0),
            to: Seconds(240.0),
        });
        let scene = timeline.scene();

        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 240.0));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            super::scene(
                &ui.painter_at(rect),
                rect,
                &scene,
                Curve::Volume,
                Selection::Nothing,
                no_file,
            );
        });
        let shapes = std::mem::take(&mut output.shapes);
        // The font texture the pass built belongs to a painter this test has
        // none of, and `egui` asks to be told that on the way out.
        output.drop_without_applying_deltas();
        shapes
            .into_iter()
            .filter_map(|clipped| match clipped.shape {
                Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_lane_label_says_the_tracks_gain_with_its_sign() {
        let labels = labels(&HashMap::new());
        assert!(
            labels
                .iter()
                .any(|label| label.contains("kalifornia") && label.contains("-6.0 dB")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|label| label.contains("mahadeva") && label.contains("+2.5 dB")),
            "{labels:?}"
        );
    }

    #[test]
    fn a_lane_whose_file_is_missing_says_so_and_no_other_lane_does() {
        let labels = labels(&HashMap::from([(1, NoFile::Missing)]));
        let said: Vec<&String> = labels
            .iter()
            .filter(|label| label.contains("the file is missing"))
            .collect();
        assert_eq!(said.len(), 1, "{labels:?}");
        assert!(said[0].contains("mahadeva"), "{said:?}");
    }

    #[test]
    fn a_lane_whose_file_cannot_be_read_says_so() {
        let labels = labels(&HashMap::from([(1, NoFile::Unreadable)]));
        let said: Vec<&String> = labels
            .iter()
            .filter(|label| label.contains("the file cannot be read"))
            .collect();
        assert_eq!(said.len(), 1, "{labels:?}");
        assert!(said[0].contains("mahadeva"), "{said:?}");
    }

    #[test]
    fn a_lane_whose_file_has_other_bytes_says_the_file_has_changed() {
        let labels = labels(&HashMap::from([(0, NoFile::Changed)]));
        let said: Vec<&String> = labels
            .iter()
            .filter(|label| label.contains("the file has changed"))
            .collect();
        assert_eq!(said.len(), 1, "{labels:?}");
        assert!(said[0].contains("kalifornia"), "{said:?}");
        assert!(
            !labels.iter().any(|label| label.contains("is missing")),
            "{labels:?}"
        );
    }
}
