//! Acceptance tests for tag reading and file name parsing. A coder agent makes
//! these pass without editing them.
//!
//! The file name table holds real paths from the reference corpus, so the
//! parser is measured against the naming conventions it will meet.

use std::path::{Path, PathBuf};

use dermixen_library::{MetadataSource, metadata_of, parse_filename, read_tags};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/audio")
        .join(name)
}

/// Real paths from the reference corpus, relative to its root, with the
/// artist and title the parser must give for each. An empty artist means
/// the parser must give none.
const NAMES: [(&str, &str, &str); 30] = [
    (
        "12-inch_&_ep/Crop Circles - Full Mental Jackpot EP [DATEP001]/01 Crop Circles - Full Mental Jackpot.mp3",
        "Crop Circles",
        "Full Mental Jackpot",
    ),
    (
        "artist/Biot - Saturation Beta 2CD [BTM105]/101 Biot - Ignition.mp3",
        "Biot",
        "Ignition",
    ),
    (
        "artist/Astral Projection - Astral Files [PHNKL2054-2]/01 Ionised.mp3",
        "Astral Projection",
        "Ionised",
    ),
    (
        "comp/VA - Concept In Dance 2 [MM80036-2]/02 Various - Man With No Name - Silicon Trip.mp3",
        "Man With No Name",
        "Silicon Trip",
    ),
    (
        "comp/VA - Destination Goa 2 [WHYCD02]/203 Astral Projection - Mahadeva ('96 Remix).mp3",
        "Astral Projection",
        "Mahadeva ('96 Remix)",
    ),
    (
        "artist/Etnica - Etnica Volume 1/Etnica - Ashtronauts.mp3",
        "Etnica",
        "Ashtronauts",
    ),
    (
        "12-inch_&_ep/Nervasystem & Aether - Mineralien Molecule E.P/Nervasystem & Aether - Mineralien Molecule E.P. - 01 Mineralien Myriad.wav",
        "Nervasystem & Aether",
        "Mineralien Myriad",
    ),
    (
        "artist/OOOD - aLIVE (CDLP 1997 inc. artwork)/OOOD - aLIVE - 04 - Cosmic Ripple.mp3",
        "OOOD",
        "Cosmic Ripple",
    ),
    (
        "artist/Koxbox - Live @ Burning Man 1996 [DAT]/(01)-KoxBox_-_Acid_Vol._3.wav",
        "KoxBox",
        "Acid Vol. 3",
    ),
    (
        "unreleased/EPCC/Etnica_-_Asian_Code_7.wav",
        "Etnica",
        "Asian Code 7",
    ),
    (
        "comp/VA - Analog Visions [DATCD011]/DAT Records - VA - Analog Visions - 05 Green Nuns Of The Revolution - Conflict (Live In New York 1997 Mix).wav",
        "Green Nuns Of The Revolution",
        "Conflict (Live In New York 1997 Mix)",
    ),
    (
        "artist/Digitalis - The Early Years 1995-2000/Digitalis - The Early Years- 1995-2000 - 31 The Improbable Voyage.mp3",
        "Digitalis",
        "The Improbable Voyage",
    ),
    (
        "artist/Prana - Cyclone [MPCD01]/Classic Goa Trax - Prana - Cyclone - 05 Message For Eastedge (Trance Express Mix).wav",
        "Prana",
        "Message For Eastedge (Trance Express Mix)",
    ),
    (
        "artist/Miranda - Cosmic Treasure Vol.1- Best Of 1995-2000 Remastered [SPITCD053]/Spiral Trax - Miranda - Cosmic Treasure Vol.1- Best Of 1995-2000 Remastered (SPITCD053 - Spiral Trax) - 02 Gnocchi.wav",
        "Miranda",
        "Gnocchi",
    ),
    (
        "artist/Nemo Zimmer - Awaaz - Paranoid Travelling EP - Ganesh Taal EP/Nemo Zimmer - Awaaz - Paranoid Travelling EP - Ganesh Taal EP - 01 Nemo - Paranoid Delusion.mp3",
        "Nemo",
        "Paranoid Delusion",
    ),
    (
        "comp/VA - First Flight [AFRCD01]/wav/01 Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix) - Slinky Wizard - Lunar Juice (Hallucinogen Moon Strudel Remix).wav",
        "Slinky Wizard",
        "Lunar Juice (Hallucinogen Moon Strudel Remix)",
    ),
    (
        "artist/Shakta - Any Old Irony [PHANTASM]/Phantasm Records - ANY OLD IRONY - 01 Kick The Baby.mp3",
        "Shakta",
        "Kick The Baby",
    ),
    (
        "comp/VA - Fill Your Head with Phantasm Vol 3/VARIOUS ARTISTS - FILL YOUR HEAD WITH PHANTASM VOL.3 - 10 O.O.O.D. - Kundalini.mp3",
        "O.O.O.D.",
        "Kundalini",
    ),
    (
        "comp/VA - Teleportation (4CD) [TRIPBX10]/408 Chris Organic - The Shaman (Ambience Dub).mp3",
        "Chris Organic",
        "The Shaman (Ambience Dub)",
    ),
    (
        "artist/Dimension 5 - Transdimensional (Reissue) [SUNCD08]/Dimension 5 - Transdimensional (Reissue) [SUNCD08]/06 Dimension 5 - Psychic Influence.mp3",
        "Dimension 5",
        "Psychic Influence",
    ),
    (
        "artist/Mindfield - Odyssey of the Mind 2CD [PTM142CD]/CD2/03 Mindfield - Ten Years After.mp3",
        "Mindfield",
        "Ten Years After",
    ),
    (
        "artist/Hunab-Ku - Magik Universe [BMPHQCD03]/wav/09 Hunab Ku - Matrix.wav",
        "Hunab Ku",
        "Matrix",
    ),
    (
        "unreleased/The Delta - Travelling At The Speed Of Thought (Montauk P Remix).mp3",
        "The Delta",
        "Travelling At The Speed Of Thought (Montauk P Remix)",
    ),
    (
        "vinyl_rips/AFR010 Slinky Wizard - Slick Witch v3.0.wav",
        "Slinky Wizard",
        "Slick Witch v3.0",
    ),
    (
        "vinyl_rips/KR005-B1 Ominus - Acid Tester (Mirrors of Sense mix).wav",
        "Ominus",
        "Acid Tester (Mirrors of Sense mix)",
    ),
    (
        "vinyl_rips/Esion God - Psychedelic Medication [AQUA12-B].wav",
        "Esion God",
        "Psychedelic Medication",
    ),
    (
        "unreleased/[DREAM]_list/Psionyx_-_Deimos_Vista_Extended.mp3",
        "Psionyx",
        "Deimos Vista Extended",
    ),
    (
        "comp/VA - Abstract Phaze [MPCD4]/Alienated - Matsuri Classics Vol.1 - abstract phaze - 08 Free Return.wav",
        "",
        "Free Return",
    ),
    (
        "artist/Alphanaut/Alphanaut Abduction (acid 303 mix).wav",
        "",
        "Alphanaut Abduction (acid 303 mix)",
    ),
    (
        "misc/Life892/sr_program_2010_08_12_15_08_09.mp3",
        "",
        "sr program 2010 08 12 15 08 09",
    ),
];

#[test]
fn real_file_names_parse_to_their_artist_and_title() {
    let root = Path::new("/Volumes/goa");
    let mut wrong = Vec::new();
    for (relative, artist, title) in NAMES {
        let parsed = parse_filename(&root.join(relative));
        let expected_artist = (!artist.is_empty()).then(|| artist.to_owned());
        if parsed.artist != expected_artist || parsed.title.as_deref() != Some(title) {
            wrong.push(format!(
                "{relative}\n    expected {expected_artist:?} / {title:?}\n    got      {:?} / {:?}",
                parsed.artist, parsed.title
            ));
        }
        assert_eq!(parsed.source, MetadataSource::Filename, "{relative}");
        assert_eq!(parsed.year, None, "{relative}");
    }
    assert!(
        wrong.is_empty(),
        "{} of {} names parsed wrongly:\n{}",
        wrong.len(),
        NAMES.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_name_with_nothing_to_parse_still_gives_a_title() {
    let parsed = parse_filename(Path::new("/music/.mp3"));
    assert!(parsed.title.is_some());
    assert_eq!(parsed.source, MetadataSource::Filename);
    let parsed = parse_filename(Path::new("/music/01 .mp3"));
    assert!(parsed.title.is_some());
}

#[test]
fn tags_are_read_from_an_mp3_a_flac_and_an_m4a() {
    for name in ["tagged.mp3", "tagged.flac", "tagged.m4a"] {
        let found = read_tags(&fixture(name)).unwrap().unwrap();
        assert_eq!(found.artist.as_deref(), Some("Slinky Wizard"), "{name}");
        assert_eq!(
            found.title.as_deref(),
            Some("Lunar Juice (Hallucinogen Moon Strudel Remix)"),
            "{name}"
        );
        assert_eq!(found.year, Some(1996), "{name}");
        assert_eq!(found.source, MetadataSource::Tags, "{name}");
        // The whole path is the same answer.
        assert_eq!(metadata_of(&fixture(name)), found, "{name}");
    }
}

#[test]
fn a_file_without_tags_falls_back_to_its_name() {
    let path = fixture("sine-440-44k.wav");
    assert_eq!(read_tags(&path).unwrap(), None);
    let found = metadata_of(&path);
    assert_eq!(found.source, MetadataSource::Filename);
    assert_eq!(found.title.as_deref(), Some("sine-440-44k"));
    assert_eq!(found.artist, None);
    assert_eq!(found.year, None);
}

#[test]
fn a_file_that_cannot_be_read_is_an_error_for_tags_and_a_name_for_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("03 Prana - Boundless.mp3");
    std::fs::write(&path, b"this is not an mp3").unwrap();
    let problem = read_tags(&path).unwrap_err();
    assert!(problem.to_string().contains("Boundless"), "{problem}");
    let found = metadata_of(&path);
    assert_eq!(found.source, MetadataSource::Filename);
    assert_eq!(found.artist.as_deref(), Some("Prana"));
    assert_eq!(found.title.as_deref(), Some("Boundless"));

    let missing = dir.path().join("missing.mp3");
    assert!(read_tags(&missing).is_err());
    assert_eq!(metadata_of(&missing).title.as_deref(), Some("missing"));
}

#[test]
fn blank_tag_values_count_as_absent() {
    let found = read_tags(&fixture("tagged-blank.mp3")).unwrap();
    // The file's tags hold an artist and a title of spaces, so the artist
    // is read and the title is absent.
    let found = found.expect("a tag with an artist is usable");
    assert_eq!(found.artist.as_deref(), Some("Slinky Wizard"));
    assert_eq!(found.title, None);
    assert_eq!(found.year, None);
}
