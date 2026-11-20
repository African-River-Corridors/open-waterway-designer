<script lang="ts">
  import type { OptionTable } from "./api";

  let { table }: { table: OptionTable | null } = $props();
</script>

<section aria-label="Checks" data-testid="checks">
  <h3>Checks</h3>
  {#if table && table.checks.length}
    <ul data-testid="checks-list">
      {#each table.checks as c}
        <li class:failed={!c.passed} data-testid="check" data-passed={c.passed}>
          <span class="mark {c.passed ? 'ok' : 'fail'}">{c.passed ? "✓" : "✕"}</span>
          <strong>{c.option}</strong> {c.check}: {c.detail}
        </li>
      {/each}
    </ul>
  {:else}
    <p class="slot">No checks yet.</p>
  {/if}
</section>

<style>
  ul { list-style: none; margin: 0; padding: 0; font-size: var(--fs-12); }
  li { padding: 5px 0; border-bottom: var(--lw-thin) solid var(--rule); overflow-wrap: anywhere; }
  li.failed { color: var(--fail); }
  .mark { font: 600 var(--fs-11) var(--font-ui); padding: 1px 7px; border-radius: var(--radius); margin-right: var(--sp-1); }
  .mark.fail { color: var(--fail); background: var(--fail-soft); }
  .mark.ok { color: var(--pass); background: var(--pass-soft); }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2) 0; }
</style>
