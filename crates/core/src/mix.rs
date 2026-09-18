//! The mix document: an ordered list of tracks, each with its anchors,
//! envelopes, and tempo nodes, and the timeline that follows from them.
//!
//! The document is what a project file holds. Its shape is a public contract
//! described in `docs/project-file.md`; the structures here mirror that file
//! field for field.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::anchors::Anchors;
use crate::beat_grid::BeatGrid;
use crate::envelope::Envelope;
use crate::hash::ContentHash;
use crate::tempo::{PlacedTrack, TempoCurve, TempoNode};
use crate::units::{Beats, Decibels, Samples, Seconds};

/// The version of the project file format this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// The largest magnitude of any beat in a document: an anchor, a tempo node's
/// beat, or an envelope node's beat.
///
/// Whole beats of this size add exactly in an `f64` over any number of
/// tracks a file can hold, so the layout's running sum of anchors is exact.
pub const MAX_BEAT: Beats = Beats(10_000_000.0);

/// The longest track a document may name, which is 90 minutes of audio, so
/// that a whole DJ set can be one track of a mix. The magnitude of a grid's
/// first beat has the same limit.
pub const LONGEST_TRACK: Samples = Samples(90 * 60 * crate::units::SAMPLE_RATE as i64);

/// The longest mix the app reads, lays out, or renders, which is 24 hours
/// from the first sample heard to the last.
pub const LONGEST_MIX: Seconds = Seconds(24.0 * 3_600.0);

/// The three EQ envelopes of a track, one per band.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqEnvelopes {
    /// The low band.
    pub low: Envelope,
    /// The mid band.
    pub mid: Envelope,
    /// The high band.
    pub high: Envelope,
}

/// One track in a mix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// Where the audio file was last seen.
    pub path: PathBuf,
    /// The hash of the audio file's bytes, which identifies it if it moves.
    pub hash: ContentHash,
    /// The length of the audio.
    #[serde(rename = "length_samples")]
    pub length: Samples,
    /// The track's beat grid, including its original tempo.
    pub grid: BeatGrid,
    /// Where the track joins its neighboring tracks.
    pub anchors: Anchors,
    /// Whether to keep the track's pitch when its speed changes.
    pub keylock: bool,
    /// A gain applied to the whole track on top of its volume envelope,
    /// which is how volume leveling brings every track to one loudness.
    ///
    /// `dermixen mix add` and the library panel write it from the track's
    /// measured loudness when the track joins a mix, by the rule in
    /// [`leveling_gain`](crate::leveling::leveling_gain). A project file may
    /// omit it, which means zero. A written file always has it.
    #[serde(rename = "gain_db", default)]
    pub gain: Decibels,
    /// The volume envelope.
    pub volume: Envelope,
    /// The three EQ envelopes.
    pub eq: EqEnvelopes,
    /// This track's contribution to the mix tempo curve.
    ///
    /// Each node's position is a beat of this track. Nodes travel with the
    /// track when the playlist is reordered, and the layout places them on the
    /// mix timeline. They may be given in any order.
    pub tempo: Vec<TempoNode>,
}

/// A mix document: the ordered playlist that is also the timeline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mix {
    /// The tracks in playlist order.
    pub tracks: Vec<Track>,
}

/// A problem found while reading a project file.
///
/// The field is a path into the document such as `tracks[2].grid.bpm`, or
/// `version` for a file of another version, or empty when the text is not
/// JSON at all. The message says what is wrong with the value there.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {message}")]
pub struct MixFileError {
    /// Where in the document the problem is.
    pub field: String,
    /// What is wrong there.
    pub message: String,
}

/// The mix laid out on the timeline: the resolved tempo curve and where each track sits.
#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    /// The tempo curve of the whole mix, in mix beats.
    pub curve: TempoCurve,
    /// One placement per track, in playlist order.
    pub tracks: Vec<PlacedTrack>,
}

/// The whole project file: the format version and the playlist.
///
/// A [`Mix`] holds only the playlist, so reading and writing a file goes
/// through this structure, which adds the `version` field and puts it first.
/// The type parameter lets the same field order serve both directions: the
/// reader fills a `Document<Vec<Track>>`, and the writer borrows the mix's
/// tracks as a `Document<&[Track]>` rather than copying them.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document<T> {
    version: u32,
    tracks: T,
}

/// Turns the path the serde library reports into the form the messages of
/// [`MixFileError`] use, in which a list index is written in square brackets,
/// as in `tracks[2].grid.bpm`.
fn field_path(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;

    let mut field = String::new();
    for segment in path.iter() {
        match segment {
            Segment::Seq { index } => field.push_str(&format!("[{index}]")),
            Segment::Map { key } => {
                if !field.is_empty() {
                    field.push('.');
                }
                field.push_str(key);
            }
            Segment::Enum { variant } => {
                if !field.is_empty() {
                    field.push('.');
                }
                field.push_str(variant);
            }
            Segment::Unknown => field.push_str(".?"),
        }
    }
    field
}

/// Checks the two top-level fields, `version` and `tracks`, before any track
/// is read.
///
/// These checks live here rather than with the reading of the whole document
/// because the serde library reports a problem with a top-level field without
/// a path to it, and an error with no field named is reserved for text that is
/// not JSON at all. A file of another version is also better refused by naming
/// the version than by whatever the rest of the file happens to look like.
fn check_shape(document: &serde_json::Value) -> Result<(), MixFileError> {
    let named = |field: &str, message: String| MixFileError {
        field: field.to_owned(),
        message,
    };
    let Some(fields) = document.as_object() else {
        return Err(named(
            "version",
            "a project file is a JSON object holding a version and a list of tracks".to_owned(),
        ));
    };
    let Some(version) = fields.get("version") else {
        return Err(named(
            "version",
            format!(
                "missing: a project file begins with a version, and dermixen reads version {FORMAT_VERSION}"
            ),
        ));
    };
    match version.as_u64() {
        Some(found) if found == u64::from(FORMAT_VERSION) => {}
        Some(found) => {
            return Err(named(
                "version",
                format!(
                    "this file says version {found}, and dermixen reads version {FORMAT_VERSION}"
                ),
            ));
        }
        None => {
            return Err(named(
                "version",
                format!("must be the whole number {FORMAT_VERSION}, not {version}"),
            ));
        }
    }
    if !fields.contains_key("tracks") {
        return Err(named(
            "tracks",
            "missing: a project file holds a list of tracks, which may be empty".to_owned(),
        ));
    }
    Ok(())
}

/// Checks the values that JSON itself cannot rule out: a length that is
/// negative, a tempo that is not a positive finite number, and an anchor that
/// is not a whole beat.
fn check_track(index: usize, track: &Track) -> Result<(), MixFileError> {
    let named = |field: String, message: String| MixFileError {
        field: format!("tracks[{index}].{field}"),
        message,
    };
    if track.length < Samples::ZERO {
        return Err(named(
            "length_samples".to_owned(),
            format!("must not be negative, and this one is {}", track.length.0),
        ));
    }
    if !track.grid.bpm.is_valid() {
        return Err(named(
            "grid.bpm".to_owned(),
            format!("must be a positive finite number, not {}", track.grid.bpm.0),
        ));
    }
    for (field, beat) in [
        ("anchors.intro_beat", track.anchors.intro),
        ("anchors.outro_beat", track.anchors.outro),
    ] {
        if !beat.is_whole() {
            return Err(named(
                field.to_owned(),
                format!("must be a whole beat, not {}", beat.0),
            ));
        }
    }
    for (node_index, node) in track.tempo.iter().enumerate() {
        if !node.bpm.is_valid() {
            return Err(named(
                format!("tempo[{node_index}].bpm"),
                format!("must be a positive finite number, not {}", node.bpm.0),
            ));
        }
    }
    Ok(())
}

impl Mix {
    /// An empty mix.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads a project file.
    ///
    /// The text must be a JSON object with a `version` of [`FORMAT_VERSION`]
    /// and the fields described in `docs/project-file.md`. Unknown fields,
    /// missing fields, values of the wrong type, a negative length, a hash
    /// that is not 64 hexadecimal digits, tempos that are not positive finite
    /// numbers, anchors that are not whole beats, and envelopes with two
    /// nodes at one beat are all errors, each naming the field. Text that is
    /// not JSON at all gives an error with an empty field and a message that
    /// begins `not JSON`.
    pub fn from_json(text: &str) -> Result<Mix, MixFileError> {
        let document: serde_json::Value =
            serde_json::from_str(text).map_err(|problem| MixFileError {
                field: String::new(),
                message: format!("not JSON: {problem}"),
            })?;
        check_shape(&document)?;
        let document: Document<Vec<Track>> =
            serde_path_to_error::deserialize(&document).map_err(|problem| MixFileError {
                field: field_path(problem.path()),
                message: problem.into_inner().to_string(),
            })?;
        let mix = Mix {
            tracks: document.tracks,
        };
        for (index, track) in mix.tracks.iter().enumerate() {
            check_track(index, track)?;
        }
        Ok(mix)
    }

    /// Checks that this mix is one [`Mix::from_json`] accepts, so that it is
    /// safe to lay out, to write, and to render.
    ///
    /// The error names the first field out of range, as [`Mix::from_json`]
    /// does. A mix that passes lays out without a panic, and its length is at
    /// most [`LONGEST_MIX`].
    pub fn check(&self) -> Result<(), MixFileError> {
        Ok(())
    }

    /// The project file text for this mix, or the reason [`Mix::from_json`]
    /// would refuse that text.
    ///
    /// Every writer of a mix document calls this before it touches the disk,
    /// so no command and no window action replaces a document with one that
    /// cannot be opened again.
    pub fn checked_json(&self) -> Result<String, MixFileError> {
        Ok(self.to_json())
    }

    /// Writes the project file text for this mix, indented for reading.
    pub fn to_json(&self) -> String {
        let document = Document {
            version: FORMAT_VERSION,
            tracks: self.tracks.as_slice(),
        };
        let mut text = serde_json::to_string_pretty(&document)
            .expect("every field of a mix has a JSON form, so writing one cannot fail");
        text.push('\n');
        text
    }

    /// Lays the mix out on the timeline, or returns `None` for a mix with no tracks.
    ///
    /// The first track's beat zero is at mix beat zero. Each later track is
    /// placed so that its intro anchor falls on the mix beat of the previous
    /// track's outro anchor.
    ///
    /// The tempo curve begins with a node at mix beat zero holding the first
    /// track's original tempo, which is how the first track sets the starting
    /// tempo, and then gathers every track's own tempo nodes, placed the same
    /// way as the tracks and sorted by mix beat. Nodes that land on the same
    /// mix beat keep playlist order, with the starting node first. A mix with
    /// no tempo nodes of its own therefore plays at the first track's original
    /// tempo throughout.
    ///
    /// # Panics
    ///
    /// Panics if a track's tempo, or the tempo of one of its tempo nodes, is
    /// not a positive finite number, or if an anchor or a tempo node's beat is
    /// not a finite number. [`Mix::from_json`] refuses a project file that
    /// holds any of those, so only a mix built in memory from unchecked
    /// numbers can reach this.
    pub fn timeline(&self) -> Option<Timeline> {
        let first = self.tracks.first()?;
        // The starting node is what makes the first track set the tempo the
        // mix opens at. It is not written in the file.
        let mut nodes = vec![TempoNode {
            at: Beats::ZERO,
            bpm: first.grid.bpm,
        }];
        let mut placed = Vec::with_capacity(self.tracks.len());
        let mut origin = Beats::ZERO;
        for (index, track) in self.tracks.iter().enumerate() {
            if index > 0 {
                // This track's intro anchor lands on the previous track's
                // outro anchor, which is what fixes the mix beat of this
                // track's own beat zero.
                origin += self.tracks[index - 1].anchors.outro - track.anchors.intro;
                assert!(
                    origin.0.is_finite(),
                    "Mix::from_json accepts only anchors that are finite whole beats"
                );
            }
            placed.push(PlacedTrack {
                origin,
                grid: track.grid,
                length: track.length,
            });
            nodes.extend(track.tempo.iter().map(|node| {
                let at = origin + node.at;
                TempoNode {
                    // The curve begins at mix beat zero, so a node that lands
                    // before the mix begins is heard at mix beat zero: the mix
                    // opens at the first track's tempo and takes that node's
                    // tempo at beat zero. Only a beat that is a finite number
                    // is moved; a beat that is not finite passes through, so
                    // that TempoCurve::new refuses it rather than this
                    // comparison quietly turning it into beat zero.
                    at: if at.0.is_finite() && at.0 < 0.0 {
                        Beats::ZERO
                    } else {
                        at
                    },
                    bpm: node.bpm,
                }
            }));
        }
        // TempoCurve::new sorts the nodes by mix beat and keeps the order they
        // are given in among nodes at one beat, which here is playlist order
        // with the starting node first.
        let curve = TempoCurve::new(nodes).expect(
            "Mix::from_json accepts only tempos that are positive finite numbers, \
             at anchors and node beats that are finite",
        );
        Some(Timeline {
            curve,
            tracks: placed,
        })
    }
}

impl Timeline {
    /// The mix time at which the earliest track starts.
    ///
    /// This is the moment the mix's first sample is heard. It is usually
    /// before mix time zero, because mix time zero is the first track's beat
    /// zero and that track's audio ahead of that beat plays first. A timeline
    /// with no tracks starts at mix time zero.
    pub fn start(&self) -> Seconds {
        self.tracks
            .iter()
            .map(|track| track.start(&self.curve))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or(Seconds::ZERO)
    }

    /// The mix time at which the latest track ends. A timeline with no tracks
    /// ends at mix time zero.
    pub fn end(&self) -> Seconds {
        self.tracks
            .iter()
            .map(|track| track.end(&self.curve))
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap_or(Seconds::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Bpm;

    /// A track that a mix can be built from, with every value valid.
    fn a_track() -> Track {
        Track {
            path: PathBuf::from("/Users/dermixenuser/audio/goa/track.mp3"),
            hash: ContentHash([0; 32]),
            length: Samples(44_100),
            grid: BeatGrid {
                first_beat: Samples(0),
                bpm: Bpm(138.0),
            },
            anchors: Anchors {
                intro: Beats(0.0),
                outro: Beats(64.0),
            },
            keylock: true,
            gain: crate::units::Decibels::UNITY,
            volume: Envelope::new(),
            eq: EqEnvelopes::default(),
            tempo: Vec::new(),
        }
    }

    #[test]
    fn a_file_without_a_tracks_list_names_the_tracks_field() {
        let problem = Mix::from_json("{\"version\": 1}").unwrap_err();
        assert_eq!(problem.field, "tracks");
        assert!(problem.message.contains("tracks"), "{problem}");
    }

    #[test]
    fn json_that_is_not_an_object_still_names_a_field() {
        for text in ["[1, 2]", "7", "null", "\"a mix\""] {
            let problem = Mix::from_json(text).unwrap_err();
            assert_eq!(problem.field, "version", "reading {text}");
            assert!(
                problem.message.contains("JSON object"),
                "reading {text}: {problem}"
            );
        }
    }

    #[test]
    fn a_tempo_node_before_mix_beat_zero_is_placed_at_mix_beat_zero() {
        let mut track = a_track();
        track.tempo = vec![TempoNode {
            at: Beats(-32.0),
            bpm: Bpm(140.0),
        }];
        let timeline = Mix {
            tracks: vec![track],
        }
        .timeline()
        .unwrap();
        let beats: Vec<f64> = timeline
            .curve
            .nodes()
            .iter()
            .map(|node| node.at.0)
            .collect();
        assert_eq!(beats, vec![0.0, 0.0]);
        // The starting node is still first, so the mix opens at the track's
        // own tempo and takes the moved node's tempo at mix beat zero.
        assert_eq!(timeline.curve.bpm_at_beat(Beats(-1.0)), Bpm(138.0));
        assert_eq!(timeline.curve.bpm_at_beat(Beats::ZERO), Bpm(140.0));
    }

    #[test]
    #[should_panic(expected = "finite")]
    fn a_tempo_node_at_a_beat_that_is_not_a_number_is_refused_loudly() {
        let mut track = a_track();
        track.tempo = vec![TempoNode {
            at: Beats(f64::NAN),
            bpm: Bpm(140.0),
        }];
        let mix = Mix {
            tracks: vec![track],
        };
        mix.timeline();
    }

    #[test]
    #[should_panic(expected = "finite")]
    fn an_anchor_that_is_not_a_number_is_refused_loudly() {
        let mut second = a_track();
        second.anchors.intro = Beats(f64::INFINITY);
        second.tempo = vec![TempoNode {
            at: Beats(16.0),
            bpm: Bpm(140.0),
        }];
        let mix = Mix {
            tracks: vec![a_track(), second],
        };
        mix.timeline();
    }
}
