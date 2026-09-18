//! Acceptance tests for analyzing a file into a record and scanning a
//! folder into the index. A coder agent makes these pass without editing them.

use std::fs;
use std::path::{Path, PathBuf};

use dermixen_analysis::{
    AnalysisError, Camelot, CountedPhrases, EdgeAnchors, FixedTempo, KeyAnalysis, KeyAnalyzer,
    Letter, UnknownKey, measure_loudness,
};
use dermixen_core::{Beats, Bpm, Samples, Seconds};
use dermixen_library::{
    Analyzers, Change, Index, MetadataSource, Release, ReleaseDataSource, ScanOptions,
    analyze_file, scan_into,
};
use dermixen_media::{Audio, WavDepth, decode, hash_file, write_wav};
use dermixen_testkit::synth;

/// A key analyzer that fails on every track.
struct NoKey;

impl KeyAnalyzer for NoKey {
    fn name(&self) -> &str {
        "no key"
    }

    fn analyze(&self, _audio: &Audio) -> Result<KeyAnalysis, AnalysisError> {
        Err(AnalysisError::Failed("no key on purpose".to_owned()))
    }
}

fn kicks_file(dir: &Path, relative: &str, bpm: f64) -> PathBuf {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let audio = synth::kicks(Bpm(bpm), Seconds(0.5), Seconds(20.0));
    write_wav(&path, &audio, WavDepth::Int16).unwrap();
    path
}

fn beats_at_130() -> FixedTempo {
    FixedTempo(Bpm(130.0))
}

#[test]
fn a_file_is_analyzed_into_a_complete_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = kicks_file(
        dir.path(),
        "Etnica - Alien Protein [BF001]/01 Etnica - Alien Protein.wav",
        130.0,
    );
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: Some(&UnknownKey),
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let record = analyze_file(&path, &analyzers).unwrap();
    assert_eq!(record.hash, hash_file(&path).unwrap());
    assert_eq!(record.path, path);
    assert_eq!(record.length, Samples(20 * 44_100));
    assert_eq!(record.grid.bpm, Bpm(130.0));
    // Beat zero is the first beat the analyzer announced, which the fixed
    // tempo analyzer puts at the first sample.
    assert_eq!(record.grid.first_beat, Samples::ZERO);
    assert_eq!(record.grid_confidence, 0.0);
    assert_eq!(record.grid_analyzer, "fixed tempo");
    let key = record.key.expect("a key analyzer answered");
    assert_eq!(key.camelot, Camelot::new(8, Letter::B).unwrap());
    assert_eq!(key.analyzer, "unknown key");
    assert_eq!(key.confidence, 0.0);
    // The kicks start half a second in, which is beat one and a fraction at
    // this tempo, so the music begins there and the intro anchor rounds to beat 1.
    assert!(record.extent.begins >= Seconds(0.5).to_samples());
    assert!(record.extent.begins < Seconds(0.51).to_samples());
    assert_eq!(record.anchors.intro, Beats(1.0));
    assert!(record.anchors.outro > record.anchors.intro);
    assert_eq!(record.anchor_analyzer, "edges");
    assert_eq!(record.metadata.artist.as_deref(), Some("Etnica"));
    assert_eq!(record.metadata.title.as_deref(), Some("Alien Protein"));
    assert_eq!(record.metadata.source, MetadataSource::Filename);
}

#[test]
fn a_file_is_analyzed_with_its_loudness() {
    let dir = tempfile::tempdir().unwrap();
    let path = kicks_file(dir.path(), "01 Etnica - Alien Protein.wav", 130.0);
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let record = analyze_file(&path, &analyzers).unwrap();
    let expected = measure_loudness(&decode(&path).unwrap().audio);
    assert!(
        expected.is_some(),
        "twenty seconds of kicks have a loudness"
    );
    assert_eq!(record.loudness, expected);
}

#[test]
fn a_scan_completes_a_record_that_has_no_loudness() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    let path = kicks_file(&root, "01 Etnica - Alien Protein.wav", 130.0);
    kicks_file(&root, "02 Prana - Scarab.wav", 140.0);
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions::default();
    let first = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!((first.added, first.completed), (2, 0));

    // A record stored without a loudness.
    let hash = hash_file(&path).unwrap();
    let mut record = index.get(hash).unwrap().unwrap();
    let measured = record.loudness.take().expect("the scan measured it");
    index.upsert(&record).unwrap();
    assert_eq!(index.get(hash).unwrap().unwrap().loudness, None);

    let mut changes = Vec::new();
    let second = scan_into(&mut index, &root, &options, &analyzers, &mut |progress| {
        changes.push((progress.path.to_path_buf(), progress.change));
        true
    })
    .unwrap();
    assert_eq!(
        (second.added, second.unchanged, second.completed),
        (0, 1, 1)
    );
    assert_eq!(changes[0], (path.clone(), Change::Completed));
    assert_eq!(changes[1].1, Change::Unchanged);
    let completed = index.get(hash).unwrap().unwrap();
    assert_eq!(completed.loudness, Some(measured));
    assert_eq!(
        completed.grid, record.grid,
        "the rest of the record is as it was"
    );
    assert_eq!(completed.phrases, record.phrases);

    let third = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!((third.unchanged, third.completed), (2, 0));
}

#[test]
fn a_key_analyzer_that_fails_leaves_the_key_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = kicks_file(dir.path(), "a.wav", 130.0);
    let beats = beats_at_130();
    let record = analyze_file(
        &path,
        &Analyzers {
            beats: &beats,
            key: Some(&NoKey),
            anchors: &EdgeAnchors,
            phrases: &CountedPhrases,
        },
    )
    .unwrap();
    assert_eq!(record.key, None);
    let record = analyze_file(
        &path,
        &Analyzers {
            beats: &beats,
            key: None,
            anchors: &EdgeAnchors,
            phrases: &CountedPhrases,
        },
    )
    .unwrap();
    assert_eq!(record.key, None);
}

#[test]
fn a_file_that_cannot_be_decoded_is_an_error_that_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.wav");
    fs::write(&path, b"this is not a wave file").unwrap();
    let beats = beats_at_130();
    let problem = analyze_file(
        &path,
        &Analyzers {
            beats: &beats,
            key: None,
            anchors: &EdgeAnchors,
            phrases: &CountedPhrases,
        },
    )
    .unwrap_err();
    assert!(problem.to_string().contains("broken.wav"), "{problem}");
}

/// A folder with two distinct tracks, a copy of one of them, a file that is
/// not audio at all, and a mix in the excluded folder.
fn a_folder(root: &Path) {
    kicks_file(root, "artist/A - One/01 A - Alpha.wav", 130.0);
    kicks_file(root, "artist/B - Two/01 B - Beta.wav", 140.0);
    fs::create_dir_all(root.join("comp")).unwrap();
    fs::copy(
        root.join("artist/A - One/01 A - Alpha.wav"),
        root.join("comp/03 A - Alpha.wav"),
    )
    .unwrap();
    fs::write(root.join("artist/broken.mp3"), b"not an mp3").unwrap();
    kicks_file(root, "mixes/set.wav", 135.0);
}

#[test]
fn a_scan_adds_new_files_reports_duplicates_and_failures_and_skips_exclusions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    a_folder(root);
    let mut index = Index::open(&root.join("library.sqlite")).unwrap();
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions {
        exclude: vec![PathBuf::from("mixes")],
    };
    let mut seen: Vec<(PathBuf, Change, usize, usize)> = Vec::new();
    let summary = scan_into(&mut index, root, &options, &analyzers, &mut |progress| {
        seen.push((
            progress.path.to_path_buf(),
            progress.change,
            progress.done,
            progress.total,
        ));
        true
    })
    .unwrap();

    assert_eq!(summary.added, 2);
    assert_eq!(summary.moved, 0);
    assert_eq!(summary.unchanged, 0);
    assert_eq!(
        summary.duplicates,
        vec![(
            root.join("comp/03 A - Alpha.wav"),
            root.join("artist/A - One/01 A - Alpha.wav")
        )]
    );
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].0, root.join("artist/broken.mp3"));
    assert!(!summary.failed[0].1.is_empty());
    assert!(summary.unreadable.is_empty());

    // Progress came once per file, in path order, with the running count.
    assert_eq!(
        seen,
        vec![
            (
                root.join("artist/A - One/01 A - Alpha.wav"),
                Change::Added,
                1,
                4
            ),
            (
                root.join("artist/B - Two/01 B - Beta.wav"),
                Change::Added,
                2,
                4
            ),
            (root.join("artist/broken.mp3"), Change::Failed, 3, 4),
            (root.join("comp/03 A - Alpha.wav"), Change::Duplicate, 4, 4),
        ]
    );

    assert_eq!(index.len().unwrap(), 2);
    let alpha = index
        .get(hash_file(&root.join("comp/03 A - Alpha.wav")).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(alpha.path, root.join("artist/A - One/01 A - Alpha.wav"));
    assert_eq!(alpha.metadata.artist.as_deref(), Some("A"));
}

#[test]
fn a_second_scan_analyzes_nothing_and_a_moved_file_keeps_its_record() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    a_folder(root);
    fs::remove_file(root.join("comp/03 A - Alpha.wav")).unwrap();
    fs::remove_file(root.join("artist/broken.mp3")).unwrap();
    let mut index = Index::open(&root.join("library.sqlite")).unwrap();
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions {
        exclude: vec![PathBuf::from("mixes")],
    };
    let first = scan_into(&mut index, root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!((first.added, first.unchanged), (2, 0));

    let mut changes = Vec::new();
    let second = scan_into(&mut index, root, &options, &analyzers, &mut |progress| {
        changes.push(progress.change);
        true
    })
    .unwrap();
    assert_eq!((second.added, second.moved, second.unchanged), (0, 0, 2));
    assert_eq!(changes, vec![Change::Unchanged, Change::Unchanged]);

    // Moving a file leaves its bytes, so its record follows it.
    let old = root.join("artist/B - Two/01 B - Beta.wav");
    let new = root.join("artist/B - Two/renamed.wav");
    let hash = hash_file(&old).unwrap();
    fs::rename(&old, &new).unwrap();
    let third = scan_into(&mut index, root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!((third.added, third.moved, third.unchanged), (0, 1, 1));
    let record = index.get(hash).unwrap().unwrap();
    assert_eq!(record.path, new);
    assert_eq!(record.grid.bpm, Bpm(130.0));
    assert_eq!(index.len().unwrap(), 2);
}

#[test]
fn a_scan_leaves_a_corrected_year_and_a_recorded_release_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    let kept = kicks_file(&root, "01 Etnica - Alien Protein.wav", 130.0);
    let completed = kicks_file(&root, "02 Prana - Scarab.wav", 140.0);
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions::default();
    scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();

    // What the Discogs programs write: a corrected year on one record, an
    // estimate on the other, and a release on both. The second record also
    // loses its loudness, so the next scan completes it and must leave the
    // rest alone.
    let release = Release {
        label: Some("Flying Rhino".to_owned()),
        catalog_number: Some("FLYCD002".to_owned()),
        title: Some("Rhinoceros".to_owned()),
        track_number: Some(3),
        data_source: Some(ReleaseDataSource::DiscogsApi),
    };
    let kept_hash = hash_file(&kept).unwrap();
    let mut record = index.get(kept_hash).unwrap().unwrap();
    record.metadata.year = Some(1996);
    record.release = release.clone();
    index.upsert(&record).unwrap();
    let completed_hash = hash_file(&completed).unwrap();
    let mut record = index.get(completed_hash).unwrap().unwrap();
    record.metadata.year = Some(1997);
    record.metadata.year_is_approximate = true;
    record.release = release.clone();
    record.loudness = None;
    index.upsert(&record).unwrap();

    let second = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!(
        (second.added, second.unchanged, second.completed),
        (0, 1, 1)
    );
    let kept_record = index.get(kept_hash).unwrap().unwrap();
    assert_eq!(kept_record.metadata.year, Some(1996));
    assert!(!kept_record.metadata.year_is_approximate);
    assert_eq!(kept_record.release, release);
    let completed_record = index.get(completed_hash).unwrap().unwrap();
    assert_eq!(completed_record.metadata.year, Some(1997));
    assert!(completed_record.metadata.year_is_approximate);
    assert_eq!(completed_record.release, release);
    assert!(
        completed_record.loudness.is_some(),
        "the scan completed the loudness and changed nothing else"
    );

    // A moved file keeps them too, the estimate mark included.
    let moved = root.join("renamed.wav");
    fs::rename(&kept, &moved).unwrap();
    let moved_marked = root.join("renamed-marked.wav");
    fs::rename(&completed, &moved_marked).unwrap();
    let third = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!((third.moved, third.unchanged), (2, 0));
    let moved_record = index.get(kept_hash).unwrap().unwrap();
    assert_eq!(moved_record.path, moved);
    assert_eq!(moved_record.metadata.year, Some(1996));
    assert_eq!(moved_record.release, release);
    let marked_record = index.get(completed_hash).unwrap().unwrap();
    assert_eq!(marked_record.path, moved_marked);
    assert_eq!(marked_record.metadata.year, Some(1997));
    assert!(marked_record.metadata.year_is_approximate);
    assert_eq!(marked_record.release, release);
}

#[test]
fn a_first_scan_records_no_release_and_marks_no_year() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    let path = kicks_file(&root, "01 Etnica - Alien Protein.wav", 130.0);
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions::default();
    let first = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!(first.added, 1);
    // A scan reads no release and estimates no year: the file's tags name
    // the pressing, and only the Discogs programs write either.
    let record = index.get(hash_file(&path).unwrap()).unwrap().unwrap();
    assert_eq!(record.release, Release::default());
    assert!(!record.metadata.year_is_approximate);
    assert_eq!(record.metadata.year, None);
}

#[test]
fn a_progress_answer_of_false_stops_the_scan_after_that_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    kicks_file(&root, "01 Etnica - Alpha.wav", 130.0);
    kicks_file(&root, "02 Prana - Beta.wav", 140.0);
    kicks_file(&root, "03 Hallucinogen - Gamma.wav", 135.0);
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions::default();

    let mut seen = Vec::new();
    let summary = scan_into(&mut index, &root, &options, &analyzers, &mut |progress| {
        seen.push((progress.done, progress.total));
        progress.done < 2
    })
    .unwrap();
    assert_eq!(
        seen,
        vec![(1, 3), (2, 3)],
        "the file that answered false was still reported"
    );
    assert!(summary.stopped, "the summary says the scan stopped");
    assert_eq!(summary.added, 2, "the two files dealt with were stored");
    assert_eq!(index.len().unwrap(), 2, "the third file was not looked at");

    // A false answer on the first file stops the scan after that file.
    let mut seen = Vec::new();
    let summary = scan_into(&mut index, &root, &options, &analyzers, &mut |progress| {
        seen.push(progress.done);
        false
    })
    .unwrap();
    assert_eq!(seen, vec![1], "one file, then the scan stopped");
    assert!(summary.stopped);
    // A false answer on the last file changes nothing: the scan was over.
    let summary = scan_into(&mut index, &root, &options, &analyzers, &mut |progress| {
        progress.done < 3
    })
    .unwrap();
    assert!(
        !summary.stopped,
        "false on the last file is not a stop, since there was nothing after it"
    );
    assert_eq!(summary.unchanged, 2);
    assert_eq!(summary.added, 1);
}

/// A beat analyzer that panics on any track shorter than fifteen seconds, as
/// a decoder or an analyzer with a defect panics on one hostile file.
struct PanicsOnShortTracks;

impl dermixen_analysis::BeatAnalyzer for PanicsOnShortTracks {
    fn name(&self) -> &str {
        "panics on short tracks"
    }

    fn analyze(&self, audio: &Audio) -> Result<dermixen_analysis::BeatAnalysis, AnalysisError> {
        assert!(audio.duration() >= Seconds(15.0), "this track is hostile");
        dermixen_analysis::BeatAnalyzer::analyze(&beats_at_130(), audio)
    }
}

#[test]
fn a_file_that_panics_the_analysis_fails_alone_and_the_scan_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    kicks_file(&root, "01 Etnica - Alien Protein.wav", 130.0);
    let hostile = root.join("02 Hostile - File.wav");
    let short = synth::kicks(Bpm(130.0), Seconds(0.5), Seconds(10.0));
    write_wav(&hostile, &short, WavDepth::Int16).unwrap();
    kicks_file(&root, "03 Prana - Scarab.wav", 140.0);

    let library = dir.path().join("library.sqlite");
    let mut index = Index::open(&library).unwrap();
    let analyzers = Analyzers {
        beats: &PanicsOnShortTracks,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    for run in 0..2 {
        let mut seen = Vec::new();
        let summary = scan_into(
            &mut index,
            &root,
            &ScanOptions::default(),
            &analyzers,
            &mut |progress| {
                seen.push((progress.path.to_path_buf(), progress.change));
                true
            },
        )
        .unwrap();
        assert_eq!(summary.failed.len(), 1, "run {run}: {:?}", summary.failed);
        assert_eq!(summary.failed[0].0, hostile, "run {run}");
        assert!(
            summary.failed[0].1.contains("this track is hostile"),
            "run {run}: {}",
            summary.failed[0].1
        );
        assert_eq!(seen.len(), 3, "run {run}: {seen:?}");
        assert!(
            seen.contains(&(hostile.clone(), Change::Failed)),
            "run {run}"
        );
        let (added, unchanged) = if run == 0 { (2, 0) } else { (0, 2) };
        assert_eq!(
            (summary.added, summary.unchanged),
            (added, unchanged),
            "run {run}"
        );
    }
    assert_eq!(index.len().unwrap(), 2);
}

#[test]
fn a_scan_replaces_a_row_it_cannot_read_and_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("music");
    kicks_file(&root, "01 Etnica - Alien Protein.wav", 130.0);
    let damaged = kicks_file(&root, "02 Prana - Scarab.wav", 140.0);
    let library = dir.path().join("library.sqlite");
    let beats = beats_at_130();
    let analyzers = Analyzers {
        beats: &beats,
        key: None,
        anchors: &EdgeAnchors,
        phrases: &CountedPhrases,
    };
    let options = ScanOptions::default();
    let mut index = Index::open(&library).unwrap();
    let first = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!(first.added, 2);
    drop(index);

    // A row damaged from outside, and a file that arrives after it in the scan's order.
    let connection = rusqlite::Connection::open(&library).unwrap();
    let changed = connection
        .execute(
            "UPDATE tracks SET bpm = 'fast' WHERE path = ?1",
            [damaged.to_str().unwrap()],
        )
        .unwrap();
    assert_eq!(changed, 1);
    drop(connection);
    let later = kicks_file(&root, "03 Slinky Wizard - Lunar Juice.wav", 135.0);

    let mut index = Index::open(&library).unwrap();
    let second = scan_into(&mut index, &root, &options, &analyzers, &mut |_| true).unwrap();
    assert_eq!(
        second.failed,
        [],
        "the damaged row is not a failure of the file"
    );
    assert_eq!((second.added, second.unchanged), (2, 1), "{second:?}");
    let (records, skipped) = index.query_with_skipped(&Default::default()).unwrap();
    assert_eq!((records.len(), skipped), (3, 0));
    assert!(index.get_by_path(&damaged).unwrap().is_some());
    assert!(index.get_by_path(&later).unwrap().is_some());
}
