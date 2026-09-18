//! Acceptance tests for the mix document: the project file and the timeline
//! layout. A coder agent makes these pass without editing them.

use std::fs;
use std::path::{Path, PathBuf};

use dermixen_core::{
    Anchors, BeatGrid, Beats, Bpm, ContentHash, Decibels, Envelope, EnvelopeNode, EqEnvelopes,
    FORMAT_VERSION, Mix, Samples, Seconds, TempoNode, Track,
};
use proptest::prelude::*;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/mix")
}

fn read_fixture(relative: &str) -> String {
    let path = fixture_dir().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + a.abs().max(b.abs()))
}

#[test]
fn the_format_version_is_one() {
    assert_eq!(FORMAT_VERSION, 1);
}

#[test]
fn the_two_track_fixture_reads_as_expected() {
    let mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    assert_eq!(mix.tracks.len(), 2);
    let first = &mix.tracks[0];
    assert_eq!(
        first.path,
        PathBuf::from(
            "/Users/dermixenuser/audio/goa/comp/VA - Goa Vibes [GV001]/03 Slinky Wizard - Lunar Juice.mp3"
        )
    );
    assert_eq!(
        first.hash.to_string(),
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    );
    assert_eq!(first.length, Samples(18_522_000));
    assert_eq!(first.grid.first_beat, Samples(4410));
    assert_eq!(first.grid.bpm, Bpm(138.0));
    assert_eq!(first.anchors.intro, Beats(64.0));
    assert_eq!(first.anchors.outro, Beats(896.0));
    assert!(first.keylock);
    assert_eq!(first.gain, Decibels(-2.5));
    assert_eq!(first.volume.len(), 2);
    assert_eq!(first.volume.value_at(Beats(912.0)), Decibels(-25.0));
    assert_eq!(first.eq.low.len(), 2);
    assert!(first.eq.mid.is_empty());
    assert!(first.eq.high.is_empty());
    assert_eq!(
        first.tempo,
        vec![TempoNode {
            at: Beats(896.0),
            bpm: Bpm(138.0)
        }]
    );
    let second = &mix.tracks[1];
    assert!(!second.keylock);
    assert_eq!(second.gain, Decibels::UNITY, "the file omits gain_db");
    assert_eq!(second.grid.bpm, Bpm(140.0));
    assert_eq!(second.anchors.intro, Beats(32.0));
}

#[test]
fn the_gain_is_always_written_and_reads_back() {
    let mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    let json = mix.to_json();
    assert!(json.contains("\"gain_db\": -2.5"), "{json}");
    // The second track had no gain in the file, and the written form says so in numbers.
    assert!(json.contains("\"gain_db\": 0.0"), "{json}");
    assert_eq!(Mix::from_json(&json).unwrap(), mix);
}

#[test]
fn the_written_form_is_readable_and_starts_with_the_version() {
    let mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    let json = mix.to_json();
    assert!(
        json.starts_with("{\n  \"version\": 1,\n  \"tracks\": ["),
        "unexpected opening: {}",
        &json[..json.len().min(60)]
    );
    assert!(json.contains("\n      \"length_samples\": 18522000,\n"));
    assert_eq!(Mix::from_json(&json).unwrap(), mix);
}

#[test]
fn an_empty_mix_reads_and_writes() {
    let mix = Mix::new();
    let json = mix.to_json();
    assert_eq!(Mix::from_json(&json).unwrap(), mix);
    assert!(
        Mix::from_json("{\"version\": 1, \"tracks\": []}")
            .unwrap()
            .tracks
            .is_empty()
    );
    assert!(mix.timeline().is_none());
}

#[test]
fn every_invalid_fixture_reports_its_expected_error() {
    let dir = fixture_dir().join("invalid");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("dmx") {
            continue;
        }
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let text = fs::read_to_string(&path).unwrap();
        let expected: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join(format!("{name}.expected.json"))).unwrap(),
        )
        .unwrap();
        let field_starts_with = expected["field_starts_with"].as_str().unwrap();
        let message_contains = expected["message_contains"].as_str().unwrap();
        let error = match Mix::from_json(&text) {
            Ok(_) => panic!("{name}.dmx was accepted"),
            Err(e) => e,
        };
        assert!(
            error.field.starts_with(field_starts_with),
            "{name}.dmx: field {:?} does not start with {field_starts_with:?}; message was {:?}",
            error.field,
            error.message
        );
        assert!(
            error.message.contains(message_contains),
            "{name}.dmx: message {:?} does not mention {message_contains:?}",
            error.message
        );
        assert!(!error.message.is_empty(), "{name}.dmx: empty message");
        assert_eq!(
            error.to_string(),
            format!("{}: {}", error.field, error.message)
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        12,
        "expected twelve invalid fixtures in {}",
        dir.display()
    );
}

#[test]
fn the_timeline_places_each_track_by_aligning_anchors() {
    let mut mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    let mut third = mix.tracks[1].clone();
    third.anchors = Anchors {
        intro: Beats(16.0),
        outro: Beats(500.0),
    };
    third.tempo = vec![TempoNode {
        at: Beats(48.0),
        bpm: Bpm(142.0),
    }];
    mix.tracks.push(third);

    let timeline = mix.timeline().unwrap();
    assert_eq!(timeline.tracks.len(), 3);
    // The first track's beat zero is mix beat zero.
    assert_eq!(timeline.tracks[0].origin, Beats::ZERO);
    // The second track's intro anchor (beat 32) sits on the first track's outro anchor (beat 896).
    assert_eq!(timeline.tracks[1].origin, Beats(896.0 - 32.0));
    // The third track's intro anchor (beat 16) sits on the second track's outro anchor (beat 1024).
    assert_eq!(
        timeline.tracks[2].origin,
        Beats(896.0 - 32.0 + 1024.0 - 16.0)
    );
    for (placed, track) in timeline.tracks.iter().zip(&mix.tracks) {
        assert_eq!(placed.grid, track.grid);
        assert_eq!(placed.length, track.length);
    }
    // The curve starts at mix beat zero with the first track's original tempo, and the tempo
    // nodes of every track are gathered onto the mix timeline after it, in order.
    let nodes: Vec<(f64, f64)> = timeline
        .curve
        .nodes()
        .iter()
        .map(|n| (n.at.0, n.bpm.0))
        .collect();
    assert_eq!(
        nodes,
        vec![
            (0.0, 138.0),
            (896.0, 138.0),
            (864.0 + 64.0, 140.0),
            (864.0 + 1008.0 + 48.0, 142.0),
        ]
    );
    // The mix starts when the first track's first sample is heard, which is before mix beat zero
    // because that track's first beat is not at its first sample, and ends with the last track.
    let curve = &timeline.curve;
    assert_eq!(timeline.start(), timeline.tracks[0].start(curve));
    assert!(timeline.start() < Seconds::ZERO);
    assert_eq!(timeline.end(), timeline.tracks[2].end(curve));
    assert!(timeline.end() > timeline.tracks[1].end(curve));
}

#[test]
fn a_mix_without_tempo_nodes_plays_at_the_first_tracks_tempo() {
    let mut mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    for track in &mut mix.tracks {
        track.tempo.clear();
    }
    let timeline = mix.timeline().unwrap();
    for beat in [-100.0, 0.0, 500.0, 2000.0] {
        assert_eq!(timeline.curve.bpm_at_beat(Beats(beat)), Bpm(138.0));
    }
    // With the curve flat at the first track's tempo, that track plays at its original speed.
    let first = &timeline.tracks[0];
    assert!(close(
        first.rate_at(&timeline.curve, first.start(&timeline.curve)),
        1.0
    ));
    // The second track, at 140 BPM, is slowed to 138.
    let second = &timeline.tracks[1];
    assert!(close(
        second.rate_at(&timeline.curve, second.end(&timeline.curve)),
        138.0 / 140.0
    ));
}

#[test]
fn the_first_track_sets_the_starting_tempo_even_when_only_a_later_track_has_nodes() {
    let mut mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    mix.tracks[0].tempo.clear();
    // The only node is the second track's, 140 BPM at its beat 64, which is mix beat 928.
    let timeline = mix.timeline().unwrap();
    let nodes: Vec<(f64, f64)> = timeline
        .curve
        .nodes()
        .iter()
        .map(|n| (n.at.0, n.bpm.0))
        .collect();
    assert_eq!(nodes, vec![(0.0, 138.0), (928.0, 140.0)]);
    assert_eq!(timeline.curve.bpm_at_beat(Beats(-100.0)), Bpm(138.0));
    assert_eq!(timeline.curve.bpm_at_beat(Beats(0.0)), Bpm(138.0));
    let halfway = timeline.curve.bpm_at_beat(Beats(464.0)).0;
    assert!(halfway > 138.0 && halfway < 140.0, "{halfway}");
    assert_eq!(timeline.curve.bpm_at_beat(Beats(2000.0)), Bpm(140.0));
}

#[test]
fn a_node_at_the_first_tracks_beat_zero_comes_after_the_starting_node() {
    let mut mix = Mix::from_json(&read_fixture("valid/two-tracks.dmx")).unwrap();
    mix.tracks[0].tempo = vec![
        TempoNode {
            at: Beats(896.0),
            bpm: Bpm(138.0),
        },
        TempoNode {
            at: Beats(0.0),
            bpm: Bpm(136.0),
        },
    ];
    let timeline = mix.timeline().unwrap();
    let nodes: Vec<(f64, f64)> = timeline
        .curve
        .nodes()
        .iter()
        .map(|n| (n.at.0, n.bpm.0))
        .collect();
    assert_eq!(
        nodes,
        vec![(0.0, 138.0), (0.0, 136.0), (896.0, 138.0), (928.0, 140.0)]
    );
    // Before beat zero the starting tempo applies; at beat zero the track's own node wins.
    assert_eq!(timeline.curve.bpm_at_beat(Beats(-1.0)), Bpm(138.0));
    assert_eq!(timeline.curve.bpm_at_beat(Beats(0.0)), Bpm(136.0));
}

fn envelopes() -> impl Strategy<Value = Envelope> {
    prop::collection::btree_set(-2000i64..20_000, 0..6).prop_flat_map(|set| {
        let positions: Vec<f64> = set.into_iter().map(|p| p as f64 / 4.0).collect();
        let count = positions.len();
        (
            Just(positions),
            prop::collection::vec(-90.0f64..12.0, count),
        )
            .prop_map(|(positions, values)| {
                Envelope::from_nodes(
                    positions
                        .into_iter()
                        .zip(values)
                        .map(|(at, value)| EnvelopeNode {
                            at: Beats(at),
                            value: Decibels(value),
                        })
                        .collect(),
                )
                .unwrap()
            })
    })
}

fn tempo_lists() -> impl Strategy<Value = Vec<TempoNode>> {
    prop::collection::vec((-100.0f64..3000.0, 60.0f64..200.0), 0..4).prop_map(|pairs| {
        pairs
            .into_iter()
            .map(|(at, bpm)| TempoNode {
                at: Beats(at),
                bpm: Bpm(bpm),
            })
            .collect()
    })
}

fn tracks() -> impl Strategy<Value = Track> {
    (
        "[A-Za-z0-9 _-]{1,20}(\\.mp3|\\.wav|\\.flac)",
        any::<[u8; 32]>(),
        0i64..40_000_000,
        (0i64..100_000, 60.0f64..200.0),
        (-64i64..2000, -64i64..2000),
        any::<bool>(),
        -12.0f64..12.0,
        envelopes(),
        (envelopes(), envelopes(), envelopes()),
        tempo_lists(),
    )
        .prop_map(
            |(
                name,
                hash,
                length,
                (first, bpm),
                (intro, outro),
                keylock,
                gain,
                volume,
                eq,
                tempo,
            )| {
                let (low, mid, high) = eq;
                Track {
                    path: PathBuf::from("/Users/dermixenuser/audio/goa/流体動力").join(name),
                    hash: ContentHash(hash),
                    length: Samples(length),
                    grid: BeatGrid {
                        first_beat: Samples(first),
                        bpm: Bpm(bpm),
                    },
                    anchors: Anchors {
                        intro: Beats(intro as f64),
                        outro: Beats(outro as f64),
                    },
                    keylock,
                    gain: Decibels(gain),
                    volume,
                    eq: EqEnvelopes { low, mid, high },
                    tempo,
                }
            },
        )
}

fn mixes() -> impl Strategy<Value = Mix> {
    prop::collection::vec(tracks(), 0..5).prop_map(|tracks| Mix { tracks })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]

    fn any_mix_round_trips_through_its_project_file(mix in mixes()) {
        let json = mix.to_json();
        let back = Mix::from_json(&json).unwrap();
        prop_assert_eq!(back, mix);
    }

    #[test]

    fn every_track_is_placed_by_the_anchors_before_it(mix in mixes()) {
        prop_assume!(!mix.tracks.is_empty());
        let timeline = mix.timeline().unwrap();
        prop_assert_eq!(timeline.tracks.len(), mix.tracks.len());
        let mut origin = 0.0;
        for (i, placed) in timeline.tracks.iter().enumerate() {
            if i > 0 {
                origin += mix.tracks[i - 1].anchors.outro.0 - mix.tracks[i].anchors.intro.0;
            }
            prop_assert_eq!(placed.origin, Beats(origin));
        }
        let expected_nodes = mix.tracks.iter().map(|t| t.tempo.len()).sum::<usize>() + 1;
        prop_assert_eq!(timeline.curve.nodes().len(), expected_nodes);
        prop_assert_eq!(timeline.curve.nodes()[0], TempoNode { at: Beats::ZERO, bpm: mix.tracks[0].grid.bpm });
        prop_assert!(timeline.curve.nodes().windows(2).all(|w| w[0].at <= w[1].at));
        prop_assert!(timeline.start() <= timeline.end());
    }
}
