pub mod bitfield;
pub mod combo;
pub mod distance;
pub mod graph;
pub mod route;
pub mod time;
pub mod types;
pub mod walk;

use std::collections::{HashMap, HashSet};

use wasm_bindgen::prelude::*;

use crate::combo::{find_best_combo_sync, TrainGroups};
use crate::graph::{Graph, GraphInput};
use crate::route::{AutorouteResult, RouteOutput};
use crate::time::Duration;
use crate::walk::walk_all_routes;

/// Primary WASM entry point: run both walk and combo phases.
///
/// Arguments:
///   graph_json — JSON string matching GraphInput schema
///   revenue_callback — JS function(route_combo_json: string) -> number
///     Called with JSON array of {connection_hexes, node_signatures, train_id} for each route
///     in a promising combo. Returns the real revenue from game engine, or -1 on error.
///   progress_callback — JS function(best_routes_json: string) -> bool
///     Called periodically with current best routes. Returns false to cancel.
///
/// Returns: JSON string matching AutorouteResult schema.
#[wasm_bindgen]
pub fn find_best_routes(
    graph_json: &str,
    revenue_callback: &js_sys::Function,
    progress_callback: &js_sys::Function,
) -> String {
    console_error_panic_hook::set_once();

    // Parse input
    let input: GraphInput = match serde_json::from_str(graph_json) {
        Ok(i) => i,
        Err(e) => {
            let _ = report_error(progress_callback, &format!("JSON parse error: {}", e));
            return error_result();
        }
    };

    // Build processed graph
    let graph = Graph::from_input(&input);

    // Determine skip paths from static routes
    let skip_path_ids: HashSet<usize> = input
        .static_routes
        .iter()
        .flat_map(|sr| sr.path_ids.iter().copied())
        .collect();

    // Skip trains that have static routes
    let static_train_ids: HashSet<&str> = input
        .static_routes
        .iter()
        .map(|sr| sr.train_id.as_str())
        .collect();
    let active_trains: Vec<_> = input
        .trains
        .iter()
        .filter(|t| !static_train_ids.contains(t.id.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    // Train IDs in order
    let train_ids: Vec<String> = active_trains.iter().map(|t| t.id.clone()).collect();

    // Walk phase
    let path_timeout = Duration::from_millis(input.config.path_timeout_ms);
    let walk_result = walk_all_routes(
        &graph,
        &active_trains,
        &input.start_nodes,
        &skip_path_ids,
        path_timeout,
        input.config.route_limit,
    );

    // Report walk completion to progress callback
    let _ = report_progress(progress_callback, "walk_complete", &walk_result.train_routes);

    // Determine train groups
    let groups = parse_train_groups(&input.config.train_autoroute_groups);

    // Build revenue callback wrapper that calls into JS
    let rev_cb = |routes: &[&crate::route::CandidateRoute]| -> i32 {
        // Serialize the route combo as JSON for the JS callback
        let combo_data: Vec<serde_json::Value> = routes
            .iter()
            .map(|r| {
                serde_json::json!({
                    "connection_hexes": r.connection_hexes,
                    "node_signatures": r.node_signatures,
                    "train_id": r.train_id,
                    "estimate_revenue": r.estimate_revenue,
                })
            })
            .collect();
        let json = serde_json::to_string(&combo_data).unwrap_or_default();
        let js_str = JsValue::from_str(&json);

        match revenue_callback.call1(&JsValue::NULL, &js_str) {
            Ok(val) => val.as_f64().unwrap_or(-1.0) as i32,
            Err(_) => -1,
        }
    };

    // Combo phase — synchronous since Rust is fast enough (~100-200ms)
    let combo_timeout = Duration::from_millis(input.config.route_timeout_ms);
    let combo_result = find_best_combo_sync(
        &walk_result.train_routes,
        &train_ids,
        &groups,
        Some(&rev_cb),
        combo_timeout,
    );

    // Build output
    let routes: Vec<RouteOutput> = combo_result
        .best_routes
        .iter()
        .map(|r| RouteOutput {
            connection_hexes: r.connection_hexes.clone(),
            node_signatures: r.node_signatures.clone(),
            revenue: r.estimate_revenue,
            train_id: r.train_id.clone(),
        })
        .collect();

    let result = AutorouteResult {
        routes,
        total_revenue: combo_result.best_revenue,
        walk_timed_out: walk_result.timed_out,
        combo_timed_out: combo_result.timed_out,
    };

    serde_json::to_string(&result).unwrap_or_default()
}

/// Walk-only mode: returns candidate routes per train for diagnostics.
#[wasm_bindgen]
pub fn walk_paths(graph_json: &str) -> String {
    let input: GraphInput = match serde_json::from_str(graph_json) {
        Ok(i) => i,
        Err(_) => return "{}".to_string(),
    };

    let graph = Graph::from_input(&input);
    let skip_path_ids: HashSet<usize> = HashSet::new();
    let path_timeout = Duration::from_millis(input.config.path_timeout_ms);

    let walk_result = walk_all_routes(
        &graph,
        &input.trains,
        &input.start_nodes,
        &skip_path_ids,
        path_timeout,
        input.config.route_limit,
    );

    // Serialize route counts per train
    let mut output: HashMap<String, Vec<RouteOutput>> = HashMap::new();
    for (train_id, routes) in &walk_result.train_routes {
        output.insert(
            train_id.clone(),
            routes
                .iter()
                .map(|r| RouteOutput {
                    connection_hexes: r.connection_hexes.clone(),
                    node_signatures: r.node_signatures.clone(),
                    revenue: r.estimate_revenue,
                    train_id: r.train_id.clone(),
                })
                .collect(),
        );
    }

    serde_json::to_string(&output).unwrap_or_default()
}

/// Version check.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn parse_train_groups(value: &Option<serde_json::Value>) -> TrainGroups {
    match value {
        None | Some(serde_json::Value::Null) => TrainGroups::AllShare,
        Some(serde_json::Value::String(s)) if s == "each_train_separate" => {
            TrainGroups::EachSeparate
        }
        Some(serde_json::Value::Array(arr)) => {
            let groups: Vec<Vec<String>> = arr
                .iter()
                .filter_map(|v| {
                    v.as_array().map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(String::from))
                            .collect()
                    })
                })
                .collect();
            TrainGroups::Groups(groups)
        }
        _ => TrainGroups::AllShare,
    }
}

fn error_result() -> String {
    serde_json::to_string(&AutorouteResult {
        routes: Vec::new(),
        total_revenue: 0,
        walk_timed_out: false,
        combo_timed_out: false,
    })
    .unwrap_or_default()
}

fn report_error(progress_callback: &js_sys::Function, msg: &str) -> Result<JsValue, JsValue> {
    let json = serde_json::json!({"error": msg}).to_string();
    progress_callback.call1(&JsValue::NULL, &JsValue::from_str(&json))
}

fn report_progress(
    progress_callback: &js_sys::Function,
    phase: &str,
    train_routes: &HashMap<String, Vec<crate::route::CandidateRoute>>,
) -> Result<JsValue, JsValue> {
    let counts: HashMap<&str, usize> = train_routes
        .iter()
        .map(|(k, v)| (k.as_str(), v.len()))
        .collect();
    let json = serde_json::json!({"phase": phase, "route_counts": counts}).to_string();
    progress_callback.call1(&JsValue::NULL, &JsValue::from_str(&json))
}
