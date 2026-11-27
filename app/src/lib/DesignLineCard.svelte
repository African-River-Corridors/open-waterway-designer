<script lang="ts">
  import type { OptionTable, Screening, Snapshot, Verdict } from "./api";
  import OptionSwatch from "./OptionSwatch.svelte";
  import { demNotLoaded } from "./words";

  let {
    table,
    snap,
    indexOf,
    selectedId,
    onSelect,
    stretchesOpen,
    onStretches,
  }: {
    table: OptionTable | null;
    snap: Snapshot | null;
    /** A design line's place in the design's list: its colour and hatch, as on the map. */
    indexOf: (id: string) => number;
    selectedId: string | null;
    onSelect: (id: string) => void;
    /** The constraint stretches drawer is open. */
    stretchesOpen: boolean;
    /** Open or close the constraint stretches drawer. */
    onStretches: () => void;
  } = $props();

  /** The screening outputs of the design line, from the document's volumes results. */
  const screeningOf = (id: string | null): Screening | undefined => {
    const row = snap?.document.stages.volumes.results;
    return id && row?.id === id ? row.screening : undefined;
  };
  let screening = $derived(screeningOf(selectedId));

  // Each tag is a word and a shape, never colour alone.
  const VERDICT: Record<Verdict, { shape: string; words: string; tone: string }> = {
    "navigable-as-is": { shape: "✓", words: "navigable as is", tone: "ok" },
    "navigable-with-works": { shape: "◐", words: "navigable with works", tone: "warn" },
    "not-navigable": { shape: "✕", words: "not navigable", tone: "fail" },
  };

  // One display rounding rule per quantity: volumes to whole cubic metres, metres to one decimal.
  const m3 = (x: number | null) => (x == null ? "—" : Math.round(x).toLocaleString("en"));
  const m = (x: number | null, digits = 1) => (x == null ? "—" : x.toFixed(digits));
  const volumesNote = $derived(
    table?.volumes_status === "stale"
      ? demNotLoaded(snap?.warnings)
        ? "Volumes cannot be checked: the elevation model is not loaded. Load the DEM."
        : "Volumes are stale: they were not run on the current inputs."
      : table?.volumes_status === "not run"
        ? "Volumes have not been run: run the volumes stage to fill the dredge, dry cut and tallest face."
        : null,
  );
</script>

<section aria-label="Design line" data-testid="design-line">
  <h3 data-testid="design-line-heading">Design line</h3>
  {#if table && table.rows.length}
    <div class="cards" data-testid="option-table">
      {#each table.rows as r (r.id)}
        <!-- The whole card picks its line with the pointer; the name is the button for the keyboard. -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div
          class="card"
          class:failed={r.failed_checks.length > 0}
          class:selected={r.id === selectedId}
          data-testid="option-row"
          data-option={r.id}
          data-failed={r.failed_checks.length > 0}
          onclick={() => onSelect(r.id)}
        >
          <button class="pick" aria-pressed={r.id === selectedId} title="Show this design line on the map and in the panels" onclick={(e) => { e.stopPropagation(); onSelect(r.id); }}>
            <OptionSwatch index={indexOf(r.id)} />
            <span class="name" data-col="name">{r.name}</span>
          </button>
          <div class="verdict" data-col="verdict">
            {#if screeningOf(r.id)}
              {@const v = VERDICT[screeningOf(r.id)!.verdict]}
              <span class="mark {v.tone}" data-testid="verdict" data-verdict={screeningOf(r.id)!.verdict}><span aria-hidden="true">{v.shape}</span> {v.words}</span>
              {#if r.verdict_reason}<span class="reason" data-testid="verdict-reason">{r.verdict_reason}</span>{/if}
            {:else}<span class="dim">Verdict —</span>{/if}
          </div>
          <dl>
            <dt>Dredge m³</dt><dd data-col="dredge">{m3(r.dredge_m3)}</dd>
            <dt>Dry cut m³</dt><dd data-col="dry-cut">{m3(r.dry_cut_m3)}</dd>
            <dt>Tallest face m</dt><dd data-col="tallest-face">{m(r.tallest_face_m)}</dd>
            <dt title="The design line's own minimum radius">Min radius m</dt><dd data-col="min-radius">{m(r.min_radius_m, 0)}</dd>
            <dt title="The design line's minimum radius against R_min (the R_min check)">Radius ok</dt>
            <dd data-col="radius-ok">
              {#if r.radius_ok === false}<span class="mark fail">✕ fails</span>{:else if r.radius_ok}<span class="mark ok">✓</span>{:else}<span class="dim">—</span>{/if}
            </dd>
            <dt>Checks</dt>
            <dd data-col="checks">
              {#if r.failed_checks.length}<span class="mark fail wrap" aria-label="fails {r.failed_checks.join(', ')}">✕ {r.failed_checks.join(", ")}</span>{:else}<span class="mark ok">✓</span>{/if}
            </dd>
          </dl>
        </div>
      {/each}
    </div>
    {#if volumesNote}<p class="note" data-testid="volumes-note">{volumesNote}</p>{/if}
  {:else}
    <p class="slot" data-testid="no-options">Load bank lines and a river polygon, then run alignment, to see the design line here.</p>
  {/if}
  <button class="stretches" data-testid="stretches-toggle" aria-expanded={stretchesOpen} aria-controls="stretches-drawer" onclick={onStretches} title="The stretches where the river falls short of a Rule, in a drawer under the map">
    Constraint stretches{#if screening}{" "}({screening.constraint_stretches.length}){/if}
  </button>
</section>

<style>
  .cards { display: flex; flex-direction: column; gap: var(--sp-2); }
  .card { padding: var(--sp-2); border: var(--lw-thin) solid var(--rule); border-radius: var(--radius); background: var(--paper); cursor: pointer; font-size: var(--fs-12); }
  .card.selected { background: var(--accent-soft); box-shadow: inset 3px 0 0 var(--ink); border-color: var(--rule-strong); }
  .pick { display: flex; gap: var(--sp-2); align-items: center; width: 100%; padding: 0; border: none; background: none; text-align: left; cursor: pointer; }
  .name { font: 600 var(--fs-13) var(--font-ui); color: var(--ink); min-width: 0; overflow-wrap: anywhere; }
  .verdict { margin: var(--sp-1) 0; }
  .reason { display: block; margin-top: 2px; font-size: var(--fs-11); color: var(--ink-2); }
  dl { display: grid; grid-template-columns: 1fr auto; gap: 2px var(--sp-2); margin: 0; }
  dt { color: var(--ink-2); }
  dd { margin: 0; text-align: right; color: var(--ink); font-variant-numeric: tabular-nums; min-width: 0; }
  /* Status: a word and a shape on a soft ground, never colour alone. */
  .mark { font: 600 var(--fs-11) var(--font-ui); padding: 1px 7px; border-radius: var(--radius); white-space: nowrap; }
  .mark.wrap { white-space: normal; display: inline-block; text-align: left; }
  .mark.fail { color: var(--fail); background: var(--fail-soft); }
  .mark.ok { color: var(--pass); background: var(--pass-soft); }
  .mark.warn { color: var(--warn); background: var(--warn-soft); }
  .dim { color: var(--ink-3); }
  .note { margin: var(--sp-2) 0 0; font-size: var(--fs-12); color: var(--warn); }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2) 0; }
  .stretches { margin-top: var(--sp-2); width: 100%; text-align: left; cursor: pointer; }
  .stretches::after { content: "▸"; float: right; }
  .stretches[aria-expanded="true"] { background: var(--ink); color: var(--paper); border-color: var(--ink); }
  .stretches[aria-expanded="true"]::after { content: "▾"; }
</style>
