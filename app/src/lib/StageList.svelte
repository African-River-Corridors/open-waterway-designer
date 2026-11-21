<script lang="ts">
  import { useWaterChoice } from "./api";
  import type { CascadeFlood, FloodSetting, RunMethod, Snapshot, StageName, StageState, UseWater } from "./api";

  let {
    snap,
    busy,
    samples,
    onRun,
    onCancel,
    onUseWater,
    onOptimiseLength,
    onOptimiseFlood,
    lineName = null,
  }: {
    snap: Snapshot | null;
    /** A stage is running (the name), or nothing is. */
    busy: StageName | null;
    /** How many terrain points the running stage has read: the only progress the engine gives. */
    samples: number;
    onRun: (stage: StageName, method?: RunMethod) => void;
    onCancel: () => void;
    /** Set the rolling circle's water-aware mode (#494). */
    onUseWater: (choice: UseWater) => void;
    /** What made the design line on the map and in the table (its name in the table), if there is one. */
    /** Set the cost of one metre of design line for the pool search (a number as typed). */
    onOptimiseLength: (weight: string) => void;
    /** Set a flood setting of the weir search (a number as typed; an empty limit is no limit). */
    onOptimiseFlood: (setting: FloodSetting, value: string) => void;
    lineName?: string | null;
  } = $props();
  // The stored flood settings: the cost is 0 (off) and the limit is none until set, and neither is written then.
  let floodSettings = $derived(
    snap?.document.stages.optimise.settings as { flood_cost_per_ha?: number; max_above_bank_m?: number | null } | null,
  );
  let storedFloodCost = $derived(floodSettings?.flood_cost_per_ha ?? 0);
  let storedBankLimit = $derived(floodSettings?.max_above_bank_m ?? null);
  let floodCostText = $state("");
  let bankLimitText = $state("");
  let floodCostSeen: number | null = null;
  let bankLimitSeen: number | null | undefined = undefined;
  $effect(() => {
    if (floodCostSeen !== storedFloodCost) {
      floodCostSeen = storedFloodCost;
      floodCostText = String(storedFloodCost);
    }
    if (bankLimitSeen !== storedBankLimit) {
      bankLimitSeen = storedBankLimit;
      bankLimitText = storedBankLimit == null ? "" : String(storedBankLimit);
    }
  });
  // What the pools of the last run flood: there when a flood setting was on for that run.
  let flood = $derived(
    ((snap?.document.stages.optimise.results as { cascade?: { flood?: CascadeFlood } } | null)?.cascade?.flood ?? null) as CascadeFlood | null,
  );
  const fixed = (x: number | null | undefined, digits: number) => (x == null ? "–" : x.toFixed(digits));
  // The stored cost per metre of line; 0 (off) is not written into the design.
  let storedLength = $derived(
    (snap?.document.stages.optimise.settings as { length_weight?: number } | null)?.length_weight ?? 0,
  );
  let lengthText = $state("");
  let lengthSeen: number | null = null;
  $effect(() => {
    if (lengthSeen !== storedLength) {
      lengthSeen = storedLength;
      lengthText = String(storedLength);
    }
  });
  const METHOD_LABEL: Record<RunMethod, string> = { rolling_circle: "Rolling circle", reroute: "Re-route" };

  // Display labels only: the stage names and their order come from the session.
  const LABEL: Record<StageName, string> = {
    sizing: "Sizing",
    alignment: "Alignment",
    optimise: "Optimise",
    volumes: "Volumes",
  };
  const CHIP: Record<StageState, string> = { not_run: "not run", fresh: "fresh", stale: "stale" };
  // Sizing runs by itself when the vessel changes; the other three have a Run button.
  const RUNNABLE: StageName[] = ["alignment", "optimise", "volumes"];
  let method = $state<RunMethod>("rolling_circle");

  // The rolling circle's water-aware mode: the design's setting, and what the last run used.
  const WATER: Record<UseWater, string> = { default: "Default", true: "Keep to the water", false: "Ignore the water" };
  let useWater = $derived(useWaterChoice(snap?.document.stages.alignment.settings?.rolling_circle?.use_water));
  let lastUsed = $derived.by(() => {
    const line = snap?.document.stages.alignment.results;
    const used = line?.method === "rolling_circle" ? line.settings?.use_water : undefined;
    return used == null ? null : WATER[useWaterChoice(used)];
  });
</script>

<section aria-label="Stages">
  <h3>Stages</h3>
  <ol class="stages">
    {#each snap?.stages ?? [] as s}
      <li data-testid="stage-{s.name}">
        <span class="name">{LABEL[s.name]}</span>
        <span class="chip {s.status}" class:running={busy === s.name} data-testid="status-{s.name}">{busy === s.name ? "running" : CHIP[s.status]}</span>
        {#if RUNNABLE.includes(s.name)}
          {#if s.name === "alignment"}
            <select aria-label="Alignment method" bind:value={method} disabled={busy !== null}>
              <option value="rolling_circle">Rolling circle</option>
              <option value="reroute">Re-route</option>
            </select>
          {/if}
          {#if busy === s.name}
            <button data-testid="cancel-{s.name}" onclick={onCancel}>Cancel</button>
          {:else}
            <button
              data-testid="run-{s.name}"
              disabled={busy !== null}
              onclick={() => onRun(s.name, s.name === "alignment" ? method : undefined)}>Run</button
            >
          {/if}
        {/if}
      </li>
      {#if s.name === "alignment"}
        <li class="sub lines" data-testid="alignment-line">
          <span data-testid="line-on-screen">Line on screen: {lineName ?? "none yet"}</span>
          <span data-testid="line-next-run">Next run uses: {METHOD_LABEL[method]}</span>
        </li>
      {/if}
      {#if s.name === "optimise"}
        <li class="sub length" data-testid="length-weight-row">
          <label class="name">
            Cost per metre of line (m³ of cut per m; 0 = off)
            <input type="text" inputmode="decimal" data-testid="length-weight" bind:value={lengthText} disabled={busy !== null} />
          </label>
          <button data-testid="set-length-weight" disabled={busy !== null} onclick={() => onOptimiseLength(lengthText)}>Set</button>
          <span class="dim help" data-testid="length-weight-help">The pool search pays this for every metre of line, so a longer line through deep water is not free. A high value shortens the line and raises the cut.</span>
          <span class="dim" data-testid="length-weight-stored">Stored: {storedLength}</span>
        </li>
        <li class="sub length" data-testid="flood-cost-row">
          <label class="name">
            Cost per hectare of flooded land (0 = off)
            <input type="text" inputmode="decimal" data-testid="flood-cost" bind:value={floodCostText} disabled={busy !== null} />
          </label>
          <button data-testid="set-flood-cost" disabled={busy !== null} onclick={() => onOptimiseFlood("flood_cost_per_ha", floodCostText)}>Set</button>
          <span class="dim help" data-testid="flood-cost-help">The weir search pays this for each hectare of land a pool floods outside the river, so it stops taking the highest lift at every weir. A starting value is 3000, in the money of the cut cost; the right value is yours to set.</span>
          <span class="dim" data-testid="flood-cost-stored">Stored: {storedFloodCost}</span>
        </li>
        <li class="sub length" data-testid="bank-limit-row">
          <label class="name">
            Most a pool may stand above bank-full (m; empty = no limit)
            <input type="text" inputmode="decimal" data-testid="bank-limit" bind:value={bankLimitText} disabled={busy !== null} />
          </label>
          <button data-testid="set-bank-limit" disabled={busy !== null} onclick={() => onOptimiseFlood("max_above_bank_m", bankLimitText)}>Set</button>
          <span class="dim help" data-testid="bank-limit-help">No pool may stand higher than this above the banks anywhere along it. Bank-full is an estimate from the DEM beside the river, so check it against what you know of the banks.</span>
          <span class="dim" data-testid="bank-limit-stored">Stored: {storedBankLimit ?? "no limit"}</span>
        </li>
        {#if flood}
          <li class="sub length" data-testid="flood-pools-row">
            <span data-testid="flood-total">Flooded land: {fixed(flood.flooded_ha, 1)} ha outside the river, flood cost {Math.round(flood.flood_cost).toLocaleString()}</span>
            <table class="pools" data-testid="flood-pools">
              <thead>
                <tr><th>Pool level (m)</th><th>Lift (m)</th><th>Flooded (ha)</th><th>Above bank-full (m)</th></tr>
              </thead>
              <tbody>
                {#each flood.pools as p}
                  <tr data-testid="flood-pool">
                    <td>{fixed(p.level_m, 2)}</td>
                    <td>{fixed(p.lift_m, 2)}</td>
                    <td>{fixed(p.flooded_ha, 1)}{p.reaches_limit ? " or more" : ""}</td>
                    <td>{fixed(p.max_above_bank_m, 2)}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </li>
        {/if}
      {/if}
      {#if s.name === "alignment" && method === "rolling_circle"}
        <li class="sub" data-testid="use-water-row">
          <label class="name">
            Water
            <select
              aria-label="Rolling circle water mode"
              data-testid="use-water"
              title="Default: keep to the water when the design has a river polygon"
              value={useWater}
              disabled={busy !== null}
              onchange={(e) => onUseWater(e.currentTarget.value as UseWater)}
            >
              {#each Object.entries(WATER) as [value, label]}<option {value}>{label}</option>{/each}
            </select>
          </label>
          {#if lastUsed}<span class="dim" data-testid="use-water-last">Last run: {lastUsed}</span>{/if}
        </li>
      {/if}
    {/each}
  </ol>
  {#if busy}
    <p class="progress" role="status" data-testid="progress">
      Running {LABEL[busy].toLowerCase()}{samples > 0 ? `: ${samples.toLocaleString()} DEM points read` : "…"}
    </p>
  {/if}
</section>

<style>
  .stages { list-style: none; margin: 0; padding: 0; }
  li { display: flex; gap: var(--sp-2); align-items: center; padding: 5px 0; border-bottom: var(--lw-thin) solid var(--rule); }
  .name { flex: 1; }
  .sub { padding-left: var(--sp-3); font-size: var(--fs-12); }
  .sub label { display: flex; gap: var(--sp-2); align-items: center; }
  .lines { flex-direction: column; align-items: flex-start; gap: 0; color: var(--ink-3); }
  .dim { color: var(--ink-3); }
  .length { flex-wrap: wrap; }
  .length label { flex-direction: column; align-items: flex-start; }
  .length input { width: 7em; font-size: var(--fs-12); }
  .length .help { flex-basis: 100%; }
  .pools { flex-basis: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  .pools th, .pools td { text-align: right; padding: 2px 6px; border-bottom: var(--lw-thin) solid var(--rule); }
  .pools th { font-weight: 600; color: var(--ink-3); }
  select { font-size: var(--fs-12); padding: 3px 6px; max-width: 120px; }
  /* A word and a shape on every chip; the shape comes from CSS so the word stays the only text. */
  .chip { font-size: var(--fs-11); font-weight: 600; padding: 1px 7px; border-radius: var(--radius); color: var(--ink-3); background: var(--surface-2); }
  .chip.fresh { color: var(--pass); background: var(--pass-soft); }
  .chip.fresh::before { content: "✓ "; }
  .chip.stale { color: var(--warn); background: var(--warn-soft); }
  .chip.stale::before { content: "! "; }
  .chip.running { color: var(--ink); background: var(--accent-soft); }
  .chip.running::before { content: "… "; }
  .progress { margin: var(--sp-2) 0 0; font-size: var(--fs-12); color: var(--ink-3); }
</style>
