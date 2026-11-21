<script lang="ts">
  import type { Choices, CustomFields, FloodSetting, InputRole, OptionTable, ProjectStatus, RunMethod, Snapshot, StageName, UseWater } from "./api";
  import ProjectPanel from "./ProjectPanel.svelte";
  import ChecksList from "./ChecksList.svelte";
  import DesignLineCard from "./DesignLineCard.svelte";
  import DataSummary from "./DataSummary.svelte";
  import SizingForm from "./SizingForm.svelte";
  import StageList from "./StageList.svelte";

  let {
    snap,
    choices,
    onRuleSets,
    onVessel,
    onCustomVessel,
    busy,
    samples,
    dropTarget,
    onRun,
    onCancel,
    onUseWater,
    onLoad,
    onFetch,
    onWater,
    onTerrainLimit,
    onDownstream,
    onMouth,
    onRunInputs,
    onOptimiseLength,
    onOptimiseLand,
    onCorridorLimit,
    onOptimiseFlood,
    lineName = null,
    table,
    indexOf,
    selectedId,
    onSelect,
    stretchesOpen,
    onStretches,
    project = null,
    onProjectLoad = () => {},
  }: {
    snap: Snapshot | null;
    choices: Choices | null;
    onRuleSets: (ids: string[], inputs: Record<string, string>) => void;
    onVessel: (vessel: string, draught: string) => void;
    onCustomVessel: (fields: CustomFields) => void;
    busy: StageName | null;
    samples: number;
    dropTarget: InputRole | null;
    onRun: (stage: StageName, method?: RunMethod) => void;
    onCancel: () => void;
    onUseWater: (choice: UseWater) => void;
    onLoad: (role: InputRole) => void;
    onFetch: () => void;
    onWater: (level: string, fall: string) => void;
    onTerrainLimit: (gib: string) => void;
    onDownstream: (point: string) => void;
    onMouth: (mouth: string) => void;
    onRunInputs: (objective: string, limits: string, level: string) => void;
    onOptimiseLength: (weight: string) => void;
    onOptimiseLand: (weight: string) => void;
    onCorridorLimit: (on: boolean) => void;
    onOptimiseFlood: (setting: FloodSetting, value: string) => void;
    lineName?: string | null;
    /** The design line and its checks, as the session tables them. */
    table: OptionTable | null;
    indexOf: (id: string) => number;
    selectedId: string | null;
    onSelect: (id: string) => void;
    stretchesOpen: boolean;
    onStretches: () => void;
    /** The open project folder (PROJ-1, web app), if any: shown above the Data section. */
    project?: ProjectStatus | null;
    onProjectLoad?: (key: string) => void;
  } = $props();
</script>

<div class="left">
  <StageList {snap} {busy} {samples} {onRun} {onCancel} {onUseWater} {onOptimiseLength} {onOptimiseFlood} {onOptimiseLand} {onCorridorLimit} {lineName} />
  <DesignLineCard {table} {snap} {indexOf} {selectedId} {onSelect} {stretchesOpen} {onStretches} />
  <ChecksList {table} />
  {#if project}<ProjectPanel {project} busy={busy !== null} onLoad={onProjectLoad} />{/if}
  <DataSummary {snap} busy={busy !== null} {dropTarget} {onLoad} {onFetch} {onWater} {onTerrainLimit} {onDownstream} {onMouth} {onRunInputs} />
  <SizingForm {snap} {choices} {onRuleSets} {onVessel} {onCustomVessel} />
</div>

<style>
  .left { padding: var(--sp-3) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
</style>
