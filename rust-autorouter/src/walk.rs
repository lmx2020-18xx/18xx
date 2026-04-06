use std::collections::{HashMap, HashSet};

use crate::time::{Duration, Instant};

use crate::bitfield::Bitfield;
use crate::distance;
use crate::graph::{Graph, TrainInput};
use crate::route::CandidateRoute;

/// Result of the walk phase.
pub struct WalkResult {
    /// Map of train_id to list of candidate routes, sorted by revenue descending.
    pub train_routes: HashMap<String, Vec<CandidateRoute>>,
    /// Whether the walk timed out.
    pub timed_out: bool,
    /// Total number of unique connections found.
    pub connections_found: usize,
    /// Total number of callback invocations (for diagnostics).
    pub total_callbacks: usize,
}

/// Action returned from the connection callback to control walk behavior.
enum WalkAction {
    Continue,
    Abort,
}

/// Ordered set that preserves insertion order (like Ruby Hash).
/// Uses a Vec for order and HashSet for O(1) lookups.
struct OrderedPathSet {
    order: Vec<usize>,
    set: HashSet<usize>,
}

impl OrderedPathSet {
    fn new() -> Self {
        OrderedPathSet {
            order: Vec::new(),
            set: HashSet::new(),
        }
    }

    fn contains(&self, id: &usize) -> bool {
        self.set.contains(id)
    }

    fn insert(&mut self, id: usize) {
        if self.set.insert(id) {
            self.order.push(id);
        }
    }

    fn remove(&mut self, id: &usize) {
        if self.set.remove(id) {
            self.order.retain(|x| x != id);
        }
    }

    fn ordered_keys(&self) -> &[usize] {
        &self.order
    }
}

/// Walk all routes from start nodes for all trains.
pub fn walk_all_routes(
    graph: &Graph,
    trains: &[TrainInput],
    start_nodes: &[usize],
    skip_path_ids: &HashSet<usize>,
    path_timeout: Duration,
    route_limit: usize,
) -> WalkResult {
    let start = Instant::now();
    let mut connections: HashSet<Vec<usize>> = HashSet::new();
    let mut train_routes: HashMap<String, Vec<CandidateRoute>> = HashMap::new();
    let mut hexside_bits: HashMap<String, usize> = HashMap::new();
    let mut next_hexside_bit: usize = 0;
    let mut global_route_index: usize = 0;
    let mut timed_out = false;
    let mut total_callbacks: usize = 0;

    // Initialize train_routes for each train
    for train in trains {
        train_routes.entry(train.id.clone()).or_default();
    }

    for &node_id in start_nodes {
        if start.elapsed() > path_timeout {
            timed_out = true;
            break;
        }

        // Walk from this node
        let mut visited_nodes: HashMap<usize, bool> = HashMap::new();
        let mut visited_paths = OrderedPathSet::new();
        let mut counter: HashMap<String, i32> = HashMap::new();

        node_walk(
            graph,
            node_id,
            skip_path_ids,
            &mut visited_nodes,
            &mut visited_paths,
            &mut counter,
            true, // converging_path
            &mut |vp: &OrderedPathSet| {
                total_callbacks += 1;

                // Build chains from visited paths (in insertion order)
                let path_ids = vp.ordered_keys();
                let (chains, connection_key) = build_chains(graph, path_ids);

                if chains.is_empty() {
                    return WalkAction::Continue;
                }

                if connections.contains(&connection_key) {
                    return WalkAction::Continue;
                }
                connections.insert(connection_key);

                // Build connection_hexes and node_signatures from chains
                let (conn_hexes, node_sigs, visited_node_ids) =
                    extract_route_data(graph, &chains);

                // Compute bitfield
                let bitfield =
                    bitfield_from_chains(graph, &chains, &mut hexside_bits, &mut next_hexside_bit);

                // Try each train
                let mut any_train_accepted = false;
                let mut all_too_long = true;

                for train in trains {
                    // Check distance
                    let dist_ok =
                        distance::check_distance(graph, &visited_node_ids, &train.distance);
                    match dist_ok {
                        Ok(()) => {
                            all_too_long = false;
                            // Check token requirement
                            if !train.local
                                && !visited_node_ids.iter().any(|&ni| graph.node_tokened(ni))
                            {
                                continue; // NoToken
                            }
                            // Check minimum stops
                            if !train.local && visited_node_ids.len() < 2 && !conn_hexes.is_empty()
                            {
                                continue; // TooShort
                            }

                            let base_revenue = distance::estimate_revenue(
                                graph,
                                &visited_node_ids,
                                &train.distance,
                            );
                            let bonus = distance::estimate_bonuses(graph, &visited_node_ids);
                            let revenue = base_revenue + bonus;
                            if revenue <= 0 {
                                continue;
                            }

                            let route = CandidateRoute {
                                global_index: global_route_index,
                                train_id: train.id.clone(),
                                connection_hexes: conn_hexes.clone(),
                                node_signatures: node_sigs.clone(),
                                estimate_revenue: revenue,
                                bitfield: bitfield.clone(),
                                visited_nodes: visited_node_ids.clone(),
                            };
                            global_route_index += 1;
                            train_routes
                                .entry(train.id.clone())
                                .or_default()
                                .push(route);
                            any_train_accepted = true;
                        }
                        Err(()) => {
                            // TooLong for this train
                        }
                    }
                }

                // If all trains said too long, abort this path branch
                if all_too_long && !any_train_accepted {
                    return WalkAction::Abort;
                }

                WalkAction::Continue
            },
        );

    }

    // Sort and truncate per train
    for routes in train_routes.values_mut() {
        routes.sort_by(|a, b| b.estimate_revenue.cmp(&a.estimate_revenue));
        routes.truncate(route_limit);
    }

    let connections_found = connections.len();
    WalkResult {
        train_routes,
        timed_out,
        connections_found,
        total_callbacks,
    }
}

/// Recursive node walk — port of Node#walk from node.rb:43-101.
fn node_walk(
    graph: &Graph,
    node_id: usize,
    skip_paths: &HashSet<usize>,
    visited_nodes: &mut HashMap<usize, bool>,
    visited_paths: &mut OrderedPathSet,
    counter: &mut HashMap<String, i32>,
    converging_path: bool,
    callback: &mut dyn FnMut(&OrderedPathSet) -> WalkAction,
) {
    if visited_nodes.contains_key(&node_id) {
        return;
    }
    visited_nodes.insert(node_id, true);

    let node = &graph.nodes[node_id];
    let path_indices = node.path_indices.clone();

    for &path_idx in &path_indices {
        let path = &graph.paths[path_idx];

        // Skip if track type matches skip_track
        if let Some(skip) = graph.skip_track {
            if path.track == skip {
                continue;
            }
        }
        if path.ignore {
            continue;
        }

        path_walk(
            graph,
            path_idx,
            None,  // skip edge
            None,  // jskip
            skip_paths,
            visited_paths,
            counter,
            converging_path,
            &mut |path_id, vp, ct, converging| {
                let action = callback(vp);

                if matches!(action, WalkAction::Abort) {
                    return WalkAction::Abort;
                }

                let path = &graph.paths[path_id];
                if path.terminal {
                    return WalkAction::Continue;
                }

                // Visit next nodes on this path
                for &next_node_id in &path.node_indices {
                    if next_node_id == node_id {
                        continue;
                    }
                    // Corporation blocking check
                    if graph.node_blocks(next_node_id) {
                        continue;
                    }

                    node_walk(
                        graph,
                        next_node_id,
                        skip_paths,
                        visited_nodes,
                        vp,
                        ct,
                        converging_path || converging,
                        callback,
                    );
                }

                WalkAction::Continue
            },
        );
    }

    if converging_path {
        visited_nodes.remove(&node_id);
    }
}

/// Recursive path walk — port of Path#walk from path.rb:133-210.
fn path_walk(
    graph: &Graph,
    path_idx: usize,
    skip_edge: Option<u8>,
    jskip: Option<usize>,
    skip_paths: &HashSet<usize>,
    visited: &mut OrderedPathSet,
    counter: &mut HashMap<String, i32>,
    converging: bool,
    callback: &mut dyn FnMut(usize, &mut OrderedPathSet, &mut HashMap<String, i32>, bool) -> WalkAction,
) {
    let path = &graph.paths[path_idx];

    // Skip if already visited or in skip set
    if visited.contains(&path_idx) || skip_paths.contains(&path_idx) {
        return;
    }

    // Skip if junction reused too many times
    if let Some(jid) = path.junction_id {
        let jkey = format!("j{}", jid);
        if *counter.get(&jkey).unwrap_or(&0) > 1 {
            return;
        }
    }

    // Skip if any edge+lane already used
    let edge_sum: i32 = path
        .edge_nums
        .iter()
        .map(|e| {
            let lane_idx = path.exit_lanes.get(e).map(|l| l[1]).unwrap_or(0);
            let ekey = format!("{}:{}:{}", path.hex_id, e, lane_idx);
            *counter.get(&ekey).unwrap_or(&0)
        })
        .sum();
    if edge_sum > 0 {
        return;
    }

    // Skip if track type matches skip_track
    if let Some(skip) = graph.skip_track {
        if path.track == skip {
            return;
        }
    }

    // Skip terminal junctions
    if path.junction_id.is_some() && path.terminal {
        return;
    }

    // Mark as visited (preserves insertion order)
    visited.insert(path_idx);
    if let Some(jid) = path.junction_id {
        let jkey = format!("j{}", jid);
        *counter.entry(jkey).or_insert(0) += 1;
    }

    // Yield to callback
    // Note: In Ruby, path_walk always traverses junctions and edges after yielding,
    // regardless of the callback's return value. The :abort return only prevents
    // next-node traversal (handled inside node_walk's closure), NOT junction/edge
    // exploration. We must match this behavior — always traverse.
    let _action = callback(path_idx, visited, counter, converging);

    // Traverse junction paths
    if let Some(jid) = path.junction_id {
        if Some(jid) != jskip && jid < graph.junctions.len() {
            let junction_paths = graph.junctions[jid].path_indices.clone();
            for &jp_idx in &junction_paths {
                if jp_idx == path_idx {
                    continue;
                }
                path_walk(
                    graph,
                    jp_idx,
                    None,
                    Some(jid),
                    skip_paths,
                    visited,
                    counter,
                    converging,
                    callback,
                );
            }
        }
    }

    // Traverse edges to neighbor hexes
    let edge_nums = path.edge_nums.clone();
    for &edge in &edge_nums {
        if Some(edge) == skip_edge {
            continue;
        }

        // Get neighbor hex
        if let Some(neighbors) = graph.hex_neighbors.get(&path.hex_id) {
            if let Some((neighbor_hex_id, inverted_edge)) = neighbors.get(&edge) {
                // Counter key includes lane index, matching Ruby's edge.id format
                let lane_idx = path.exit_lanes.get(&edge).map(|l| l[1]).unwrap_or(0);
                let ekey = format!("{}:{}:{}", path.hex_id, edge, lane_idx);
                *counter.entry(ekey.clone()).or_insert(0) += 1;

                // Get paths on the inverted edge of the neighbor hex
                let neighbor_path_indices = graph
                    .hex_edge_paths
                    .get(&(neighbor_hex_id.clone(), *inverted_edge))
                    .cloned()
                    .unwrap_or_default();

                for &np_idx in &neighbor_path_indices {
                    // Lane matching
                    if !lane_match(graph, path_idx, edge, np_idx, *inverted_edge) {
                        continue;
                    }

                    // Track gauge matching
                    let np = &graph.paths[np_idx];
                    if !path.track.matches(np.track, true) {
                        continue;
                    }

                    // Check if this edge is converging
                    let is_converging =
                        converging || graph.is_converging_exit(&path.hex_id, edge);

                    path_walk(
                        graph,
                        np_idx,
                        Some(*inverted_edge),
                        None,
                        skip_paths,
                        visited,
                        counter,
                        is_converging,
                        callback,
                    );
                }

                *counter.get_mut(&ekey).unwrap() -= 1;
            }
        }
    }

    // Unvisit
    if converging {
        visited.remove(&path_idx);
    }
    if let Some(jid) = path.junction_id {
        let jkey = format!("j{}", jid);
        *counter.get_mut(&jkey).unwrap() -= 1;
    }
}

/// Lane matching between two paths on adjacent hex edges.
/// Port of Path#lane_match? from path.rb:213-240.
fn lane_match(
    graph: &Graph,
    path_a_idx: usize,
    edge_a: u8,
    path_b_idx: usize,
    edge_b: u8,
) -> bool {
    let path_a = &graph.paths[path_a_idx];
    let path_b = &graph.paths[path_b_idx];

    let lanes_a = match path_a.exit_lanes.get(&edge_a) {
        Some(l) => l,
        None => return false,
    };
    let lanes_b = match path_b.exit_lanes.get(&edge_b) {
        Some(l) => l,
        None => return false,
    };

    let width_a = lanes_a[0];
    let index_a = lanes_a[1];
    let width_b = lanes_b[0];
    let index_b = lanes_b[1];

    if width_a == width_b {
        // Same width: indices must be mirror images
        index_b == lane_invert_index(index_a, width_a)
    } else {
        // Different widths: always match (single track meets multi-track)
        true
    }
}

/// Invert a lane index within a given width.
/// For width 1: always 0. For width 2: 0↔1. For width 3: 0↔2, 1 stays.
fn lane_invert_index(index: u8, width: u8) -> u8 {
    if width <= 1 {
        0
    } else {
        width - 1 - index
    }
}

/// Chain represents a connection between two nodes through a sequence of paths.
pub struct Chain {
    pub left_node: Option<usize>,
    pub right_node: Option<usize>,
    pub path_ids: Vec<usize>,
    pub hex_ids: Vec<String>,
}

/// Build chains from visited path IDs (must be in visit/insertion order).
/// Port of the chain-building logic in auto_router.rb:99-152.
/// Returns (chains, connection_key).
fn build_chains(graph: &Graph, path_ids: &[usize]) -> (Vec<Chain>, Vec<usize>) {
    let mut chains: Vec<Chain> = Vec::new();
    let mut current_paths: Vec<usize> = Vec::new();
    let mut left: Option<usize> = None;
    let mut right: Option<usize> = None;
    let mut last_left: Option<usize> = None;
    let mut last_right: Option<usize> = None;

    let complete = |left: &mut Option<usize>,
                        right: &mut Option<usize>,
                        last_left: &mut Option<usize>,
                        last_right: &mut Option<usize>,
                        current_paths: &mut Vec<usize>,
                        chains: &mut Vec<Chain>| {
        let hex_ids: Vec<String> = current_paths
            .iter()
            .map(|&pid| graph.paths[pid].hex_id.clone())
            .collect();
        chains.push(Chain {
            left_node: *left,
            right_node: *right,
            path_ids: current_paths.clone(),
            hex_ids,
        });
        *last_left = *left;
        *last_right = *right;
        *left = None;
        *right = None;
        current_paths.clear();
    };

    for &path_id in path_ids {
        current_paths.push(path_id);
        let path = &graph.paths[path_id];

        // Get nodes on this path
        let mut a: Option<usize> = None;
        let mut b: Option<usize> = None;
        for (i, &ni) in path.node_indices.iter().enumerate() {
            if i == 0 {
                a = Some(ni);
            } else {
                b = Some(ni);
            }
        }

        if a.is_some() || b.is_some() {
            // assign logic matching Ruby's assign lambda
            if a.is_some() && b.is_some() {
                if a == last_left || b == last_right {
                    left = b;
                    right = a;
                } else {
                    left = a;
                    right = b;
                }
                complete(
                    &mut left,
                    &mut right,
                    &mut last_left,
                    &mut last_right,
                    &mut current_paths,
                    &mut chains,
                );
            } else if left.is_none() {
                left = a.or(b);
            } else if right.is_none() {
                right = a.or(b);
                complete(
                    &mut left,
                    &mut right,
                    &mut last_left,
                    &mut last_right,
                    &mut current_paths,
                    &mut chains,
                );
            }
        }
    }

    // Handle local trains (1-city, no chains)
    if chains.is_empty() {
        if let Some(l) = left {
            chains.push(Chain {
                left_node: Some(l),
                right_node: None,
                path_ids: Vec::new(),
                hex_ids: Vec::new(),
            });
            // For local trains, use node ID as key
            return (chains, vec![l]);
        }
        return (chains, Vec::new());
    }

    // Connection key: sorted list of all path IDs
    let mut key: Vec<usize> = chains.iter().flat_map(|c| c.path_ids.iter().copied()).collect();
    key.sort();

    (chains, key)
}

/// Extract connection_hexes, node_signatures, and visited node indices from chains.
fn extract_route_data(
    graph: &Graph,
    chains: &[Chain],
) -> (Vec<Vec<String>>, Vec<String>, Vec<usize>) {
    let mut connection_hexes: Vec<Vec<String>> = Vec::new();
    let mut node_set: HashSet<usize> = HashSet::new();
    let mut visited_nodes: Vec<usize> = Vec::new();

    for chain in chains {
        // Connection hexes: hex IDs of paths in this chain
        if !chain.path_ids.is_empty() {
            let hex_ids: Vec<String> = chain
                .path_ids
                .iter()
                .map(|&pid| graph.paths[pid].hex_id.clone())
                .collect();
            connection_hexes.push(hex_ids);
        }

        // Collect visited nodes
        if let Some(l) = chain.left_node {
            if node_set.insert(l) {
                visited_nodes.push(l);
            }
        }
        if let Some(r) = chain.right_node {
            if node_set.insert(r) {
                visited_nodes.push(r);
            }
        }
    }

    // Node signatures: "hex_id-index" for each unique node
    let node_signatures: Vec<String> = visited_nodes
        .iter()
        .map(|&ni| {
            let node = &graph.nodes[ni];
            format!("{}-{}", node.hex_id, node.index)
        })
        .collect();

    (connection_hexes, node_signatures, visited_nodes)
}

/// Compute bitfield from chains, matching auto_router.rb's bitfield_from_connection.
fn bitfield_from_chains(
    graph: &Graph,
    chains: &[Chain],
    hexside_bits: &mut HashMap<String, usize>,
    next_bit: &mut usize,
) -> Bitfield {
    let mut bitfield = Bitfield::new();

    for chain in chains {
        let paths = &chain.path_ids;
        if paths.len() == 1 {
            // Special case for tiny intra-tile path
            let path = &graph.paths[paths[0]];
            for &ni in &path.node_indices {
                let node_id = format!("node:{}", ni);
                set_hexside_bit(&mut bitfield, &node_id, hexside_bits, next_bit);
            }
        } else {
            for i in 0..paths.len().saturating_sub(1) {
                let p1 = &graph.paths[paths[i]];
                let p2 = &graph.paths[paths[i + 1]];

                match p1.edge_nums.len() {
                    1 => {
                        let hs_left = format!("{}:{}", p1.hex_id, p1.edge_nums[0]);
                        let hs_right = format!("{}:{}", p2.hex_id, p2.edge_nums[0]);
                        set_hexside_bit(&mut bitfield, &hs_left, hexside_bits, next_bit);
                        set_hexside_bit(&mut bitfield, &hs_right, hexside_bits, next_bit);
                    }
                    2 => {
                        let hs_left = format!("{}:{}", p1.hex_id, p1.edge_nums[0]);
                        let hs_right = format!("{}:{}", p1.hex_id, p1.edge_nums[1]);
                        set_hexside_bit(&mut bitfield, &hs_left, hexside_bits, next_bit);
                        set_hexside_bit(&mut bitfield, &hs_right, hexside_bits, next_bit);
                        let hs_next = format!("{}:{}", p2.hex_id, p2.edge_nums[0]);
                        set_hexside_bit(&mut bitfield, &hs_next, hexside_bits, next_bit);
                    }
                    _ => {}
                }
            }
        }
    }

    bitfield
}

fn set_hexside_bit(
    bitfield: &mut Bitfield,
    hexside_id: &str,
    hexside_bits: &mut HashMap<String, usize>,
    next_bit: &mut usize,
) {
    let bit = if let Some(&b) = hexside_bits.get(hexside_id) {
        b
    } else {
        let b = *next_bit;
        hexside_bits.insert(hexside_id.to_string(), b);
        *next_bit += 1;
        b
    };
    bitfield.set(bit);
}
