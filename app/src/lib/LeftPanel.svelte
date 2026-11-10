<script lang="ts">
  import type { Choices, CustomFields, InputRole, RunMethod, Snapshot, StageName, UseWater } from "./api";
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
  } = $props();
</script>

<div class="left">
  <StageList {snap} {busy} {samples} {onRun} {onCancel} {onUseWater} />
  <DataSummary {snap} busy={busy !== null} {dropTarget} {onLoad} {onFetch} {onWater} {onTerrainLimit} {onDownstream} />
  <SizingForm {snap} {choices} {onRuleSets} {onVessel} {onCustomVessel} />
</div>

<style>
  .left { padding: var(--sp-3) var(--sp-4); display: flex; flex-direction: column; gap: var(--sp-5); }
</style>
