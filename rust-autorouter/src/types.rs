use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    City,
    Town,
    Offboard,
    Junction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackGauge {
    Broad,
    Narrow,
    Dual,
}

impl TrackGauge {
    /// Check if two track gauges are compatible for traversal.
    /// broad matches broad|dual, narrow matches narrow|dual, dual matches dual only (unless dual_ok).
    pub fn matches(self, other: TrackGauge, dual_ok: bool) -> bool {
        match (self, other) {
            (TrackGauge::Broad, TrackGauge::Broad) => true,
            (TrackGauge::Broad, TrackGauge::Dual) => true,
            (TrackGauge::Narrow, TrackGauge::Narrow) => true,
            (TrackGauge::Narrow, TrackGauge::Dual) => true,
            (TrackGauge::Dual, TrackGauge::Dual) => true,
            (TrackGauge::Dual, TrackGauge::Broad) if dual_ok => true,
            (TrackGauge::Dual, TrackGauge::Narrow) if dual_ok => true,
            _ => false,
        }
    }
}

/// Train distance can be simple (a number) or complex (array of node-type constraints).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrainDistance {
    Simple(u32),
    Complex(Vec<DistanceRule>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistanceRule {
    pub nodes: Vec<String>,
    pub pay: u32,
    pub visit: u32,
    #[serde(default)]
    pub multiplier: Option<i32>,
}

/// Endpoint of a path — either a node, an edge, or a junction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Endpoint {
    #[serde(rename = "node")]
    Node { index: usize },
    #[serde(rename = "edge")]
    Edge { num: u8 },
    #[serde(rename = "junction")]
    Junction { index: usize },
}

/// Configuration for train autoroute groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrainGroupConfig {
    /// null — all trains share one conflict group
    AllShare,
    /// "each_train_separate" — each train has its own group
    EachSeparate,
    /// Array of arrays — trains in same sub-array share a group
    Groups(Vec<Vec<String>>),
}

// Custom deserialization: null -> AllShare, string -> EachSeparate, array -> Groups
// The untagged enum handles the array case, but we need special handling for null and string.
// We'll handle this in the graph deserialization instead.
