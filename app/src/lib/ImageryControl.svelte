<script lang="ts">
  // The Imagery setting: the pictures under the map, chosen per user and kept on this machine only.
  import {
    CUSTOM_FAILED, DEFAULT_IMAGERY, ESRI_FAILED, layerFor, loadImagery, maskKey, saveImagery, usable, validTileUrl, esriMetadataUrl,
    type ImageryLayer, type ImagerySettings, type ImagerySource,
  } from "./imagery";

  let {
    openTiles,
    layer = $bindable(layerFor(DEFAULT_IMAGERY)),
    note = $bindable<string | null>(null),
  }: {
    /** The open default's tile address. */
    openTiles: string;
    /** The imagery the map should draw now. */
    layer?: ImageryLayer;
    /** One plain line when the chosen imagery failed and the map fell back; null otherwise. */
    note?: string | null;
  } = $props();

  const initial = loadImagery();
  let settings = $state<ImagerySettings>(initial);
  let open = $state(false);
  let keyDraft = $state("");
  let urlDraft = $state(initial.customUrl);
  let attrDraft = $state(initial.customAttribution);
  /** The source the page fell back from, until the settings change. */
  let fellBack = $state(false);
  /** Esri's data attribution, once its service has answered with the key: until then the open imagery shows. */
  let esriCopyright = $state<string | null>(null);
  let esriReady = $state(false);
  let check = 0;

  const failMessage = (s: ImagerySource) => (s === "esri" ? ESRI_FAILED : CUSTOM_FAILED);

  function apply(next: ImagerySettings) {
    settings = next;
    saveImagery(next);
    fellBack = false;
    note = null;
    esriReady = false;
    esriCopyright = null;
    const mine = ++check;
    if (next.source === "esri" && usable(next)) {
      // The service's own answer tells us at once whether the key works, and gives the attribution Esri asks for.
      // A failure is reported in plain words; the error itself (which could carry the address) is never kept.
      fetch(esriMetadataUrl(next.esriKey))
        .then((r) => (r.ok ? r.json() : Promise.reject(null)))
        .then((j: { copyrightText?: unknown; error?: unknown }) => {
          if (mine !== check) return;
          if (j.error) throw null;
          esriCopyright = typeof j.copyrightText === "string" ? j.copyrightText : null;
          esriReady = true;
        })
        .catch(() => {
          if (mine === check) fail("esri");
        });
    }
  }

  /** The chosen imagery failed: show one line and go back to the open default. */
  export function fail(source: ImagerySource) {
    if (settings.source !== source || fellBack) return;
    fellBack = true;
    note = failMessage(source);
  }

  $effect(() => {
    const useOpen = settings.source === "open" || fellBack || (settings.source === "esri" && !esriReady);
    layer = layerFor(useOpen ? { ...settings, source: "open" } : settings, openTiles, esriCopyright);
  });

  const onChoice = (e: Event) => apply({ ...settings, source: (e.currentTarget as HTMLSelectElement).value as ImagerySource });
  const setKey = () => {
    const k = keyDraft.trim();
    keyDraft = "";
    if (k) apply({ ...settings, esriKey: k });
  };
  const clearKey = () => apply({ ...settings, esriKey: "" });
  const setCustom = () => apply({ ...settings, customUrl: urlDraft.trim(), customAttribution: attrDraft.trim() });

  // Start-up: a remembered Esri choice is checked again.
  apply(initial);
</script>

<span class="imagery">
  <button aria-expanded={open} data-testid="imagery-toggle" onclick={() => (open = !open)}>Imagery</button>
  {#if open}
    <div class="panel" data-testid="imagery-panel">
      <label>
        Background imagery
        <select aria-label="Imagery" data-testid="imagery-select" value={settings.source} onchange={onChoice}>
          <option value="open">Sentinel-2 cloudless 2016 (EOX, open)</option>
          <option value="esri">Esri World Imagery (your Esri key)</option>
          <option value="custom">Custom tiles</option>
        </select>
      </label>
      {#if settings.source === "esri"}
        {#if settings.esriKey}
          <div class="row">
            <span>Esri key</span>
            <code data-testid="imagery-key-masked">{maskKey(settings.esriKey)}</code>
            <button data-testid="imagery-key-clear" onclick={clearKey}>Clear</button>
          </div>
        {:else}
          <div class="row">
            <input type="password" autocomplete="off" spellcheck="false" aria-label="Esri API key" data-testid="imagery-key" placeholder="Your Esri API key" bind:value={keyDraft} onkeydown={(e) => e.key === "Enter" && setKey()} />
            <button data-testid="imagery-key-set" disabled={!keyDraft.trim()} onclick={setKey}>Set</button>
          </div>
        {/if}
        <p class="hint">The key stays on this machine. It is used only to ask Esri for tiles. Your own Esri terms apply.</p>
      {:else if settings.source === "custom"}
        <input type="text" autocomplete="off" spellcheck="false" aria-label="Tile address" data-testid="imagery-url" placeholder="https://your-tile-server/{'{z}'}/{'{x}'}/{'{y}'}.png" bind:value={urlDraft} />
        <input type="text" aria-label="Attribution" data-testid="imagery-attribution" placeholder="Attribution text (required)" bind:value={attrDraft} />
        <div class="row">
          <button data-testid="imagery-custom-apply" disabled={!validTileUrl(urlDraft) || !attrDraft.trim()} onclick={setCustom}>Use these tiles</button>
        </div>
        <p class="hint">An XYZ address with {"{z}"}, {"{x}"} and {"{y}"}. The attribution is shown on the map.</p>
      {/if}
      <p class="hint">Chosen per person, kept on this machine, never saved in the design. Drawings and reports use no imagery.</p>
    </div>
  {/if}
</span>

<style>
  .imagery { position: relative; display: inline-block; font-size: var(--fs-12); }
  .panel { position: absolute; right: 0; top: 100%; z-index: 20; width: 320px; display: flex; flex-direction: column; gap: var(--sp-2); padding: var(--sp-3); background: var(--surface); color: var(--ink); border: var(--lw-thin) solid var(--ink); }
  .panel label { display: flex; flex-direction: column; gap: var(--sp-1); color: var(--ink-3); }
  .panel input { width: 100%; box-sizing: border-box; }
  .row { display: flex; gap: var(--sp-2); align-items: center; }
  .row input { flex: 1; }
  .hint { margin: 0; color: var(--ink-3); }
</style>
