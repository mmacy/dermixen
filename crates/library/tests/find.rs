//! Acceptance tests for fuzzy find. A coder agent makes these pass without editing them.
//!
//! The records are built from real names in the reference corpus and the
//! queries are the kind of line a tracklisting holds.

use std::path::{Path, PathBuf};

use dermixen_analysis::Extent;
use dermixen_core::{Anchors, BeatGrid, Beats, Bpm, ContentHash, Samples};
use dermixen_library::{Metadata, MetadataSource, Release, TrackRecord, find};

fn record(byte: u8, path: &str, artist: Option<&str>, title: &str) -> TrackRecord {
    TrackRecord {
        hash: ContentHash([byte; 32]),
        path: PathBuf::from(path),
        length: Samples(18_522_000),
        grid: BeatGrid {
            first_beat: Samples::ZERO,
            bpm: Bpm(140.0),
        },
        grid_confidence: 1.0,
        grid_analyzer: "given".to_owned(),
        key: None,
        extent: Extent {
            begins: Samples::ZERO,
            ends: Samples(18_522_000),
        },
        anchors: Anchors {
            intro: Beats::ZERO,
            outro: Beats(64.0),
        },
        anchor_confidence: 0.0,
        anchor_analyzer: "edges".to_owned(),
        phrases: None,
        loudness: None,
        metadata: Metadata {
            artist: artist.map(str::to_owned),
            title: Some(title.to_owned()),
            year: None,
            year_is_approximate: false,
            source: MetadataSource::Filename,
        },
        release: Release::default(),
    }
}

fn library() -> Vec<TrackRecord> {
    vec![
        record(
            1,
            "/goa/comp/VA - First Flight [AFRCD01]/wav/01 Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix).wav",
            Some("Slinky Wizard"),
            "Lunar Juice (Hallucinogen Moon Strudel Remix)",
        ),
        record(
            2,
            "/goa/artist/Slinky Wizard - Wizard Ball [FLR001]/02 Slinky Wizard - Lunar Juice.mp3",
            Some("Slinky Wizard"),
            "Lunar Juice",
        ),
        record(
            3,
            "/goa/comp/VA - Concept In Dance 2 [MM80036-2]/02 Various - Man With No Name - Silicon Trip.mp3",
            Some("Man With No Name"),
            "Silicon Trip",
        ),
        record(
            4,
            "/goa/artist/Etnica - Etnica Volume 1/Etnica - Ashtronauts.mp3",
            Some("Etnica"),
            "Ashtronauts",
        ),
        record(
            5,
            "/goa/comp/VA - Analog Visions [DATCD011]/05 Green Nuns Of The Revolution - Conflict (Live In New York 1997 Mix).wav",
            Some("Green Nuns Of The Revolution"),
            "Conflict (Live In New York 1997 Mix)",
        ),
        record(
            6,
            "/goa/artist/Man With No Name - Moment of Truth [DICCD125]/01 Man With No Name - Moment of Truth.mp3",
            Some("Man With No Name"),
            "Moment of Truth",
        ),
        record(
            7,
            "/goa/artist/Hallucinogen - Twisted [DFLCD01]/03 Hallucinogen - LSD.mp3",
            Some("Hallucinogen"),
            "LSD",
        ),
        record(
            8,
            "/goa/misc/Alphanaut Beta Centauri.wav",
            None,
            "Alphanaut Beta Centauri",
        ),
        record(
            9,
            "/goa/artist/Man With No Name - Teleport [PERFCD22]/02 Man With No Name - Vavoom.mp3",
            Some("Man With No Name"),
            "Vavoom",
        ),
        record(
            10,
            "/goa/artist/Man With No Name - Teleport [PERFCD22]/01 Man With No Name - Teleport.mp3",
            Some("Man With No Name"),
            "Teleport",
        ),
    ]
}

/// Lines a tracklisting might hold, each with the file that must rank first
/// and the lowest score that first match may have.
const LINES: [(&str, &str, f64); 12] = [
    (
        "Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix)",
        "/goa/comp/VA - First Flight [AFRCD01]/wav/01 Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix).wav",
        0.99,
    ),
    (
        "Slinky Wizard - Lunar Juice",
        "/goa/artist/Slinky Wizard - Wizard Ball [FLR001]/02 Slinky Wizard - Lunar Juice.mp3",
        0.99,
    ),
    (
        "03. Slinky Wizard - Lunar Juice",
        "/goa/artist/Slinky Wizard - Wizard Ball [FLR001]/02 Slinky Wizard - Lunar Juice.mp3",
        0.99,
    ),
    (
        "[12:34] Man With No Name - Silicon Trip",
        "/goa/comp/VA - Concept In Dance 2 [MM80036-2]/02 Various - Man With No Name - Silicon Trip.mp3",
        0.99,
    ),
    (
        "1:02:34 Man With No Name - Silicon Trip",
        "/goa/comp/VA - Concept In Dance 2 [MM80036-2]/02 Various - Man With No Name - Silicon Trip.mp3",
        0.99,
    ),
    (
        "lunar juice",
        "/goa/artist/Slinky Wizard - Wizard Ball [FLR001]/02 Slinky Wizard - Lunar Juice.mp3",
        0.8,
    ),
    (
        "Lunar Juice Slinky Wizard",
        "/goa/artist/Slinky Wizard - Wizard Ball [FLR001]/02 Slinky Wizard - Lunar Juice.mp3",
        0.99,
    ),
    (
        "Slinky Wizzard - Lunar Juice",
        "/goa/artist/Slinky Wizard - Wizard Ball [FLR001]/02 Slinky Wizard - Lunar Juice.mp3",
        0.99,
    ),
    (
        "hallucinogen lsd",
        "/goa/artist/Hallucinogen - Twisted [DFLCD01]/03 Hallucinogen - LSD.mp3",
        0.99,
    ),
    (
        "ETNICA - ASHTRONAUTS",
        "/goa/artist/Etnica - Etnica Volume 1/Etnica - Ashtronauts.mp3",
        0.99,
    ),
    (
        "Green Nuns of the Revolution - Conflict",
        "/goa/comp/VA - Analog Visions [DATCD011]/05 Green Nuns Of The Revolution - Conflict (Live In New York 1997 Mix).wav",
        0.8,
    ),
    (
        "Alphanaut - Beta Centauri",
        "/goa/misc/Alphanaut Beta Centauri.wav",
        0.99,
    ),
];

#[test]
fn tracklisting_lines_rank_the_right_file_first() {
    let records = library();
    let mut wrong = Vec::new();
    for (line, expected, at_least) in LINES {
        let matches = find(&records, line, 5);
        let Some(first) = matches.first() else {
            wrong.push(format!("{line:?}: no matches"));
            continue;
        };
        if first.record.path != Path::new(expected) || first.score < at_least {
            wrong.push(format!(
                "{line:?}\n    expected {expected} at {at_least} or more\n    got      {} at {:.3}",
                first.record.path.display(),
                first.score
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} lines resolved wrongly:\n{}",
        wrong.len(),
        LINES.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_line_that_names_nothing_in_the_library_is_not_a_match() {
    let records = library();
    let matches = find(&records, "Juno Reactor - Feel The Universe", 5);
    if let Some(first) = matches.first() {
        assert!(
            first.score < 0.5,
            "{} scored {}",
            first.record.path.display(),
            first.score
        );
    }
    assert!(find(&records, "", 5).is_empty());
    assert!(find(&records, "xyzzy plugh", 5).is_empty());
}

#[test]
fn matches_are_sorted_by_score_then_path_and_limited() {
    let records = library();
    let matches = find(&records, "Man With No Name", 10);
    assert_eq!(matches.len(), 4, "{matches:?}");
    for window in matches.windows(2) {
        assert!(window[0].score >= window[1].score, "{matches:?}");
        // Among matches that score alike, the earlier path comes first.
        if window[0].score == window[1].score {
            assert!(window[0].record.path < window[1].record.path, "{matches:?}");
        }
    }
    assert!(matches.iter().all(|m| m.score > 0.0 && m.score <= 1.0));
    // "Teleport" and "Vavoom" have the same number of words and the same
    // words matched, so they tie, and the track numbered 01 sorts first.
    let teleport = matches
        .iter()
        .position(|m| {
            m.record
                .path
                .ends_with("01 Man With No Name - Teleport.mp3")
        })
        .unwrap();
    let vavoom = matches
        .iter()
        .position(|m| m.record.path.ends_with("02 Man With No Name - Vavoom.mp3"))
        .unwrap();
    assert_eq!(matches[teleport].score, matches[vavoom].score);
    assert!(teleport < vavoom, "{matches:?}");
    // A track with fewer words beyond the ones named scores higher.
    assert!(
        matches[0]
            .record
            .path
            .to_string_lossy()
            .contains("Teleport"),
        "{matches:?}"
    );

    let one = find(&records, "Man With No Name", 1);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].record.path, matches[0].record.path);
    assert!(find(&records, "Man With No Name", 0).is_empty());
}

#[test]
fn the_score_favors_the_fuller_name_among_tracks_that_share_a_title() {
    let records = library();
    let matches = find(&records, "Slinky Wizard - Lunar Juice", 5);
    assert!(matches.len() >= 2, "{matches:?}");
    assert!(
        matches[0]
            .record
            .path
            .ends_with("02 Slinky Wizard - Lunar Juice.mp3")
    );
    assert!(
        matches[1]
            .record
            .path
            .to_string_lossy()
            .contains("Hallucinogen Moon Strudel Remix")
    );
    assert!(matches[0].score > matches[1].score);
    // Naming the remix picks the remix, and the plain track still scores well.
    let matches = find(
        &records,
        "Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix)",
        5,
    );
    assert!(
        matches[0]
            .record
            .path
            .to_string_lossy()
            .contains("Hallucinogen Moon Strudel Remix")
    );
    assert!(
        matches[1]
            .record
            .path
            .ends_with("02 Slinky Wizard - Lunar Juice.mp3")
    );
    assert!(matches[1].score >= 0.5, "{}", matches[1].score);
}
