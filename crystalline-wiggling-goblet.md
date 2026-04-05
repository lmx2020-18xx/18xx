# Plan: Autorouter Path Walk Cache via localStorage

## Context

The autorouter's path walk phase (`lib/engine/auto_router.rb`, `path()` method) is the main bottleneck for long trains (10+ stations). It does an exponential recursive DFS from each token node. Between player turns (5-10 minute waits), the board changes minimally (1-3 hexes from tile lays/token placements). By caching walk results in localStorage and diffing hex fingerprints, we can skip walking unchanged regions and reduce a 30s walk to ~1-2s.

## Key Design Decisions

### Players only — no caching for spectators
Autoroute caching only activates for players who are participants in the game. Spectators/viewers can still use the autoroute button, but results are not cached. Reason: there's no reliable cleanup path for spectator caches — we don't know when they'll stop watching, and game-end cleanup only fires for participants. The check is whether the current user's ID is in the game's player list.

### Activation threshold: 11+ total stops
Only activate caching when the sum of `train.distance` across all `route_trains(corporation)` is >= 11. Below that (e.g. 2x5T), the old code is fast enough. This avoids cache overhead for simple cases.

### Cache level: per-train routes via connection_hexes
The `connections` map (raw walk results) uses live Path objects as keys — not serializable. Per-train routes serialized via `connection_hexes` (hex ID arrays) and `node_signatures` are already serializable and reconstructable via `Route.new(..., connection_hexes:)`.

### Corp-specific walks prevent full route sharing
`node.walk(corporation:)` prunes paths based on token blocking — each corp's walks yield different results. Route-level cache cannot be shared across corporations. However, hex fingerprints and change detection ARE shareable since all players see the same board.

### Cache keyed by board-state fingerprint hash (undo/redo support)
Instead of one cache entry per corporation, cache entries are keyed by `game_id + corp_id + fingerprint_hash`. Each distinct board state gets its own cache slot. This makes undo/redo free cache hits:

```
Undo/redo flow with fingerprint-keyed cache:

1. Board S0 → lay track → S1 → autoroute → cache[fp(S1)] written
2. Undo track → S0 → cache[fp(S0)] hit from earlier → instant!
3. Lay different track → S2 → cache miss → full walk → cache[fp(S2)] written
4. Undo → S0 → cache[fp(S0)] hit again → instant!
5. Redo original → S1 → cache[fp(S1)] hit → instant!

Back-to-back corps (Corp A then Corp B, same player):
6. Corp A routes on S0, cache[A+fp(S0)] written
7. Corp A lays track → S1, places token → S2
8. Corp B routes on S2, cache[B+fp(S2)] written
9. Undo Corp B track back to S2 → cache[B+fp(S2)] hit
10. Undo past Corp A token → S1 → cache[B+fp(S1)] miss (B never routed on S1)
11. Undo Corp A track → S0 → cache[A+fp(S0)] hit!
```

**No invalidation logic needed.** Undo changes the fingerprint hash to a previous value; if we cached that state, it's a hit. Stale entries from explored-but-not-chosen board states are harmless — cleaned up by game-end or LRU eviction.

### localStorage budget per corporation
Each cache entry stores ~200 routes per distance class. At ~200-500 bytes per serialized route, one entry is ~50-100KB. Allow up to 5 board-state entries per corp (LRU eviction beyond that). With ~2 corps per player, total ~500KB-1MB — well within localStorage's 5-10MB limit.

### Route reconstruction cost
`Route.new(..., connection_hexes:)` triggers `find_matching_chains` -> `get_node_chains` which does targeted walks. For 200 cached routes this adds up. Mitigation: cap cached routes aggressively, reconstruct only the top routes by stored revenue.

## Implementation (6 phases, incremental)

### Phase 1: Fingerprinting + Cache Write (no behavioral change)

**Files: `lib/engine/auto_router.rb`, `assets/app/view/game/route_selector.rb`**

1. Add activation guards at top of `path()`. Both must pass for caching to activate:
   - **Player check**: `AutoRouter.new` receives a `participant:` flag from `route_selector.rb`. The flag comes from `@participant` (already computed in `actionable.rb:39` — checks if `@user` ID is in the game's player list). Spectators get normal autorouting with no caching.
   - **Stops threshold**: `total_stops = trains.sum { |t| t.distance.is_a?(Numeric) ? t.distance : t.distance.sum { |h| h['visit'] || 0 } }`. If `total_stops < 11`, skip cache logic.

2. Add `hex_fingerprint_map(corporation)` private method:
   - Get `graph.connected_hexes(corporation).keys`
   - For each hex: `{ hex.id => "#{tile.name}:#{tile.rotation}:#{city_token_str}" }`
   - City tokens: encode corporation IDs on each city slot (captures token blocking)

3. Add `fingerprint_hash(fingerprint_map)` private method:
   - Sort the map, compute a stable hash string (e.g. SHA256 or simple `.hash.to_s(36)`)
   - This becomes part of the cache key: `autoroute_cache_#{game_id}_#{corp_id}_#{fp_hash}`

4. Add `save_route_cache(corporation, train_routes, fp_hash, fingerprint_map)` private method:
   - Group routes by `train.distance` (distance class, not train identity — trains get bought/sold)
   - For each route serialize: `{ connection_hexes, node_signatures, revenue }`
   - Cap at 200 routes per distance class (sorted by revenue desc)
   - Write to `Lib::Storage["autoroute_cache_#{game_id}_#{corp_id}_#{fp_hash}"]`
   - Also store `fingerprint_map` in the entry (for Phase 3 diffing against other entries)
   - Guard with `RUBY_ENGINE == 'opal'`
   - Wrap in `begin/rescue` for localStorage quota errors
   - LRU eviction: if more than 5 entries exist for this `game_id + corp_id`, delete the oldest (by stored timestamp)

5. Call `save_route_cache` at end of `path()` (after line 207 sort/take)

### Phase 2: Cache Read + Full Skip on Exact Match

**File: `lib/engine/auto_router.rb`**

1. Add `load_route_cache(corporation, trains)` private method:
   - Compute current `fingerprint_map` and `fp_hash`
   - Look up exact key `autoroute_cache_#{game_id}_#{corp_id}_#{fp_hash}`
   - On exact hit: board state is identical to when cache was written
     - Match cached routes to available trains by distance class
     - Reconstruct as `Engine::Route.new(@game, @game.phase, train, connection_hexes:)`
     - Validate each with `route.revenue(suppress_check_route_combination: true)` — discard on exception
     - Return `{ cached_routes: Hash[train -> [Route]], exact_hit: true }`
   - On miss: return `{ exact_hit: false, current_fingerprints: fingerprint_map }`

2. In `path()`, after threshold check but before the walk loop (~line 87):
   - Call `load_route_cache`
   - If exact hit → skip walk entirely, use cached routes
   - Recompute bitfields for cached routes, run sort/take, return

### Phase 3: Near-Miss — Merge from Closest Cached State

**File: `lib/engine/auto_router.rb`**

When no exact cache hit exists, check for a **near-miss** — a cached entry for the same corp with a different fingerprint hash:

1. Scan localStorage for keys matching `autoroute_cache_#{game_id}_#{corp_id}_*`
2. For each entry, load its stored `fingerprint_map` and diff against current:
   - `changed_hex_ids` = hexes where fingerprint differs, plus added/removed hexes
3. Pick the entry with the **fewest** changed hexes (closest board state)
4. From that entry, keep cached routes whose `connection_hexes` do NOT intersect `changed_hex_ids`
5. Run the normal full walk (all nodes), producing fresh routes
6. Merge surviving cached routes with fresh routes (deduplicate by `connection_hexes`)
7. Recompute bitfields for all merged routes, sort/take as normal

**Why this helps undo/redo with changes:**
- Player routes on S0 (cached), lays track (S1), routes on S1 (cached), undoes, lays DIFFERENT track (S2)
- S2 has no exact hit, but S0 and S1 are cached. S2 differs from S0 by 1 hex (the new track) and from S1 by 1 hex (different track). Either near-miss provides routes that don't touch the changed hex.
- The full walk runs but the merge preserves high-revenue routes the walk might miss due to timeout.

### Phase 4: Background Precompute During Opponent Turns

**File: `assets/app/view/game/route_selector.rb`**

- On entering route selection view (`render` method), if no exact cache exists for current corporation AND threshold met, trigger a background precompute using `requestIdleCallback` (via `%x{}` JS block)
- Creates a temporary `AutoRouter`, runs only `path()` (which populates cache), discards result
- Short timeout (10s) to avoid hogging resources
- Future enhancement: trigger when opponent action received via MessageBus (before user enters route view)

### Phase 5: UI — Delete Cache Button

**File: `assets/app/view/game/game_info.rb`** (or Tools section)

- Add "Clear Autoroute Cache" button under Tools in the game menu
- On click: delete all localStorage keys matching `autoroute_cache_#{@game.id}_*`
- Show flash message confirming cache cleared
- Essential escape hatch if cache corruption causes bad routing

### Phase 6: Lifecycle Cleanup

**Files: `lib/engine/auto_router.rb`, `assets/app/view/game/actionable.rb`, `assets/app/view/home.rb`**

1. **Game end cleanup**: When game finishes, delete all autoroute caches for that game.
   - Hook into `actionable.rb` where `game.finished` is detected
   - Delete keys matching `autoroute_cache_#{game_id}_*`

2. **Frontpage stale cache sweep**: When `Home` renders, run a one-shot cleanup:
   - Scan `Lib::Storage.all_keys` for keys matching `autoroute_cache_*`
   - Parse the `ts` field from each entry
   - Delete any entry older than 2 weeks (1_209_600 seconds)
   - Since caching only activates for participants (Phase 1 guard), all cache entries belong to games the user is in — no need to cross-check game membership during sweep
   - This handles slow-moving games where game-end cleanup hasn't fired
   - Run once per page load (guard with a class-level flag to avoid re-running on re-renders)
   - Keeps localStorage from accumulating multi-MB records across many concurrent games

3. **LRU eviction** (also in Phase 1 write): Keep max 5 entries per `game_id + corp_id`. On write, if over limit, delete entry with oldest timestamp.

4. **`skip_paths` integration**: When user has pre-set routes, filter cached routes whose connection_hexes overlap with skip_paths hexes.

5. **localStorage quota handling**: If write fails (QuotaExceededError), scan all `autoroute_cache_*` keys, delete entries from other finished/old games first, then retry. If still fails, reduce route cap to 100 and retry.

## Cache Key + Entry Structure

```
Key: "autoroute_cache_{game_id}_{corp_id}_{fp_hash}"

Value (JSON):
{
  "ts": 1700000000,                    // write timestamp for LRU
  "fingerprints": { "A1": "57:0:PRR,", "B2": "14:3:", ... },
  "routes": {
    "12": [                             // keyed by train distance
      { "ch": [["A1","B2"],["B2","C3"]], "ns": ["A1-0","C3-0"], "rev": 340 },
      ...
    ],
    "8": [ ... ]
  }
}
```

## Files to Modify

| File | Phase | Changes |
|------|-------|---------|
| `lib/engine/auto_router.rb` | 1-3 | Threshold check, fingerprinting, fp_hash, cache read/write, near-miss merge |
| `assets/app/view/game/route_selector.rb` | 4 | Background precompute trigger |
| `assets/app/view/game/game_info.rb` | 5 | Delete cache button in Tools |
| `assets/app/view/game/actionable.rb` | 6 | Game-end cache cleanup |
| `assets/app/view/home.rb` | 6 | Frontpage stale cache sweep (entries older than 2 weeks) |

## Existing Code to Reuse

- `Lib::Storage` (`assets/app/lib/storage.rb`) — JSON localStorage wrapper with `[]`, `[]=`, `delete`, `all_keys`
- `RUBY_ENGINE == 'opal'` guard pattern (used in `lib/engine/tile.rb`, `lib/engine/game/base.rb`)
- `Route.new(..., connection_hexes:)` reconstruction path (`lib/engine/route.rb:371-421`)
- `route.connection_hexes` serialization (`lib/engine/route.rb:363-369`)
- `route.node_signatures` (`lib/engine/route.rb:91`)
- `train.distance` attribute (`lib/engine/train.rb`)
- `graph.connected_hexes(corporation)` for fingerprint source

## Future Considerations (not in scope)

- **Cross-corp fingerprint sharing via MessageBus**: All players see the same board, so fingerprint maps and changed_hex_ids are identical. Could broadcast "board hash at action N" to let other clients skip fingerprint computation. Deferred because fingerprinting is cheap relative to walking.
- **Opponent-turn precompute trigger**: Start cache warming when MessageBus delivers opponent's action, not just when entering route view. Requires hooking into action receipt in `actionable.rb`.
- **Cross-corp route sharing**: Would require walking WITHOUT corporation-specific blocking (superset of paths), then filtering per-corp on read. Makes individual walks slower (no blocking pruning). Net benefit unclear — revisit if fingerprint sharing proves valuable.

## Verification

1. **Phase 1**: Run autoroute on a 12-train game, inspect localStorage for cache key with fp_hash. Run autoroute on a 4-train game, verify NO cache key written.
2. **Phase 2**: Run autoroute twice with no board changes — second run should complete in <1s. Undo a track lay, re-autoroute — should hit the earlier cache entry instantly.
3. **Phase 3**: Lay track, autoroute (cached). Undo, lay different track, autoroute — near-miss merge should preserve routes not touching changed hex.
4. **Phase 4**: Enter route view, verify cache populated in background before clicking Auto.
5. **Phase 5**: Click "Clear Autoroute Cache", verify localStorage keys removed, next autoroute does full walk.
6. **Undo stress test**: Route → undo → different track → route → undo → undo → route. Verify each step uses cache when available, results always correct.
7. **End-to-end**: Play test games 35 and 36 with autorouting, verify results match non-cached autorouter.

## Test Games

- **Game 35**: Moderate autorouting challenge (next move requires substantive routing)
- **Game 36**: Very long autorouting task (stress test for cache performance)
