use std::collections::HashSet;

use rust_autorouter::combo::{find_best_combo_sync, TrainGroups};
use rust_autorouter::graph::{Graph, GraphInput};
use std::time::Duration;
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

struct AutorouteReport {
    game: String,
    corporation: String,
    num_nodes: usize,
    num_paths: usize,
    trains: Vec<String>,
    walk_ms: f64,
    walk_timed_out: bool,
    routes_per_train: Vec<(String, usize)>,
    combo_ms: f64,
    best_revenue: i32,
    best_routes: Vec<(String, i32, Vec<String>)>, // (train_id, revenue, node_signatures)
    total_connections: usize,
    total_callbacks: usize,
}

fn run_autoroute(fixture_name: &str) -> AutorouteReport {
    let input = load_fixture(fixture_name);
    let graph = Graph::from_input(&input);

    let train_ids: Vec<String> = input.trains.iter().map(|t| t.id.clone()).collect();

    // Walk phase
    let walk_start = std::time::Instant::now();
    let walk = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &HashSet::new(),
        Duration::from_millis(input.config.path_timeout_ms),
        input.config.route_limit,
    );
    let walk_ms = walk_start.elapsed().as_secs_f64() * 1000.0;

    let routes_per_train: Vec<(String, usize)> = train_ids
        .iter()
        .map(|id| {
            let count = walk.train_routes.get(id).map(|r| r.len()).unwrap_or(0);
            (id.clone(), count)
        })
        .collect();

    // Combo phase (estimate only, no real_revenue callback)
    let combo_start = std::time::Instant::now();
    let combo = find_best_combo_sync(&walk.train_routes, &train_ids, &TrainGroups::AllShare, None, std::time::Duration::from_secs(10));
    let combo_ms = combo_start.elapsed().as_secs_f64() * 1000.0;

    let best_routes: Vec<(String, i32, Vec<String>)> = combo
        .best_routes
        .iter()
        .map(|r| {
            (
                r.train_id.clone(),
                r.estimate_revenue,
                r.node_signatures.clone(),
            )
        })
        .collect();

    AutorouteReport {
        game: fixture_name.to_string(),
        corporation: input.corporation_id.clone(),
        num_nodes: input.nodes.len(),
        num_paths: input.paths.len(),
        trains: train_ids,
        walk_ms,
        walk_timed_out: walk.timed_out,
        routes_per_train,
        combo_ms,
        best_revenue: combo.best_revenue,
        best_routes,
        total_connections: walk.connections_found,
        total_callbacks: walk.total_callbacks,
    }
}

fn print_report(r: &AutorouteReport) {
    eprintln!("=== {} (corp: {}) ===", r.game, r.corporation);
    eprintln!(
        "  Graph: {} nodes, {} paths, trains: {:?}",
        r.num_nodes, r.num_paths, r.trains
    );
    eprintln!(
        "  Walk: {:.1}ms, timed_out={}, connections={}, callbacks={}",
        r.walk_ms, r.walk_timed_out, r.total_connections, r.total_callbacks
    );
    for (train_id, count) in &r.routes_per_train {
        eprintln!("    {}: {} routes", train_id, count);
    }
    eprintln!("  Combo: {:.1}ms, best_revenue={}", r.combo_ms, r.best_revenue);
    for (train_id, rev, nodes) in &r.best_routes {
        eprintln!("    {}: rev={}, stops={:?}", train_id, rev, nodes);
    }
    eprintln!(
        "  TOTAL: {:.1}ms",
        r.walk_ms + r.combo_ms
    );
    eprintln!();
}

#[test]
fn test_game34_autoroute() {
    let r = run_autoroute("game_34_graph");
    print_report(&r);

    assert_eq!(r.corporation, "MP");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert_eq!(r.best_routes.len(), 2, "Should find routes for both trains");

    // Each train should have routes
    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    // Routes should not overlap (bitfield check is in combo)
    eprintln!("Game 34: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}

#[test]
fn test_game35_autoroute() {
    let r = run_autoroute("game_35_graph");
    print_report(&r);

    assert_eq!(r.corporation, "MKT");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert!(!r.best_routes.is_empty(), "Should find at least one route");

    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    eprintln!("Game 35: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}

#[test]
fn test_game36_autoroute() {
    let r = run_autoroute("game_36_graph");
    print_report(&r);

    assert_eq!(r.corporation, "SP");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert!(!r.best_routes.is_empty(), "Should find at least one route");

    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    eprintln!("Game 36: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}

/// Run all three games and compare timings
#[test]
fn test_all_games_timing_summary() {
    let games = vec!["game_34_graph", "game_35_graph", "game_36_graph"];
    let mut reports = Vec::new();

    for game in &games {
        let r = run_autoroute(game);
        reports.push(r);
    }

    eprintln!("\n========== TIMING SUMMARY ==========");
    eprintln!("{:<12} {:>8} {:>8} {:>8} {:>8} {:>10}", "Game", "Walk ms", "Combo ms", "Total ms", "Revenue", "Routes");
    for r in &reports {
        let total_routes: usize = r.routes_per_train.iter().map(|(_, c)| c).sum();
        eprintln!("{:<12} {:>8.1} {:>8.1} {:>8.1} {:>8} {:>10}",
            r.game.replace("_graph", ""),
            r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms,
            r.best_revenue, total_routes);
    }
    eprintln!("====================================\n");

    // All games should complete in under 60 seconds total (release mode)
    let total: f64 = reports.iter().map(|r| r.walk_ms + r.combo_ms).sum();
    assert!(total < 60_000.0, "All games should complete in under 60s, took {:.0}ms", total);
}

#[test]
fn test_game37_autoroute() {
    let r = run_autoroute("game_37_graph");
    print_report(&r);

    assert_eq!(r.corporation, "TP");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert!(!r.best_routes.is_empty(), "Should find at least one route");

    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    eprintln!("Game 37: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}

#[test]
fn test_game38_autoroute() {
    let r = run_autoroute("game_38_graph");
    print_report(&r);

    assert_eq!(r.corporation, "MKT");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert!(!r.best_routes.is_empty(), "Should find at least one route");

    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    eprintln!("Game 38: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}

#[test]
fn test_game39_autoroute() {
    let r = run_autoroute("game_39_graph");
    print_report(&r);

    assert_eq!(r.corporation, "IC");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert!(!r.best_routes.is_empty(), "Should find at least one route");

    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    eprintln!("Game 39: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}

#[test]
fn test_game40_autoroute() {
    let r = run_autoroute("game_40_graph");
    print_report(&r);

    assert_eq!(r.corporation, "B&O");
    assert!(!r.walk_timed_out, "Walk should not time out");
    assert!(r.best_revenue > 0, "Should find positive revenue");
    assert!(!r.best_routes.is_empty(), "Should find at least one route");

    for (train_id, count) in &r.routes_per_train {
        assert!(*count > 0, "Train {} should have routes", train_id);
    }

    eprintln!("Game 40: walk={:.0}ms combo={:.0}ms total={:.0}ms revenue={}",
        r.walk_ms, r.combo_ms, r.walk_ms + r.combo_ms, r.best_revenue);
}
