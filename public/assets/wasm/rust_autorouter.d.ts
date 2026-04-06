/* tslint:disable */
/* eslint-disable */

/**
 * Primary WASM entry point: run both walk and combo phases.
 *
 * Arguments:
 *   graph_json — JSON string matching GraphInput schema
 *   revenue_callback — JS function(route_combo_json: string) -> number
 *     Called with JSON array of {connection_hexes, node_signatures, train_id} for each route
 *     in a promising combo. Returns the real revenue from game engine, or -1 on error.
 *   progress_callback — JS function(best_routes_json: string) -> bool
 *     Called periodically with current best routes. Returns false to cancel.
 *
 * Returns: JSON string matching AutorouteResult schema.
 */
export function find_best_routes(graph_json: string, revenue_callback: Function, progress_callback: Function): string;

/**
 * Version check.
 */
export function version(): string;

/**
 * Walk-only mode: returns candidate routes per train for diagnostics.
 */
export function walk_paths(graph_json: string): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly find_best_routes: (a: number, b: number, c: any, d: any) => [number, number];
    readonly version: () => [number, number];
    readonly walk_paths: (a: number, b: number) => [number, number];
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
