// Stand-in types for the wasm-bindgen output (app/wasm-pkg/, built by tools/build_web.sh, not in git),
// so `npm run check` works before the WebAssembly build has run.
declare module "*/wasm-pkg/owd_web.js" {
  export class Session {
    constructor();
    snapshot(): string;
    sizing_choices(): string;
    load_rule_set_text(text: string): string;
    new_design(): string;
    open_text(name: string, text: string, dem?: Uint8Array): string;
    open_project(design: string, text: string, dem_path?: string, dem?: Uint8Array): string;
    open_text_file(name: string, text: string, len?: number, read?: (start: number, len: number) => Uint8Array): string;
    open_project_file(design: string, text: string, dem_path?: string, len?: number, read?: (start: number, len: number) => Uint8Array): string;
    project_read(text: string): string;
    project_find(listing: string): string;
    project_adopt(listing: string, title: string, design?: string, dem?: string): string;
    project_export_path(name: string): string;
    set_water_pools(table: string): string;
    save_text(name: string): string;
    saved(name: string): string;
    is_dirty(): boolean;
    set_rule_sets(ids: string, inputs: string): string;
    set_vessel(vessel_standard_id: string, draught: string): string;
    set_custom_vessel(fields: string): string;
    check_custom_vessel(fields: string): string;
    stretches_geojson(): string;
    run_stale(): string;
    load_input(role: string, name: string, bytes: Uint8Array, layer?: string): string;
    load_dem(name: string, bytes: Uint8Array): string;
    load_dem_file(name: string, len: number, read: (start: number, len: number) => Uint8Array): string;
    set_water(level: string, fall: string): string;
    set_alignment_settings(method: string, settings: string): string;
    set_optimise_settings(settings: string): string;
    set_terrain_limit(gib: string): string;
    set_downstream_point(point: string): string;
    set_mouth(mouth: string): string;
    set_run_inputs(objective: string, limits: string, target_level: string): string;
    run_stage(stage: string, method?: string, progress?: (samples: number) => void): string;
    option_table(): string;
    map_geometry(): string;
    hillshade(): string;
    terrain_info(): string;
    terrain_tile(state: string, z: number, x: number, y: number): Uint8Array;
    contour_tile(state: string, z: number, x: number, y: number, interval_m: number): Uint8Array;
    channel_geometry(): string;
    cut_tile(z: number, x: number, y: number): Uint8Array;
    cut_at(lon: number, lat: number): string;
    map_structures(): string;
    pool_water_info(): string;
    pool_water_tile(id: number, z: number, x: number, y: number): Uint8Array;
    pool_surface_tile(id: number, state: string, z: number, x: number, y: number): Uint8Array;
    pool_water_at(id: number, lon: number, lat: number): string;
    long_section(option: string): string;
    cross_section(option: string, chainage: number): string;
    station_near(option: string, lon: number, lat: number): number;
    export_geojson(): string;
    export_csv(): string;
    reach_route(river: string, points: string, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_search(text: string, country: string, once: boolean, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_river(osm_id: string, also: string, search: string, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_cut(points: string): string;
    reach_restore(at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    reach_route_only(): boolean;
    reach_add_tile(name: string, bytes: Uint8Array): string;
    reach_step(step: string, percentile: number | undefined, at: string, fetch?: (url: string, start: number, len: number) => Uint8Array): string;
    set_bed(depth: string, table: string, season: string): string;
    run_fits(): string;
    run_envelope(): string;
  }
  export function engine_version(): string;
  export default function init(input?: unknown): Promise<unknown>;
}
declare module "*.wasm?url" {
  const url: string;
  export default url;
}
