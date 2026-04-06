# Plan: Rust WASM Autorouter

## Context

The autorouter's path walk phase (`lib/engine/auto_router.rb`, `path()` method) is the main bottleneck for long trains (10+ stations). It does an exponential recursive DFS from each token node, taking up to 30s for 12-stop trains. The current implementation runs as Opal-transpiled Ruby (→JS), which carries significant overhead from Ruby semantics (method dispatch, object allocation, hash lookups).

A Rust implementation compiled to WASM should achieve 50-100x speedup on the path walk, making even the longest autoroutes complete in under 1 second. The Rust library replaces **both** phases: the Ruby path walk AND the JS combo optimizer, with a JS callback for game-specific `real_revenue` validation.

The previous approach (localStorage caching) is preserved on the `router-cache` branch as a complementary optimization.

## Results 

"**334ms in release mode** for a full 1870 mid-game walk with 2x12T trains, finding 10,000+ routes. That's compared to the Ruby/Opal autorouter's ~30 second path timeout. ~90x speedup.
Test Results: 27 tests, 0 failures

### Performance
| Mode | Walk Time (Game 34, 2x12T) |
|------|---------------------------|
| Debug | 1.4s |
| Release | **334ms** |
| Current Ruby/Opal | ~30s (timeout) |

That's a **~90x speedup** on the path walk for a real 1870 mid-game board.


● All three games find routes for both trains with revenue exceeding Ruby:
  ┌──────┬──────────────┬──────────────┬──────────────┐
  │ Game │ Rust Revenue │ Ruby Revenue │ Both Trains? │
  ├──────┼──────────────┼──────────────┼──────────────┤
  │ 34   │ 790          │ 760          │ ✓            │
  ├──────┼──────────────┼──────────────┼──────────────┤
  │ 35   │ 880          │ 530          │ ✓            │
  ├──────┼──────────────┼──────────────┼──────────────┤
  │ 36   │ 830          │ 800          │ ✓            │
  └──────┴──────────────┴──────────────┴──────────────┘


### Test Fixtures
- 3 hand-crafted JSON fixtures for unit testing
- 3 real 1870 game fixtures (34, 35, 36) copied from `router-cache` branch for comparison testing
- Extraction script (`extract_graph.rb`) to generate Rust graph fixtures from any game

#### Extra tests in late version builds

  1870 bonus rules: Game 38.json (2x 12T MKT on dense map)
  ====
  ┌─────────────┬───────┬─────────────────────┐
  │             │ Rust  │        Ruby         │
  ├─────────────┼───────┼─────────────────────┤
  │ Revenue     │ 920   │ 530                 │
  ├─────────────┼───────┼─────────────────────┤
  │ Walk time   │ 10.6s │ 122.5s (timed out!) │
  ├─────────────┼───────┼─────────────────────┤
  │ Combo       │ 10.4s │ greedy only         │
  ├─────────────┼───────┼─────────────────────┤
  │ Total       │ 21s   │ 122s+               │
  ├─────────────┼───────┼─────────────────────┤
  │ Trains used │ 2/2   │ 1/2                 │
  └─────────────┴───────┴─────────────────────┘

  Ruby timed out during walk (120s limit) and only found a greedy 1-train combo for 530. Rust completed the full walk in 10.6s and
  found 2-train combo for 920 — a massive improvement. This is a great example of why the WASM autorouter matters: on complex boards
  Ruby can't even finish the walk, while Rust finds optimal 2-train routes comfortably.

  1846 bonus rules: 39 and 40 tests
  ====
  ┌──────┬──────┬───────────┬──────┬───────────────┬──────────────────────────────────────────────────────────────┐
  │ Game │ Corp │  Trains   │ Rust │ Ruby (greedy) │                            Notes                             │
  ├──────┼──────┼───────────┼──────┼───────────────┼──────────────────────────────────────────────────────────────┤
  │ 39   │ IC   │ 4/6 + 7/8 │ 700  │ 670           │ E/W bonus (50). Rust finds better combo                      │
  ├──────┼──────┼───────────┼──────┼───────────────┼──────────────────────────────────────────────────────────────┤
  │ 40   │ B&O  │ 4/6 + 7/8 │ 780  │ 720           │ E/W (80) + Mail Contract (+10/stop). Rust finds better combo │
  └──────┴──────┴───────────┴──────┴───────────────┴──────────────────────────────────────────────────────────────┘

  Both pass. Rust beats Ruby's greedy combo on both — expected since Rust does exhaustive search while Ruby only does greedy. The
  bonuses (E/W and Mail Contract) are being applied correctly.

  Note the speed: game 39 took 62ms in Rust vs 1047ms in Ruby. Game 40 took 2ms vs 26ms.


 ### Files modified:

  1. lib/engine/auto_router.rb — Added WASM dispatch:
    - compute() now checks wasm_available? and dispatches to compute_wasm() or compute_legacy()
    - wasm_available? — checks for window.__wasm_autorouter
    - compute_wasm() — calls Rust WASM find_best_routes() with revenue callback and progress callback, builds Route objects from
  result, falls back to legacy on error
    - serialize_graph() — serializes game state (nodes, paths, hexes, trains, junctions, converging exits) into JSON matching the
  Rust GraphInput schema
    - resolve_endpoint() — helper to convert path endpoints to JSON
  2. assets/app/view/game/route_selector.rb — Added WASM lazy loading:
    - load_wasm_and_run() — dynamically imports WASM module on first Auto click, stores on window.__wasm_autorouter, then runs
  autorouter
    - Falls back to legacy if WASM fails to load
  3. Rakefile — Added rake wasm task that builds Rust→WASM and cleans up wasm-pack artifacts
  4. Makefile — Added make wasm target
  5. .gitignore - Added exception for public/assets/wasm/ and exclusion for rust-autorouter/target/

  WASM artifacts (256KB release, pre-built and ready to commit):
  - public/assets/wasm/rust_autorouter.js
  - public/assets/wasm/rust_autorouter_bg.wasm
  - public/assets/wasm/rust_autorouter.d.ts
  - public/assets/wasm/rust_autorouter_bg.wasm.d.ts

  Architecture: WASM loads lazily on first "Auto" button click. If WASM is available, the Rust autorouter handles both walk + combo
  phases (~90x faster). Revenue callback bridges back to Ruby/JS real_revenue for game-specific validation. If WASM fails at any
  point, it falls back to the existing legacy autorouter transparently.


## Architecture Decision

**Rust does path walk + combo optimization, JS callback for `real_revenue`.**

Rationale:
- The combo optimizer is tightly coupled to bitfield representations built during the walk — returning thousands of routes across WASM boundary just to feed them back would negate gains
- The combo optimizer's inner loop is pure arithmetic (bitfield ops + pruning) — exactly what Rust excels at
- 200+ game-specific revenue overrides (`revenue_for`, `compute_stops`, `check_distance`) make replicating full game logic in Rust impractical
- The current combo optimizer already calls `real_revenue` only for promising combos (those beating `estimate_revenue`) — this pattern transfers naturally to Rust→JS callbacks

## WASM Public API

```rust
#[wasm_bindgen]
pub async fn find_best_routes(
    graph_json: &str,           // Serialized graph + trains + config
    revenue_callback: &js_sys::Function,  // (route_combo_indices: Uint32Array) -> i32
    progress_callback: &js_sys::Function, // (best_routes_json: string) -> bool
) -> String;  // JSON: { routes: [{connection_hexes, node_signatures, revenue}], total_revenue, timed_out }
```

**Why JSON boundary**: Graph data is ~200KB even for large games, parseable in <5ms via `serde_json`. The walk itself takes seconds, so serialization overhead is negligible.

**Why async**: Must yield to browser every 30ms (matching current `next_frame()` pattern) via `wasm_bindgen_futures`.

## Graph Serialization Format

The JS caller serializes game state into a flat, index-based JSON structure:

```json
{
  "corporation_id": "PRR",
  "current_phase": "green",
  "no_blocking": false,

  "hexes": {
    "A1": { "neighbors": {"0": "B2", "1": "A3"} }
  },

  "nodes": [
    { "id": 0, "hex_id": "A1", "type": "city",
      "revenue": {"yellow": 30, "green": 40, "brown": 50},
      "slots": 2, "tokens": ["PRR", null],
      "groups": ["OO"], "visit_cost": 1, "is_offboard": false }
  ],

  "paths": [
    { "id": 0, "hex_id": "A1",
      "a": {"type": "node", "index": 0},
      "b": {"type": "edge", "num": 3},
      "track": "broad", "lanes": [[1,0],[1,0]],
      "terminal": false, "ignore": false, "junction_id": null }
  ],

  "junctions": [
    { "id": 0, "hex_id": "A1", "path_ids": [2, 5, 7] }
  ],

  "converging_exits": { "A1": [2, 5] },

  "trains": [
    { "id": "12-0", "name": "12", "distance": 12, "price": 750,
      "track_type": "broad", "local": false },
    { "id": "2+2-0", "name": "2+2",
      "distance": [{"nodes":["town"],"pay":2,"visit":2},
                    {"nodes":["city","offboard"],"pay":2,"visit":2}],
      "price": 200, "track_type": "broad", "local": false }
  ],

  "start_nodes": [0, 3, 7],
  "static_routes": [],

  "config": {
    "path_timeout_ms": 30000,
    "route_timeout_ms": 10000,
    "route_limit": 10000,
    "train_autoroute_groups": null
  }
}
```

Key design choices:
- **Nodes indexed globally** (not per-hex) so path endpoints use integer indices — avoids string lookups in the hot loop
- **`start_nodes`** pre-sorted by JS (same ordering as current `nodes.sort_by` in `path()`) since ordering requires `route_revenue` per node
- **`converging_exits`** per hex so Rust walker implements converging path logic

## Rust Library Structure

```
rust-autorouter/
├── Cargo.toml
├── src/
│   ├── lib.rs          # WASM entry points, JSON boundary types
│   ├── graph.rs        # Graph, Hex, Node, Path, Junction structs
│   ├── walk.rs         # DFS path walker (the hot path)
│   ├── chain.rs        # Chain builder (visited paths → connection_data)
│   ├── route.rs        # CandidateRoute: paths, revenue estimate, bitfield
│   ├── combo.rs        # Combo optimizer with pruning + async yield
│   ├── bitfield.rs     # Vec<u32> bitfield: set/test/merge/conflicts
│   ├── distance.rs     # Train distance checking (simple + complex)
│   └── types.rs        # NodeType, TrackGauge, TrainDistance enums
├── tests/
│   ├── walk_tests.rs
│   ├── combo_tests.rs
│   └── fixtures/       # JSON graph snapshots from real games
└── benches/
    └── walk_bench.rs
```

### Core Data Structures

```rust
pub struct Graph {
    hexes: HashMap<String, HexData>,
    nodes: Vec<NodeData>,
    paths: Vec<PathData>,
    junctions: Vec<JunctionData>,
}

pub struct NodeData {
    id: usize,
    hex_id: String,
    node_type: NodeType,        // City | Town | Offboard | Junction
    revenue: HashMap<String, i32>,  // phase -> revenue
    slots: u8,
    tokens: Vec<Option<String>>,
    groups: Vec<String>,
    visit_cost: u8,
    path_indices: Vec<usize>,   // precomputed: paths connected to this node
}

pub struct PathData {
    id: usize,
    hex_id: String,
    a: Endpoint,                // Node(idx) | Edge(num) | Junction(idx)
    b: Endpoint,
    track: TrackGauge,          // Broad | Narrow | Dual
    exit_lanes: HashMap<u8, [u8; 2]>,  // edge_num -> [width, index]
    terminal: bool,
    ignore: bool,
    junction_id: Option<usize>,
    node_indices: Vec<usize>,   // precomputed
    edge_nums: Vec<u8>,         // precomputed
}

pub struct Bitfield(Vec<u32>);  // set/conflicts/merge operations
```

### Walk Algorithm (walk.rs)

Faithful port of `Node#walk` (node.rb:43-101) and `Path#walk` (path.rb:133-210):

- `node_walk`: Iterates connected paths, calls `path_walk`, recursively visits next nodes
- `path_walk`: Checks visited/counter/track constraints, traverses junctions and edges
- **Corporation blocking**: Skip nodes where all token slots full and corp not present
- **Edge reuse prevention**: Counter-based (same as Ruby `counter[edge_id]`)
- **Lane matching**: Same arithmetic as `Path#lane_match?` and `lane_invert`
- **Converging paths**: `visited.remove(self)` on return when converging flag set
- **Track gauge matching**: broad↔broad|dual, narrow↔narrow|dual

The callback receives visited paths and builds chains (same logic as the Ruby lambda chain builder in `auto_router.rb:106-131`), then validates each chain as a route for each train.

### Combo Optimizer (combo.rs)

Direct port of JS `Autorouter.find_best_combo` (auto_router.rb:432-471):

- DFS through route combinations (one per train, plus null = skip)
- **Pruning**: `estimate_revenue + max_remaining <= best_so_far` → prune branch
- **Bitfield conflict**: Routes in same train group can't share hexsides
- **Train groups**: Supports `null` (all share), `each_train_separate`, and array-of-groups
- **Async yield**: Every 30ms, yield to browser via `requestAnimationFrame` promise
- **Revenue callback**: When estimate beats best, call JS `real_revenue` for authoritative score

### Revenue Estimation (route.rs)

For pruning, Rust computes base revenue: `sum of stop.revenue[current_phase]`. This matches the default `revenue_for` for ~90% of games. Game-specific bonuses (E/W, bridges, etc.) are handled by the JS `real_revenue` callback — called only for promising combos.

## Revenue Callback Protocol

When combo optimizer finds a promising combination:

```rust
// Rust side: encode as array of global route indices
let indices = Uint32Array::from(&combo_indices[..]);
let revenue: i32 = revenue_callback.call1(&JsValue::NULL, &indices).as_f64() as i32;
```

```javascript
// JS side: maintain flat array of Route objects from walk phase
function revenueCallback(routeIndices) {
    const routes = Array.from(routeIndices).map(idx => allCandidateRoutes[idx]);
    return router.$real_revenue(routes);
}
```

This requires the JS side to maintain a parallel array of `Engine::Route` objects. During the walk phase, as Rust discovers valid routes, it returns enough data (connection_hexes, node_signatures) for JS to construct Route objects. The Rust route index maps into this JS array.

## Integration with Existing Code

### Files to Modify

| File | Changes |
|------|---------|
| `lib/engine/auto_router.rb` | Add `serialize_graph(corp)`, `compute_wasm(corp, **opts)`, WASM dispatch in `compute()` |
| `assets/app/view/game/route_selector.rb` | WASM module lazy loading, `revenue_callback` bridge |
| `Makefile` | Add `wasm` build target |
| `Dockerfile` | Add Rust toolchain + `wasm-pack build` step |

### `auto_router.rb` Changes

```ruby
def compute(corporation, **opts)
  @running = true
  if RUBY_ENGINE == 'opal' && wasm_available?
    compute_wasm(corporation, **opts)
  else
    compute_legacy(corporation, **opts)
  end
end

def compute_legacy(corporation, **opts)
  # existing code (path walk + JS combo), renamed
end

def compute_wasm(corporation, **opts)
  graph_json = serialize_graph(corporation, opts)
  # invoke WASM find_best_routes with callbacks
  # deserialize result into Route objects
end

def serialize_graph(corporation, opts)
  graph = @game.graph_for_entity(corporation)
  # Build JSON with nodes, paths, hexes, trains, config
end
```

### WASM Module Loading (route_selector.rb)

```javascript
let wasmModule = null;
async function loadWasmAutorouter() {
  if (!wasmModule) {
    const mod = await import('/assets/wasm/rust_autorouter.js');
    await mod.default();
    wasmModule = mod;
  }
  return wasmModule;
}
```

### Fallback Strategy

- WASM path wrapped in try/catch — falls back to legacy Ruby+JS autorouter on any failure
- Feature-flagged via user setting (alongside existing `path_timeout`/`route_timeout` settings)
- Legacy autorouter remains fully functional and unchanged

## Build Pipeline

### Cargo.toml

```toml
[package]
name = "rust-autorouter"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
js-sys = "0.3"
web-sys = { version = "0.3", features = ["Window", "Performance"] }

[dev-dependencies]
wasm-bindgen-test = "0.3"
criterion = "0.5"

[profile.release]
opt-level = "z"
lto = true
```

### Build Commands

```bash
# Development (via Rake)
bundle exec rake wasm

# Production (via Rake, optimized)
RACK_ENV=production bundle exec rake wasm

# Direct (without Rake)
cd rust-autorouter && wasm-pack build --target web --out-dir ../public/assets/wasm
```

Output: `public/assets/wasm/rust_autorouter.js` + `rust_autorouter_bg.wasm`

### Rake Build Task (Phase 6)

Add to `Rakefile`:

```ruby
desc 'Build Rust autorouter WASM module'
task :wasm do
  wasm_src = 'rust-autorouter'
  wasm_out = File.join('public', 'assets', 'wasm')

  raise "Rust autorouter source not found at #{wasm_src}/" unless Dir.exist?(wasm_src)

  # Check for wasm-pack
  unless system('wasm-pack --version > /dev/null 2>&1')
    raise 'wasm-pack not found. Install via: cargo install wasm-pack'
  end

  profile = ENV['RACK_ENV'] == 'production' ? '--release' : '--dev'
  sh "cd #{wasm_src} && wasm-pack build #{profile} --target web --out-dir #{File.join('..', wasm_out)}"

  puts "WASM artifacts written to #{wasm_out}/"
end
```

Integration points:
- **Development**: Run `rake wasm` manually after Rust code changes. Dev server picks up files from `public/assets/wasm/`.
- **Production deploy** (`make prod_deploy`): Add `rake wasm` before `rake precompile` in the deploy pipeline. The WASM files get rsynced alongside other assets.
- **Docker build**: WASM artifacts are pre-built and committed to the repo. Docker just copies them — no Rust toolchain needed in the container:
  ```dockerfile
  # No Rust install needed — WASM artifacts are pre-built
  COPY public/assets/wasm/ public/assets/wasm/
  ```
- **Makefile**: Add convenience target:
  ```makefile
  wasm:
  	$(CONTAINER_COMPOSE) exec rack rake wasm
  ```
- **`prod_deploy`**: Update to include WASM artifacts in rsync:
  ```makefile
  prod_deploy : clean
  	$(CONTAINER_COMPOSE) run rack rake wasm && \
  	$(CONTAINER_COMPOSE) run rack rake precompile && \
  	rsync ... public/assets/wasm/*.wasm public/assets/wasm/*.js deploy@18xx:~/18xx/public/assets/wasm/ && \
  	...
  ```
- **`.gitignore`**: Add `rust-autorouter/target/` (Rust build cache). WASM artifacts in `public/assets/wasm/` are committed to the repo since they're pre-built and static — no Rust toolchain needed for deployment.

## Implementation Phases

| Phase | Description | Key Deliverables |
|-------|-------------|-----------------|
| 1 | Project setup + graph model | `Cargo.toml`, `graph.rs`, `types.rs`, `bitfield.rs`, deserialization tests |
| 2 | Walk algorithm | `walk.rs`, `chain.rs`, `distance.rs`, `route.rs` — walk produces candidate routes with correct bitfields |
| 3 | Combo optimizer | `combo.rs` with async yield, revenue callback, pruning — complete `find_best_routes` API |
| 4 | JS integration | `serialize_graph` in `auto_router.rb`, WASM loading, `revenue_callback` bridge, fallback |
| 5 | Testing + tuning | Fixture comparison vs legacy autorouter, performance benchmarks, edge cases |
| 6 | Rake build task | `rake wasm` builds Rust→WASM and places artifacts for dev and prod |

## Testing Strategy

### Rust Unit Tests
- **Walk**: Hand-crafted 3-5 hex graphs as JSON fixtures → verify correct route counts and revenues
- **Bitfield**: Multi-word bitfields, set/conflicts/merge edge cases
- **Combo**: Known-optimal combinations on small inputs (2-3 trains, 5-10 routes each)
- **Distance**: Simple numeric + complex distance arrays with various stop types
- **Lane matching**: Width/index inversion edge cases

### Existing 1870 Fixture Games (from `router-cache` branch)

Three real 1870 mid-game states with long trains already exist on the `router-cache` branch:
- `spec/data/autorouter/34.json` — 1870, 5 players, mid-game
- `spec/data/autorouter/35.json` — 1870, 2×12T, highly constrained board (greedy can't find non-overlapping routes)
- `spec/data/autorouter/36.json` — 1870, 10T+12T, very long autorouting, cross-distance stress test

These fixtures are ideal for Rust vs legacy comparison because they have long trains (12-stop) that exercise the full walk complexity. The `router-cache` branch also has a spec (`auto_router_cache_spec.rb`) with helper methods (`greedy_best_combo`, `walk_with_bitfields`) that can be adapted for comparison tests.

**Comparison test approach** (Ruby RSpec):
1. Load game from fixture JSON (34, 35, 36)
2. Run legacy autorouter (`compute_legacy`) → get `legacy_routes` and `legacy_revenue`
3. Run WASM autorouter (`compute_wasm`) → get `wasm_routes` and `wasm_revenue`
4. Assert: `wasm_revenue >= legacy_revenue` (Rust should match or beat legacy)
5. Assert: all trains filled in both results
6. Assert: route `connection_hexes` are valid (can reconstruct Route objects)

Copy fixtures from `router-cache` branch into the new test infrastructure:
```bash
git checkout router-cache -- spec/data/autorouter/34.json spec/data/autorouter/35.json spec/data/autorouter/36.json
```

### Browser Integration
Manual testing in hotseat mode with WASM enabled, comparing against legacy autorouter results visually.

## Key Source Files (Reference)

| File | Relevance |
|------|-----------|
| `lib/engine/auto_router.rb` | Primary integration point — both phases live here (Ruby walk: lines 45-210, JS combo: lines 289-519) |
| `lib/engine/part/node.rb:43-101` | Node DFS walk — must be faithfully ported |
| `lib/engine/part/path.rb:133-210` | Path walk with edge/junction traversal — the hot inner loop |
| `lib/engine/route.rb` | Route construction, `connection_data`/`connection_hexes` format, revenue calculation |
| `lib/engine/graph.rb` | Graph computation, `connected_nodes`, `connected_hexes` |
| `lib/engine/train.rb` | Train distance format (simple + complex) |
| `assets/app/view/game/route_selector.rb` | UI integration — where autorouter is invoked |

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Game-specific revenue overrides cause estimate vs real_revenue mismatch | Estimate is only for pruning. Add configurable `estimate_padding` (1.1x) to reduce false negatives. JS `real_revenue` callback is authoritative. |
| WASM fails to load on some browsers | Legacy autorouter remains fully functional as fallback, wrapped in try/catch |
| Graph serialization bugs | Fixture comparison tests against legacy autorouter catch discrepancies |
| `wasm-pack` adds Docker build complexity | Can pre-build WASM artifacts and commit to repo; Docker only needs to copy them |
| Walk algorithm port misses edge cases (converging paths, lane matching, junctions) | Exhaustive unit tests on hand-crafted graphs + fixture comparison against legacy for real games |


## Porting revenue_for considerations

 Here's the breakdown of all 76 revenue_for overrides:

  Most common patterns by frequency:

  ┌──────────────────┬───────┬───────────────────────────────────────────────────────────────────────┐
  │     Pattern      │ Count │                              Description                              │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ HEX_BONUS        │ ~40   │ Flat bonus if route touches specific hex. Usually via hex.assigned?() │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ CONNECTION       │ ~33   │ Bonus if route connects two regions (E-W, N-S, hex group A to B)      │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ CUSTOM_FORMULA   │ ~20   │ Completely custom revenue calc (unique per game)                      │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ PER_STOP         │ ~17   │ +N per stop matching a condition (per town, per tokened city, etc.)   │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ TOKEN_BONUS      │ ~10   │ Bonus related to token presence at stops                              │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ ABILITY_BONUS    │ ~9    │ Revenue from ability objects (:hex_bonus, :hexes_bonus)               │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ ROUTE_MULTIPLIER │ ~7    │ Multiply entire route revenue (e.g., super * 2)                       │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ ICON_BONUS       │ ~7    │ Revenue from tile icons                                               │
  ├──────────────────┼───────┼───────────────────────────────────────────────────────────────────────┤
  │ DESTINATION      │ ~3    │ Bonus for reaching destination hex as endpoint                        │
  └──────────────────┴───────┴───────────────────────────────────────────────────────────────────────┘