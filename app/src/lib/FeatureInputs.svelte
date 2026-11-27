<script lang="ts">
  // The buildings, points of interest and settlements (BLD-1): files the design names by path and
  // hash, never copies. Each row says what is loaded, and has Load… and, when loaded, Remove. The
  // building filters (confidence, smallest area, distance from the cut) are set here.
  import * as api from "./api";
  import type { FeatureKey, InputRole, Snapshot } from "./api";

  let {
    snap,
    busy,
    dropTarget,
    onLoad,
    onAction,
  }: {
    snap: Snapshot | null;
    busy: boolean;
    dropTarget: InputRole | null;
    onLoad: (role: InputRole) => void;
    onAction: (action: () => Promise<Snapshot | null>) => void;
  } = $props();

  const ROWS: { key: FeatureKey; label: string; noun: string }[] = [
    { key: "buildings", label: "Buildings", noun: "buildings" },
    { key: "points_of_interest", label: "Places", noun: "places" },
    { key: "settlements", label: "Settlements", noun: "settlements" },
  ];
  const info = (k: FeatureKey) => snap?.feature_inputs?.[k];
  const value = (k: FeatureKey, noun: string) => {
    const i = info(k);
    if (!i) return "not loaded";
    if (!i.attached) return `missing: ${i.path}`;
    if (k === "buildings" && i.counted != null) return `${i.count.toLocaleString()} (${i.counted.toLocaleString()} counted)`;
    return `${i.count.toLocaleString()} ${noun}`;
  };
  let b = $derived(info("buildings"));
  let conf = $state("");
  let area = $state("");
  let near = $state("");
  $effect(() => {
    conf = b?.min_confidence != null ? String(b.min_confidence) : "";
    area = b?.min_area_m2 != null ? String(b.min_area_m2) : "";
    near = b?.near_cut_m != null ? String(b.near_cut_m) : "";
  });
  const num = (t: string) => (t.trim() === "" ? undefined : Number(t));
</script>

<table class="features" data-testid="feature-inputs">
  <tbody>
    {#each ROWS as r (r.key)}
      <tr class:drop={dropTarget === r.key} data-drop-role={r.key} class:missing={info(r.key) && !info(r.key)?.attached}>
        <th>{r.label}</th>
        <td data-testid="data-{r.key}" title={info(r.key)?.path}>{value(r.key, r.noun)}</td>
        <td class="act">
          <button disabled={busy} data-testid="load-{r.key}" onclick={() => onLoad(r.key)}>Load…</button>
          {#if info(r.key)}<button disabled={busy} data-testid="clear-{r.key}" title="Remove it from the design (the file stays where it is)" onclick={() => onAction(() => api.clearFeatures(r.key))}>×</button>{/if}
        </td>
      </tr>
    {/each}
  </tbody>
</table>
{#if b}
  <div class="filters" data-testid="building-filters">
    <label>Confidence at least <input aria-label="Building confidence threshold" inputmode="decimal" bind:value={conf} disabled={busy} data-testid="building-min-confidence" /></label>
    <label>Area at least, m² <input aria-label="Smallest building area" inputmode="decimal" bind:value={area} disabled={busy} data-testid="building-min-area" /></label>
    <label>Near the cut within, m <input aria-label="Near the cut distance" inputmode="decimal" bind:value={near} disabled={busy} data-testid="building-near-cut" /></label>
    <button disabled={busy} data-testid="set-building-filters" onclick={() => onAction(() => api.setBuildingFilters({ min_confidence: num(conf), min_area_m2: num(area), near_cut_m: num(near) }))}>Set</button>
    <p class="hint">A building below the confidence or the area is counted apart and never costed. {#if b.below_confidence != null}Now {b.below_confidence.toLocaleString()} below the confidence, {b.below_area?.toLocaleString()} below the area.{/if}</p>
  </div>
{/if}

<style>
  table { width: 100%; border-collapse: collapse; margin-top: var(--sp-2); }
  th { text-align: left; font-weight: 500; color: var(--ink); padding: var(--sp-1) 0; font-size: var(--fs-12); border-bottom: var(--lw-thin) solid var(--rule); }
  td { text-align: right; padding: var(--sp-1) 0; font-size: var(--fs-12); color: var(--ink-2); font-variant-numeric: tabular-nums; border-bottom: var(--lw-thin) solid var(--rule); }
  td.act { width: 1%; padding-left: var(--sp-2); white-space: nowrap; }
  tr.missing td { color: var(--warn); }
  button { padding: 2px 8px; }
  tr.drop { background: var(--accent-soft); outline: var(--lw-thick) dashed var(--ink); outline-offset: -2px; }
  .filters { display: flex; flex-wrap: wrap; gap: var(--sp-1) var(--sp-2); margin-top: var(--sp-1); align-items: flex-end; }
  .filters label { display: flex; flex-direction: column; gap: 2px; font-size: var(--fs-11); color: var(--ink-2); }
  .filters input { width: 5em; }
  .hint { flex-basis: 100%; font-size: var(--fs-11); color: var(--ink-3); margin: 0; }
</style>
