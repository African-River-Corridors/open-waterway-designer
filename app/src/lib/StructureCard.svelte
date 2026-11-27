<script lang="ts">
  // The small card of one weir (MAP-4): what the placeholder model assumes there.
  import type { MapStructures, PoolWaterInfo, WeirSite } from "./api";
  import { PLACEHOLDER_NOTE, chamberText, km, levelChangeText } from "./structures";

  let {
    weir,
    structures,
    poolWater = null,
    onClose,
  }: {
    weir: WeirSite;
    structures: MapStructures;
    /** The flooded ground of each pool, once the pool water has been worked out. */
    poolWater?: PoolWaterInfo | null;
    onClose: () => void;
  } = $props();

  const m1 = (v: number) => v.toFixed(1);
  const land = $derived(poolWater?.pools.find((p) => p.number === weir.upstream_pool) ?? null);
</script>

<section class="card" data-testid="weir-card" aria-label="Weir {weir.number}">
  <header>
    <h3>Weir {weir.number} and lock</h3>
    <button class="close" data-testid="weir-card-close" aria-label="Close" onclick={onClose}>×</button>
  </header>
  <p class="note" data-testid="weir-card-note">{PLACEHOLDER_NOTE}</p>
  <dl>
    <dt>Chainage</dt>
    <dd data-testid="weir-card-chainage">{km(weir.chainage_m)} ({Math.round(weir.chainage_m)} m)</dd>
    <dt>Upstream pool level</dt>
    <dd data-testid="weir-card-upstream">{m1(weir.upstream_level_m)} m<span class="sub">the minimum level this weir holds{weir.upstream_pool ? `, pool ${weir.upstream_pool}` : ""}</span></dd>
    <dt>Downstream level</dt>
    <dd data-testid="weir-card-downstream">{m1(weir.downstream_level_m)} m{#if weir.downstream_pool}<span class="sub">pool {weir.downstream_pool}</span>{/if}</dd>
    <dt>Lift</dt>
    <dd data-testid="weir-card-lift">{m1(weir.lift_m)} m</dd>
    <dt>Lock chamber</dt>
    <dd data-testid="weir-card-chamber">{chamberText(structures)}</dd>
    <dt>Weir length</dt>
    <dd data-testid="weir-card-length">{Math.round(weir.weir_length_m)} m<span class="sub">{weir.clipped_to_river ? "the river polygon's width here" : "a default length across the design line: the line is not inside a river polygon here"}</span></dd>
    <dt>Land flooded</dt>
    <dd data-testid="weir-card-land">
      {#if land}
        {land.outside_river_ha != null ? `${m1(land.outside_river_ha)} ha outside the river polygon` : `${m1(land.flooded_ha)} ha flooded in all (no river polygon)`}<span class="sub">by pool {land.number} at {m1(land.level_m)} m</span>
      {:else}
        <span class="sub">tick Pool water in Layers to work it out</span>
      {/if}
    </dd>
  </dl>
  <p class="rules" data-testid="weir-card-level">{levelChangeText(weir)}</p>
  <p class="rules" data-testid="weir-card-rules">Sizes of this {PLACEHOLDER_NOTE}: {structures.model.rules}.</p>
</section>

<style>
  .card { position: absolute; left: var(--sp-3); bottom: 44px; z-index: 5; width: 290px; max-width: calc(100% - 2 * var(--sp-3)); max-height: calc(100% - 90px); overflow-y: auto; padding: var(--sp-2) var(--sp-3); font-size: var(--fs-12); color: var(--ink); background: var(--surface); border: var(--lw-thin) solid var(--ink); }
  header { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-2); }
  h3 { margin: 0; font-size: var(--fs-12); font-weight: 600; }
  .close { width: 22px; height: 22px; padding: 0; line-height: 1; background: none; border: var(--lw-thin) solid var(--rule-strong); border-radius: var(--radius); color: var(--ink); }
  .note { margin: 2px 0 var(--sp-2); padding: 1px var(--sp-1); display: inline-block; color: var(--warn); border: var(--lw-thin) solid var(--warn); font-weight: 600; }
  dl { margin: 0; display: grid; grid-template-columns: auto 1fr; gap: 3px var(--sp-3); }
  dt { color: var(--ink-2); }
  dd { margin: 0; font-variant-numeric: tabular-nums; }
  .sub { display: block; color: var(--ink-3); }
  .rules { margin: var(--sp-2) 0 0; color: var(--ink-2); }
</style>
