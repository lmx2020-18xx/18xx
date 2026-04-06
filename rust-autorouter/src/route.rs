use serde::Serialize;

use crate::bitfield::Bitfield;

/// A candidate route discovered during the walk phase.
#[derive(Clone)]
pub struct CandidateRoute {
    /// Global index for this route (used in revenue callback to JS).
    pub global_index: usize,
    /// Train ID this route is for.
    pub train_id: String,
    /// Connection hexes for Route reconstruction in Ruby/JS.
    /// Each inner Vec is a chain of hex IDs.
    pub connection_hexes: Vec<Vec<String>>,
    /// Node signatures for UI highlighting.
    pub node_signatures: Vec<String>,
    /// Estimated revenue (sum of node revenues, no game-specific overrides).
    pub estimate_revenue: i32,
    /// Bitfield of hexside edges used (for overlap detection).
    pub bitfield: Bitfield,
    /// Visited node indices (for distance checking).
    pub visited_nodes: Vec<usize>,
}

/// Output format for the WASM boundary.
#[derive(Serialize)]
pub struct RouteOutput {
    pub connection_hexes: Vec<Vec<String>>,
    pub node_signatures: Vec<String>,
    pub revenue: i32,
    pub train_id: String,
}

/// Final result returned from find_best_routes.
#[derive(Serialize)]
pub struct AutorouteResult {
    pub routes: Vec<RouteOutput>,
    pub total_revenue: i32,
    pub walk_timed_out: bool,
    pub combo_timed_out: bool,
}
