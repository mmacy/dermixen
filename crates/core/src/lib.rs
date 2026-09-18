#![forbid(unsafe_code)]

//! The mix document model of Dermixen: unit types, beat grids, the mix tempo
//! curve, envelopes, anchors, the project file, and the settings file. This
//! crate is pure logic with no signal processing. The one file it reads and
//! writes is the settings file, so that the window and the command agree on
//! every setting by sharing the code that reads it.

pub mod anchors;
pub mod beat_grid;
pub mod edit;
pub mod envelope;
pub mod hash;
pub mod leveling;
pub mod mix;
pub mod settings;
pub mod tempo;
pub mod transition;
pub mod units;

pub use anchors::Anchors;
pub use beat_grid::BeatGrid;
pub use edit::{
    Anchor, Curve, Edit, EditError, HISTORY_DEPTH, History, apply_edit, clear_incoming,
    clear_outgoing, fits_between_the_anchors, outgoing_from, owner_of, span_of,
};
pub use envelope::{Envelope, EnvelopeError, EnvelopeNode};
pub use hash::{ContentHash, ContentHashParseError};
pub use leveling::{TARGET_LOUDNESS, TRUE_PEAK_CEILING, leveling_gain};
pub use mix::{EqEnvelopes, FORMAT_VERSION, Mix, MixFileError, Timeline, Track};
pub use settings::{
    DEFAULT_GRID_STRIP_COLLAPSED, DEFAULT_LIBRARY_COLLAPSED, DEFAULT_LIBRARY_FILE,
    DEFAULT_LIBRARY_WORD_WRAP, DEFAULT_METRONOME, DEFAULT_MUSIC_FOLDER, SETTINGS_FILE,
    SETTINGS_FOLDER, SETTINGS_VARIABLE, Settings, SettingsError,
};
pub use tempo::{PlacedTrack, TempoCurve, TempoCurveError, TempoNode};
pub use transition::{
    BLEND_LEAD_BARS, BLEND_RISE_BARS, DEFAULT_BARS, Preset, apply, beatmix, blend, outro_for,
};
pub use units::{BEATS_PER_BAR, Beats, Bpm, Decibels, Lufs, SAMPLE_RATE, Samples, Seconds};
