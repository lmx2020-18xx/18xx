use std::collections::HashSet;
use std::time::Duration;

use rust_autorouter::combo::{find_best_combo_sync, TrainGroups};
use rust_autorouter::graph::{Graph, GraphInput};
use rust_autorouter::walk::walk_all_routes;

fn load_fixture(name: &str) -> GraphInput {
    let path = format!(
        "{}/tests/fixtures/{}.json",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    let data = std::fs::read_to_string(&path).expect(&format!("Failed to read fixture: {}", path));
    serde_json::from_str(&data).expect("Failed to parse fixture JSON")
}

/// Test with real 1870 game 34 board — 32 nodes, 197 paths, 99 hexes.
/// Corporation MP in blue phase with 2 trains.
#[test]
fn test_game34_graph_loads() {
    let input = load_fixture("game_34_graph");
    assert_eq!(input.corporation_id, "MP");
    assert_eq!(input.nodes.len(), 32);
    assert_eq!(input.paths.len(), 220);
    assert_eq!(input.trains.len(), 2);
    assert_eq!(input.start_nodes.len(), 32);

    let graph = Graph::from_input(&input);
    assert_eq!(graph.nodes.len(), 32);
    assert_eq!(graph.paths.len(), 220);
}

#[test]
fn test_game34_walk_finds_routes() {
    let input = load_fixture("game_34_graph");
    let graph = Graph::from_input(&input);

    let start = std::time::Instant::now();
    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(30),
        10_000,
    );
    let elapsed = start.elapsed();

    eprintln!("Game 34 walk completed in {:.2?}", elapsed);
    eprintln!("Timed out: {}", walk.timed_out);
    eprintln!("Connections found: {}", walk.connections_found);

    for (train_id, routes) in &walk.train_routes {
        eprintln!("Train {}: {} routes", train_id, routes.len());
        if let Some(best) = routes.first() {
            eprintln!(
                "  Best: rev={}, nodes={:?}",
                best.estimate_revenue, best.node_signatures
            );
        }
    }

    // Should find routes for each train
    for train in &input.trains {
        let routes = walk.train_routes.get(&train.id).unwrap();
        assert!(
            !routes.is_empty(),
            "Train {} should have at least one route",
            train.id
        );
    }

    // Walk should complete within 30 seconds (Rust should be much faster)
    assert!(
        elapsed < Duration::from_secs(30),
        "Walk should complete in under 30 seconds"
    );

    // Best route should have significant revenue (1870 mid-game)
    let all_routes: Vec<_> = walk.train_routes.values().flat_map(|r| r.iter()).collect();
    let max_revenue = all_routes.iter().map(|r| r.estimate_revenue).max().unwrap_or(0);
    assert!(
        max_revenue > 50,
        "Best route should have more than $50 revenue, got {}",
        max_revenue
    );
}

#[test]
fn test_game34_combo() {
    let input = load_fixture("game_34_graph");
    let graph = Graph::from_input(&input);

    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(30),
        10_000,
    );

    let train_ids: Vec<String> = input.trains.iter().map(|t| t.id.clone()).collect();

    let start = std::time::Instant::now();
    let result =
        find_best_combo_sync(&walk.train_routes, &train_ids, &TrainGroups::AllShare, None, std::time::Duration::from_secs(10));
    let elapsed = start.elapsed();

    eprintln!("Game 34 combo completed in {:.2?}", elapsed);
    eprintln!("Best revenue: {}", result.best_revenue);
    for route in &result.best_routes {
        eprintln!(
            "  Train {}: rev={}, nodes={:?}",
            route.train_id, route.estimate_revenue, route.node_signatures
        );
    }

    assert!(result.best_revenue > 0, "Should find a positive-revenue combo");
    assert!(
        !result.best_routes.is_empty(),
        "Should find at least one route in combo"
    );

    // Routes in the combo should not have conflicting bitfields (same group)
    if result.best_routes.len() >= 2 {
        for i in 0..result.best_routes.len() {
            for j in (i + 1)..result.best_routes.len() {
                assert!(
                    !result.best_routes[i]
                        .bitfield
                        .conflicts(&result.best_routes[j].bitfield),
                    "Routes {} and {} should not overlap in combo",
                    i,
                    j
                );
            }
        }
    }
}

#[test]
fn test_game34_performance() {
    let input = load_fixture("game_34_graph");
    let graph = Graph::from_input(&input);

    // Run walk 3 times and report timing
    let mut times = Vec::new();
    for _ in 0..3 {
        let start = std::time::Instant::now();
        let _walk = walk_all_routes(
            &graph,
            &input.trains,
            &input.start_nodes,
            &HashSet::new(),
            Duration::from_secs(30),
            10_000,
        );
        times.push(start.elapsed());
    }

    eprintln!("Game 34 walk times: {:?}", times);
    let avg = times.iter().map(|t| t.as_millis()).sum::<u128>() / times.len() as u128;
    eprintln!("Average: {}ms", avg);

    // Native Rust should complete well under 5 seconds for a mid-game 1870 board
    assert!(
        avg < 5_000,
        "Walk should average under 5 seconds, got {}ms",
        avg
    );
}
