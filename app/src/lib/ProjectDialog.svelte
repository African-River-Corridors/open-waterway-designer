<script lang="ts">
  // A folder with no `owd-project.json` yet (PROJ-1): what OWD found in the standard places, and
  // one button to use it and write the text file. With more than one design or DEM the user picks.
  import type { ProjectLook } from "./api";

  let {
    look,
    busy,
    onUse,
    onClose,
  }: {
    look: ProjectLook;
    busy: boolean;
    /** Write `owd-project.json` naming these (paths from the folder, or null), then open the project. */
    onUse: (design: string | null, dem: string | null) => void;
    onClose: () => void;
  } = $props();

  let found = $derived(look.found!);
  let design = $state<string | null>(null);
  let dem = $state<string | null>(null);
  // One of a kind is taken; with more than one the user picks; a DEM is optional.
  $effect(() => {
    design = found.designs.length === 1 ? found.designs[0] : null;
    dem = found.dems.length === 1 ? found.dems[0] : null;
  });
  let nothing = $derived(found.designs.length === 0 && found.dems.length === 0);
  let dialog = $state<HTMLDivElement | undefined>();
  $effect(() => dialog?.focus());
</script>

<div class="veil">
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Project folder" tabindex="-1" bind:this={dialog} data-testid="project-dialog" onkeydown={(e) => e.key === "Escape" && onClose()}>
    <h3>Project folder: {look.folder}/</h3>
    {#if nothing}
      <p data-testid="project-nothing">OWD found no design file (.owd) and no DEM (.tif) in this folder. A project folder holds <code>owd-project.json</code>, the design in <code>design/</code>, the DEM in <code>terrain/</code>, the source data in <code>inputs/</code>, and <code>exports/</code> for what OWD writes out.</p>
    {:else}
      <p>This folder has no <code>owd-project.json</code> yet. OWD looked in <code>design/</code>, <code>terrain/</code>, <code>inputs/</code> and the top of the folder, and found:</p>
      <table>
        <tbody>
          <tr>
            <th>Design</th>
            <td>
              {#if found.designs.length === 0}
                <span class="none" data-testid="project-found-design">none: put the design file in design/, then open the project again</span>
              {:else if found.designs.length === 1}
                <span data-testid="project-found-design">{found.designs[0]}</span>
              {:else}
                <select aria-label="Design" bind:value={design} data-testid="project-pick-design">
                  <option value={null}>Pick one of {found.designs.length}…</option>
                  {#each found.designs as d}<option value={d}>{d}</option>{/each}
                </select>
              {/if}
            </td>
          </tr>
          <tr>
            <th>DEM</th>
            <td>
              {#if found.dems.length === 0}
                <span class="none" data-testid="project-found-dem">none (load one later with Load…)</span>
              {:else if found.dems.length === 1}
                <span data-testid="project-found-dem">{found.dems[0]}</span>
              {:else}
                <select aria-label="DEM" bind:value={dem} data-testid="project-pick-dem">
                  <option value={null}>Pick one of {found.dems.length}…</option>
                  {#each found.dems as d}<option value={d}>{d}</option>{/each}
                </select>
              {/if}
            </td>
          </tr>
          {#each found.inputs as i}
            <tr><th>{i.label}</th><td data-testid="project-found-{i.key}">{i.path}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
    <p class="buttons">
      {#if !nothing}
        <button class="primary" data-testid="project-adopt" disabled={busy || !design || (found.dems.length > 1 && !dem)} onclick={() => onUse(design, dem)} title="Writes the small text file into the folder, then opens the design and loads the DEM">Use these and write owd-project.json</button>
      {/if}
      <button data-testid="project-dialog-close" onclick={onClose}>{nothing ? "Close" : "Cancel"}</button>
    </p>
  </div>
</div>

<style>
  .veil { position: fixed; inset: 0; z-index: 40; display: flex; align-items: center; justify-content: center; background: color-mix(in srgb, var(--ink) 35%, transparent); }
  .dialog { background: var(--paper); color: var(--ink); border: var(--lw-thin) solid var(--ink); padding: var(--sp-4); max-width: 560px; width: calc(100% - 2 * var(--sp-4)); font-size: var(--fs-13); }
  h3 { margin: 0 0 var(--sp-2); }
  p { margin: 0 0 var(--sp-2); }
  table { width: 100%; border-collapse: collapse; margin-bottom: var(--sp-3); }
  th { text-align: left; font-weight: 500; padding: var(--sp-1) var(--sp-2) var(--sp-1) 0; border-bottom: var(--lw-thin) solid var(--rule); white-space: nowrap; vertical-align: top; }
  td { padding: var(--sp-1) 0; border-bottom: var(--lw-thin) solid var(--rule); overflow-wrap: anywhere; }
  .none { color: var(--ink-3); }
  .buttons { display: flex; gap: var(--sp-2); justify-content: flex-end; margin: 0; }
</style>
