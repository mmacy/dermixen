//! Per-track automation envelopes for volume and the three EQ bands.

use serde::{Deserialize, Serialize};

use crate::units::{Beats, Decibels};

/// One node of an envelope: a level at a position within the track.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeNode {
    /// The position within the track, in track beats.
    #[serde(rename = "beat")]
    pub at: Beats,
    /// The level at that position.
    #[serde(rename = "db")]
    pub value: Decibels,
}

/// A level that varies over a track, described by nodes joined by straight lines.
///
/// At a node's own position the level is that node's value exactly. Between
/// two nodes the level changes linearly in decibels. Before the first node
/// the level is the first node's value, after the last node it is the last
/// node's value, and an envelope with no nodes is [`Decibels::UNITY`]
/// everywhere. Nodes are kept in increasing order of position, and no two
/// nodes share a position.
///
/// In a project file an envelope is written as a bare list of nodes, in any
/// order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<EnvelopeNode>", into = "Vec<EnvelopeNode>")]
pub struct Envelope {
    nodes: Vec<EnvelopeNode>,
}

/// The reason a list of nodes could not become an [`Envelope`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EnvelopeError {
    /// Two nodes were placed at the same beat.
    #[error("two nodes at beat {0}")]
    DuplicatePosition(f64),
    /// A node's position or value was not a finite number.
    #[error("node {index} is not a finite number")]
    NotFinite {
        /// The index of the offending node in the list as given, or zero for a single node.
        index: usize,
    },
}

impl Envelope {
    /// An envelope with no nodes, which is unity gain everywhere.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an envelope from nodes given in any order.
    ///
    /// Fails if two nodes share a position or any number is not finite.
    pub fn from_nodes(nodes: Vec<EnvelopeNode>) -> Result<Self, EnvelopeError> {
        for (index, node) in nodes.iter().enumerate() {
            if !node.at.0.is_finite() || !node.value.0.is_finite() {
                return Err(EnvelopeError::NotFinite { index });
            }
        }
        let mut nodes = nodes;
        nodes.sort_by(|a, b| a.at.0.partial_cmp(&b.at.0).unwrap());
        if let Some(pair) = nodes.windows(2).find(|pair| pair[0].at.0 == pair[1].at.0) {
            return Err(EnvelopeError::DuplicatePosition(pair[0].at.0));
        }
        Ok(Self { nodes })
    }

    /// The nodes in increasing order of position.
    pub fn nodes(&self) -> &[EnvelopeNode] {
        &self.nodes
    }

    /// The number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the envelope has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Adds a node, replacing any node already at the same position.
    ///
    /// Fails, leaving the envelope unchanged, if the node's position or value
    /// is not a finite number.
    pub fn insert(&mut self, node: EnvelopeNode) -> Result<(), EnvelopeError> {
        if !node.at.0.is_finite() || !node.value.0.is_finite() {
            return Err(EnvelopeError::NotFinite { index: 0 });
        }
        match self
            .nodes
            .binary_search_by(|existing| existing.at.0.partial_cmp(&node.at.0).unwrap())
        {
            Ok(index) => self.nodes[index] = node,
            Err(index) => self.nodes.insert(index, node),
        }
        Ok(())
    }

    /// Removes and returns the node at `index` in position order, or `None` if there is no such node.
    pub fn remove(&mut self, index: usize) -> Option<EnvelopeNode> {
        if index < self.nodes.len() {
            Some(self.nodes.remove(index))
        } else {
            None
        }
    }

    /// The level at a position within the track.
    pub fn value_at(&self, at: Beats) -> Decibels {
        let Some(first) = self.nodes.first() else {
            return Decibels::UNITY;
        };
        if at.0 <= first.at.0 {
            return first.value;
        }
        let last = self.nodes.last().expect("checked non-empty above");
        if at.0 >= last.at.0 {
            return last.value;
        }
        for pair in self.nodes.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            if at.0 == before.at.0 {
                return before.value;
            }
            if at.0 < after.at.0 {
                let fraction = (at.0 - before.at.0) / (after.at.0 - before.at.0);
                return Decibels(before.value.0 + (after.value.0 - before.value.0) * fraction);
            }
        }
        unreachable!("at is bounded by the first and last node above, so some pair must span it")
    }
}

impl TryFrom<Vec<EnvelopeNode>> for Envelope {
    type Error = EnvelopeError;

    fn try_from(nodes: Vec<EnvelopeNode>) -> Result<Self, Self::Error> {
        Envelope::from_nodes(nodes)
    }
}

impl From<Envelope> for Vec<EnvelopeNode> {
    fn from(envelope: Envelope) -> Self {
        envelope.nodes
    }
}
