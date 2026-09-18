#![forbid(unsafe_code)]

//! The desktop shell's view-models: plain state the window paints and
//! commands the window calls, tested without a window. The engine stays a library and
//! the window a thin shell over these types, as `DESIGN.md` requires under
//! "Architecture". The threads that read the mix's files and keep their
//! audio, in [`audio`], live here rather than in the window's binary so that
//! tests can start one and watch what it sends.

pub mod audio;
pub mod autosave;
pub mod corrections;
pub mod document;
pub mod fields;
pub mod grid;
pub mod library;
pub mod playback;
pub mod scanning;
pub mod timeline;

pub use autosave::{
    Offer, autosave_path, forget, forget_file, offered, offered_untitled, untitled_autosave_path,
    write_atomically,
};
pub use corrections::write_correction;
pub use document::{Answer, Document, EDITED, Intent, Next, Step, UNTITLED};
pub use fields::{Finish, TempoField, parse_buffer_frames};
pub use grid::{
    DRAG_PX, GridBeat, GridColumn, GridEditor, GridScene, GridView, HIGHEST_BPM, LOWEST_BPM,
    MIN_SAMPLES_PER_PX, TAP_GAP, TAPS_FOR_A_TEMPO, arrow_step,
};
pub use library::{Column, Filters, LibraryPanel, LibraryRow, Sort};
pub use playback::{Playback, PlaybackState, TransportOrder};
pub use scanning::{Report, Scan, open_the_library};
pub use timeline::{
    AnchorMark, DEFAULT_TEMPO, EDGE_PX, HIT_PX, Lane, MAX_LANE_PX, MIN_BAR_PX, MIN_LANE_PX,
    NodeMark, PhraseMark, Point, REACH_MARGIN, Scene, Selection, TEMPO_MARGIN, TempoLane,
    TempoMark, Timeline, View, WaveColumn, WheelGesture, ZOOM_WHEEL_PX, wheel_gesture,
};
