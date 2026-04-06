use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::{Endpoint, NodeType, TrackGauge, TrainDistance};

/// Complete graph input deserialized from JSON.
#[derive(Debug, Serialize, Deserialize)]
pub struct GraphInput {
    pub corporation_id: String,
    pub current_phase: String,
    pub no_blocking: bool,
    #[serde(default)]
    pub skip_track: Option<String>,

    pub hexes: HashMap<String, HexInput>,
    pub nodes: Vec<NodeInput>,
    pub paths: Vec<PathInput>,
    #[serde(default)]
    pub junctions: Vec<JunctionInput>,
    #[serde(default)]
    pub converging_exits: HashMap<String, Vec<u8>>,

    pub trains: Vec<TrainInput>,
    pub start_nodes: Vec<usize>,
    #[serde(default)]
    pub static_routes: Vec<StaticRouteInput>,

    #[serde(default)]
    pub bonuses: Vec<BonusInput>,

    pub config: ConfigInput,
}

/// Game-specific revenue bonus, pre-evaluated on the Ruby side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum BonusInput {
    /// Flat bonus per route if any visited node is in the list.
    /// E.g., 1870 SCC cattle: +10 if route touches an SCC hex.
    #[serde(rename = "hex_route")]
    HexRoute { nodes: Vec<usize>, amount: i32 },

    /// Bonus at destination hex: adds the node's revenue again if it's
    /// the first or last stop. E.g., 1870 destination runs.
    #[serde(rename = "destination")]
    Destination { node: usize },

    /// East/West bonus: if route touches at least one east node AND one west node,
    /// add east_amount + west_amount. Amounts pre-computed from tile icons by Ruby.
    #[serde(rename = "east_west")]
    EastWest {
        east_nodes: Vec<usize>,
        west_nodes: Vec<usize>,
        east_amount: i32,
        west_amount: i32,
    },

    /// Per-stop bonus applied to every route (optimistic estimate).
    /// Real revenue callback will apply it only to the longest route.
    #[serde(rename = "per_stop")]
    PerStop { amount: i32 },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HexInput {
    /// Maps edge number (as string key) to neighbor hex ID.
    pub neighbors: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeInput {
    pub id: usize,
    pub hex_id: String,
    #[serde(default)]
    pub index: u8,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub revenue: HashMap<String, i32>,
    #[serde(default = "default_slots")]
    pub slots: u8,
    #[serde(default)]
    pub tokens: Vec<Option<String>>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default = "default_visit_cost")]
    pub visit_cost: u8,
    #[serde(default)]
    pub is_offboard: bool,
    /// Extra tokens (e.g. from special abilities) that don't occupy a regular slot
    /// but still count for `tokened_by?` checks.
    #[serde(default)]
    pub extra_tokens: Vec<String>,
}

fn default_slots() -> u8 {
    0
}
fn default_visit_cost() -> u8 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PathInput {
    pub id: usize,
    pub hex_id: String,
    pub a: Endpoint,
    pub b: Endpoint,
    pub track: TrackGauge,
    /// [[width, index], [width, index]] for endpoints a and b.
    #[serde(default)]
    pub lanes: Option<[[u8; 2]; 2]>,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub ignore: bool,
    #[serde(default)]
    pub junction_id: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JunctionInput {
    pub id: usize,
    pub hex_id: String,
    pub path_ids: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainInput {
    pub id: String,
    pub name: String,
    pub distance: TrainDistance,
    pub price: i32,
    #[serde(default = "default_track_type")]
    pub track_type: Option<TrackGauge>,
    #[serde(default)]
    pub local: bool,
}

fn default_track_type() -> Option<TrackGauge> {
    None
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StaticRouteInput {
    pub train_id: String,
    pub path_ids: Vec<usize>,
    pub connection_hexes: Vec<Vec<String>>,
    pub revenue: i32,
    #[serde(default)]
    pub bitfield: Vec<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigInput {
    #[serde(default = "default_path_timeout")]
    pub path_timeout_ms: u64,
    #[serde(default = "default_route_timeout")]
    pub route_timeout_ms: u64,
    #[serde(default = "default_route_limit")]
    pub route_limit: usize,
    /// null = all share, "each_train_separate", or array of arrays
    pub train_autoroute_groups: Option<serde_json::Value>,
}

fn default_path_timeout() -> u64 {
    30_000
}
fn default_route_timeout() -> u64 {
    10_000
}
fn default_route_limit() -> usize {
    10_000
}

/// Processed graph ready for the walk algorithm.
/// Built from GraphInput with precomputed adjacency lookups.
pub struct Graph {
    pub corporation_id: String,
    pub current_phase: String,
    pub no_blocking: bool,
    pub skip_track: Option<TrackGauge>,

    pub nodes: Vec<ProcessedNode>,
    pub paths: Vec<ProcessedPath>,
    pub junctions: Vec<ProcessedJunction>,

    /// hex_id -> { edge_num -> (neighbor_hex_id, inverted_edge_num) }
    pub hex_neighbors: HashMap<String, HashMap<u8, (String, u8)>>,
    /// hex_id -> set of converging exit edge numbers
    pub converging_exits: HashMap<String, Vec<u8>>,
    /// hex_id + edge_num -> list of path indices on that edge
    pub hex_edge_paths: HashMap<(String, u8), Vec<usize>>,

    /// Game-specific revenue bonuses, pre-evaluated by the Ruby side.
    pub bonuses: Vec<BonusInput>,
}

pub struct ProcessedNode {
    pub id: usize,
    pub hex_id: String,
    pub index: u8,
    pub node_type: NodeType,
    pub revenue: HashMap<String, i32>,
    pub slots: u8,
    pub tokens: Vec<Option<String>>,
    pub groups: Vec<String>,
    pub visit_cost: u8,
    pub is_offboard: bool,
    /// Extra tokens that don't occupy regular slots but count for tokened_by? checks.
    pub extra_tokens: Vec<String>,
    /// Indices of paths connected to this node.
    pub path_indices: Vec<usize>,
}

pub struct ProcessedPath {
    pub id: usize,
    pub hex_id: String,
    pub a: Endpoint,
    pub b: Endpoint,
    pub track: TrackGauge,
    pub terminal: bool,
    pub ignore: bool,
    pub junction_id: Option<usize>,
    /// Node indices on this path (0, 1, or 2 nodes).
    pub node_indices: Vec<usize>,
    /// Edge numbers on this path (0, 1, or 2 edges).
    pub edge_nums: Vec<u8>,
    /// Exit lane info: edge_num -> [width, index].
    pub exit_lanes: HashMap<u8, [u8; 2]>,
}

pub struct ProcessedJunction {
    pub id: usize,
    pub path_indices: Vec<usize>,
}

impl Graph {
    /// Build a processed graph from the deserialized input.
    pub fn from_input(input: &GraphInput) -> Self {
        let skip_track = input.skip_track.as_deref().and_then(|s| match s {
            "broad" => Some(TrackGauge::Broad),
            "narrow" => Some(TrackGauge::Narrow),
            "dual" => Some(TrackGauge::Dual),
            _ => None,
        });

        // Build hex neighbor map with edge inversion
        let mut hex_neighbors: HashMap<String, HashMap<u8, (String, u8)>> = HashMap::new();
        for (hex_id, hex) in &input.hexes {
            let entry = hex_neighbors.entry(hex_id.clone()).or_default();
            for (edge_str, neighbor_id) in &hex.neighbors {
                let edge: u8 = edge_str.parse().unwrap_or(0);
                let inverted = (edge + 3) % 6;
                entry.insert(edge, (neighbor_id.clone(), inverted));
            }
        }

        // Build processed nodes
        let mut nodes: Vec<ProcessedNode> = input
            .nodes
            .iter()
            .map(|n| ProcessedNode {
                id: n.id,
                hex_id: n.hex_id.clone(),
                index: n.index,
                node_type: n.node_type,
                revenue: n.revenue.clone(),
                slots: n.slots,
                tokens: n.tokens.clone(),
                groups: n.groups.clone(),
                visit_cost: n.visit_cost,
                is_offboard: n.is_offboard,
                extra_tokens: n.extra_tokens.clone(),
                path_indices: Vec::new(),
            })
            .collect();

        // Build processed paths and precompute lookups
        let mut hex_edge_paths: HashMap<(String, u8), Vec<usize>> = HashMap::new();
        let mut paths: Vec<ProcessedPath> = Vec::with_capacity(input.paths.len());

        for p in &input.paths {
            let mut node_indices = Vec::new();
            let mut edge_nums = Vec::new();
            let mut exit_lanes: HashMap<u8, [u8; 2]> = HashMap::new();

            // Extract node indices and edge nums from endpoints
            match &p.a {
                Endpoint::Node { index } => node_indices.push(*index),
                Endpoint::Edge { num } => {
                    edge_nums.push(*num);
                    if let Some(lanes) = &p.lanes {
                        exit_lanes.insert(*num, lanes[0]);
                    }
                }
                Endpoint::Junction { .. } => {}
            }
            match &p.b {
                Endpoint::Node { index } => node_indices.push(*index),
                Endpoint::Edge { num } => {
                    edge_nums.push(*num);
                    if let Some(lanes) = &p.lanes {
                        exit_lanes.insert(*num, lanes[1]);
                    }
                }
                Endpoint::Junction { .. } => {}
            }

            // Register this path on each edge it touches
            for &edge in &edge_nums {
                hex_edge_paths
                    .entry((p.hex_id.clone(), edge))
                    .or_default()
                    .push(p.id);
            }

            // Register this path on its connected nodes
            for &ni in &node_indices {
                if ni < nodes.len() {
                    nodes[ni].path_indices.push(p.id);
                }
            }

            paths.push(ProcessedPath {
                id: p.id,
                hex_id: p.hex_id.clone(),
                a: p.a.clone(),
                b: p.b.clone(),
                track: p.track,
                terminal: p.terminal,
                ignore: p.ignore,
                junction_id: p.junction_id,
                node_indices,
                edge_nums,
                exit_lanes,
            });
        }

        // Build processed junctions
        let junctions: Vec<ProcessedJunction> = input
            .junctions
            .iter()
            .map(|j| ProcessedJunction {
                id: j.id,
                path_indices: j.path_ids.clone(),
            })
            .collect();

        Graph {
            corporation_id: input.corporation_id.clone(),
            current_phase: input.current_phase.clone(),
            no_blocking: input.no_blocking,
            skip_track,
            nodes,
            paths,
            junctions,
            hex_neighbors,
            converging_exits: input.converging_exits.clone(),
            hex_edge_paths,
            bonuses: input.bonuses.clone(),
        }
    }

    /// Check if a node blocks the corporation from traversing through it.
    /// A node blocks if all token slots are full and none belong to the corporation.
    pub fn node_blocks(&self, node_idx: usize) -> bool {
        if self.no_blocking {
            return false;
        }
        let node = &self.nodes[node_idx];
        if node.slots == 0 {
            return false;
        }
        let filled = node.tokens.iter().filter(|t| t.is_some()).count() as u8;
        if filled < node.slots {
            return false;
        }
        // All slots full — blocked unless we have a token (in regular or extra slots)
        let in_regular = node
            .tokens
            .iter()
            .any(|t| t.as_deref() == Some(&self.corporation_id));
        let in_extra = node
            .extra_tokens
            .iter()
            .any(|t| t == &self.corporation_id);
        !(in_regular || in_extra)
    }

    /// Get the revenue for a node in the current phase.
    pub fn node_revenue(&self, node_idx: usize) -> i32 {
        let node = &self.nodes[node_idx];
        node.revenue
            .get(&self.current_phase)
            .copied()
            .unwrap_or(0)
    }

    /// Check if a node is tokened by the routing corporation.
    pub fn node_tokened(&self, node_idx: usize) -> bool {
        self.nodes[node_idx]
            .tokens
            .iter()
            .any(|t| t.as_deref() == Some(&self.corporation_id))
    }

    /// Check if a converging exit exists for a hex edge.
    pub fn is_converging_exit(&self, hex_id: &str, edge: u8) -> bool {
        self.converging_exits
            .get(hex_id)
            .map_or(false, |exits| exits.contains(&edge))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_simple_graph() {
        let json = r#"{
            "corporation_id": "PRR",
            "current_phase": "green",
            "no_blocking": false,
            "hexes": {
                "A1": { "neighbors": { "0": "B2" } },
                "B2": { "neighbors": { "3": "A1" } }
            },
            "nodes": [
                { "id": 0, "hex_id": "A1", "type": "city",
                  "revenue": { "yellow": 30, "green": 40 },
                  "slots": 2, "tokens": ["PRR", null], "is_offboard": false }
            ],
            "paths": [
                { "id": 0, "hex_id": "A1",
                  "a": { "type": "node", "index": 0 },
                  "b": { "type": "edge", "num": 0 },
                  "track": "broad" }
            ],
            "junctions": [],
            "trains": [
                { "id": "4-0", "name": "4", "distance": 4, "price": 300, "local": false }
            ],
            "start_nodes": [0],
            "config": {
                "path_timeout_ms": 30000,
                "route_timeout_ms": 10000,
                "route_limit": 10000,
                "train_autoroute_groups": null
            }
        }"#;

        let input: GraphInput = serde_json::from_str(json).expect("Failed to parse graph JSON");
        assert_eq!(input.corporation_id, "PRR");
        assert_eq!(input.nodes.len(), 1);
        assert_eq!(input.paths.len(), 1);
        assert_eq!(input.trains.len(), 1);

        let graph = Graph::from_input(&input);
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.paths.len(), 1);
        assert!(!graph.node_blocks(0));
        assert!(graph.node_tokened(0));
        assert_eq!(graph.node_revenue(0), 40); // green phase
    }

    #[test]
    fn test_blocking_logic() {
        let json = r#"{
            "corporation_id": "PRR",
            "current_phase": "green",
            "no_blocking": false,
            "hexes": {},
            "nodes": [
                { "id": 0, "hex_id": "A1", "type": "city",
                  "revenue": { "green": 40 },
                  "slots": 2, "tokens": ["B&O", "NYC"], "is_offboard": false },
                { "id": 1, "hex_id": "B2", "type": "city",
                  "revenue": { "green": 30 },
                  "slots": 2, "tokens": ["PRR", null], "is_offboard": false },
                { "id": 2, "hex_id": "C3", "type": "city",
                  "revenue": { "green": 20 },
                  "slots": 2, "tokens": [null, null], "is_offboard": false }
            ],
            "paths": [],
            "junctions": [],
            "trains": [],
            "start_nodes": [],
            "config": { "train_autoroute_groups": null }
        }"#;

        let input: GraphInput = serde_json::from_str(json).expect("parse");
        let graph = Graph::from_input(&input);

        assert!(graph.node_blocks(0)); // full, no PRR token
        assert!(!graph.node_blocks(1)); // has PRR token
        assert!(!graph.node_blocks(2)); // not full

        // Test extra_tokens: node 0 blocks, but if PRR has an extra_token it shouldn't
        let json2 = r#"{
            "corporation_id": "PRR",
            "current_phase": "green",
            "no_blocking": false,
            "hexes": {},
            "nodes": [
                { "id": 0, "hex_id": "A1", "type": "city",
                  "revenue": { "green": 40 },
                  "slots": 2, "tokens": ["B&O", "NYC"],
                  "extra_tokens": ["PRR"], "is_offboard": false }
            ],
            "paths": [],
            "junctions": [],
            "trains": [],
            "start_nodes": [],
            "config": { "train_autoroute_groups": null }
        }"#;
        let input2: GraphInput = serde_json::from_str(json2).expect("parse");
        let graph2 = Graph::from_input(&input2);
        assert!(!graph2.node_blocks(0)); // full, but PRR has extra_token
    }

    #[test]
    fn test_complex_train_distance() {
        let json = r#"{
            "corporation_id": "PRR",
            "current_phase": "green",
            "no_blocking": false,
            "hexes": {},
            "nodes": [],
            "paths": [],
            "junctions": [],
            "trains": [
                { "id": "2+2-0", "name": "2+2",
                  "distance": [
                    { "nodes": ["town"], "pay": 2, "visit": 2 },
                    { "nodes": ["city", "offboard"], "pay": 2, "visit": 2 }
                  ],
                  "price": 200, "local": false }
            ],
            "start_nodes": [],
            "config": { "train_autoroute_groups": null }
        }"#;

        let input: GraphInput = serde_json::from_str(json).expect("parse");
        assert_eq!(input.trains.len(), 1);
        match &input.trains[0].distance {
            TrainDistance::Complex(rules) => {
                assert_eq!(rules.len(), 2);
                assert_eq!(rules[0].nodes, vec!["town"]);
                assert_eq!(rules[1].nodes, vec!["city", "offboard"]);
            }
            _ => panic!("Expected complex distance"),
        }
    }

    #[test]
    fn test_edge_inversion() {
        let json = r#"{
            "corporation_id": "PRR",
            "current_phase": "green",
            "no_blocking": false,
            "hexes": {
                "A1": { "neighbors": { "2": "B2" } },
                "B2": { "neighbors": { "5": "A1" } }
            },
            "nodes": [],
            "paths": [],
            "junctions": [],
            "trains": [],
            "start_nodes": [],
            "config": { "train_autoroute_groups": null }
        }"#;

        let input: GraphInput = serde_json::from_str(json).expect("parse");
        let graph = Graph::from_input(&input);

        // Edge 2 on A1 should connect to B2 with inverted edge 5
        let a1_neighbors = &graph.hex_neighbors["A1"];
        let (neighbor_id, inverted) = &a1_neighbors[&2];
        assert_eq!(neighbor_id, "B2");
        assert_eq!(*inverted, 5); // (2+3)%6 = 5
    }
}
