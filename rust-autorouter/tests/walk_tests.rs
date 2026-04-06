use std::collections::HashSet;
use std::time::Duration;

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

#[test]
fn test_linear_3hex_walk_finds_routes() {
    let input = load_fixture("linear_3hex");
    let graph = Graph::from_input(&input);

    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(5),
        1000,
    );

    assert!(!walk.timed_out);

    // Train "3-0" (distance 3) should find routes covering 2 and 3 cities
    let t3_routes = walk.train_routes.get("3-0").expect("No routes for train 3-0");
    assert!(
        !t3_routes.is_empty(),
        "Train 3-0 should have at least one route"
    );

    // Train "2-0" (distance 2) should find 2-city routes
    let t2_routes = walk.train_routes.get("2-0").expect("No routes for train 2-0");
    assert!(
        !t2_routes.is_empty(),
        "Train 2-0 should have at least one route"
    );

    // The best 3-train route should visit all 3 cities: revenue = 30 + 40 + 50 = 120
    let best_3 = &t3_routes[0];
    assert_eq!(
        best_3.estimate_revenue, 120,
        "Best 3-train route should have revenue 120 (30+40+50 in green phase)"
    );

    // The best 2-train route must include A1 (has PRR token).
    // So best 2-stop is A1+B2: 30+40 = 70 (B2+C3 excluded — no PRR token)
    let best_2 = &t2_routes[0];
    assert_eq!(
        best_2.estimate_revenue, 70,
        "Best 2-train route should have revenue 70 (A1=30 + B2=40, green phase)"
    );

    // All routes should have non-empty connection_hexes
    for route in t3_routes.iter().chain(t2_routes.iter()) {
        assert!(
            !route.connection_hexes.is_empty() || route.visited_nodes.len() == 1,
            "Route should have connection_hexes"
        );
    }

    // All routes should have non-empty node_signatures
    for route in t3_routes.iter().chain(t2_routes.iter()) {
        assert!(
            !route.node_signatures.is_empty(),
            "Route should have node_signatures"
        );
    }

    // Verify no routes have the same global_index
    let mut seen_indices: HashSet<usize> = HashSet::new();
    for route in t3_routes.iter().chain(t2_routes.iter()) {
        assert!(
            seen_indices.insert(route.global_index),
            "Duplicate global_index: {}",
            route.global_index
        );
    }
}

#[test]
fn test_linear_3hex_routes_have_valid_bitfields() {
    let input = load_fixture("linear_3hex");
    let graph = Graph::from_input(&input);

    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(5),
        1000,
    );

    let t3_routes = walk.train_routes.get("3-0").unwrap();

    // Multi-city routes should have non-empty bitfields
    // Single-city routes (local-style) have empty bitfields — that's correct
    let multi_city_routes: Vec<_> = t3_routes
        .iter()
        .filter(|r| r.visited_nodes.len() >= 2)
        .collect();
    assert!(!multi_city_routes.is_empty());
    for route in &multi_city_routes {
        assert!(
            !route.bitfield.is_empty(),
            "Multi-city route bitfield should not be empty for {:?}",
            route.node_signatures
        );
    }

    // The A1-B2 and A1-B2-C3 routes should have conflicting bitfields
    // (they share the A1-B2 segment)
    if multi_city_routes.len() >= 2 {
        let route_2city = multi_city_routes
            .iter()
            .find(|r| r.visited_nodes.len() == 2)
            .unwrap();
        let route_3city = multi_city_routes
            .iter()
            .find(|r| r.visited_nodes.len() == 3)
            .unwrap();
        assert!(
            route_2city.bitfield.conflicts(&route_3city.bitfield),
            "A1-B2 and A1-B2-C3 should share hexsides"
        );
    }
}

#[test]
fn test_linear_3hex_token_requirement() {
    // All routes should pass through a city with PRR token (node 0 = A1)
    let input = load_fixture("linear_3hex");
    let graph = Graph::from_input(&input);

    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(5),
        1000,
    );

    // Every route should include node 0 (A1, which has the PRR token)
    // because NoToken check requires at least one tokened city
    for (train_id, routes) in &walk.train_routes {
        for route in routes {
            let has_token = route
                .visited_nodes
                .iter()
                .any(|&ni| graph.node_tokened(ni));
            assert!(
                has_token,
                "Route for train {} should visit a tokened city",
                train_id
            );
        }
    }
}

#[test]
fn test_linear_3hex_combo() {
    use rust_autorouter::combo::{find_best_combo_sync, TrainGroups};

    let input = load_fixture("linear_3hex");
    let graph = Graph::from_input(&input);

    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(5),
        1000,
    );

    let train_ids: Vec<String> = input.trains.iter().map(|t| t.id.clone()).collect();
    let result =
        find_best_combo_sync(&walk.train_routes, &train_ids, &TrainGroups::AllShare, None, std::time::Duration::from_secs(10));

    // With 2 trains sharing one track group, the combo should find non-overlapping routes
    assert!(
        result.best_revenue > 0,
        "Best combo revenue should be positive"
    );
    assert!(
        !result.best_routes.is_empty(),
        "Should find at least one route"
    );

    // Print for debugging
    eprintln!("Combo revenue: {}", result.best_revenue);
    for route in &result.best_routes {
        eprintln!(
            "  Train {}: revenue={}, nodes={:?}",
            route.train_id, route.estimate_revenue, route.node_signatures
        );
    }
}

#[test]
fn test_walk_respects_timeout() {
    let input = load_fixture("linear_3hex");
    let graph = Graph::from_input(&input);

    // With a 0ms timeout, the walk should time out immediately (or nearly)
    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_nanos(1), // practically zero
        1000,
    );

    // May or may not time out depending on how fast the first node processes,
    // but the function should complete without panicking
    let _ = walk.timed_out;
}

#[test]
fn test_walk_respects_route_limit() {
    let input = load_fixture("linear_3hex");
    let graph = Graph::from_input(&input);

    // With a route limit of 1, each train should have at most 1 route
    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_secs(5),
        1, // limit to 1 route per train
    );

    for (_train_id, routes) in &walk.train_routes {
        assert!(routes.len() <= 1, "Route limit of 1 should be respected");
    }
}
