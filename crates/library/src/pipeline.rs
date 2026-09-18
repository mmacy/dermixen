//! Analyzing a file into a record, and scanning a folder into the library.

use std::path::{Path, PathBuf};

use dermixen_analysis::{
    AnalysisError, AnchorAnalyzer, BeatAnalyzer, KeyAnalyzer, KickAnchors, PhraseAnalysis,
    PhraseAnalyzer, ShiftPhrases, measure_loudness,
};
use dermixen_core::{BeatGrid, Samples};
use dermixen_media::{decode, hash_file};

use crate::index::{Index, IndexError, KeyRecord, PhraseRecord, PhraseStartRecord, TrackRecord};
use crate::metadata::{Release, metadata_of};
use crate::scan::{ScanError, ScanOptions, Unreadable, scan};

/// The analyzers a scan runs on each new file.
#[derive(Clone, Copy)]
pub struct Analyzers<'a> {
    /// Finds the tempo and the beats.
    pub beats: &'a dyn BeatAnalyzer,
    /// Finds the key, when a key analyzer is built in.
    pub key: Option<&'a dyn KeyAnalyzer>,
    /// Finds where the music begins and ends and where the anchors go.
    pub anchors: &'a dyn AnchorAnalyzer,
    /// Finds the bar lines, the phrase starts, and the section changes.
    pub phrases: &'a dyn PhraseAnalyzer,
}

/// The analyzers a scan runs, owned, so that the borrowed [`Analyzers`] a
/// scan takes can point into them.
///
/// The grid and the anchors come from Dermixen's own analyzers, which need
/// no foreign library and are therefore the same in every build. Only the
/// key detector depends on a Cargo feature, so the window and the
/// `dermixen` command both take their analyzers from here rather than each
/// settling the question for itself.
pub struct AnalyzerSet {
    beats: Box<dyn BeatAnalyzer>,
    key: Option<Box<dyn KeyAnalyzer>>,
    anchors: KickAnchors,
    phrases: ShiftPhrases,
}

impl AnalyzerSet {
    /// The built-in analyzers: the `pulse` grid analyzer, the `kick` anchor
    /// analyzer, the `shift` phrase analyzer, and the key detector this
    /// build has, which is libkeyfinder behind the `keyfinder` feature and
    /// none otherwise.
    pub fn built_in() -> AnalyzerSet {
        AnalyzerSet::with_beats(Box::new(dermixen_analysis::PulseGrid))
    }

    /// The built-in analyzers with `beats` finding the grid in place of the
    /// `pulse` analyzer, which is how a tempo a person typed replaces grid
    /// analysis while the anchors, the phrases, and the key are still found.
    pub fn with_beats(beats: Box<dyn BeatAnalyzer>) -> AnalyzerSet {
        AnalyzerSet {
            beats,
            key: built_in_key(),
            anchors: KickAnchors,
            phrases: ShiftPhrases,
        }
    }

    /// These analyzers as a scan takes them.
    pub fn as_analyzers(&self) -> Analyzers<'_> {
        Analyzers {
            beats: self.beats.as_ref(),
            key: self.key.as_deref(),
            anchors: &self.anchors,
            phrases: &self.phrases,
        }
    }
}

/// The key detector this build has, or `None` when none was built in.
#[cfg(feature = "keyfinder")]
fn built_in_key() -> Option<Box<dyn KeyAnalyzer>> {
    Some(Box::new(dermixen_analysis::KeyfinderKey))
}

/// The key detector this build has, which is none, so records hold no key.
#[cfg(not(feature = "keyfinder"))]
fn built_in_key() -> Option<Box<dyn KeyAnalyzer>> {
    None
}

/// The reason a file could not be analyzed.
#[derive(Debug, thiserror::Error)]
pub enum AnalyzeError {
    /// The file could not be decoded.
    #[error("{0}")]
    Decode(String),
    /// The beat analyzer failed.
    #[error("no beat grid: {0}")]
    Beats(AnalysisError),
    /// The anchor analyzer failed.
    #[error("no anchors: {0}")]
    Anchors(AnalysisError),
}

/// Reads one file and builds its record: the hash, the decoded length, the
/// beat grid, the key, the extent and anchors, the metadata, the phrase
/// analysis, and the loudness.
///
/// The beat analyzer is given the decoded audio, the anchor analyzer and
/// the phrase analyzer the audio and the grid the beat analyzer found, with
/// beat zero at the first beat that analyzer announced. A key analyzer that
/// fails leaves the record's key empty rather than failing the file, because
/// a track with no key is still a track that can be mixed, and a phrase
/// analyzer that fails leaves the record's phrases empty for the same
/// reason. The loudness is measured from the same decoded audio, and a
/// track the meter finds nothing in, which is one quieter than the meter's
/// gate, shorter than its block, or silent, leaves the record's loudness
/// empty. The path stored is `path` as given.
pub fn analyze_file(path: &Path, analyzers: &Analyzers<'_>) -> Result<TrackRecord, AnalyzeError> {
    let decoded = decode(path).map_err(|problem| AnalyzeError::Decode(problem.to_string()))?;
    let audio = decoded.audio;

    let beats = analyzers
        .beats
        .analyze(&audio)
        .map_err(AnalyzeError::Beats)?;
    let grid = BeatGrid {
        first_beat: beats.beats.first().copied().unwrap_or(Samples::ZERO),
        bpm: beats.bpm,
    };

    let key = analyzers.key.and_then(|analyzer| {
        let found = analyzer.analyze(&audio).ok()?;
        Some(KeyRecord {
            key: found.key,
            camelot: found.key.camelot(),
            confidence: found.confidence,
            analyzer: analyzer.name().to_owned(),
        })
    });

    let placed = analyzers
        .anchors
        .analyze(&audio, &grid)
        .map_err(AnalyzeError::Anchors)?;

    let phrases = analyzers
        .phrases
        .analyze(&audio, &grid)
        .ok()
        .map(|found| phrase_record(analyzers.phrases.name(), &found));

    Ok(TrackRecord {
        hash: decoded.hash,
        path: path.to_path_buf(),
        length: audio.len(),
        grid,
        grid_confidence: beats.confidence,
        grid_analyzer: analyzers.beats.name().to_owned(),
        key,
        extent: placed.extent,
        anchors: placed.anchors,
        anchor_confidence: placed.confidence,
        anchor_analyzer: analyzers.anchors.name().to_owned(),
        phrases,
        loudness: measure_loudness(&audio),
        metadata: metadata_of(path),
        // A scan reads no release identification. The tags name the pressing
        // this file was ripped from, and a Discogs lookup fills the release
        // afterwards.
        release: Release::default(),
    })
}

/// What one phrase analyzer found, as the record the library stores. The
/// beats are the ones the analyzer answered with, which belong to the grid
/// it was given, so the record describes the record's own grid without
/// moving it.
fn phrase_record(analyzer: &str, found: &PhraseAnalysis) -> PhraseRecord {
    PhraseRecord {
        analyzer: analyzer.to_owned(),
        confidence: found.confidence,
        downbeat: found.downbeat,
        starts: found
            .phrases
            .iter()
            .map(|start| PhraseStartRecord {
                beat: start.at,
                bars: start.bars,
            })
            .collect(),
        sections: found.sections.clone(),
    }
}

/// What a scan did with one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The file was new: it was analyzed and its record stored.
    Added,
    /// The file's bytes were already in the library under a path that no
    /// longer exists, so the record now points at the file being scanned.
    Moved,
    /// The file was already in the library at this path.
    Unchanged,
    /// The file's bytes were already in the library under another path that
    /// still exists. The record keeps that path and this file is not stored.
    Duplicate,
    /// The file could not be analyzed.
    Failed,
    /// The file was already in the library at this path with a record that
    /// had no loudness, so the file was decoded, its loudness measured, and
    /// the record completed.
    Completed,
}

/// Where a scan has got to, reported once per file after the file is dealt with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress<'a> {
    /// How many files have been dealt with, including this one.
    pub done: usize,
    /// How many files the scan found.
    pub total: usize,
    /// The file.
    pub path: &'a Path,
    /// What was done with it.
    pub change: Change,
}

/// What a scan did overall.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    /// How many files were analyzed and stored.
    pub added: usize,
    /// How many records were pointed at a new path.
    pub moved: usize,
    /// How many files were already in the library where they are.
    pub unchanged: usize,
    /// How many records that had no loudness were completed with one.
    pub completed: usize,
    /// Each file whose bytes are already in the library under another path,
    /// with the path the record kept.
    pub duplicates: Vec<(PathBuf, PathBuf)>,
    /// Each file that could not be analyzed, with the reason.
    pub failed: Vec<(PathBuf, String)>,
    /// Each folder the scan could not read.
    pub unreadable: Vec<Unreadable>,
    /// Whether the scan was told to stop before it had dealt with every
    /// file, in which case the files after the one it stopped on were not
    /// looked at.
    pub stopped: bool,
}

/// The reason a scan into the library could not finish.
#[derive(Debug, thiserror::Error)]
pub enum ScanIntoError {
    /// The folder could not be scanned.
    #[error(transparent)]
    Scan(#[from] ScanError),
    /// The library could not be read or written.
    #[error(transparent)]
    Index(#[from] IndexError),
}

/// Scans a folder and brings the library up to date with it.
///
/// Every audio file found is hashed. The scan does no more work for a hash
/// the library already contains: the record is left alone when its path still
/// exists, and pointed at the file when it does not. The one exception is a
/// record at the same path whose loudness is absent: that file is decoded
/// and its loudness measured, the rest of the record is left as it was, and
/// the scan reports [`Change::Completed`]. A new hash means the file is
/// analyzed with `analyzers` and stored, and so does a hash whose row the
/// library cannot read, because analyzing the file again is what replaces
/// such a row. The scan reports [`Change::Added`] for either. A file that
/// cannot be decoded or analyzed is recorded in the summary and the scan goes
/// on, and a file whose decoding or analysis panics is recorded the same way,
/// with the panic's message as the reason. `progress` is
/// called once per file, in the order the files were found, and answers
/// whether the scan goes on: `false` ends the scan after that file, with
/// the summary's `stopped` set and the files after it untouched. Only a
/// folder that cannot be scanned at all, or a library that cannot be read
/// or written, stops the scan otherwise.
pub fn scan_into(
    index: &mut Index,
    root: &Path,
    options: &ScanOptions,
    analyzers: &Analyzers<'_>,
    progress: &mut dyn FnMut(&Progress<'_>) -> bool,
) -> Result<ScanSummary, ScanIntoError> {
    let found = scan(root, options)?;
    let total = found.files.len();
    let mut summary = ScanSummary {
        unreadable: found.unreadable,
        ..ScanSummary::default()
    };
    for (dealt_with, file) in found.files.iter().enumerate() {
        let change = deal_with(index, file, analyzers, &mut summary)?;
        match change {
            Change::Added => summary.added += 1,
            Change::Moved => summary.moved += 1,
            Change::Unchanged => summary.unchanged += 1,
            Change::Completed => summary.completed += 1,
            // Each duplicate and each failure was recorded in full, with
            // its detail, where the scan met it.
            Change::Duplicate | Change::Failed => {}
        }
        let go_on = progress(&Progress {
            done: dealt_with + 1,
            total,
            path: file,
            change,
        });
        if !go_on {
            summary.stopped = dealt_with + 1 < total;
            break;
        }
    }
    Ok(summary)
}

/// Runs work that decodes or analyzes one file, turning a panic in that work
/// into the reason that one file failed.
///
/// A decoder or an analyzer that meets a file it was never written for can
/// panic, and a panic that escapes ends the whole scan and every scan after it
/// at the same file. Catching it here costs the person that one file and
/// leaves the rest of the folder to be scanned.
///
/// The analyzers this call wraps take the audio and answer, and the scan
/// gives the answer straight to the library, so a panic part way through
/// leaves no half-written analysis behind. Nothing the caught work touched is
/// read again, since the file it was working on is recorded as failed and
/// nothing of that file is stored.
///
/// One piece of state does outlive a single file. The key detector builds
/// libkeyfinder's two tone profiles once per process, guarded by a
/// `std::sync::Once`, and a `Once` whose closure panics is poisoned, which
/// makes every later call panic. That closure is a single call into the C++
/// shim, which catches every exception itself and returns, so the closure has
/// nothing to panic with and the `Once` is never poisoned. A panic that would
/// have to unwind through the C++ library never reaches here either, because
/// Rust ends the process at that boundary instead.
///
/// The panic still prints to the error output, as every panic does, so a
/// defect stays visible while the scan goes on.
fn without_panicking<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).map_err(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "the panic gave no message".to_owned());
        format!("reading this file panicked: {message}")
    })
}

/// Brings the library up to date with one file. A duplicate and a failure are
/// the two outcomes with more to say than which they were, so the scan
/// records the detail of each in `summary`.
fn deal_with(
    index: &mut Index,
    file: &Path,
    analyzers: &Analyzers<'_>,
    summary: &mut ScanSummary,
) -> Result<Change, ScanIntoError> {
    let hash = match hash_file(file) {
        Ok(hash) => hash,
        Err(problem) => {
            summary
                .failed
                .push((file.to_path_buf(), problem.to_string()));
            return Ok(Change::Failed);
        }
    };
    // A row the library cannot read is no record for the scan's purpose. The
    // file in front of the scan is the one thing that can mend such a row, so
    // the scan analyzes the file again and stores the answer over the row,
    // which is why this one error becomes `None` rather than ending the scan.
    let stored = match index.get(hash) {
        Ok(found) => found,
        Err(IndexError::Record { .. }) => None,
        Err(problem) => return Err(problem.into()),
    };
    // The hash is the track's identity, so a file the library already contains
    // is never decoded or analyzed again, however it was renamed or moved.
    if let Some(mut record) = stored {
        if record.path == file {
            // A record stored without a loudness gets one here, and
            // measuring it needs the audio. Nothing else about the record
            // is analyzed again, so the file is decoded once and the meter
            // reads it.
            if record.loudness.is_some() {
                return Ok(Change::Unchanged);
            }
            let measured =
                without_panicking(|| decode(file).map(|decoded| measure_loudness(&decoded.audio)));
            let measured = match measured {
                Ok(Ok(measured)) => measured,
                Ok(Err(problem)) => {
                    summary
                        .failed
                        .push((file.to_path_buf(), problem.to_string()));
                    return Ok(Change::Failed);
                }
                Err(panic) => {
                    summary.failed.push((file.to_path_buf(), panic));
                    return Ok(Change::Failed);
                }
            };
            let Some(loudness) = measured else {
                // The meter finds nothing in a track quieter than its gate,
                // shorter than its block, or silent, and there is nothing to
                // store for one, so the record stays as it is. A record can
                // contain no answer of that kind, so every later scan decodes
                // the file again and reports it unchanged.
                return Ok(Change::Unchanged);
            };
            record.loudness = Some(loudness);
            index.upsert(&record)?;
            return Ok(Change::Completed);
        }
        if record.path.exists() {
            summary.duplicates.push((file.to_path_buf(), record.path));
            return Ok(Change::Duplicate);
        }
        index.set_path(hash, file)?;
        return Ok(Change::Moved);
    }
    // The catch goes around the decoding and the analysis alone. A failure to
    // read or write the library is the caller's to hear about, so the library
    // calls stay outside it.
    match without_panicking(|| analyze_file(file, analyzers)) {
        Ok(Ok(record)) => {
            index.upsert(&record)?;
            Ok(Change::Added)
        }
        Ok(Err(problem)) => {
            summary
                .failed
                .push((file.to_path_buf(), problem.to_string()));
            Ok(Change::Failed)
        }
        Err(panic) => {
            summary.failed.push((file.to_path_buf(), panic));
            Ok(Change::Failed)
        }
    }
}
