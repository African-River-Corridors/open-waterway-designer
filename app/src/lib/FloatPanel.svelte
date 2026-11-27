<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { clampRect, storedRect, storeRect, type PanelRect } from "./layout";

  let {
    id,
    title,
    place,
    onClose,
    returnFocus = null,
    children,
  }: {
    /** The panel's name in storage and in its test ids, for example `profile`. */
    id: string;
    title: string;
    /** Where the panel starts when the user has not placed it: given the map area's size. */
    place: (width: number, height: number) => PanelRect;
    onClose: () => void;
    /** What takes the focus when Escape closes the panel: the button that opened it. */
    returnFocus?: HTMLElement | null;
    children: Snippet;
  } = $props();

  let el: HTMLElement;
  /** Where the user put the panel (now or in an earlier visit); none yet: the default place applies. */
  let placed = $state<PanelRect | null>(null);
  /** The size of the map area the panel floats over. */
  let area = $state<{ width: number; height: number } | null>(null);
  /** Where the panel shows: its place, kept inside the area as the area is now. */
  let rect = $derived(area ? clampRect(placed ?? place(area.width, area.height), area.width, area.height) : null);

  onMount(() => {
    placed = storedRect(id);
    const parent = el.offsetParent as HTMLElement | null;
    const measure = () => {
      const [width, height] = [parent?.clientWidth ?? window.innerWidth, parent?.clientHeight ?? window.innerHeight];
      if (width > 0 && height > 0) area = { width, height };
    };
    measure();
    // The map area changes size with the window, the left panel and the drawer: keep the panel inside it.
    const ro = new ResizeObserver(measure);
    if (parent) ro.observe(parent);
    return () => ro.disconnect();
  });

  /** Move (by the title bar) or resize (by the corner) with the pointer; remember the place on release. */
  function grab(kind: "move" | "resize") {
    return (down: PointerEvent) => {
      if (!rect || !area || down.button !== 0 || (down.target as HTMLElement).closest("button")) return;
      down.preventDefault();
      const target = down.currentTarget as HTMLElement;
      try {
        target.setPointerCapture(down.pointerId);
      } catch {
        /* no such pointer (a scripted event): the bar still gets the moves made over it */
      }
      const start = { ...rect };
      const { width, height } = area;
      const move = (e: PointerEvent) => {
        const dx = e.clientX - down.clientX;
        const dy = e.clientY - down.clientY;
        placed =
          kind === "move"
            ? clampRect({ ...start, x: start.x + dx, y: start.y + dy }, width, height)
            : clampRect({ ...start, w: Math.min(start.w + dx, width - start.x), h: Math.min(start.h + dy, height - start.y) }, width, height);
      };
      const up = () => {
        target.removeEventListener("pointermove", move);
        target.removeEventListener("pointerup", up);
        target.removeEventListener("pointercancel", up);
        if (placed) storeRect(id, placed);
      };
      target.addEventListener("pointermove", move);
      target.addEventListener("pointerup", up);
      target.addEventListener("pointercancel", up);
    };
  }

  /** Take the keyboard focus: Escape then closes the panel. */
  export function focus() {
    el?.focus();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    onClose();
    returnFocus?.focus();
  }
</script>

<!-- The key handler is Escape, which closes this panel from anywhere inside it. -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section
  class="float"
  class:ready={rect !== null}
  bind:this={el}
  aria-label={title}
  tabindex="-1"
  data-testid="{id}-panel"
  style:left="{rect?.x ?? 0}px"
  style:top="{rect?.y ?? 0}px"
  style:width="{rect?.w ?? 0}px"
  style:height="{rect?.h ?? 0}px"
  onkeydown={onKey}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="bar" data-testid="{id}-bar" title="Drag to move" onpointerdown={grab("move")}>
    <span class="title">{title}</span>
    <button class="close" data-testid="{id}-close" aria-label="Close the {title.toLowerCase()} panel" title="Close (Escape)" onclick={onClose}>Close</button>
  </header>
  <div class="content">{@render children()}</div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="grip" data-testid="{id}-grip" title="Drag to resize" aria-hidden="true" onpointerdown={grab("resize")}></div>
</section>

<style>
  /* The imagery shows through faintly; the ink on it stays solid. */
  .float { position: absolute; z-index: 4; display: flex; flex-direction: column; visibility: hidden; min-width: 0; color: var(--ink); background: color-mix(in srgb, var(--surface) 82%, transparent); border: var(--lw-thin) solid var(--ink); border-radius: var(--radius-lg); overflow: hidden; }
  .float.ready { visibility: visible; }
  .float:focus-visible { outline: var(--lw-thick) solid var(--ink); outline-offset: 1px; }
  .bar { flex: none; display: flex; align-items: center; gap: var(--sp-2); padding: 3px var(--sp-1) 3px var(--sp-2); cursor: move; touch-action: none; user-select: none; background: color-mix(in srgb, var(--surface-2) 88%, transparent); border-bottom: var(--lw-thin) solid var(--rule-strong); }
  .title { flex: 1; font: 600 var(--fs-11) var(--font-ui); letter-spacing: var(--track-caps); text-transform: uppercase; color: var(--ink); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .close { padding: 1px 8px; font-size: var(--fs-11); cursor: pointer; }
  .content { flex: 1; min-height: 0; min-width: 0; display: flex; flex-direction: column; }
  .grip { position: absolute; right: 0; bottom: 0; width: 16px; height: 16px; cursor: nwse-resize; touch-action: none; background: linear-gradient(135deg, transparent 50%, var(--ink) 50%, var(--ink) 58%, transparent 58%, transparent 72%, var(--ink) 72%, var(--ink) 80%, transparent 80%); }
</style>
