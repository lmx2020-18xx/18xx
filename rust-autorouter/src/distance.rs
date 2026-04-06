use crate::graph::{BonusInput, Graph};
use crate::types::{DistanceRule, NodeType, TrainDistance};

/// Check if a set of visited node indices exceeds the train's distance limit.
/// Returns Ok(()) if within limits, Err(()) if too long.
pub fn check_distance(
    graph: &Graph,
    visited_nodes: &[usize],
    distance: &TrainDistance,
) -> Result<(), ()> {
    match distance {
        TrainDistance::Simple(max) => {
            let total_cost: u32 = visited_nodes
                .iter()
                .map(|&ni| graph.nodes[ni].visit_cost as u32)
                .sum();
            if total_cost > *max {
                Err(())
            } else {
                Ok(())
            }
        }
        TrainDistance::Complex(rules) => check_complex_distance(graph, visited_nodes, rules),
    }
}

fn check_complex_distance(
    graph: &Graph,
    visited_nodes: &[usize],
    rules: &[DistanceRule],
) -> Result<(), ()> {
    // Count visits per node type
    let mut remaining_visits: Vec<u32> = rules.iter().map(|r| r.visit).collect();

    for &ni in visited_nodes {
        let node = &graph.nodes[ni];
        let type_name = node_type_name(node.node_type);

        // Find the first rule that matches this node type and has remaining capacity
        let mut matched = false;
        for (i, rule) in rules.iter().enumerate() {
            if rule.nodes.iter().any(|n| n == type_name) && remaining_visits[i] > 0 {
                remaining_visits[i] -= 1;
                matched = true;
                break;
            }
        }
        if !matched {
            return Err(());
        }
    }
    Ok(())
}

/// Get the maximum number of stops a train can visit.
pub fn max_stops(distance: &TrainDistance) -> u32 {
    match distance {
        TrainDistance::Simple(n) => *n,
        TrainDistance::Complex(rules) => rules.iter().map(|r| r.visit).sum(),
    }
}

/// Estimate revenue for a set of visited nodes given the train's distance constraints.
/// For simple distance: sum all node revenues.
/// For complex distance: select the best-revenue subset respecting type limits.
pub fn estimate_revenue(
    graph: &Graph,
    visited_nodes: &[usize],
    distance: &TrainDistance,
) -> i32 {
    match distance {
        TrainDistance::Simple(_) => visited_nodes.iter().map(|&ni| graph.node_revenue(ni)).sum(),
        TrainDistance::Complex(rules) => estimate_complex_revenue(graph, visited_nodes, rules),
    }
}

fn estimate_complex_revenue(
    graph: &Graph,
    visited_nodes: &[usize],
    rules: &[DistanceRule],
) -> i32 {
    // Group nodes by their matching rule, sorted by revenue descending
    let mut buckets: Vec<Vec<(usize, i32)>> = rules.iter().map(|_| Vec::new()).collect();

    for &ni in visited_nodes {
        let node = &graph.nodes[ni];
        let type_name = node_type_name(node.node_type);
        let rev = graph.node_revenue(ni);

        for (i, rule) in rules.iter().enumerate() {
            if rule.nodes.iter().any(|n| n == type_name) {
                buckets[i].push((ni, rev));
                break;
            }
        }
    }

    // Sort each bucket by revenue descending, take up to pay limit
    let mut total = 0;
    for (i, bucket) in buckets.iter_mut().enumerate() {
        bucket.sort_by(|a, b| b.1.cmp(&a.1));
        let pay_limit = rules[i].pay as usize;
        for &(_, rev) in bucket.iter().take(pay_limit) {
            let multiplier = rules[i].multiplier.unwrap_or(1);
            total += rev * multiplier;
        }
    }
    total
}

/// Calculate bonus revenue for a route's visited nodes.
/// Called after base revenue estimation to add game-specific bonuses.
pub fn estimate_bonuses(graph: &Graph, visited_nodes: &[usize]) -> i32 {
    let mut bonus = 0;

    for b in &graph.bonuses {
        match b {
            BonusInput::HexRoute { nodes, amount } => {
                // +amount once per route if any visited node is in the bonus node list
                if visited_nodes.iter().any(|ni| nodes.contains(ni)) {
                    bonus += amount;
                }
            }
            BonusInput::Destination { node } => {
                // Double the node's revenue if it's the first or last visited node
                if visited_nodes.len() >= 2 {
                    let first = visited_nodes[0];
                    let last = visited_nodes[visited_nodes.len() - 1];
                    if first == *node || last == *node {
                        bonus += graph.node_revenue(*node);
                    }
                }
            }
            BonusInput::EastWest {
                east_nodes,
                west_nodes,
                east_amount,
                west_amount,
            } => {
                let has_east = visited_nodes.iter().any(|ni| east_nodes.contains(ni));
                let has_west = visited_nodes.iter().any(|ni| west_nodes.contains(ni));
                if has_east && has_west {
                    bonus += east_amount + west_amount;
                }
            }
            BonusInput::PerStop { amount } => {
                bonus += amount * visited_nodes.len() as i32;
            }
        }
    }

    bonus
}

fn node_type_name(t: NodeType) -> &'static str {
    match t {
        NodeType::City => "city",
        NodeType::Town => "town",
        NodeType::Offboard => "offboard",
        NodeType::Junction => "junction",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_stops_simple() {
        assert_eq!(max_stops(&TrainDistance::Simple(5)), 5);
    }

    #[test]
    fn test_max_stops_complex() {
        let dist = TrainDistance::Complex(vec![
            DistanceRule {
                nodes: vec!["town".to_string()],
                pay: 2,
                visit: 2,
                multiplier: None,
            },
            DistanceRule {
                nodes: vec!["city".to_string()],
                pay: 3,
                visit: 3,
                multiplier: None,
            },
        ]);
        assert_eq!(max_stops(&dist), 5);
    }

    fn make_bonus_graph(bonuses: Vec<BonusInput>) -> Graph {
        use std::collections::HashMap;
        let mut nodes = Vec::new();
        for (i, rev) in [40, 50, 30].iter().enumerate() {
            let mut revenue = HashMap::new();
            revenue.insert("green".to_string(), *rev);
            nodes.push(crate::graph::ProcessedNode {
                id: i,
                hex_id: format!("H{}", i),
                index: 0,
                node_type: NodeType::City,
                revenue,
                slots: 2,
                tokens: vec![Some("CORP".to_string()), None],
                groups: Vec::new(),
                visit_cost: 1,
                is_offboard: false,
                extra_tokens: Vec::new(),
                path_indices: Vec::new(),
            });
        }
        Graph {
            corporation_id: "CORP".to_string(),
            current_phase: "green".to_string(),
            no_blocking: false,
            skip_track: None,
            nodes,
            paths: Vec::new(),
            junctions: Vec::new(),
            hex_neighbors: HashMap::new(),
            converging_exits: HashMap::new(),
            hex_edge_paths: HashMap::new(),
            bonuses,
        }
    }

    #[test]
    fn test_hex_route_bonus_triggers() {
        let graph = make_bonus_graph(vec![
            BonusInput::HexRoute { nodes: vec![1], amount: 10 },
        ]);
        // Route visits node 0 and 1 — touches bonus node 1
        assert_eq!(estimate_bonuses(&graph, &[0, 1]), 10);
        // Route visits node 0 and 2 — doesn't touch bonus node
        assert_eq!(estimate_bonuses(&graph, &[0, 2]), 0);
    }

    #[test]
    fn test_destination_bonus_at_endpoints() {
        let graph = make_bonus_graph(vec![
            BonusInput::Destination { node: 1 },
        ]);
        // Node 1 is first stop — bonus = node 1 revenue (50)
        assert_eq!(estimate_bonuses(&graph, &[1, 0, 2]), 50);
        // Node 1 is last stop — bonus = 50
        assert_eq!(estimate_bonuses(&graph, &[0, 2, 1]), 50);
        // Node 1 is in the middle — no bonus
        assert_eq!(estimate_bonuses(&graph, &[0, 1, 2]), 0);
        // Single-stop route — no destination bonus
        assert_eq!(estimate_bonuses(&graph, &[1]), 0);
    }

    #[test]
    fn test_multiple_bonuses_stack() {
        let graph = make_bonus_graph(vec![
            BonusInput::HexRoute { nodes: vec![1], amount: 10 },
            BonusInput::Destination { node: 2 },
        ]);
        // Route: [2, 1] — hex_route bonus (touches node 1) + destination bonus (node 2 is first)
        assert_eq!(estimate_bonuses(&graph, &[2, 1]), 10 + 30);
    }

    #[test]
    fn test_east_west_bonus() {
        let graph = make_bonus_graph(vec![BonusInput::EastWest {
            east_nodes: vec![0],
            west_nodes: vec![2],
            east_amount: 40,
            west_amount: 60,
        }]);
        // Route touches both east and west
        assert_eq!(estimate_bonuses(&graph, &[0, 1, 2]), 100);
        // Only east
        assert_eq!(estimate_bonuses(&graph, &[0, 1]), 0);
        // Only west
        assert_eq!(estimate_bonuses(&graph, &[1, 2]), 0);
    }

    #[test]
    fn test_per_stop_bonus() {
        let graph = make_bonus_graph(vec![BonusInput::PerStop { amount: 10 }]);
        assert_eq!(estimate_bonuses(&graph, &[0, 1, 2]), 30);
        assert_eq!(estimate_bonuses(&graph, &[0, 1]), 20);
        assert_eq!(estimate_bonuses(&graph, &[0]), 10);
    }
}
