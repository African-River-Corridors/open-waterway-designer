<script lang="ts">
  import { useWaterChoice } from "./api";
  import type { CascadeFlood, FloodSetting, JoinSetting, OptimiseMethod, RunMethod, Snapshot, StageName, StageState, UseWater, WindowRow } from "./api";

  let {
    snap,
    busy,
    samples,
    onRun,
    onCancel,
    onUseWater,
    onRiverFollowing,
    onOptimiseLength,
    onOptimiseLand,
    onCorridorLimit,
    onOptimiseFlood,
    onJoinSetting,
    onOptimiseMethod,
    onWindowLength,
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
    /** Set river-following settings (OPT-4). */
    onRiverFollowing: (settings: Record<string, unknown>) => void;
    /** What made the design line on the map and in the table (its name in the table), if there is one. */
    /** Set the cost of one metre of design line for the pool search (a number as typed). */
    onOptimiseLength: (weight: string) => void;
    /** Set the cost of one m² of land taken for the pool search (a number as typed). */
    onOptimiseLand: (weight: string) => void;
    /** Hold the pool search inside the corridor, or not. */
    onCorridorLimit: (on: boolean) => void;
    /** Set a flood setting of the weir search (a number as typed; an empty limit is no limit). */
    onOptimiseFlood: (setting: FloodSetting, value: string) => void;
    onJoinSetting: (key: JoinSetting, value: boolean | string) => void;
    /** Choose the optimise stage's method (OPT-5). */
    onOptimiseMethod: (method: OptimiseMethod) => void;
    /** Set the window optimiser's window length (metres, as typed). */
    onWindowLength: (metres: string) => void;
    lineName?: string | null;
  } = $props();
  // The stored flood settings: the cost is 0 (off) and the limit is none until set, and neither is written then.
  let floodSettings = $derived(
    snap?.document.stages.optimise.settings as { flood_cost_per_ha?: number; max_above_bank_m?: number | null; building_cost?: number } | null,
  );
  let storedFloodCost = $derived(floodSettings?.flood_cost_per_ha ?? 0);
  // BLD-1: the cost per building flooded; 0 (off) until set.
  let storedBuildingCost = $derived(floodSettings?.building_cost ?? 0);
  let buildingCostText = $state("");
  let buildingCostSeen: number | null = null;
  $effect(() => {
    if (buildingCostSeen !== storedBuildingCost) {
      buildingCostSeen = storedBuildingCost;
      buildingCostText = String(storedBuildingCost);
    }
  });
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
  // The optimise stage's method (OPT-5): `lattice` unless the design says otherwise.
  type WindowsStored = { method?: OptimiseMethod; windows?: { window_target_m?: number } };
  let optimiseStored = $derived((snap?.document.stages.optimise.settings as WindowsStored | null) ?? {});
  let optimiseMethodNow = $derived<OptimiseMethod>(optimiseStored.method ?? "lattice");
  let storedWindow = $derived(optimiseStored.windows?.window_target_m ?? 1500);
  let windowText = $state("");
  let windowSeen: number | null = null;
  $effect(() => {
    if (windowSeen !== storedWindow) {
      windowSeen = storedWindow;
      windowText = String(storedWindow);
    }
  });
  // The windows of the last window run, from its design line's details.
  let windowRows = $derived(
    (((snap?.document.stages.optimise.results as { design_line?: { method?: string; details?: { windows?: WindowRow[] } } } | null)
      ?.design_line?.method === "windows"
      ? (snap?.document.stages.optimise.results as { design_line: { details: { windows?: WindowRow[] } } }).design_line.details.windows
      : null) ?? null) as WindowRow[] | null,
  );
  const WHY: Record<string, string> = {
    gain: "cheaper",
    polished_start: "polished start cheaper",
    under_threshold: "less than 1 % cheaper",
    dearer: "dearer",
    loop: "loops",
    radius: "breaks R_min",
    left_river: "leaves the river",
    not_closed: "could not join the seams",
    cap: "search limit",
    timeout: "time limit",
    no_path: "no line found",
    no_ground: "no ground",
    too_short: "too short",
    crosses_line: "crosses the line",
    frozen: "frozen",
    cancelled: "cancelled",
  };
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
  // The stored cost per m² of land; 0 (off) is not written into the design.
  let storedLand = $derived(
    (snap?.document.stages.optimise.settings as { land_weight?: number } | null)?.land_weight ?? 0,
  );
  let landText = $state("");
  let landSeen: number | null = null;
  $effect(() => {
    if (landSeen !== storedLand) {
      landSeen = storedLand;
      landText = String(storedLand);
    }
  });
  // Whether the pool search is held to the corridor; off is not written into the design.
  let storedCorridorLimit = $derived(
    (snap?.document.stages.optimise.settings as { corridor_limit?: boolean } | null)?.corridor_limit ?? false,
  );
  // The optimise stage ran with no DEM: its line was optimised on plan area, not volume.
  let onPlanArea = $derived(
    (snap?.document.stages.optimise.results as { optimised_on?: string } | null)?.optimised_on === "plan_area",
  );
  // The join settings of the pool search (OPT-3); none is written into the design at its default.
  type JoinStored = Partial<Record<JoinSetting, number | boolean | null>>;
  let joins = $derived((snap?.document.stages.optimise.settings as JoinStored | null) ?? {});
  const JOIN_TICKS: { key: JoinSetting; id: string; label: string; help: string }[] = [
    {
      key: "seams_on_straights",
      id: "seams-on-straights",
      label: "Put seams on straights",
      help: "A long pool is searched in pieces; this puts each seam between two pieces where the river line is straightest and inside the river.",
    },
    {
      key: "free_seams",
      id: "free-seams",
      label: "Free the line at seams and weirs",
      help: "The line may pass a seam or a weir site anywhere across the river instead of on the river line's exact point and heading, and it runs straight through a weir site.",
    },
  ];
  const JOIN_LENGTHS: { key: JoinSetting; id: string; label: string; help: string; unset: string }[] = [
    {
      key: "seam_window_m",
      id: "seam-window",
      label: "Seam search window (m)",
      help: "How far a seam may move from the even split of a long pool to find a straight.",
      unset: "1500",
    },
    {
      key: "min_straight_m",
      id: "min-straight",
      label: "Straight between opposite curves (m; 0 = off)",
      help: "The line runs straight for at least this length between a left curve and a right curve.",
      unset: "0",
    },
    {
      key: "weir_straight_m",
      id: "weir-straight",
      label: "Straight through a weir site (m; empty = two vessel lengths)",
      help: "With the line freed at weirs, it runs straight for this length through each weir site, half upstream and half downstream.",
      unset: "",
    },
  ];
  const joinStored = (key: JoinSetting, unset: string) => {
    const v = joins[key];
    return v === undefined || v === null ? unset : String(v);
  };
  let joinText = $state<Record<string, string>>({});
  let joinSeen: Record<string, string> = {};
  $effect(() => {
    for (const f of JOIN_LENGTHS) {
      const now = joinStored(f.key, f.unset);
      if (joinSeen[f.key] !== now) {
        joinSeen[f.key] = now;
        joinText[f.key] = now;
      }
    }
  });
  const METHOD_LABEL: Record<RunMethod, string> = { rolling_circle: "Rolling circle", reroute: "Re-route", river_following: "River-following" };

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

  // The river-following line (OPT-4): its record, the stored settings, the tolerance as typed.
  let rf = $derived(snap?.document.stages.alignment.results?.river_following ?? null);
  let rfSettings = $derived(snap?.document.stages.alignment.settings?.river_following ?? {});
  let rfForce = $derived((rfSettings.cut_force ?? {}) as Record<string, boolean>);
  let tolText = $state("");
  let tolSeen: string | null = null;
  $effect(() => {
    const now = rfSettings.seed_tolerance_m == null ? "" : String(rfSettings.seed_tolerance_m);
    if (tolSeen !== now) {
      tolSeen = now;
      tolText = now;
    }
  });
  /** Force a cut-through on or off by its id, or give it back to the rule. */
  function force(id: string, choice: string) {
    const next: Record<string, boolean> = { ...rfForce };
    if (choice === "rule") delete next[id];
    else next[id] = choice === "on";
    onRiverFollowing({ cut_force: next });
  }
  const forceChoice = (id: string) => (id in rfForce ? (rfForce[id] ? "on" : "off") : "rule");
  const FORCED: Record<string, string> = { tight_bend: "River tighter than R_min", straight: "Moved to keep a straight", off_river: "Out of the river" };
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
              <option value="river_following">River-following</option>
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
        {#if onPlanArea}
          <li class="sub length" data-testid="plan-area-note">
            <span class="help">Optimised on plan area, not volume: no elevation model is loaded. The search kept the channel inside the river polygon; it chose no weirs. Load a DEM and run Optimise again to optimise on volume.</span>
          </li>
        {/if}
        <li class="sub length" data-testid="optimise-method-row">
          <label class="name">
            Method
            <select aria-label="Optimise method" data-testid="optimise-method" value={optimiseMethodNow} disabled={busy !== null} onchange={(e) => onOptimiseMethod(e.currentTarget.value as OptimiseMethod)}>
              <option value="lattice">Weirs and pool lattice</option>
              <option value="windows">Window optimiser</option>
            </select>
          </label>
          <span class="dim help" data-testid="optimise-method-help">The window optimiser redesigns the design line in windows of 1 to 2 km, pricing every move with the volumes' own cross-section, and keeps a window's change only when it is at least 1 % cheaper. It keeps the pools and weirs as they are.</span>
        </li>
        {#if optimiseMethodNow === "windows"}
          <li class="sub length" data-testid="window-length-row">
            <label class="name">
              Window length (m)
              <input type="text" inputmode="decimal" data-testid="window-length" bind:value={windowText} disabled={busy !== null} />
            </label>
            <button data-testid="set-window-length" disabled={busy !== null} onclick={() => onWindowLength(windowText)}>Set</button>
            <span class="dim" data-testid="window-length-stored">Stored: {storedWindow}</span>
          </li>
        {/if}
        {#if windowRows}
          <li class="sub length" data-testid="windows-row">
            <table class="pools" data-testid="windows">
              <thead>
                <tr><th>Start (m)</th><th>End (m)</th><th>Cost before</th><th>Cost after</th><th>Kept</th></tr>
              </thead>
              <tbody>
                {#each windowRows as w (w.id)}
                  <tr data-testid="window">
                    <td>{fixed(w.start_m, 0)}</td>
                    <td>{fixed(w.end_m, 0)}</td>
                    <td>{fixed(w.full_start.cost, 0)}</td>
                    <td>{fixed(w.full_found?.cost, 0)}</td>
                    <td>{w.adopted === "none" ? `not adopted: ${WHY[w.reason] ?? w.reason}` : w.adopted === "found" ? `adopted${w.pass === 2 ? " (pass 2)" : ""}` : "polished start"}</td>
                  </tr>
                {/each}
              </tbody>
            </table>
          </li>
        {/if}
        <li class="sub length" data-testid="length-weight-row">
          <label class="name">
            Cost per metre of line (m³ of cut per m; 0 = off)
            <input type="text" inputmode="decimal" data-testid="length-weight" bind:value={lengthText} disabled={busy !== null} />
          </label>
          <button data-testid="set-length-weight" disabled={busy !== null} onclick={() => onOptimiseLength(lengthText)}>Set</button>
          <span class="dim help" data-testid="length-weight-help">The pool search pays this for every metre of line, so a longer line through deep water is not free. A high value shortens the line and raises the cut.</span>
          <span class="dim" data-testid="length-weight-stored">Stored: {storedLength}</span>
        </li>
        <li class="sub length" data-testid="land-weight-row">
          <label class="name">
            Cost per m² of land (m³ of cut per m²; 0 = off)
            <input type="text" inputmode="decimal" data-testid="land-weight" bind:value={landText} disabled={busy !== null} />
          </label>
          <button data-testid="set-land-weight" disabled={busy !== null} onclick={() => onOptimiseLand(landText)}>Set</button>
          <span class="dim help" data-testid="land-weight-help">The pool search pays this for every m² of channel outside the river polygon, even where nothing is cut, so the line keeps to the river. A value equal to the channel depth prices land like a full-depth cut.</span>
          <span class="dim" data-testid="land-weight-stored">Stored: {storedLand}</span>
        </li>
        <li class="sub length" data-testid="corridor-limit-row">
          <label class="tick">
            <input type="checkbox" data-testid="corridor-limit" checked={storedCorridorLimit} disabled={busy !== null} onchange={(e) => onCorridorLimit(e.currentTarget.checked)} />
            Keep the line inside the corridor
          </label>
          <span class="dim help" data-testid="corridor-limit-help">The pool search may not go further from the baseline than the corridor's half-width. A pool with no line inside the corridor keeps the baseline and says so.</span>
        </li>
        {#each JOIN_TICKS as t (t.key)}
          <li class="sub length" data-testid="{t.id}-row">
            <label class="tick">
              <input type="checkbox" data-testid={t.id} checked={joins[t.key] === true} disabled={busy !== null} onchange={(e) => onJoinSetting(t.key, e.currentTarget.checked)} />
              {t.label}
            </label>
            <span class="dim help" data-testid="{t.id}-help">{t.help}</span>
          </li>
        {/each}
        {#each JOIN_LENGTHS as f (f.key)}
          <li class="sub length" data-testid="{f.id}-row">
            <label class="name">
              {f.label}
              <input type="text" inputmode="decimal" data-testid={f.id} bind:value={joinText[f.key]} disabled={busy !== null} />
            </label>
            <button data-testid="set-{f.id}" disabled={busy !== null} onclick={() => onJoinSetting(f.key, joinText[f.key] ?? "")}>Set</button>
            <span class="dim help" data-testid="{f.id}-help">{f.help}</span>
            <span class="dim" data-testid="{f.id}-stored">Stored: {joinStored(f.key, f.unset)}</span>
          </li>
        {/each}
        <li class="sub length" data-testid="flood-cost-row">
          <label class="name">
            Cost per hectare of flooded land (0 = off)
            <input type="text" inputmode="decimal" data-testid="flood-cost" bind:value={floodCostText} disabled={busy !== null} />
          </label>
          <button data-testid="set-flood-cost" disabled={busy !== null} onclick={() => onOptimiseFlood("flood_cost_per_ha", floodCostText)}>Set</button>
          <span class="dim help" data-testid="flood-cost-help">The weir search pays this for each hectare of land a pool floods outside the river, so it stops taking the highest lift at every weir. A starting value is 3000, in the money of the cut cost; the right value is yours to set.</span>
          <span class="dim" data-testid="flood-cost-stored">Stored: {storedFloodCost}</span>
        </li>
        <li class="sub length" data-testid="building-cost-row">
          <label class="name">
            Cost per building flooded (0 = off)
            <input type="text" inputmode="decimal" data-testid="building-cost" bind:value={buildingCostText} disabled={busy !== null} />
          </label>
          <button data-testid="set-building-cost" disabled={busy !== null} onclick={() => onOptimiseFlood("building_cost", buildingCostText)}>Set</button>
          <span class="dim help" data-testid="building-cost-help">The weir search pays this for each building a pool floods, so it moves weirs and lowers pools to flood fewer. It is your price, for example a resettlement cost, not a fact: it needs a buildings file in Data.</span>
          <span class="dim" data-testid="building-cost-stored">Stored: {storedBuildingCost}</span>
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
            {#if flood.buildings}<span data-testid="flood-buildings">Buildings flooded: {flood.buildings.flooded.toLocaleString()} of {flood.buildings.loaded.toLocaleString()} (water at least {flood.buildings.min_depth_m} m deep), building cost {Math.round(flood.buildings.building_cost).toLocaleString()}; {flood.buildings.flooded_below_confidence + flood.buildings.flooded_below_area} more counted apart (below the confidence or the area).</span>{/if}
            <table class="pools" data-testid="flood-pools">
              <thead>
                <tr><th>Pool level (m)</th><th>Lift (m)</th><th>Flooded (ha)</th><th>Above bank-full (m)</th>{#if flood.buildings}<th>Buildings</th>{/if}{#if flood.settlement_flooded_ha != null}<th>Settlement (ha)</th>{/if}</tr>
              </thead>
              <tbody>
                {#each flood.pools as p}
                  <tr data-testid="flood-pool">
                    <td>{fixed(p.level_m, 2)}</td>
                    <td>{fixed(p.lift_m, 2)}</td>
                    <td>{fixed(p.flooded_ha, 1)}{p.reaches_limit ? " or more" : ""}</td>
                    <td>{fixed(p.max_above_bank_m, 2)}</td>
                    {#if flood.buildings}<td data-testid="flood-pool-buildings">{p.buildings_flooded ?? 0}</td>{/if}
                    {#if flood.settlement_flooded_ha != null}<td>{fixed(p.settlement_flooded_ha ?? 0, 1)}</td>{/if}
                  </tr>
                {/each}
              </tbody>
            </table>
          </li>
        {/if}
      {/if}
      {#if s.name === "alignment" && method === "river_following"}
        <li class="sub length" data-testid="rf-tolerance-row">
          <label class="name">
            Seed tolerance to the centreline (m; empty = default)
            <input type="text" inputmode="decimal" data-testid="rf-tolerance" bind:value={tolText} disabled={busy !== null} />
          </label>
          <button data-testid="set-rf-tolerance" disabled={busy !== null} onclick={() => onRiverFollowing({ seed_tolerance_m: tolText.trim() === "" ? null : Number(tolText) })}>Set</button>
          <span class="dim help" data-testid="rf-tolerance-help">How far the seed may stand from the river centreline outside the bends tighter than R_min. The default (5 m, or an eighth of the bottom width when more) gives one arc per bend; a tighter value gives compound curves.</span>
        </li>
        {#if rf}
          <li class="sub lines" data-testid="rf-summary">
            <span>Seed: {rf.seed.arc_count} arcs, {rf.seed.element_count} elements, largest gap {fixed(rf.seed.gap_m, 1)} m; costed on {rf.costed_on === "volume" ? "volume" : "plan area (no DEM)"}</span>
            {#each rf.warnings as w}<span class="help" data-testid="rf-warning">{w}</span>{/each}
          </li>
          {#if rf.cut_throughs.length > 0}
            <li class="sub length" data-testid="rf-cuts-row">
              <table class="pools" data-testid="rf-cuts">
                <thead>
                  <tr><th>Cut-through</th><th>Saves (m)</th><th>Stay cost</th><th>Cut cost</th><th>Face (m)</th><th>Decision</th><th>Force</th></tr>
                </thead>
                <tbody>
                  {#each rf.cut_throughs as c (c.id)}
                    <tr data-testid="rf-cut">
                      <td>{c.id}</td>
                      <td>{fixed(c.options[c.offered].saved_m, 0)}</td>
                      <td>{Math.round(c.stay.cost).toLocaleString()}</td>
                      <td>{Math.round(c.options[c.offered].numbers.cost).toLocaleString()}</td>
                      <td>{fixed(c.options[c.offered].numbers.face_max_m, 1)}</td>
                      <td title={c.refused_because ?? ""}>{c.accepted ? "taken" : "refused"} ({c.decided_by})</td>
                      <td>
                        <select aria-label="Force {c.id}" data-testid="rf-force" value={forceChoice(c.id)} disabled={busy !== null} onchange={(e) => force(c.id, e.currentTarget.value)}>
                          <option value="rule">By the rule</option>
                          <option value="on">On</option>
                          <option value="off">Off</option>
                        </select>
                      </td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </li>
          {/if}
          {#if rf.forced.length > 0}
            <li class="sub lines" data-testid="rf-forced">
              {#each rf.forced as f (f.id)}
                <span data-testid="rf-forced-stretch">{FORCED[f.kind] ?? f.kind}: {fixed(f.from, 0)} to {fixed(f.to, 0)} m{f.river_radius_m != null ? `, river radius ${fixed(f.river_radius_m, 0)} m` : ""}, up to {fixed(f.farthest_from_centreline_m, 1)} m from the centreline</span>
              {/each}
            </li>
          {/if}
          <li class="sub lines" data-testid="rf-bends">
            {#each rf.seed.bends as b, i}
              <span data-testid="rf-bend">Bend {i + 1}: R {fixed(b.radius_m, 0)} m {b.hand}, gap {fixed(b.gap_m, 1)} m{b.tight ? ", held at the least radius" : ""}</span>
            {/each}
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
  .length label.tick { flex-direction: row; align-items: center; gap: 0.4em; }
  .length label.tick input { width: auto; }
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
