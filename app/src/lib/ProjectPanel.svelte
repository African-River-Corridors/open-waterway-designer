<script lang="ts">
  // The open project folder (PROJ-1): its title, each item `owd-project.json` names and how it
  // stands, and a Load button for what is in the folder but not in the design yet.
  import type { ProjectStatus } from "./api";
  import { STATE_WORDS } from "./project";

  let {
    project,
    busy,
    onLoad,
  }: {
    project: ProjectStatus;
    busy: boolean;
    onLoad: (key: string) => void;
  } = $props();

  /** The states that have something to load. */
  const LOADS = ["found", "changed", "set"];
</script>

<section aria-label="Project" data-testid="project-panel">
  <h3>Project</h3>
  <p class="title"><span data-testid="project-title">{project.title}</span> <span class="folder" data-testid="project-folder" title="The project folder's name. Save writes the design back into it; exports go into its exports/ folder.">in {project.folder}/</span></p>
  <table>
    <tbody>
      {#each project.items as item (item.key)}
        <tr data-testid="project-item-{item.key}" data-state={item.state}>
          <th>{item.label}</th>
          <td title={item.path ?? undefined}>
            {#if item.path}<span class="path">{item.path}</span>{/if}
            <span class="state" class:bad={item.state === "missing" || item.state === "changed"} data-testid="project-state-{item.key}">{STATE_WORDS[item.state]}</span>
          </td>
          <td class="act">
            {#if LOADS.includes(item.state)}
              <button disabled={busy} data-testid="project-load-{item.key}" onclick={() => onLoad(item.key)} title={item.state === "changed" ? "Use this DEM: the design then records it, and the volumes go stale" : `Load ${item.path} into the design`}>{item.state === "changed" ? "Use this DEM" : "Load"}</button>
            {/if}
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
  <p class="hint">Listed in owd-project.json. Nothing leaves this machine.</p>
</section>

<style>
  table { width: 100%; border-collapse: collapse; }
  th { text-align: left; font-weight: 500; color: var(--ink); padding: var(--sp-1) 0; font-size: var(--fs-12); border-bottom: var(--lw-thin) solid var(--rule); vertical-align: top; white-space: nowrap; padding-right: var(--sp-2); }
  td { text-align: right; padding: var(--sp-1) 0; font-size: var(--fs-12); color: var(--ink-2); border-bottom: var(--lw-thin) solid var(--rule); }
  td.act { width: 1%; padding-left: var(--sp-2); white-space: nowrap; }
  button { padding: 2px 8px; }
  .title { margin: 0 0 var(--sp-1); font-size: var(--fs-13); color: var(--ink); font-weight: 600; }
  .folder { font-weight: 400; color: var(--ink-3); font-size: var(--fs-12); }
  .path { display: block; overflow-wrap: anywhere; }
  .state { color: var(--ink-3); }
  .state.bad { color: var(--warn); font-weight: 600; }
  .hint { font-size: var(--fs-11); color: var(--ink-3); margin: var(--sp-1) 0 0; }
</style>
