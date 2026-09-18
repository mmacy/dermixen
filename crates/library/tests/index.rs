//! Acceptance tests for the SQLite index. A coder agent makes these pass without editing them.

use std::path::{Path, PathBuf};

use dermixen_analysis::{Camelot, Extent, Key, Letter, Loudness, Mode, PitchClass};
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Lufs, Samples};
use dermixen_library::{
    Index, IndexError, KeyRecord, Metadata, MetadataSource, Query, Release, ReleaseDataSource,
    SCHEMA_VERSION, TrackRecord,
};

fn hash(byte: u8) -> ContentHash {
    ContentHash([byte; 32])
}

fn key(tonic: PitchClass, mode: Mode) -> KeyRecord {
    let key = Key { tonic, mode };
    KeyRecord {
        key,
        camelot: key.camelot(),
        confidence: 0.8,
        analyzer: "keyfinder".to_owned(),
    }
}

/// A complete record with every field set to something a test can check.
fn record(
    byte: u8,
    path: &str,
    bpm: f64,
    year: Option<u16>,
    key: Option<KeyRecord>,
) -> TrackRecord {
    TrackRecord {
        hash: hash(byte),
        path: PathBuf::from(path),
        length: Samples(18_522_000),
        grid: BeatGrid {
            first_beat: Samples(4_410),
            bpm: Bpm(bpm),
        },
        grid_confidence: 0.91,
        grid_analyzer: "aubio".to_owned(),
        key,
        extent: Extent {
            begins: Samples(4_410),
            ends: Samples(18_000_000),
        },
        anchors: Anchors {
            intro: Beats(64.0),
            outro: Beats(896.0),
        },
        anchor_confidence: 0.5,
        anchor_analyzer: "edges".to_owned(),
        phrases: None,
        loudness: None,
        metadata: Metadata {
            artist: Some("Slinky Wizard".to_owned()),
            title: Some(
                Path::new(path)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            ),
            year,
            year_is_approximate: false,
            source: MetadataSource::Tags,
        },
        release: Release {
            label: Some("Flying Rhino".to_owned()),
            catalog_number: Some("FLYCD002".to_owned()),
            title: Some("Rhinoceros".to_owned()),
            track_number: Some(3),
            data_source: Some(ReleaseDataSource::DiscogsExport),
        },
    }
}

fn a_minor() -> KeyRecord {
    key(PitchClass::A, Mode::Minor)
}

#[test]
fn a_record_round_trips_through_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    let mut index = Index::open(&path).unwrap();
    assert_eq!(index.path(), path);
    assert_eq!(index.len().unwrap(), 0);
    assert!(index.is_empty().unwrap());

    let with_key = record(
        1,
        "/music/a/01 Lunar Juice.mp3",
        138.0,
        Some(1996),
        Some(a_minor()),
    );
    let without = record(2, "/music/b/02 Boundless.wav", 140.5, None, None);
    index.upsert(&with_key).unwrap();
    index.upsert(&without).unwrap();
    assert_eq!(index.len().unwrap(), 2);

    assert_eq!(index.get(hash(1)).unwrap(), Some(with_key.clone()));
    assert_eq!(index.get(hash(2)).unwrap(), Some(without.clone()));
    assert_eq!(index.get(hash(3)).unwrap(), None);
    assert_eq!(
        index
            .get_by_path(Path::new("/music/b/02 Boundless.wav"))
            .unwrap(),
        Some(without)
    );
    assert_eq!(
        index.get_by_path(Path::new("/music/nowhere.wav")).unwrap(),
        None
    );

    // A filename-sourced record with no artist survives the round trip too.
    let mut guessed = record(3, "/music/c/03 Scarab.flac", 141.0, None, None);
    guessed.metadata = Metadata {
        artist: None,
        title: Some("Scarab".to_owned()),
        year: None,
        year_is_approximate: false,
        source: MetadataSource::Filename,
    };
    index.upsert(&guessed).unwrap();
    assert_eq!(index.get(hash(3)).unwrap(), Some(guessed));
}

#[test]
fn the_index_persists_across_openings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    {
        let mut index = Index::open(&path).unwrap();
        index
            .upsert(&record(1, "/music/a.mp3", 138.0, None, Some(a_minor())))
            .unwrap();
    }
    let index = Index::open(&path).unwrap();
    assert_eq!(index.len().unwrap(), 1);
    let found = index.get(hash(1)).unwrap().unwrap();
    assert_eq!(
        found.key.unwrap().camelot,
        Camelot::new(8, Letter::A).unwrap()
    );
}

#[test]
fn storing_a_record_again_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    index
        .upsert(&record(1, "/music/a.mp3", 138.0, None, None))
        .unwrap();
    let updated = record(1, "/music/moved/a.mp3", 139.0, Some(1997), Some(a_minor()));
    index.upsert(&updated).unwrap();
    assert_eq!(index.len().unwrap(), 1);
    assert_eq!(index.get(hash(1)).unwrap(), Some(updated));
    assert_eq!(index.get_by_path(Path::new("/music/a.mp3")).unwrap(), None);
}

#[test]
fn a_record_can_be_pointed_at_a_new_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    index
        .upsert(&record(1, "/music/a.mp3", 138.0, None, None))
        .unwrap();
    assert!(
        index
            .set_path(hash(1), Path::new("/music/moved/a.mp3"))
            .unwrap()
    );
    assert!(
        !index
            .set_path(hash(9), Path::new("/music/nowhere.mp3"))
            .unwrap()
    );
    let found = index.get(hash(1)).unwrap().unwrap();
    assert_eq!(found.path, PathBuf::from("/music/moved/a.mp3"));
    // Everything else about the record is as it was.
    assert_eq!(found.grid.bpm, Bpm(138.0));
    assert_eq!(index.get_by_path(Path::new("/music/a.mp3")).unwrap(), None);
}

#[test]
fn an_index_of_another_version_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    // The version lives in SQLite's own user version field.
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.pragma_update(None, "user_version", 999).unwrap();
    drop(connection);
    match Index::open(&path) {
        Err(IndexError::Schema {
            path: named,
            found,
            expected,
        }) => {
            assert_eq!(named, path);
            assert_eq!(found, 999);
            assert_eq!(expected, SCHEMA_VERSION);
        }
        other => panic!("expected a schema error, got {other:?}"),
    }
    let message = Index::open(&path).unwrap_err().to_string();
    assert!(
        message.contains("999") && message.contains("scan again"),
        "{message}"
    );
}

/// The version SQLite's own user version field holds for the file.
fn version_of(path: &Path) -> u32 {
    let connection = rusqlite::Connection::open(path).unwrap();
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

#[test]
fn the_layout_is_version_five_and_a_record_keeps_its_loudness_and_its_release() {
    assert_eq!(SCHEMA_VERSION, 5);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    let mut index = Index::open(&path).unwrap();
    assert_eq!(version_of(&path), 5);

    let mut measured = record(1, "/music/a.wav", 138.0, Some(1996), Some(a_minor()));
    measured.loudness = Some(Loudness {
        integrated: Lufs(-12.3),
        true_peak: Decibels(-0.4),
    });
    let unmeasured = record(2, "/music/b.wav", 140.0, None, None);
    index.upsert(&measured).unwrap();
    index.upsert(&unmeasured).unwrap();
    assert_eq!(index.get(measured.hash).unwrap(), Some(measured.clone()));
    assert_eq!(
        index.get(unmeasured.hash).unwrap(),
        Some(unmeasured.clone())
    );
    drop(index);
    let index = Index::open(&path).unwrap();
    assert_eq!(index.get(measured.hash).unwrap(), Some(measured));
    assert_eq!(index.get(unmeasured.hash).unwrap(), Some(unmeasured));
}

#[test]
fn a_year_marked_as_an_estimate_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    let mut index = Index::open(&path).unwrap();
    let mut estimated = record(1, "/music/a.wav", 138.0, Some(1996), None);
    estimated.metadata.year_is_approximate = true;
    let stated = record(2, "/music/b.wav", 140.0, Some(1997), None);
    index.upsert(&estimated).unwrap();
    index.upsert(&stated).unwrap();
    assert_eq!(index.get(estimated.hash).unwrap(), Some(estimated.clone()));
    assert_eq!(index.get(stated.hash).unwrap(), Some(stated.clone()));
    drop(index);
    // The mark survives the file being closed and opened, which is what a
    // later query relies on.
    let index = Index::open(&path).unwrap();
    assert!(
        index
            .get(estimated.hash)
            .unwrap()
            .unwrap()
            .metadata
            .year_is_approximate
    );
    assert!(
        !index
            .get(stated.hash)
            .unwrap()
            .unwrap()
            .metadata
            .year_is_approximate
    );
}

#[test]
fn a_record_with_no_release_recorded_round_trips_as_an_empty_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    let mut index = Index::open(&path).unwrap();
    let mut scanned = record(1, "/music/a.wav", 138.0, Some(1996), None);
    scanned.release = Release::default();
    index.upsert(&scanned).unwrap();
    assert_eq!(index.get(scanned.hash).unwrap(), Some(scanned));
}

#[test]
fn a_release_data_source_the_library_never_writes_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.sqlite");
    let mut index = Index::open(&path).unwrap();
    let stored = record(1, "/music/a.wav", 138.0, Some(1996), None);
    index.upsert(&stored).unwrap();
    drop(index);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("UPDATE tracks SET release_data_source = 'guessed'", [])
        .unwrap();
    drop(connection);
    let index = Index::open(&path).unwrap();
    assert!(index.get(stored.hash).is_err());
}

#[test]
fn an_index_of_a_lower_version_is_refused_by_name() {
    for lower in 1..SCHEMA_VERSION {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.sqlite");
        // A laid-out library file with its version field written back to a
        // lower number, which is what a library file of that version looks
        // like to the crate: the version field says which layout to expect.
        drop(Index::open(&path).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", lower)
            .unwrap();
        drop(connection);
        match Index::open(&path) {
            Err(IndexError::Schema {
                path: named,
                found,
                expected,
            }) => {
                assert_eq!(named, path, "version {lower}");
                assert_eq!(found, lower, "version {lower}");
                assert_eq!(expected, SCHEMA_VERSION, "version {lower}");
            }
            other => panic!("expected a schema error for version {lower}, got {other:?}"),
        }
        let message = Index::open(&path).unwrap_err().to_string();
        assert!(
            message.contains(&lower.to_string()) && message.contains("scan again"),
            "version {lower}: {message}"
        );
        // The file is left as it was, at the version it named.
        assert_eq!(version_of(&path), lower, "version {lower}");
    }
}

#[test]
fn a_folder_that_cannot_be_written_is_an_open_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("no/such/folder/library.sqlite");
    match Index::open(&path) {
        Err(IndexError::Open { path: named, .. }) => assert_eq!(named, path),
        other => panic!("expected an open error, got {other:?}"),
    }
}

/// Five tracks that differ in every way a query can ask about.
fn a_library() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    let records = [
        record(
            1,
            "/music/comp/VA - Goa Vibes/01 Lunar Juice.mp3",
            138.0,
            Some(1996),
            Some(a_minor()),
        ),
        record(
            2,
            "/music/comp/VA - Goa Vibes/02 Boundless.mp3",
            140.0,
            Some(1996),
            Some(key(PitchClass::E, Mode::Minor)),
        ),
        record(
            3,
            "/music/artist/Etnica/03 Ashtronauts.wav",
            142.5,
            Some(1997),
            Some(key(PitchClass::C, Mode::Major)),
        ),
        record(4, "/music/artist/Etnica/04 Vimana.wav", 145.0, None, None),
        record(
            5,
            "/music/artist/Prana/05 Scarab.flac",
            138.0,
            Some(1994),
            Some(key(PitchClass::F, Mode::Minor)),
        ),
    ];
    for mut record in records {
        if record.hash == hash(4) {
            record.metadata.artist = Some("Etnica".to_owned());
        }
        index.upsert(&record).unwrap();
    }
    (dir, index)
}

fn titles(records: &[TrackRecord]) -> Vec<String> {
    records
        .iter()
        .map(|record| record.metadata.title.clone().unwrap())
        .collect()
}

#[test]
fn a_query_with_no_conditions_lists_everything_in_path_order() {
    let (_dir, index) = a_library();
    let all = index.query(&Query::default()).unwrap();
    assert_eq!(
        titles(&all),
        vec![
            "03 Ashtronauts",
            "04 Vimana",
            "05 Scarab",
            "01 Lunar Juice",
            "02 Boundless"
        ]
    );
}

#[test]
fn each_condition_narrows_the_query() {
    let (_dir, index) = a_library();
    let query = |query: Query| titles(&index.query(&query).unwrap());

    assert_eq!(
        query(Query {
            under: Some(PathBuf::from("/music/artist")),
            ..Query::default()
        }),
        vec!["03 Ashtronauts", "04 Vimana", "05 Scarab"]
    );
    // A folder condition matches whole path components, not text prefixes.
    assert_eq!(
        query(Query {
            under: Some(PathBuf::from("/music/artist/Et")),
            ..Query::default()
        }),
        Vec::<String>::new()
    );
    assert_eq!(
        query(Query {
            bpm: Some((Bpm(138.0), Bpm(140.0))),
            ..Query::default()
        }),
        vec!["05 Scarab", "01 Lunar Juice", "02 Boundless"]
    );
    assert_eq!(
        query(Query {
            year: Some((1996, 1997)),
            ..Query::default()
        }),
        vec!["03 Ashtronauts", "01 Lunar Juice", "02 Boundless"]
    );
    assert_eq!(
        query(Query {
            key: Some(Camelot::new(8, Letter::A).unwrap()),
            ..Query::default()
        }),
        vec!["01 Lunar Juice"]
    );
    // Compatible with A minor: A minor itself, E minor and D minor beside
    // it, and C major across from it. F minor is not, and no key is not.
    assert_eq!(
        query(Query {
            compatible_with: Some(Camelot::new(8, Letter::A).unwrap()),
            ..Query::default()
        }),
        vec!["03 Ashtronauts", "01 Lunar Juice", "02 Boundless"]
    );
    assert_eq!(
        query(Query {
            artist: Some("etnica".to_owned()),
            ..Query::default()
        }),
        vec!["04 Vimana"]
    );
    assert_eq!(
        query(Query {
            title: Some("BOUND".to_owned()),
            ..Query::default()
        }),
        vec!["02 Boundless"]
    );
}

#[test]
fn conditions_combine_and_an_impossible_query_is_empty() {
    let (_dir, index) = a_library();
    let found = index
        .query(&Query {
            under: Some(PathBuf::from("/music/comp")),
            bpm: Some((Bpm(139.0), Bpm(150.0))),
            year: Some((1990, 1999)),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(titles(&found), vec!["02 Boundless"]);
    let found = index
        .query(&Query {
            under: Some(PathBuf::from("/music/comp")),
            artist: Some("prana".to_owned()),
            ..Query::default()
        })
        .unwrap();
    assert!(found.is_empty());
}

#[test]
fn a_length_condition_narrows_the_query_by_decoded_length() {
    use dermixen_core::Seconds;
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    for (byte, path, minutes) in [
        (1, "/music/a/01 Short.mp3", 5.0),
        (2, "/music/a/02 Seven.mp3", 7.0),
        (3, "/music/a/03 Eight and a half.mp3", 8.5),
        (4, "/music/a/04 Nine.mp3", 9.0),
        (5, "/music/a/05 Long.mp3", 12.0),
    ] {
        let mut record = record(byte, path, 140.0, None, None);
        record.length = Seconds(minutes * 60.0).to_samples();
        index.upsert(&record).unwrap();
    }
    let query = |low: f64, high: f64| {
        titles(
            &index
                .query(&Query {
                    length: Some((Seconds(low), Seconds(high))),
                    ..Query::default()
                })
                .unwrap(),
        )
    };
    // Both ends are included: seven minutes and nine minutes exactly both match.
    assert_eq!(
        query(420.0, 540.0),
        vec!["02 Seven", "03 Eight and a half", "04 Nine"]
    );
    assert_eq!(query(0.0, 300.0), vec!["01 Short"]);
    assert_eq!(query(541.0, 600.0), Vec::<String>::new());
    // A length condition combines with the others.
    let found = index
        .query(&Query {
            length: Some((Seconds(420.0), Seconds(540.0))),
            title: Some("nine".to_owned()),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(titles(&found), vec!["04 Nine"]);
}

#[test]
fn a_query_can_leave_estimated_years_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    let stated = record(1, "/music/01 Stated.mp3", 138.0, Some(1996), None);
    let mut estimated = record(2, "/music/02 Estimated.mp3", 140.0, Some(1996), None);
    estimated.metadata.year_is_approximate = true;
    let undated = record(3, "/music/03 Undated.mp3", 142.0, None, None);
    // The layout lets the mark be set on a record with no year, which no
    // program writes. The mark is what the condition reads.
    let mut marked_undated = record(4, "/music/04 Marked.mp3", 144.0, None, None);
    marked_undated.metadata.year_is_approximate = true;
    for record in [&stated, &estimated, &undated, &marked_undated] {
        index.upsert(record).unwrap();
    }
    let query = |query: Query| titles(&index.query(&query).unwrap());

    // Without the condition an estimate counts as the year it estimates.
    assert_eq!(
        query(Query {
            year: Some((1996, 1996)),
            ..Query::default()
        }),
        vec!["01 Stated", "02 Estimated"]
    );
    // With it, only a year some source states is a year.
    assert_eq!(
        query(Query {
            year: Some((1996, 1996)),
            exclude_approximate_years: true,
            ..Query::default()
        }),
        vec!["01 Stated"]
    );
    // On its own the condition leaves out every marked track and nothing
    // else: a track with no year and no mark has no estimate to leave out.
    assert_eq!(
        query(Query {
            exclude_approximate_years: true,
            ..Query::default()
        }),
        vec!["01 Stated", "03 Undated"]
    );
    assert_eq!(
        query(Query::default()),
        vec!["01 Stated", "02 Estimated", "03 Undated", "04 Marked"]
    );
}

#[test]
fn a_query_can_require_a_least_grid_and_anchor_confidence() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = Index::open(&dir.path().join("library.sqlite")).unwrap();
    // Three records whose two confidences pull apart, so a condition on one
    // is told from a condition on the other.
    let mut sure_grid = record(1, "/music/01 Sure grid.mp3", 138.0, Some(1996), None);
    sure_grid.grid_confidence = 0.9;
    sure_grid.anchor_confidence = 0.3;
    let mut on_the_line = record(2, "/music/02 On the line.mp3", 140.0, Some(1996), None);
    on_the_line.grid_confidence = 0.5;
    on_the_line.anchor_confidence = 0.5;
    let mut sure_anchors = record(3, "/music/03 Sure anchors.mp3", 142.0, Some(1996), None);
    sure_anchors.grid_confidence = 0.2;
    sure_anchors.anchor_confidence = 0.9;
    for record in [&sure_grid, &on_the_line, &sure_anchors] {
        index.upsert(record).unwrap();
    }
    let query = |query: Query| titles(&index.query(&query).unwrap());

    // The bound is included: a track scoring exactly it matches.
    assert_eq!(
        query(Query {
            min_grid_confidence: Some(0.5),
            ..Query::default()
        }),
        vec!["01 Sure grid", "02 On the line"]
    );
    assert_eq!(
        query(Query {
            min_anchor_confidence: Some(0.5),
            ..Query::default()
        }),
        vec!["02 On the line", "03 Sure anchors"]
    );
    // Both conditions together, like every other pair of conditions.
    assert_eq!(
        query(Query {
            min_grid_confidence: Some(0.5),
            min_anchor_confidence: Some(0.5),
            ..Query::default()
        }),
        vec!["02 On the line"]
    );
    assert_eq!(
        query(Query {
            min_grid_confidence: Some(0.95),
            ..Query::default()
        }),
        Vec::<String>::new()
    );
    // A bound of zero leaves everything in.
    assert_eq!(
        query(Query {
            min_grid_confidence: Some(0.0),
            min_anchor_confidence: Some(0.0),
            ..Query::default()
        }),
        vec!["01 Sure grid", "02 On the line", "03 Sure anchors"]
    );
}
