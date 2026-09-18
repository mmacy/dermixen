//! Acceptance tests for envelopes. A coder agent makes these pass without editing them.

use dermixen_core::{Beats, Decibels, Envelope, EnvelopeError, EnvelopeNode};
use proptest::prelude::*;

fn node(at: f64, value: f64) -> EnvelopeNode {
    EnvelopeNode {
        at: Beats(at),
        value: Decibels(value),
    }
}

/// Between one and twelve nodes at distinct positions, in a random order.
fn distinct_nodes() -> impl Strategy<Value = Vec<EnvelopeNode>> {
    prop::collection::btree_set(-2000i64..20_000, 1..12)
        .prop_flat_map(|set| {
            let positions: Vec<f64> = set.into_iter().map(|p| p as f64 / 4.0).collect();
            let count = positions.len();
            (
                Just(positions),
                prop::collection::vec(-90.0f64..12.0, count),
                any::<u64>(),
            )
        })
        .prop_map(|(positions, values, seed)| {
            let mut nodes: Vec<EnvelopeNode> = positions
                .into_iter()
                .zip(values)
                .map(|(p, v)| node(p, v))
                .collect();
            // A cheap deterministic shuffle so the input order is not sorted.
            let count = nodes.len();
            for i in 0..count {
                let j = (seed as usize).wrapping_mul(31).wrapping_add(i * 17) % count;
                nodes.swap(i, j);
            }
            nodes
        })
}

fn sorted(nodes: &[EnvelopeNode]) -> Vec<EnvelopeNode> {
    let mut sorted = nodes.to_vec();
    sorted.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap());
    sorted
}

#[test]
fn an_empty_envelope_is_unity_everywhere() {
    let envelope = Envelope::new();
    assert!(envelope.is_empty());
    assert_eq!(envelope.len(), 0);
    for at in [-1000.0, -1.0, 0.0, 0.5, 64.0, 1e6] {
        assert_eq!(envelope.value_at(Beats(at)), Decibels::UNITY);
    }
}

#[test]
fn a_fade_in_is_linear_in_decibels() {
    let fade = Envelope::from_nodes(vec![node(96.0, 0.0), node(64.0, -50.0)]).unwrap();
    assert_eq!(fade.value_at(Beats(0.0)), Decibels(-50.0));
    assert_eq!(fade.value_at(Beats(64.0)), Decibels(-50.0));
    assert_eq!(fade.value_at(Beats(80.0)), Decibels(-25.0));
    assert_eq!(fade.value_at(Beats(72.0)), Decibels(-37.5));
    assert_eq!(fade.value_at(Beats(96.0)), Decibels(0.0));
    assert_eq!(fade.value_at(Beats(500.0)), Decibels(0.0));
}

#[test]
fn duplicate_positions_are_rejected() {
    let result = Envelope::from_nodes(vec![node(64.0, 0.0), node(10.0, -3.0), node(64.0, -6.0)]);
    assert_eq!(result, Err(EnvelopeError::DuplicatePosition(64.0)));
}

#[test]
fn non_finite_numbers_are_rejected() {
    let result = Envelope::from_nodes(vec![node(0.0, 0.0), node(f64::NAN, -3.0)]);
    assert_eq!(result, Err(EnvelopeError::NotFinite { index: 1 }));
    let result = Envelope::from_nodes(vec![node(0.0, f64::INFINITY)]);
    assert_eq!(result, Err(EnvelopeError::NotFinite { index: 0 }));
}

#[test]
fn insert_replaces_a_node_at_the_same_position() {
    let mut envelope = Envelope::from_nodes(vec![node(0.0, 0.0), node(32.0, -6.0)]).unwrap();
    envelope.insert(node(32.0, -12.0)).unwrap();
    assert_eq!(envelope.len(), 2);
    assert_eq!(envelope.value_at(Beats(32.0)), Decibels(-12.0));
    envelope.insert(node(16.0, -3.0)).unwrap();
    assert_eq!(envelope.len(), 3);
    assert_eq!(
        envelope.nodes(),
        &[node(0.0, 0.0), node(16.0, -3.0), node(32.0, -12.0)]
    );
}

#[test]
fn insert_refuses_a_node_that_is_not_finite() {
    let mut envelope = Envelope::from_nodes(vec![node(0.0, 0.0), node(32.0, -6.0)]).unwrap();
    assert_eq!(
        envelope.insert(node(f64::NAN, 0.0)),
        Err(EnvelopeError::NotFinite { index: 0 })
    );
    assert_eq!(
        envelope.insert(node(16.0, f64::NEG_INFINITY)),
        Err(EnvelopeError::NotFinite { index: 0 })
    );
    assert_eq!(envelope.nodes(), &[node(0.0, 0.0), node(32.0, -6.0)]);
}

#[test]
fn remove_takes_a_node_out_by_index() {
    let mut envelope =
        Envelope::from_nodes(vec![node(0.0, 0.0), node(16.0, -3.0), node(32.0, -6.0)]).unwrap();
    assert_eq!(envelope.remove(1), Some(node(16.0, -3.0)));
    assert_eq!(envelope.nodes(), &[node(0.0, 0.0), node(32.0, -6.0)]);
    assert_eq!(envelope.remove(5), None);
    assert_eq!(envelope.len(), 2);
    assert_eq!(envelope.value_at(Beats(16.0)), Decibels(-3.0));
}

#[test]
fn the_project_file_form_is_a_bare_list_of_nodes() {
    let envelope = Envelope::from_nodes(vec![node(96.0, 0.0), node(64.0, -50.0)]).unwrap();
    let json = serde_json::to_string(&envelope).unwrap();
    assert_eq!(json, r#"[{"beat":64.0,"db":-50.0},{"beat":96.0,"db":0.0}]"#);
    let back: Envelope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, envelope);
    let empty: Envelope = serde_json::from_str("[]").unwrap();
    assert!(empty.is_empty());
    let duplicate = serde_json::from_str::<Envelope>(r#"[{"beat":1,"db":0},{"beat":1,"db":-1}]"#);
    assert!(duplicate.is_err());
}

proptest! {
    #[test]
    fn nodes_come_out_sorted_whatever_order_they_went_in(nodes in distinct_nodes()) {
        let envelope = Envelope::from_nodes(nodes.clone()).unwrap();
        let expected = sorted(&nodes);
        prop_assert_eq!(envelope.nodes(), expected.as_slice());
        prop_assert_eq!(envelope.len(), nodes.len());
    }

    #[test]

    fn the_value_at_a_node_is_that_node(nodes in distinct_nodes()) {
        let envelope = Envelope::from_nodes(nodes.clone()).unwrap();
        for n in &nodes {
            prop_assert_eq!(envelope.value_at(n.at), n.value);
        }
    }

    #[test]

    fn the_value_is_flat_outside_the_nodes(nodes in distinct_nodes(), offset in 0.0f64..1e6) {
        let envelope = Envelope::from_nodes(nodes.clone()).unwrap();
        let sorted = sorted(&nodes);
        let first = sorted[0];
        let last = sorted[sorted.len() - 1];
        prop_assert_eq!(envelope.value_at(first.at - Beats(offset)), first.value);
        prop_assert_eq!(envelope.value_at(last.at + Beats(offset)), last.value);
    }

    #[test]

    fn the_value_is_linear_between_neighbours(nodes in distinct_nodes(), fraction in 0.0f64..1.0) {
        let envelope = Envelope::from_nodes(nodes.clone()).unwrap();
        for pair in sorted(&nodes).windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let at = Beats(a.at.0 + (b.at.0 - a.at.0) * fraction);
            let expected = a.value.0 + (b.value.0 - a.value.0) * fraction;
            let got = envelope.value_at(at).0;
            prop_assert!((got - expected).abs() < 1e-9, "at {} expected {expected} got {got}", at.0);
        }
    }

    #[test]

    fn the_value_never_leaves_the_range_of_the_nodes(nodes in distinct_nodes(), at in -3000.0f64..6000.0) {
        let envelope = Envelope::from_nodes(nodes.clone()).unwrap();
        let low = nodes.iter().map(|n| n.value.0).fold(f64::INFINITY, f64::min);
        let high = nodes.iter().map(|n| n.value.0).fold(f64::NEG_INFINITY, f64::max);
        let got = envelope.value_at(Beats(at)).0;
        prop_assert!(got >= low - 1e-9 && got <= high + 1e-9);
    }

    #[test]

    fn inserting_a_new_position_grows_the_envelope_in_order(nodes in distinct_nodes(), at in -3000.0f64..6000.0, value in -90.0f64..12.0) {
        prop_assume!(nodes.iter().all(|n| n.at.0 != at));
        let mut envelope = Envelope::from_nodes(nodes.clone()).unwrap();
        envelope.insert(node(at, value)).unwrap();
        prop_assert_eq!(envelope.len(), nodes.len() + 1);
        prop_assert!(envelope.nodes().windows(2).all(|w| w[0].at < w[1].at));
        prop_assert_eq!(envelope.value_at(Beats(at)), Decibels(value));
    }

    #[test]

    fn the_project_file_form_round_trips(nodes in distinct_nodes()) {
        let envelope = Envelope::from_nodes(nodes).unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        let back: Envelope = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, envelope);
    }
}
