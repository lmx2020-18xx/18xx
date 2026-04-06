use std::collections::HashMap;

use crate::bitfield::Bitfield;
use crate::route::CandidateRoute;
use crate::time::{Duration, Instant};

/// Result of the combo optimization phase.
pub struct ComboResult {
    /// Best route combination found (one per train, or fewer if some trains skipped).
    pub best_routes: Vec<CandidateRoute>,
    /// Revenue of the best combination (from estimate or real_revenue callback).
    pub best_revenue: i32,
    /// Whether the combo search timed out.
    pub timed_out: bool,
}

/// Train group configuration for overlap detection.
pub enum TrainGroups {
    /// All trains share one conflict group (default).
    AllShare,
    /// Each train has its own group (no overlap checking between trains).
    EachSeparate,
    /// Named groups: trains with the same group index share overlap checks.
    Groups(Vec<Vec<String>>),
}

/// Metadata tracked during the combo search for pruning.
struct ComboMetadata {
    estimate_revenue: i32,
    /// Per-group merged bitfields.
    bitfields: Vec<Bitfield>,
    /// Whether any overlap was detected within a group.
    invalid_because_overlap: bool,
}

impl ComboMetadata {
    fn new(num_groups: usize) -> Self {
        ComboMetadata {
            estimate_revenue: 0,
            bitfields: (0..num_groups).map(|_| Bitfield::new()).collect(),
            invalid_because_overlap: false,
        }
    }

    fn add_route(&self, route: &CandidateRoute, train_group: usize) -> Self {
        let mut new_bitfields = self.bitfields.clone();
        let overlap = route.bitfield.conflicts(&self.bitfields[train_group]);
        new_bitfields[train_group] = new_bitfields[train_group].merge(&route.bitfield);

        ComboMetadata {
            estimate_revenue: self.estimate_revenue + route.estimate_revenue,
            bitfields: new_bitfields,
            invalid_because_overlap: self.invalid_because_overlap || overlap,
        }
    }
}

/// Train data for the combo search — one entry per train.
struct TrainData {
    #[allow(dead_code)]
    train_id: String,
    train_group: usize,
    routes: Vec<CandidateRoute>,
    /// Max possible revenue for this train and all subsequent trains combined.
    max_possible_revenue_from_here: i32,
}

/// Find the best non-overlapping route combination across all trains.
///
/// This is a synchronous version for use in native Rust tests.
/// The async WASM version wraps this with yield points.
///
/// `revenue_fn` is called for promising combos. If None, uses estimate_revenue.
pub fn find_best_combo_sync(
    train_routes: &HashMap<String, Vec<CandidateRoute>>,
    train_ids: &[String],
    groups: &TrainGroups,
    revenue_fn: Option<&dyn Fn(&[&CandidateRoute]) -> i32>,
    combo_timeout: Duration,
) -> ComboResult {
    if train_ids.is_empty() {
        return ComboResult {
            best_routes: Vec::new(),
            best_revenue: 0,
            timed_out: false,
        };
    }

    // Determine train group assignments
    let num_groups = match groups {
        TrainGroups::AllShare => 1,
        TrainGroups::EachSeparate => train_ids.len(),
        TrainGroups::Groups(g) => g.len() + 1,
    };

    // Build train data list (reversed to compute max_possible_revenue_for_rest)
    let mut train_data_vec: Vec<TrainData> = Vec::new();
    let mut cumulative_max = 0i32;

    for (i, train_id) in train_ids.iter().enumerate().rev() {
        let routes = train_routes.get(train_id).cloned().unwrap_or_default();
        let best_for_train = routes.first().map(|r| r.estimate_revenue).unwrap_or(0);

        let train_group = match groups {
            TrainGroups::AllShare => 0,
            TrainGroups::EachSeparate => i,
            TrainGroups::Groups(g) => {
                // Find which group contains this train's name
                let train_name = routes
                    .first()
                    .map(|r| &r.train_id)
                    .unwrap_or(train_id);
                g.iter()
                    .position(|group| group.iter().any(|n| n == train_name.as_str()))
                    .map(|p| p + 1)
                    .unwrap_or(0)
            }
        };

        cumulative_max += best_for_train;

        train_data_vec.push(TrainData {
            train_id: train_id.clone(),
            train_group,
            routes,
            max_possible_revenue_from_here: cumulative_max,
        });
    }

    train_data_vec.reverse();

    // DFS combo search
    let mut best_revenue = 0i32;
    let mut best_routes: Vec<CandidateRoute> = Vec::new();
    let empty_metadata = ComboMetadata::new(num_groups);
    let start = Instant::now();
    let mut timed_out = false;

    find_best_combo_recursive(
        &train_data_vec,
        0,
        &mut Vec::new(),
        &empty_metadata,
        &mut best_revenue,
        &mut best_routes,
        revenue_fn,
        start,
        combo_timeout,
        &mut timed_out,
    );

    ComboResult {
        best_routes,
        best_revenue,
        timed_out,
    }
}

fn find_best_combo_recursive(
    train_data: &[TrainData],
    train_idx: usize,
    current_combo: &mut Vec<Option<CandidateRoute>>,
    metadata: &ComboMetadata,
    best_revenue: &mut i32,
    best_routes: &mut Vec<CandidateRoute>,
    revenue_fn: Option<&dyn Fn(&[&CandidateRoute]) -> i32>,
    start: Instant,
    timeout: Duration,
    timed_out: &mut bool,
) {
    if *timed_out {
        return;
    }
    if train_idx >= train_data.len() {
        return;
    }

    let td = &train_data[train_idx];
    let is_last_train = train_idx == train_data.len() - 1;

    // Try each route for this train (plus null = skip this train)
    // Iterate: all real routes first, then the null option
    for (iter_count, route_option) in td.routes.iter().map(Some).chain(std::iter::once(None)).enumerate() {
        if *timed_out {
            return;
        }
        // Periodic timeout check (every 1000 iterations at outer level)
        if train_idx == 0 && iter_count % 1000 == 999 && start.elapsed() > timeout {
            *timed_out = true;
            return;
        }

        // Early exit: routes are sorted by estimate_revenue descending.
        // If this route + remaining trains' max can't beat best, break.
        if let Some(route) = route_option {
            let remaining_max = if is_last_train {
                0
            } else {
                train_data[train_idx + 1].max_possible_revenue_from_here
            };
            if metadata.estimate_revenue + route.estimate_revenue + remaining_max
                <= *best_revenue
            {
                // All subsequent routes have equal or lower revenue, so no combo can beat best.
                // Still need to try the None option though (skip this train).
                current_combo.push(None);
                if !is_last_train {
                    let worth_it = !metadata.invalid_because_overlap
                        && (metadata.estimate_revenue
                            + train_data[train_idx + 1].max_possible_revenue_from_here
                            > *best_revenue);
                    if worth_it {
                        find_best_combo_recursive(
                            train_data,
                            train_idx + 1,
                            current_combo,
                            metadata,
                            best_revenue,
                            best_routes,
                            revenue_fn,
                            start,
                            timeout,
                            timed_out,
                        );
                    }
                }
                current_combo.pop();
                break;
            }
        }

        let current_metadata = if let Some(route) = route_option {
            metadata.add_route(route, td.train_group)
        } else {
            metadata.clone()
        };

        current_combo.push(route_option.cloned());

        if is_last_train {
            // Evaluate this complete combo
            if !current_metadata.invalid_because_overlap
                && current_metadata.estimate_revenue > *best_revenue
            {
                // Try real revenue if callback provided
                let actual_revenue = if let Some(rev_fn) = revenue_fn {
                    let routes_ref: Vec<&CandidateRoute> = current_combo
                        .iter()
                        .filter_map(|r| r.as_ref())
                        .collect();
                    if routes_ref.is_empty() {
                        0
                    } else {
                        rev_fn(&routes_ref)
                    }
                } else {
                    current_metadata.estimate_revenue
                };

                if actual_revenue > *best_revenue {
                    *best_revenue = actual_revenue;
                    *best_routes = current_combo.iter().filter_map(|r| r.clone()).collect();
                }
            }
        } else {
            // Check if worth continuing
            let worth_it = !current_metadata.invalid_because_overlap
                && (current_metadata.estimate_revenue
                    + train_data[train_idx + 1].max_possible_revenue_from_here
                    > *best_revenue);

            if worth_it {
                find_best_combo_recursive(
                    train_data,
                    train_idx + 1,
                    current_combo,
                    &current_metadata,
                    best_revenue,
                    best_routes,
                    revenue_fn,
                    start,
                    timeout,
                    timed_out,
                );
            }
        }

        current_combo.pop();
    }
}

impl Clone for ComboMetadata {
    fn clone(&self) -> Self {
        ComboMetadata {
            estimate_revenue: self.estimate_revenue,
            bitfields: self.bitfields.clone(),
            invalid_because_overlap: self.invalid_because_overlap,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitfield::Bitfield;

    fn make_route(idx: usize, train_id: &str, revenue: i32, bits: &[usize]) -> CandidateRoute {
        let mut bf = Bitfield::new();
        for &b in bits {
            bf.set(b);
        }
        CandidateRoute {
            global_index: idx,
            train_id: train_id.to_string(),
            connection_hexes: Vec::new(),
            node_signatures: Vec::new(),
            estimate_revenue: revenue,
            bitfield: bf,
            visited_nodes: Vec::new(),
        }
    }

    #[test]
    fn test_single_train_picks_best() {
        let mut routes = HashMap::new();
        routes.insert(
            "4-0".to_string(),
            vec![
                make_route(0, "4-0", 100, &[0, 1]),
                make_route(1, "4-0", 80, &[2, 3]),
                make_route(2, "4-0", 60, &[4, 5]),
            ],
        );

        let result =
            find_best_combo_sync(&routes, &["4-0".to_string()], &TrainGroups::AllShare, None, Duration::from_secs(10));

        assert_eq!(result.best_revenue, 100);
        assert_eq!(result.best_routes.len(), 1);
        assert_eq!(result.best_routes[0].global_index, 0);
    }

    #[test]
    fn test_two_trains_non_overlapping() {
        let mut routes = HashMap::new();
        routes.insert(
            "t1".to_string(),
            vec![
                make_route(0, "t1", 100, &[0, 1]), // overlaps with t2's best
                make_route(1, "t1", 80, &[4, 5]),   // no overlap
            ],
        );
        routes.insert(
            "t2".to_string(),
            vec![
                make_route(2, "t2", 90, &[0, 2]), // overlaps with t1's best
                make_route(3, "t2", 70, &[6, 7]),  // no overlap
            ],
        );

        let result = find_best_combo_sync(
            &routes,
            &["t1".to_string(), "t2".to_string()],
            &TrainGroups::AllShare,
            None,
            Duration::from_secs(10),
        );

        // Best non-overlapping: t1=100 (bits 0,1) conflicts with t2=90 (bit 0)
        // So: t1=100 + t2=70 = 170, or t1=80 + t2=90 = 170, or t1=80 + t2=70 = 150
        // Actually t1=100+t2=70=170 ties with t1=80+t2=90=170. Either is fine.
        assert_eq!(result.best_revenue, 170);
        assert_eq!(result.best_routes.len(), 2);
    }

    #[test]
    fn test_each_train_separate_allows_overlap() {
        let mut routes = HashMap::new();
        routes.insert(
            "t1".to_string(),
            vec![make_route(0, "t1", 100, &[0, 1])],
        );
        routes.insert(
            "t2".to_string(),
            vec![make_route(1, "t2", 90, &[0, 2])], // shares bit 0 with t1
        );

        let result = find_best_combo_sync(
            &routes,
            &["t1".to_string(), "t2".to_string()],
            &TrainGroups::EachSeparate,
            None,
            Duration::from_secs(10),
        );

        // With separate groups, overlap is allowed
        assert_eq!(result.best_revenue, 190);
        assert_eq!(result.best_routes.len(), 2);
    }

    #[test]
    fn test_pruning_skips_hopeless_branches() {
        let mut routes = HashMap::new();
        // Train 1 has one great route
        routes.insert(
            "t1".to_string(),
            vec![make_route(0, "t1", 1000, &[0])],
        );
        // Train 2 has many weak routes — pruning should skip most
        let mut t2_routes = Vec::new();
        for i in 0..100 {
            t2_routes.push(make_route(i + 1, "t2", 1, &[i + 10]));
        }
        routes.insert("t2".to_string(), t2_routes);

        let result = find_best_combo_sync(
            &routes,
            &["t1".to_string(), "t2".to_string()],
            &TrainGroups::AllShare,
            None,
            Duration::from_secs(10),
        );

        assert_eq!(result.best_revenue, 1001);
    }
}
