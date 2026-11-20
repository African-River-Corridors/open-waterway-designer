<script lang="ts">
  import type { OptionTable, Screening, Snapshot, StretchCollection, StretchKind } from "./api";
  import { demNotLoaded } from "./words";

  let {
    table,
    snap,
    stretches,
    selectedId,
    selectedStretch,
    onSelectStretch,
    onClose,
    returnFocus = null,
  }: {
    table: OptionTable | null;
    snap: Snapshot | null;
    /** The constraint stretches as the session exports them, to know which are on the map. */
    stretches: StretchCollection | null;
    selectedId: string | null;
    selectedStretch: { option: string; index: number } | null;
    onSelectStretch: (s: { option: string; index: number } | null) => void;
    onClose: () => void;
    /** What takes the focus when Escape closes the drawer: the button that opened it. */
    returnFocus?: HTMLElement | null;
  } = $props();

  /** The screening outputs of the design line, from the document's volumes results. */
  let screening = $derived.by((): Screening | undefined => {
    const row = snap?.document.stages.volumes.results;
    return selectedId && row?.id === selectedId ? row.screening : undefined;
  });
  let selectedName = $derived(table?.rows.find((r) => r.id === selectedId)?.name ?? selectedId ?? "");
  let onMap = $derived(new Set((stretches?.features ?? []).map((f) => f.properties.option_id)));

  const KIND: Record<StretchKind, string> = { width: "↔", depth: "↕", radius: "↻", infeasible: "✕" };
  const m3 = (x: number | null) => (x == null ? "—" : Math.round(x).toLocaleString("en"));
  const m = (x: number | null, digits = 1) => (x == null ? "—" : x.toFixed(digits));
  const stale = $derived(
    table?.volumes_status === "stale"
      ? demNotLoaded(snap?.warnings)
        ? "Volumes cannot be checked: the elevation model is not loaded. Load the DEM."
        : "Volumes are stale: they were not run on the current inputs."
      : table?.volumes_status === "not run"
        ? "Volumes have not been run: run the volumes stage to fill the dredge, dry cut and tallest face."
        : null,
  );

  function onKey(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    onClose();
    returnFocus?.focus();
  }
</script>

<!-- The key handler is Escape, which closes this panel from anywhere inside it. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="drawer" id="stretches-drawer" aria-label="Constraint stretches" tabindex="-1" data-testid="stretches-panel" onkeydown={onKey}>
  <header>
    <span class="title">Constraint stretches{#if screening}{" "}({screening.constraint_stretches.length}){/if}</span>
    <button data-testid="stretches-close" aria-label="Close the constraint stretches" title="Close (Escape)" onclick={onClose}>Close</button>
  </header>
  <div class="body">
    {#if screening}
      <p class="notice" data-testid="screening-notice"><strong>{screening.notice}.</strong> {screening.rules_notice}</p>
      {#if screening.blocking_not_evaluated.count}
        <p class="note" data-testid="blocking-not-evaluated">! {screening.blocking_not_evaluated.count} blocking Rule(s) were not evaluated: {screening.blocking_not_evaluated.rules.join(", ")}.</p>
      {/if}
      {#if screening.constraint_stretches.length}
        <table data-testid="stretch-table" aria-label="Constraint stretches of {selectedName}">
          <thead>
            <tr>
              <th>Kind</th><th>From m</th><th>To m</th><th>Governing Rule</th><th title="The governing limit (width, depth or radius)">Required</th><th title="The river as found, read on the design line">Found</th><th>Deficit</th><th title="What the design does here">Works</th><th title="The design's own dredge inside this stretch: part of the design total, and a station in two kinds of stretch is in both rows">Design dredge here m³</th><th title="The design's own dry cut inside this stretch: part of the design total, and a station in two kinds of stretch is in both rows">Design dry cut here m³</th>
            </tr>
          </thead>
          <tbody>
            {#each screening.constraint_stretches as c, i}
              {@const pick = { option: selectedId!, index: i }}
              <tr
                class:selected={selectedStretch?.option === selectedId && selectedStretch?.index === i}
                data-testid="stretch-row"
                data-kind={c.kind}
                data-index={i}
                onclick={() => onSelectStretch(pick)}
                tabindex="0"
                aria-selected={selectedStretch?.option === selectedId && selectedStretch?.index === i}
                onkeydown={(e) => (e.key === "Enter" || e.key === " ") && onSelectStretch(pick)}
              >
                <td class="name"><span class="mark kind"><span aria-hidden="true">{KIND[c.kind]}</span> {c.kind}</span></td>
                <td>{m(c.start_chainage_m, 0)}</td>
                <td>{m(c.end_chainage_m, 0)}</td>
                <td class="name"><code>{c.governing_rule ?? "—"}</code>{#if c.reason}<span class="sub">{c.reason}</span>{/if}</td>
                <td>{m(c.required, 2)}</td>
                <td>{m(c.river, 2)}</td>
                <td>{m(c.deficit, 2)}</td>
                <td class="name">{c.works ?? "—"}</td>
                <td>{m3(c.dredge_m3)}</td>
                <td>{m3(c.dry_cut_m3)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if table?.stretch_note}<p class="note" data-testid="stretch-note">{table.stretch_note}</p>{/if}
        {#if !onMap.has(selectedId ?? "")}<p class="note">The map shows no stretches for this option.</p>{/if}
      {:else}
        <p class="slot" data-testid="no-stretches">No constraint stretches: the river meets every Rule along {selectedName}.</p>
      {/if}
      {#if stale}<p class="note">{stale}</p>{/if}
    {:else}
      <p class="slot" data-testid="no-screening">Run volumes to see the verdict and the constraint stretches of the selected option.</p>
    {/if}
  </div>
</section>

<style>
  .drawer { flex: none; display: flex; flex-direction: column; max-height: 45%; min-height: 96px; background: var(--surface); color: var(--ink); border-top: var(--lw-thick) solid var(--ink); }
  header { flex: none; display: flex; align-items: center; gap: var(--sp-2); padding: 3px var(--sp-2); background: var(--surface-2); border-bottom: var(--lw-thin) solid var(--rule-strong); }
  .title { flex: 1; font: 600 var(--fs-11) var(--font-ui); letter-spacing: var(--track-caps); text-transform: uppercase; }
  header button { padding: 1px 8px; font-size: var(--fs-11); cursor: pointer; }
  .body { flex: 1; min-height: 0; overflow: auto; padding: 0 var(--sp-3) var(--sp-2); }
  table { width: 100%; border-collapse: collapse; font-size: var(--fs-13); }
  th { text-align: right; font: 600 var(--fs-11) var(--font-ui); letter-spacing: 0.04em; color: var(--ink-2); padding: var(--sp-1) var(--sp-2); border-bottom: var(--lw-thin) solid var(--ink); position: sticky; top: 0; background: var(--surface-2); }
  th:first-child, td.name { text-align: left; }
  td { text-align: right; padding: 6px var(--sp-2); border-bottom: var(--lw-thin) solid var(--rule); font-variant-numeric: tabular-nums; }
  tbody tr { cursor: pointer; }
  tbody tr:hover td { background: var(--surface-2); }
  tr.selected td { background: var(--accent-soft); }
  tr.selected td:first-child { box-shadow: inset 3px 0 0 var(--ink); }
  .mark { font: 600 var(--fs-11) var(--font-ui); padding: 1px 7px; border-radius: var(--radius); white-space: nowrap; }
  .mark.kind { color: var(--ink); background: var(--surface-2); }
  code { font-size: var(--fs-11); }
  .sub { display: block; font-size: var(--fs-11); color: var(--ink-3); }
  .notice { margin: var(--sp-2) 0; padding: var(--sp-1) var(--sp-2); font-size: var(--fs-12); color: var(--warn); background: var(--warn-soft); }
  .note { margin: var(--sp-2) 0 0; font-size: var(--fs-12); color: var(--warn); }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2) 0; }
</style>
