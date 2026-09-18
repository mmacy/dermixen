//! The `mix show` command, which lays a mix document out on the timeline.
//!
//! `mix add` prints the same thing when it has finished, so a command that
//! changes a mix leaves the reader looking at the mix it made.

use std::path::Path;

use dermixen_core::{Anchors, BeatGrid, ContentHash, Mix, Samples, Seconds};
use serde::Serialize;

use crate::analyze::print_json;
use crate::text::say;

/// Where one track sits in the mix, as the `mix_show` JSON document holds
/// it. `mix plan` puts the same fields in each track of its own document,
/// with the artist, the title, the Camelot code, and the tempo step added.
#[derive(Debug, Serialize)]
pub struct TrackLine {
    /// The track's place in the playlist, counting from one.
    pub position: usize,
    /// The audio file.
    pub path: String,
    /// The hash of the audio file's bytes.
    pub hash: ContentHash,
    /// The length of the audio.
    pub length_samples: i64,
    /// The track's beat grid.
    pub grid: BeatGrid,
    /// The track's anchors.
    pub anchors: Anchors,
    /// Whether the track keeps its pitch when its speed changes.
    pub keylock: bool,
    /// The gain applied to the whole track, in decibels.
    pub gain_db: f64,
    /// When the track's first sample is heard, counted from the start of the mix.
    pub start_seconds: f64,
    /// When the track's last sample has been heard, counted the same way.
    pub end_seconds: f64,
    /// How many tempo nodes the track has.
    pub tempo_nodes: usize,
    /// How many volume nodes the track has.
    pub volume_nodes: usize,
}

/// The whole mix laid out, as the `mix_show` JSON document holds it.
#[derive(Debug, Serialize)]
pub struct Layout {
    /// The mix document.
    pub path: String,
    /// The length of the rendered mix.
    pub length_samples: i64,
    /// The same length in seconds.
    pub length_seconds: f64,
    /// One entry per track, in playlist order.
    pub tracks: Vec<TrackLine>,
}

/// A length of audio written as minutes, seconds, and tenths.
///
/// The length is rounded to tenths of a second before the minutes are taken
/// from it, so a length that rounds up to a whole minute is written as the
/// next minute and no seconds rather than as sixty seconds.
pub fn length_text(length: Seconds) -> String {
    let tenths = (length.0.max(0.0) * 10.0).round() as i64;
    let minutes = tenths / 600;
    let seconds = (tenths % 600) as f64 / 10.0;
    format!("{minutes}:{seconds:04.1}")
}

/// The name of a track's file, for the line a person reads.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Lays a mix out on the timeline.
///
/// Every time is counted from the moment the mix's first sample is heard,
/// which is where a render begins, so the times here are the times in the
/// rendered file. The length is that of the whole rendered mix, and matches
/// the number of frames a render writes.
pub fn layout(mix: &Path, document: &Mix) -> Layout {
    let timeline = document.timeline();
    let opening = timeline
        .as_ref()
        .map(|timeline| timeline.start())
        .unwrap_or(Seconds::ZERO);
    let length = timeline
        .as_ref()
        .map(|timeline| timeline.end() - opening)
        .unwrap_or(Seconds::ZERO);
    let length_samples = length.to_samples().0.max(0);

    let tracks = document
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let placed = timeline.as_ref().map(|timeline| &timeline.tracks[index]);
            let (start, end) = match (&timeline, placed) {
                (Some(timeline), Some(placed)) => (
                    placed.start(&timeline.curve) - opening,
                    placed.end(&timeline.curve) - opening,
                ),
                _ => (Seconds::ZERO, Seconds::ZERO),
            };
            TrackLine {
                position: index + 1,
                path: track.path.display().to_string(),
                hash: track.hash,
                length_samples: track.length.0,
                grid: track.grid,
                anchors: track.anchors,
                keylock: track.keylock,
                gain_db: track.gain.0,
                start_seconds: start.0,
                end_seconds: end.0,
                tempo_nodes: track.tempo.len(),
                volume_nodes: track.volume.len(),
            }
        })
        .collect();

    Layout {
        path: mix.display().to_string(),
        length_samples,
        length_seconds: Samples(length_samples).to_seconds().0,
        tracks,
    }
}

/// Prints a laid-out mix, as one JSON document or as one line per track and
/// a last line holding the length of the whole mix.
pub fn print(layout: &Layout, json: bool) {
    if json {
        print_json(layout);
        return;
    }
    for track in &layout.tracks {
        say!(
            "{:>3}  {:<32}  {:>7.2} bpm  {:>+6.1} dB  intro {:>8}  outro {:>8}  {} to {}",
            track.position,
            file_name(Path::new(&track.path)),
            track.grid.bpm.0,
            track.gain_db,
            track.anchors.intro.0,
            track.anchors.outro.0,
            length_text(Seconds(track.start_seconds)),
            length_text(Seconds(track.end_seconds)),
        );
    }
    say!(
        "{} {}, {} long, {} samples",
        layout.tracks.len(),
        if layout.tracks.len() == 1 {
            "track"
        } else {
            "tracks"
        },
        length_text(Seconds(layout.length_seconds)),
        layout.length_samples
    );
}

/// Carries out `mix show`.
pub fn run(mix: &Path, json: bool) -> Result<(), String> {
    let document = crate::document::read(mix)?;
    print(&layout(mix, &document), json);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_length_is_written_as_minutes_seconds_and_tenths() {
        assert_eq!(length_text(Seconds(0.0)), "0:00.0");
        assert_eq!(length_text(Seconds(59.94)), "0:59.9");
        assert_eq!(length_text(Seconds(88.44)), "1:28.4");
    }

    #[test]
    fn a_length_that_rounds_up_to_a_whole_minute_becomes_the_next_minute() {
        assert_eq!(length_text(Seconds(59.96)), "1:00.0");
        assert_eq!(length_text(Seconds(119.97)), "2:00.0");
    }

    #[test]
    fn an_empty_mix_lays_out_as_no_tracks_and_no_length() {
        let laid = layout(Path::new("set.dmx"), &Mix::new());
        assert_eq!(laid.length_samples, 0);
        assert_eq!(laid.length_seconds, 0.0);
        assert!(laid.tracks.is_empty());
    }
}
