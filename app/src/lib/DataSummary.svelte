<script lang="ts">
  import type { InputRole, Snapshot } from "./api";
  import { platform } from "./platform";
  import { bankValue, fileNameOf, FALL_HELP, roundedFall } from "./words";

  let {
    snap,
    busy,
    dropTarget,
    onLoad,
    onFetch,
    onWater,
    onTerrainLimit,
    onDownstream,
    onMouth,
    onRunInputs,
  }: {
    snap: Snapshot | null;
    busy: boolean;
    /** The row a dragged file is over, if any. */
    dropTarget: InputRole | null;
    onLoad: (role: InputRole) => void;
    onFetch: () => void;
    onWater: (level: string, fall: string) => void;
    /** Set the terrain memory limit of this machine, in GiB (empty: the default). */
    onTerrainLimit: (gib: string) => void;
    /** Set the point the river flows towards for the next bank import, `LON,LAT` (empty: none). */
    onDownstream: (point: string) => void;
    /** State the river mouth, `downstream` or `LON,LAT` (spec 0017). */
    onMouth: (mouth: string) => void;
    /** Set the run's objective, limits (one a line) and target level (spec 0017), as `owd run-inputs` does. */
    onRunInputs: (objective: string, limits: string, level: string) => void;
  } = $props();

  let inputs = $derived(snap?.document.inputs);
  let crs = $derived(snap?.document.project.crs);
  // The CRSs the data came in, when they are not the project's own: the zone it was moved to.
  let moved = $derived(
    [...new Set(Object.values(snap?.document.project.source_crs ?? {}))].filter((c) => crs && c !== `EPSG:${crs.epsg}`),
  );
  let level = $state("");
  /** The fall as the document holds it (exact); the field shows it rounded until the user types. */
  let fallExact = $state("");
  let fallTyped = $state<string | null>(null);
  let fallShown = $derived(fallTyped ?? roundedFall(fallExact));
  let limitGib = $state("");
  let downstream = $state("");
  let mouth = $state("");
  let objective = $state("");
  let limits = $state("");
  let runLevel = $state("");
  // Show the document's run inputs in the text forms the fields take.
  $effect(() => {
    const r = inputs?.run;
    objective = r?.objective ? `${r.objective.sense}:${r.objective.measure}` : "";
    limits = (r?.limits ?? []).map((l) => `${l.measure}${l.bound === "at_most" ? "<=" : ">="}${l.value}${l.currency ? ` ${l.currency}` : ""}`).join("\n");
    runLevel = r?.target_level ? r.target_level.replace("_", "-") : "";
  });
  // Show the document's water level, as a level at the start and the fall to the end.
  $effect(() => {
    const p = inputs?.water_level?.points;
    if (!p?.length) {
      level = "";
      fallExact = "";
      fallTyped = null;
      return;
    }
    level = String(p[0][1]);
    fallTyped = null;
    const last = p[p.length - 1];
    fallExact = p.length > 1 && last[0] > p[0][0] ? String((p[0][1] - last[1]) / (last[0] - p[0][0])) : "";
  });

  const RIVER_ROWS: { role: InputRole; label: string }[] = [
    { role: "left_bank", label: "Left bank" },
    { role: "right_bank", label: "Right bank" },
    { role: "river_polygon", label: "River polygon" },
    { role: "centreline", label: "Centreline" },
  ];
  const DEM_ROW: { role: InputRole; label: string }[] = [{ role: "dem", label: "DEM" }];
  const value = (role: InputRole) => {
    if (role === "dem") return inputs?.dem ? fileNameOf(inputs.dem.path) : "not loaded";
    if (role === "river_polygon") return inputs?.river_polygon ? `${inputs.river_polygon.coordinates[0].length} positions` : "not loaded";
    return bankValue(inputs?.[role], role !== "centreline" && !!inputs?.river_polygon);
  };
  /** The full path of a loaded DEM, for the tooltip. */
  const fullPath = (role: InputRole) => (role === "dem" ? (inputs?.dem?.path ?? undefined) : undefined);
</script>

<section aria-label="Data">
  <h3>Data</h3>
  <table>
    <tbody>
      <tr><th>Project CRS</th><td data-testid="project-crs" colspan="2">{crs ? crs.name : "none yet"}</td></tr>
      {#if moved.length}
        <tr><td colspan="3" class="moved" data-testid="moved-note">Data in {moved.join(", ")} was moved to {crs?.name}.</td></tr>
      {/if}
      {#each RIVER_ROWS as r}
        <tr class:drop={dropTarget === r.role} data-drop-role={r.role}>
          <th>{r.label}</th>
          <td data-testid="data-{r.role}" title={fullPath(r.role)}>{value(r.role)}</td>
          <td class="act"><button disabled={busy} data-testid="load-{r.role}" onclick={() => onLoad(r.role)}>Load…</button></td>
        </tr>
      {/each}
    </tbody>
  </table>

  <div class="water mouth">
    <label>River mouth: where chainage 0 is
      <input aria-label="River mouth" bind:value={mouth} disabled={busy} data-testid="river-mouth" />
    </label>
    <p class="hint">Type <code>downstream</code> for the lower end of the river, or a point as lon,lat. Now: {inputs?.mouth ? `set at ${inputs.mouth.stated_as === "downstream" ? "the lower end" : "a point"}` : "none, so chainage runs from the start of the centreline"}.</p>
    <button disabled={busy} data-testid="set-river-mouth" onclick={() => onMouth(mouth)} title="The river is stored running from the mouth, and every chainage counts from it. `downstream` takes the lower end, by the water level or the DEM.">Set mouth</button>
  </div>

  <table class="dem">
    <tbody>
      {#each DEM_ROW as r}
        <tr class:drop={dropTarget === r.role} data-drop-role={r.role}>
          <th>{r.label}</th>
          <td data-testid="data-{r.role}" title={fullPath(r.role)}>{value(r.role)}</td>
          <td class="act"><button disabled={busy} data-testid="load-{r.role}" onclick={() => onLoad(r.role)}>Load…</button></td>
        </tr>
      {/each}
    </tbody>
  </table>
  <p class="row">
    {#if platform().kind === "tauri"}<button disabled={busy} data-testid="fetch-dem" onclick={onFetch} title="Download a Copernicus GLO-30 DEM for the reach, beside the saved design">Fetch DEM</button>{/if}
  </p>
  <p class="hint">Drop a file on its row to load it. GeoJSON and GeoPackage in WGS84 or UTM; DEM as GeoTIFF.</p>

  <div class="water">
    <label>Water level at the river start (m)
      <input aria-label="Water level" inputmode="decimal" bind:value={level} disabled={busy} data-testid="water-level" />
    </label>
    <label>Fall per metre of river (optional)
      <input aria-label="Water fall" inputmode="decimal" value={fallShown} oninput={(e) => (fallTyped = e.currentTarget.value)} disabled={busy} data-testid="water-fall" title={fallExact !== "" ? `Stored exactly: ${fallExact}` : undefined} />
    </label>
    <p class="hint" data-testid="water-fall-help">{FALL_HELP}</p>
    <button disabled={busy || level.trim() === ""} data-testid="set-water" onclick={() => onWater(level, fallTyped ?? fallExact)}>Set water level</button>
  </div>

  <div class="water">
    <label>Terrain memory limit (GiB, this machine; empty: {snap?.terrain_limit_bytes ? `${(snap.terrain_limit_bytes / 2 ** 30).toFixed(1)} set` : "the default"})
      <input aria-label="Terrain memory limit" inputmode="decimal" bind:value={limitGib} disabled={busy} data-testid="terrain-limit" />
    </label>
    <button disabled={busy} data-testid="set-terrain-limit" onclick={() => onTerrainLimit(limitGib)} title="A terrain file whose grid needs more memory than this is refused. Not saved in the design.">Set limit</button>
  </div>

  <div class="water">
    <label>Downstream point for the next bank import (lon,lat; empty: {snap?.downstream_lon_lat ? `${snap.downstream_lon_lat.join(", ")} set` : "the banks' own direction"})
      <input aria-label="Downstream point" bind:value={downstream} disabled={busy} data-testid="downstream-point" />
    </label>
    <button disabled={busy} data-testid="set-downstream-point" onclick={() => onDownstream(downstream)} title="The banks are run towards this point and named left and right looking downstream, whatever way they were drawn or named.">Set point</button>
  </div>

  <div class="water">
    <label>Objective (`minimise:cost`, `minimise:total_cut`…; empty: none)
      <input aria-label="Objective" bind:value={objective} disabled={busy} data-testid="run-objective" />
    </label>
    <label>Limits, one a line (`total_cut&lt;=500000`, `cost&lt;=2000000 USD`; empty: none)
      <textarea aria-label="Limits" rows="2" bind:value={limits} disabled={busy} data-testid="run-limits"></textarea>
    </label>
    <label>Target output level
      <select aria-label="Target output level" bind:value={runLevel} disabled={busy} data-testid="run-target-level">
        <option value="">none (concept)</option>
        <option value="concept">concept</option>
        <option value="pre-feasibility">pre-feasibility</option>
        <option value="feasibility">feasibility</option>
      </select>
    </label>
    <button disabled={busy} data-testid="set-run-inputs" onclick={() => onRunInputs(objective, limits, runLevel)} title="What the run aims at, what it must stay within, and the output level it aims to reach. Nothing optimises on them yet, so no stage goes stale.">Set run inputs</button>
  </div>
</section>

<style>
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: 500; color: var(--ink); padding: var(--sp-1) 0; font-size: var(--fs-12); border-bottom: var(--lw-thin) solid var(--rule); }
  td { text-align: right; padding: var(--sp-1) 0; font-size: var(--fs-12); color: var(--ink-2); font-variant-numeric: tabular-nums; border-bottom: var(--lw-thin) solid var(--rule); }
  td.act { width: 1%; padding-left: var(--sp-2); }
  button { padding: 2px 8px; }
  /* A dragged file over a row: a dashed ink outline, so colour is not the only signal. */
  tr.drop { background: var(--accent-soft); outline: var(--lw-thick) dashed var(--ink); outline-offset: -2px; }
  .moved { text-align: left; font-size: var(--fs-12); color: var(--warn); background: var(--warn-soft); padding: var(--sp-1) var(--sp-2); }
  table.dem { margin-top: var(--sp-2); }
  .mouth { margin-top: var(--sp-2); }
  .row { margin: var(--sp-2) 0 0; }
  .hint { font-size: var(--fs-11); color: var(--ink-3); margin: var(--sp-1) 0 0; }
  .water { margin-top: var(--sp-3); display: flex; flex-direction: column; gap: var(--sp-1); }
  .water label { display: flex; flex-direction: column; gap: 3px; font-size: var(--fs-12); color: var(--ink-2); }
  .water button { align-self: flex-start; margin-top: var(--sp-1); }
</style>
