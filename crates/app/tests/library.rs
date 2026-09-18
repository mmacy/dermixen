//! Acceptance tests for the library panel: the rows, the sort, the filters,
//! the search, the harmonic highlighting, and the edit that adds a track to
//! the mix. A coder agent makes these pass without editing them.

use std::path::PathBuf;

use dermixen_analysis::{Camelot, Extent, Loudness};
use dermixen_app::{Column, Filters, LibraryPanel, Sort, Timeline};
use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Edit, Envelope, EqEnvelopes, Lufs, Mix,
    Preset, Samples, Seconds, Track, beatmix,
};
use dermixen_library::{
    KeyRecord, Metadata, MetadataSource, Release, ReleaseDataSource, TrackRecord,
};

fn camelot(code: &str) -> Camelot {
    code.parse().unwrap()
}

/// A record for a five-minute track at 140 beats per minute.
fn record(
    byte: u8,
    file: &str,
    artist: Option<&str>,
    title: Option<&str>,
    source: MetadataSource,
    code: Option<&str>,
) -> TrackRecord {
    TrackRecord {
        hash: ContentHash([byte; 32]),
        path: PathBuf::from(format!("/music/{file}")),
        length: Samples(300 * 44_100),
        grid: BeatGrid {
            first_beat: Samples(441),
            bpm: Bpm(140.0),
        },
        grid_confidence: 0.9,
        grid_analyzer: "pulse".to_owned(),
        key: code.map(|code| {
            let camelot = camelot(code);
            KeyRecord {
                key: camelot.key(),
                camelot,
                confidence: 0.8,
                analyzer: "edm".to_owned(),
            }
        }),
        extent: Extent {
            begins: Samples(44_100),
            ends: Samples(290 * 44_100),
        },
        anchors: Anchors {
            intro: Beats(32.0),
            outro: Beats(512.0),
        },
        anchor_confidence: 0.7,
        anchor_analyzer: "kick".to_owned(),
        metadata: Metadata {
            artist: artist.map(str::to_owned),
            title: title.map(str::to_owned),
            year: Some(1996),
            year_is_approximate: false,
            source,
        },
        phrases: None,
        loudness: None,
        release: Release::default(),
    }
}

fn records() -> Vec<TrackRecord> {
    vec![
        record(
            1,
            "lsd.mp3",
            Some("Hallucinogen"),
            Some("LSD"),
            MetadataSource::Tags,
            Some("8A"),
        ),
        record(
            2,
            "mahadeva.mp3",
            Some("astral projection"),
            Some("Mahadeva"),
            MetadataSource::Tags,
            Some("8B"),
        ),
        record(
            3,
            "untitled.mp3",
            None,
            Some("Untitled"),
            MetadataSource::Filename,
            None,
        ),
        record(
            4,
            "alpha.mp3",
            Some("Etnica"),
            Some("Alpha"),
            MetadataSource::Filename,
            Some("9A"),
        ),
        record(
            5,
            "vamp.mp3",
            Some("Etnica"),
            Some("Vimana"),
            MetadataSource::Tags,
            Some("3B"),
        ),
    ]
}

/// A release as a Discogs lookup records one.
fn release(label: &str, catalog_number: &str, title: &str, track_number: u16) -> Release {
    Release {
        label: Some(label.to_owned()),
        catalog_number: Some(catalog_number.to_owned()),
        title: Some(title.to_owned()),
        track_number: Some(track_number),
        data_source: Some(ReleaseDataSource::DiscogsExport),
    }
}

/// Six records whose fields differ in every column: the five of
/// [`records`] with years, tempos, lengths, and releases of their own, and
/// one more in a folder below the others.
///
/// | Title | Artist | Year | BPM | Key | Label | Catalog no. | Release | Track no. | Length |
/// | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
/// | LSD | Hallucinogen | 1995 | 145.0 | 8A | Dragonfly | BFLCD 08 | Twisted | 3 | 5:00 |
/// | Mahadeva | astral projection | 1996 | 140.0 | 8B | Trust In Trance | TIT CD 001 | Trust In Trance 2 | 1 | 7:00 |
/// | Untitled | none | none | 138.0 | none | none | none | none | none | 4:10 |
/// | Alpha | Etnica | about 1996 | 142.5 | 9A | Blue Room Released | BR 013 | Alien Protein | 12 | 8:00 |
/// | Vimana | Etnica | 1997 | 145.0 | 3B | Blue Room Released | BR 020 | Alien Protein | 2 | 5:00 |
/// | Lunar Cycle | Man With No Name | 1996 | 139.0 | 12A | Perfecto Fluoro | PERF 111 | Moment Of Truth | 5 | 6:00 |
///
/// Lunar Cycle's file is `/music/goa/lunar.mp3`, and every other file is
/// directly in `/music`.
fn catalog() -> Vec<TrackRecord> {
    let mut lsd = record(
        1,
        "lsd.mp3",
        Some("Hallucinogen"),
        Some("LSD"),
        MetadataSource::Tags,
        Some("8A"),
    );
    lsd.metadata.year = Some(1995);
    lsd.grid.bpm = Bpm(145.0);
    lsd.release = release("Dragonfly", "BFLCD 08", "Twisted", 3);

    let mut mahadeva = record(
        2,
        "mahadeva.mp3",
        Some("astral projection"),
        Some("Mahadeva"),
        MetadataSource::Tags,
        Some("8B"),
    );
    mahadeva.length = Samples(420 * 44_100);
    mahadeva.release = release("Trust In Trance", "TIT CD 001", "Trust In Trance 2", 1);

    let mut untitled = record(
        3,
        "untitled.mp3",
        None,
        Some("Untitled"),
        MetadataSource::Filename,
        None,
    );
    untitled.metadata.year = None;
    untitled.grid.bpm = Bpm(138.0);
    untitled.length = Samples(250 * 44_100);

    let mut alpha = record(
        4,
        "alpha.mp3",
        Some("Etnica"),
        Some("Alpha"),
        MetadataSource::Filename,
        Some("9A"),
    );
    alpha.metadata.year_is_approximate = true;
    alpha.grid.bpm = Bpm(142.5);
    alpha.length = Samples(480 * 44_100);
    alpha.release = release("Blue Room Released", "BR 013", "Alien Protein", 12);

    let mut vimana = record(
        5,
        "vamp.mp3",
        Some("Etnica"),
        Some("Vimana"),
        MetadataSource::Tags,
        Some("3B"),
    );
    vimana.metadata.year = Some(1997);
    vimana.grid.bpm = Bpm(145.0);
    vimana.release = release("Blue Room Released", "BR 020", "Alien Protein", 2);

    let mut lunar = record(
        6,
        "goa/lunar.mp3",
        Some("Man With No Name"),
        Some("Lunar Cycle"),
        MetadataSource::Tags,
        Some("12A"),
    );
    lunar.grid.bpm = Bpm(139.0);
    lunar.length = Samples(360 * 44_100);
    lunar.release = release("Perfecto Fluoro", "PERF 111", "Moment Of Truth", 5);

    vec![lsd, mahadeva, untitled, alpha, vimana, lunar]
}

/// The row among those shown whose title is `title`.
fn row_of(panel: &LibraryPanel, title: &str) -> usize {
    panel
        .rows()
        .iter()
        .position(|row| row.title.as_deref() == Some(title))
        .unwrap_or_else(|| panic!("{title} is not among the rows shown"))
}

/// The compatible titles in alphabetical order, which is what a test that
/// does not care about the sort compares against.
fn sorted_compatible_titles(panel: &LibraryPanel) -> Vec<String> {
    let mut titles = compatible_titles(panel);
    titles.sort();
    titles
}

fn titles(panel: &LibraryPanel) -> Vec<String> {
    panel
        .rows()
        .iter()
        .map(|row| row.title.clone().unwrap_or_default())
        .collect()
}

fn compatible_titles(panel: &LibraryPanel) -> Vec<String> {
    panel
        .rows()
        .iter()
        .filter(|row| row.compatible)
        .map(|row| row.title.clone().unwrap_or_default())
        .collect()
}

fn track(name: &str, byte: u8, seconds: i64, intro: f64, outro: f64) -> Track {
    Track {
        path: PathBuf::from(name),
        hash: ContentHash([byte; 32]),
        length: Samples(seconds * 44_100),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(120.0),
        },
        anchors: Anchors {
            intro: Beats(intro),
            outro: Beats(outro),
        },
        keylock: true,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    }
}

fn two_tracks() -> Mix {
    let mut a = track("a", 1, 200, 16.0, 256.0);
    let mut b = track("b", 20, 300, 32.0, 512.0);
    beatmix(&mut a, &mut b, 8);
    Mix { tracks: vec![a, b] }
}

#[test]
fn rows_are_every_track_with_guessed_metadata_flagged() {
    let panel = LibraryPanel::new(records());
    assert_eq!(panel.search(), "");
    assert_eq!(panel.rows().len(), 5);
    let rows = panel.rows();
    let untitled = &rows[row_of(&panel, "Untitled")];
    assert_eq!(untitled.artist, None);
    assert!(untitled.guessed);
    assert_eq!(untitled.camelot, None);
    assert!(
        rows[row_of(&panel, "Alpha")].guessed,
        "Alpha's tags were guessed from its file name"
    );
    assert!(!rows[row_of(&panel, "Mahadeva")].guessed);
    let lsd = &rows[row_of(&panel, "LSD")];
    assert_eq!(lsd.hash, ContentHash([1; 32]));
    assert_eq!(lsd.path, PathBuf::from("/music/lsd.mp3"));
    assert_eq!(lsd.bpm, Bpm(140.0));
    assert_eq!(lsd.length, Seconds(300.0));
    assert_eq!(lsd.year, Some(1996));
    assert_eq!(lsd.camelot, Some(camelot("8A")));
    assert!(
        rows.iter()
            .all(|row| !row.compatible && !row.in_mix && !row.selected)
    );
    assert_eq!(panel.selected(), None);
    assert_eq!(panel.reference(), None);
}

#[test]
fn rows_start_sorted_by_artist_with_a_missing_artist_last() {
    let panel = LibraryPanel::new(catalog());
    assert_eq!(
        panel.sort(),
        Sort {
            column: Column::Artist,
            descending: false,
        }
    );
    assert_eq!(panel.filters(), &Filters::default());
    assert_eq!(
        titles(&panel),
        vec![
            "Mahadeva",
            "Alpha",
            "Vimana",
            "LSD",
            "Lunar Cycle",
            "Untitled"
        ],
        "artists without regard to case, titles within an artist, and no artist last"
    );
}

#[test]
fn columns_have_headings_in_order() {
    assert_eq!(
        Column::ALL,
        [
            Column::Artist,
            Column::Title,
            Column::Year,
            Column::Bpm,
            Column::Key,
            Column::Label,
            Column::CatalogNumber,
            Column::Release,
            Column::TrackNumber,
            Column::Duration,
        ]
    );
    let headings: Vec<&str> = Column::ALL.iter().map(|column| column.heading()).collect();
    assert_eq!(
        headings,
        vec![
            "Artist",
            "Title",
            "Year",
            "BPM",
            "Key",
            "Label",
            "Catalog no.",
            "Release",
            "Track no.",
            "Duration",
        ]
    );
}

#[test]
fn clicking_a_heading_sorts_by_that_column_and_a_second_click_reverses_it() {
    let mut panel = LibraryPanel::new(catalog());
    // Each column's ascending order, then its descending order. A row with
    // no value in the column is last both ways. Rows that tie keep the
    // artist order ascending both ways: Etnica's Alpha and Vimana, and
    // Vimana (Etnica) before LSD (Hallucinogen) where the two tie on tempo
    // and on length.
    let orders: [(Column, [&str; 6], [&str; 6]); 10] = [
        (
            Column::Artist,
            [
                "Mahadeva",
                "Alpha",
                "Vimana",
                "LSD",
                "Lunar Cycle",
                "Untitled",
            ],
            [
                "Lunar Cycle",
                "LSD",
                "Alpha",
                "Vimana",
                "Mahadeva",
                "Untitled",
            ],
        ),
        (
            Column::Title,
            [
                "Alpha",
                "LSD",
                "Lunar Cycle",
                "Mahadeva",
                "Untitled",
                "Vimana",
            ],
            [
                "Vimana",
                "Untitled",
                "Mahadeva",
                "Lunar Cycle",
                "LSD",
                "Alpha",
            ],
        ),
        (
            Column::Year,
            [
                "LSD",
                "Mahadeva",
                "Alpha",
                "Lunar Cycle",
                "Vimana",
                "Untitled",
            ],
            [
                "Vimana",
                "Mahadeva",
                "Alpha",
                "Lunar Cycle",
                "LSD",
                "Untitled",
            ],
        ),
        (
            Column::Bpm,
            [
                "Untitled",
                "Lunar Cycle",
                "Mahadeva",
                "Alpha",
                "Vimana",
                "LSD",
            ],
            [
                "Vimana",
                "LSD",
                "Alpha",
                "Mahadeva",
                "Lunar Cycle",
                "Untitled",
            ],
        ),
        (
            Column::Key,
            [
                "Vimana",
                "LSD",
                "Mahadeva",
                "Alpha",
                "Lunar Cycle",
                "Untitled",
            ],
            [
                "Lunar Cycle",
                "Alpha",
                "Mahadeva",
                "LSD",
                "Vimana",
                "Untitled",
            ],
        ),
        (
            Column::Label,
            [
                "Alpha",
                "Vimana",
                "LSD",
                "Lunar Cycle",
                "Mahadeva",
                "Untitled",
            ],
            [
                "Mahadeva",
                "Lunar Cycle",
                "LSD",
                "Alpha",
                "Vimana",
                "Untitled",
            ],
        ),
        (
            Column::CatalogNumber,
            [
                "LSD",
                "Alpha",
                "Vimana",
                "Lunar Cycle",
                "Mahadeva",
                "Untitled",
            ],
            [
                "Mahadeva",
                "Lunar Cycle",
                "Vimana",
                "Alpha",
                "LSD",
                "Untitled",
            ],
        ),
        (
            Column::Release,
            [
                "Alpha",
                "Vimana",
                "Lunar Cycle",
                "Mahadeva",
                "LSD",
                "Untitled",
            ],
            [
                "LSD",
                "Mahadeva",
                "Lunar Cycle",
                "Alpha",
                "Vimana",
                "Untitled",
            ],
        ),
        (
            Column::TrackNumber,
            [
                "Mahadeva",
                "Vimana",
                "LSD",
                "Lunar Cycle",
                "Alpha",
                "Untitled",
            ],
            [
                "Alpha",
                "Lunar Cycle",
                "LSD",
                "Vimana",
                "Mahadeva",
                "Untitled",
            ],
        ),
        (
            Column::Duration,
            [
                "Untitled",
                "Vimana",
                "LSD",
                "Lunar Cycle",
                "Mahadeva",
                "Alpha",
            ],
            [
                "Alpha",
                "Mahadeva",
                "Lunar Cycle",
                "Vimana",
                "LSD",
                "Untitled",
            ],
        ),
    ];
    for (column, ascending, descending) in orders {
        // The panel starts sorted by the artist ascending, so the first
        // click on the artist heading reverses it, and the first click on
        // any other heading sorts ascending.
        if column != Column::Artist {
            panel.click_heading(column);
            assert_eq!(
                panel.sort(),
                Sort {
                    column,
                    descending: false,
                }
            );
            assert_eq!(titles(&panel), ascending, "{column:?} ascending");
        }
        panel.click_heading(column);
        assert_eq!(
            panel.sort(),
            Sort {
                column,
                descending: true,
            }
        );
        assert_eq!(titles(&panel), descending, "{column:?} descending");
        panel.click_heading(column);
        assert_eq!(
            panel.sort(),
            Sort {
                column,
                descending: false,
            }
        );
        assert_eq!(titles(&panel), ascending, "{column:?} ascending again");
        // Leave the panel on the artist for the next column's first click.
        panel.click_heading(Column::Artist);
        if panel.sort().descending {
            panel.click_heading(Column::Artist);
        }
    }
}

#[test]
fn the_selection_follows_the_track_through_a_sort() {
    let mut panel = LibraryPanel::new(catalog());
    panel.select(row_of(&panel, "LSD"));
    assert_eq!(panel.selected(), Some(3));
    assert_eq!(panel.reference(), Some(camelot("8A")));
    panel.click_heading(Column::Artist);
    assert_eq!(
        titles(&panel)[1],
        "LSD",
        "descending by artist puts Hallucinogen second"
    );
    assert_eq!(panel.selected(), Some(1));
    assert!(panel.rows()[1].selected);
    assert_eq!(panel.reference(), Some(camelot("8A")));
    panel.click_heading(Column::Duration);
    assert_eq!(panel.selected(), Some(row_of(&panel, "LSD")));
}

#[test]
fn a_search_shows_the_tracks_whose_fields_contain_every_word() {
    let mut panel = LibraryPanel::new(catalog());
    panel.set_search("hallucinogen lsd");
    assert_eq!(panel.search(), "hallucinogen lsd");
    assert_eq!(titles(&panel), vec!["LSD"]);
    panel.set_search("lsd hallucinogen");
    assert_eq!(titles(&panel), vec!["LSD"], "the words in any order");

    panel.set_search("etnica");
    assert_eq!(
        titles(&panel),
        vec!["Alpha", "Vimana"],
        "in the order of the sort"
    );

    panel.set_search("etnica 1997");
    assert_eq!(
        titles(&panel),
        vec!["Vimana"],
        "one word in the artist and one in the year"
    );

    panel.set_search("goa");
    assert_eq!(
        titles(&panel),
        vec!["Lunar Cycle"],
        "the folder name is in the path and nowhere else"
    );

    panel.set_search("BR 013");
    assert_eq!(
        titles(&panel),
        vec!["Alpha"],
        "both Blue Room catalog numbers contain br, only one contains 013"
    );

    panel.set_search("8a");
    assert_eq!(
        titles(&panel),
        vec!["LSD"],
        "the Camelot code, whatever the case"
    );

    panel.set_search("twisted");
    assert_eq!(titles(&panel), vec!["LSD"], "the release title");

    panel.set_search("145");
    assert_eq!(
        titles(&panel),
        vec!["Vimana", "LSD"],
        "the tempo as the cell writes it, in the order of the sort"
    );

    panel.set_search("12");
    assert_eq!(
        titles(&panel),
        vec!["Alpha", "Lunar Cycle"],
        "a track number and a Camelot code"
    );

    panel.set_search("zzzz");
    assert!(panel.rows().is_empty());

    panel.set_search("unknown");
    assert!(
        panel.rows().is_empty(),
        "the text drawn in place of a missing artist is not searched"
    );
    panel.set_search("key");
    assert!(
        panel.rows().is_empty(),
        "the text drawn in place of a missing key is not searched"
    );
    panel.set_search("4:10");
    assert!(panel.rows().is_empty(), "the length is not searched");

    panel.set_search("   ");
    assert_eq!(panel.search(), "   ");
    assert_eq!(panel.rows().len(), 6, "only spaces is no search");
    panel.set_search("");
    assert_eq!(panel.rows().len(), 6);
}

#[test]
fn filters_narrow_the_rows_field_by_field() {
    let mut panel = LibraryPanel::new(catalog());
    let all = [
        "Mahadeva",
        "Alpha",
        "Vimana",
        "LSD",
        "Lunar Cycle",
        "Untitled",
    ];

    panel.set_filters(Filters {
        artist: "ETNICA".to_owned(),
        ..Filters::default()
    });
    assert_eq!(panel.filters().artist, "ETNICA");
    assert_eq!(
        titles(&panel),
        vec!["Alpha", "Vimana"],
        "without regard to case"
    );
    panel.set_filters(Filters {
        artist: " etnica ".to_owned(),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Alpha", "Vimana"],
        "spaces around the text are ignored, so a space typed after a word narrows nothing"
    );

    panel.set_filters(Filters {
        title: "un".to_owned(),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Lunar Cycle", "Untitled"],
        "part of a title, with the row that has no artist last"
    );

    panel.set_filters(Filters {
        keys: vec![camelot("8A"), camelot("8B")],
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Mahadeva", "LSD"],
        "any of the chosen keys, and a track with no key never"
    );

    panel.set_filters(Filters {
        label: "blue".to_owned(),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), vec!["Alpha", "Vimana"]);

    panel.set_filters(Filters {
        year_from: Some(1996),
        year_to: Some(1996),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Mahadeva", "Alpha", "Lunar Cycle"],
        "both ends included, an estimated year counting as its year, and no year never"
    );
    panel.set_filters(Filters {
        year_to: Some(1995),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), vec!["LSD"], "an open lower end");
    panel.set_filters(Filters {
        year_from: Some(1997),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), vec!["Vimana"], "an open upper end");

    panel.set_filters(Filters {
        bpm_from: Some(Bpm(140.0)),
        bpm_to: Some(Bpm(142.5)),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Mahadeva", "Alpha"],
        "both ends of the tempo range included"
    );
    panel.set_filters(Filters {
        bpm_from: Some(Bpm(139.0)),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Mahadeva", "Alpha", "Vimana", "LSD", "Lunar Cycle"],
        "everything at 139 or more"
    );
    panel.set_filters(Filters {
        bpm_to: Some(Bpm(139.0)),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Lunar Cycle", "Untitled"],
        "everything at 139 or less"
    );

    panel.set_filters(Filters {
        path: "goa".to_owned(),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), vec!["Lunar Cycle"]);
    panel.set_filters(Filters {
        path: "GOA".to_owned(),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Lunar Cycle"],
        "the path without regard to case"
    );
    panel.set_filters(Filters {
        path: "/MUSIC/".to_owned(),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), all);
    panel.set_filters(Filters {
        path: " goa ".to_owned(),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Lunar Cycle"],
        "the spaces around the path text are ignored"
    );

    panel.set_filters(Filters {
        artist: "  ".to_owned(),
        title: " ".to_owned(),
        label: " ".to_owned(),
        path: " ".to_owned(),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), all, "only spaces places no condition");
    panel.set_filters(Filters {
        keys: Vec::new(),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), all, "no keys chosen places no condition");

    panel.set_filters(Filters::default());
    assert_eq!(panel.filters(), &Filters::default());
    assert_eq!(titles(&panel), all);
}

#[test]
fn the_filters_the_search_and_the_sort_combine() {
    let mut panel = LibraryPanel::new(catalog());
    panel.set_filters(Filters {
        artist: "etnica".to_owned(),
        year_from: Some(1997),
        ..Filters::default()
    });
    assert_eq!(titles(&panel), vec!["Vimana"], "every filter must hold");

    panel.set_filters(Filters {
        label: "blue".to_owned(),
        ..Filters::default()
    });
    panel.set_search("alien");
    assert_eq!(titles(&panel), vec!["Alpha", "Vimana"]);
    panel.click_heading(Column::Title);
    panel.click_heading(Column::Title);
    assert_eq!(
        titles(&panel),
        vec!["Vimana", "Alpha"],
        "the matches follow the sort"
    );
    panel.set_search("alpha");
    assert_eq!(titles(&panel), vec!["Alpha"]);
    panel.set_filters(Filters {
        label: "blue".to_owned(),
        bpm_from: Some(Bpm(139.0)),
        ..Filters::default()
    });
    assert_eq!(
        titles(&panel),
        vec!["Alpha"],
        "the search still holds after the filters change, which alone would leave Vimana too"
    );

    // The selection is the track: filters that leave it out keep it and
    // the reference, and clearing them shows it selected again.
    panel.set_search("");
    panel.set_filters(Filters::default());
    panel.select(row_of(&panel, "Alpha"));
    assert_eq!(panel.reference(), Some(camelot("9A")));
    panel.set_filters(Filters {
        keys: vec![camelot("3B")],
        ..Filters::default()
    });
    assert_eq!(titles(&panel), vec!["Vimana"]);
    assert_eq!(panel.selected(), None);
    assert_eq!(panel.reference(), Some(camelot("9A")));
    panel.set_filters(Filters::default());
    assert_eq!(panel.selected(), Some(row_of(&panel, "Alpha")));
    assert!(panel.rows()[row_of(&panel, "Alpha")].selected);
}

#[test]
fn a_rows_cells_are_the_text_the_window_draws() {
    let mut records = catalog();
    // Untitled loses its title as well, so its title cell is its file name.
    records[2].metadata.title = None;
    let panel = LibraryPanel::new(records);
    let rows = panel.rows();
    let cells = |title: &str| -> Vec<String> {
        let row = rows
            .iter()
            .find(|row| row.title.as_deref() == Some(title))
            .unwrap_or_else(|| panic!("{title} is not among the rows shown"));
        Column::ALL.iter().map(|column| row.cell(*column)).collect()
    };
    assert_eq!(
        cells("LSD"),
        vec![
            "Hallucinogen",
            "LSD",
            "1995",
            "145.0",
            "8A",
            "Dragonfly",
            "BFLCD 08",
            "Twisted",
            "3",
            "5:00.0",
        ]
    );
    assert_eq!(
        cells("Alpha"),
        vec![
            "Etnica",
            "Alpha",
            "about 1996",
            "142.5",
            "9A",
            "Blue Room Released",
            "BR 013",
            "Alien Protein",
            "12",
            "8:00.0",
        ]
    );
    let untitled = rows
        .iter()
        .find(|row| row.title.is_none())
        .expect("the row with no title");
    let untitled: Vec<String> = Column::ALL
        .iter()
        .map(|column| untitled.cell(*column))
        .collect();
    assert_eq!(
        untitled,
        vec![
            "Unknown",
            "untitled.mp3",
            "no year",
            "138.0",
            "no key",
            "",
            "",
            "",
            "",
            "4:10.0",
        ]
    );
}

#[test]
fn selecting_a_row_highlights_the_tracks_that_mix_with_it() {
    let mut panel = LibraryPanel::new(records());
    // Mahadeva is 8B, Alpha 9A, Vimana 3B, LSD 8A, and Untitled has no key.
    let lsd = row_of(&panel, "LSD");
    panel.select(lsd);
    assert_eq!(panel.selected(), Some(lsd));
    assert_eq!(panel.reference(), Some(camelot("8A")));
    assert_eq!(
        sorted_compatible_titles(&panel),
        vec!["Alpha", "LSD", "Mahadeva"],
        "8B shares the number, 9A is a step around the wheel, and 8A is itself"
    );
    let rows = panel.rows();
    assert!(rows[lsd].selected);
    assert!(rows.iter().filter(|row| row.selected).count() == 1);

    // The selection is the track: a search that leaves it out has no
    // selected row but keeps the reference, and one that shows it again
    // selects it again.
    panel.set_search("etnica");
    assert_eq!(panel.selected(), None);
    assert_eq!(panel.reference(), Some(camelot("8A")));
    assert_eq!(compatible_titles(&panel), vec!["Alpha"]);
    panel.set_search("lsd");
    assert_eq!(panel.selected(), Some(0));
    assert!(panel.rows()[0].selected);
    panel.set_search("");
    assert_eq!(panel.selected(), Some(lsd));

    // A row past the last changes nothing.
    panel.select(5);
    assert_eq!(panel.selected(), Some(lsd));

    // Selecting a track with no key clears the reference.
    let untitled = row_of(&panel, "Untitled");
    panel.select(untitled);
    assert_eq!(panel.selected(), Some(untitled));
    assert_eq!(panel.reference(), None);
    assert!(compatible_titles(&panel).is_empty());
}

#[test]
fn the_reference_may_come_from_the_timeline() {
    let mut panel = LibraryPanel::new(records());
    let lsd = row_of(&panel, "LSD");
    panel.select(lsd);
    panel.set_reference(Some(camelot("3B")));
    assert_eq!(panel.reference(), Some(camelot("3B")));
    assert_eq!(panel.selected(), Some(lsd), "the selection is left alone");
    assert_eq!(compatible_titles(&panel), vec!["Vimana"]);
    panel.set_reference(Some(camelot("12A")));
    assert!(
        compatible_titles(&panel).is_empty(),
        "12A is a step from 1A and 11A, not from 3B or 8A"
    );
    panel.set_reference(Some(camelot("7B")));
    assert_eq!(
        compatible_titles(&panel),
        vec!["Mahadeva"],
        "7B is a step from 8B"
    );
    panel.set_reference(None);
    assert!(compatible_titles(&panel).is_empty());
}

#[test]
fn tracks_already_in_the_mix_are_marked() {
    let mut panel = LibraryPanel::new(records());
    let mix = Mix {
        tracks: vec![
            track("lsd", 1, 300, 32.0, 512.0),
            track("other", 9, 300, 32.0, 512.0),
        ],
    };
    panel.set_mix(&mix);
    let in_mix: Vec<String> = panel
        .rows()
        .iter()
        .filter(|row| row.in_mix)
        .map(|row| row.title.clone().unwrap_or_default())
        .collect();
    assert_eq!(in_mix, vec!["LSD"]);
    panel.set_mix(&Mix::new());
    assert!(panel.rows().iter().all(|row| !row.in_mix));
}

#[test]
fn an_inserted_track_gets_the_leveling_gain_of_its_record() {
    // LSD's hash is all ones. Minus eight LUFS with peaks at
    // minus 0.2 dBTP levels to minus six decibels, and a record with no
    // loudness gets no gain.
    let mut records = records();
    for record in records.iter_mut() {
        if record.hash == ContentHash([1; 32]) {
            record.loudness = Some(Loudness {
                integrated: Lufs(-8.0),
                true_peak: Decibels(-0.2),
            });
        }
    }
    let panel = LibraryPanel::new(records);
    let lsd = row_of(&panel, "LSD");
    match panel
        .insert_edit(lsd, 1, Preset::Beatmix { bars: 8 })
        .expect("LSD's row exists")
    {
        Edit::InsertTrack { track, .. } => {
            assert_eq!(track.hash, ContentHash([1; 32]));
            assert!((track.gain.0 + 6.0).abs() < 1e-9, "{:?}", track.gain);
        }
        other => panic!("expected an insert, got {other:?}"),
    }
    let untitled = row_of(&panel, "Untitled");
    match panel
        .insert_edit(untitled, 0, Preset::Cut)
        .expect("Untitled's row exists")
    {
        Edit::InsertTrack { track, .. } => {
            assert_ne!(track.hash, ContentHash([1; 32]));
            assert_eq!(track.gain, Decibels::UNITY);
        }
        other => panic!("expected an insert, got {other:?}"),
    }
}

#[test]
fn inserting_a_row_builds_the_track_with_the_presets_outro() {
    let panel = LibraryPanel::new(records());
    let lsd = row_of(&panel, "LSD");
    // A sixteen-bar fade puts the outro anchor eight bars earlier than the
    // analyzed outro, which was placed for an eight-bar fade:
    // 512 + 32 - 64 = 480.
    let edit = panel
        .insert_edit(lsd, 1, Preset::Beatmix { bars: 16 })
        .expect("LSD's row exists");
    let expected = Track {
        path: PathBuf::from("/music/lsd.mp3"),
        hash: ContentHash([1; 32]),
        length: Samples(300 * 44_100),
        grid: BeatGrid {
            first_beat: Samples(441),
            bpm: Bpm(140.0),
        },
        anchors: Anchors {
            intro: Beats(32.0),
            outro: Beats(480.0),
        },
        keylock: true,
        gain: dermixen_core::Decibels::UNITY,
        volume: Envelope::new(),
        eq: EqEnvelopes::default(),
        tempo: Vec::new(),
    };
    assert_eq!(
        edit,
        Edit::InsertTrack {
            at: 1,
            track: Box::new(expected.clone()),
            preset: Preset::Beatmix { bars: 16 },
        }
    );

    // A cut has no fade, so the outro stays where analysis put it for the
    // default fade of eight bars.
    match panel
        .insert_edit(lsd, 0, Preset::Cut)
        .expect("LSD's row exists")
    {
        Edit::InsertTrack { at, track, preset } => {
            assert_eq!(at, 0);
            assert_eq!(preset, Preset::Cut);
            assert_eq!(
                track.anchors,
                Anchors {
                    intro: Beats(32.0),
                    outro: Beats(512.0)
                }
            );
        }
        other => panic!("not an insert: {other:?}"),
    }
    assert_eq!(panel.insert_edit(5, 0, Preset::Cut), None);

    // A blend has no length of its own either, so the outro stays where
    // analysis put it as well.
    match panel
        .insert_edit(lsd, 0, Preset::Blend)
        .expect("LSD's row exists")
    {
        Edit::InsertTrack { at, track, preset } => {
            assert_eq!(at, 0);
            assert_eq!(preset, Preset::Blend);
            assert_eq!(
                track.anchors,
                Anchors {
                    intro: Beats(32.0),
                    outro: Beats(512.0)
                }
            );
        }
        other => panic!("not an insert: {other:?}"),
    }

    // The edit goes through the timeline like any other, and the mix then
    // holds the track where it was put.
    let mut timeline = Timeline::new(two_tracks());
    timeline
        .apply(edit)
        .expect("the insert fits between the two tracks");
    let mix = timeline.mix();
    assert_eq!(mix.tracks.len(), 3);
    assert_eq!(mix.tracks[1].hash, ContentHash([1; 32]));
    assert_eq!(mix.tracks[1].anchors, expected.anchors);
    assert!(timeline.undo());
    assert_eq!(timeline.mix().tracks.len(), 2);
}

#[test]
fn rows_say_when_a_year_is_an_estimate_and_name_the_release() {
    let mut records = records();
    // LSD's year is an estimate. Mahadeva's release is known.
    records[0].metadata.year_is_approximate = true;
    records[1].release = Release {
        label: Some("Trust In Trance".to_owned()),
        catalog_number: Some("TIT CD 001".to_owned()),
        title: Some("Trust In Trance 2".to_owned()),
        track_number: Some(1),
        data_source: Some(ReleaseDataSource::DiscogsExport),
    };
    let panel = LibraryPanel::new(records);
    let rows = panel.rows();
    assert_eq!(rows.len(), 5);

    let lsd = &rows[row_of(&panel, "LSD")];
    assert_eq!(lsd.year, Some(1996));
    assert!(lsd.year_is_approximate);
    assert_eq!(lsd.release_title, None);
    assert_eq!(lsd.label, None);
    assert_eq!(lsd.catalog_number, None);
    assert_eq!(lsd.track_number, None);

    let mahadeva = &rows[row_of(&panel, "Mahadeva")];
    assert_eq!(mahadeva.year, Some(1996));
    assert!(!mahadeva.year_is_approximate);
    assert_eq!(mahadeva.release_title.as_deref(), Some("Trust In Trance 2"));
    assert_eq!(mahadeva.label.as_deref(), Some("Trust In Trance"));
    assert_eq!(mahadeva.catalog_number.as_deref(), Some("TIT CD 001"));
    assert_eq!(mahadeva.track_number, Some(1));

    assert_eq!(
        rows.iter().filter(|row| row.year_is_approximate).count(),
        1,
        "only LSD's year is an estimate"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.release_title.is_some())
            .count(),
        1,
        "only Mahadeva's release is known"
    );
}

#[test]
fn a_rows_year_text_says_when_the_year_is_an_estimate() {
    let mut records = records();
    // LSD's year is an estimate, and Untitled has no year.
    records[0].metadata.year_is_approximate = true;
    records[2].metadata.year = None;
    let panel = LibraryPanel::new(records);
    let rows = panel.rows();
    assert_eq!(rows[row_of(&panel, "LSD")].year_text(), "about 1996");
    assert_eq!(rows[row_of(&panel, "Mahadeva")].year_text(), "1996");
    assert_eq!(rows[row_of(&panel, "Untitled")].year_text(), "no year");
}

#[test]
fn new_records_keep_the_search_the_sort_the_filters_and_the_selection() {
    let mut panel = LibraryPanel::new(records());
    panel.set_mix(&two_tracks());
    panel.click_heading(Column::Title);
    panel.click_heading(Column::Title);
    panel.set_search("etnica");
    panel.select(row_of(&panel, "Alpha"));
    assert_eq!(titles(&panel), vec!["Vimana", "Alpha"]);
    assert_eq!(panel.selected(), Some(1));
    let in_mix_before: Vec<bool> = panel.rows().iter().map(|row| row.in_mix).collect();

    // A scan added a sixth track by Etnica, and the library is read again.
    let mut grown = records();
    grown.push(record(
        6,
        "beta.mp3",
        Some("Etnica"),
        Some("Beta"),
        MetadataSource::Tags,
        Some("9A"),
    ));
    panel.set_records(grown);
    assert_eq!(
        titles(&panel),
        vec!["Vimana", "Beta", "Alpha"],
        "the new track is shown where the sort and the search put it"
    );
    assert_eq!(panel.search(), "etnica");
    assert_eq!(
        panel.sort(),
        Sort {
            column: Column::Title,
            descending: true
        }
    );
    assert_eq!(
        panel.selected(),
        Some(2),
        "the selected track is still selected at its new row"
    );
    assert_eq!(panel.reference(), Some(camelot("9A")));
    assert_eq!(
        panel
            .rows()
            .iter()
            .map(|row| row.in_mix)
            .collect::<Vec<bool>>(),
        vec![in_mix_before[0], false, in_mix_before[1]],
        "the tracks marked as in the mix are still marked"
    );

    // The filters hold as well: a key filter that leaves Vimana out still
    // leaves it out after the records change again.
    let filters = Filters {
        keys: vec![camelot("9A")],
        ..Filters::default()
    };
    panel.set_filters(filters.clone());
    assert_eq!(titles(&panel), vec!["Beta", "Alpha"]);
    panel.set_records(records());
    assert_eq!(panel.filters(), &filters);
    assert_eq!(titles(&panel), vec!["Alpha"]);
    assert_eq!(panel.selected(), Some(0));

    // A selected track whose record is gone is no longer selected, and
    // the reference key goes with it.
    let mut fewer = records();
    fewer.retain(|record| record.metadata.title.as_deref() != Some("Alpha"));
    panel.set_records(fewer);
    assert_eq!(titles(&panel), Vec::<String>::new());
    assert_eq!(panel.selected(), None);
    assert_eq!(panel.reference(), None);
    panel.set_search("");
    assert_eq!(panel.selected(), None);
}
