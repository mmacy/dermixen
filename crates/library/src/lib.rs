#![forbid(unsafe_code)]

//! The library: folder scanning, tag metadata, the SQLite library file that
//! contains the analysis results, and the query interface that the
//! command-line interface and the desktop shell share. `docs/library.md`
//! describes what the library contains and how files are named and matched.

pub mod find;
pub mod index;
pub mod metadata;
pub mod pipeline;
pub mod relink;
pub mod scan;

pub use find::{Match, find};
pub use index::{
    Index, IndexError, KeyRecord, PhraseRecord, PhraseStartRecord, Query, SCHEMA_VERSION,
    TrackRecord,
};
pub use metadata::{
    Metadata, MetadataSource, Release, ReleaseDataSource, TagError, metadata_of, parse_filename,
    read_tags,
};
pub use pipeline::{
    AnalyzeError, AnalyzerSet, Analyzers, Change, Progress, ScanIntoError, ScanSummary,
    analyze_file, scan_into,
};
pub use relink::{Relink, RelinkError, Relinked, relink};
pub use scan::{AUDIO_EXTENSIONS, ScanError, ScanOptions, Scanned, Unreadable, scan};
