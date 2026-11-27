<script lang="ts">
  import type { CrossSection } from "./api";
  import { faceView } from "./crossSectionView";

  // `busy`: a section is being fetched. `note`: why the last one was refused.
  let { section, colour, busy = false, note = null }: { section: CrossSection | null; colour: string; busy?: boolean; note?: string | null } = $props();

  const W = 360;
  const H = 220;
  const PAD = { l: 44, r: 10, t: 10, b: 28 };

  const finite = (v: number | null): v is number => v != null && Number.isFinite(v);

  // Display geometry only: the numbers all come from the session.
  let view = $derived.by(() => {
    if (!section) return null;
    const off = section.offsets_m;
    // The cut face is drawn only as far as the cut goes, and the plot is scaled to the ground and the
    // channel: the stored template runs on far above the ground (see `faceView`).
    const face = faceView(section);
    const { lo, hi } = face;
    const pad = Math.max((hi - lo) * 0.08, 0.5);
    const [xmin, xmax] = [off[0], off[off.length - 1]];
    const x = (o: number) => PAD.l + ((o - xmin) / (xmax - xmin)) * (W - PAD.l - PAD.r);
    const y = (e: number) => H - PAD.b - ((e - (lo - pad)) / (hi + pad - (lo - pad))) * (H - PAD.t - PAD.b);
    // Break a line wherever a value is missing.
    const path = (vals: (number | null)[]) => {
      let d = "";
      let pen = false;
      vals.forEach((v, i) => {
        if (!finite(v)) {
          pen = false;
          return;
        }
        d += `${pen ? "L" : "M"}${x(off[i]).toFixed(1)},${y(v).toFixed(1)}`;
        pen = true;
      });
      return d;
    };
    const ticks = (a: number, b: number, n: number) => {
      const step = (b - a) / n;
      return Array.from({ length: n + 1 }, (_, i) => a + i * step);
    };
    const facePath = face.points.map((p, i) => `${i ? "L" : "M"}${x(p.offset_m).toFixed(1)},${y(p.level_m).toFixed(1)}`).join("");
    const daylight = [face.daylight.left, face.daylight.right].filter((p) => p != null);
    return { x, y, lo, hi, terrain: path(section.terrain_m), face: facePath, daylight, xticks: ticks(xmin, xmax, 4), yticks: ticks(lo - pad, hi + pad, 4) };
  });

  const km = (m: number) => `${Math.floor(m / 1000)}+${String(Math.round(m % 1000)).padStart(3, "0")}`;
</script>

<div class="cross" data-testid="cross-section" data-chainage={section?.chainage_m ?? ""} aria-busy={busy}>
  {#if busy}
    <p class="status" role="status" data-testid="cross-busy">Working: drawing the section…</p>
  {/if}
  {#if note && !busy}
    <p class="refusal" role="alert" data-testid="cross-refusal">{note}</p>
  {:else if section && view}
    <svg viewBox="0 0 {W} {H}" role="img" aria-label="Cross-section at chainage {km(section.chainage_m)}" data-testid="cross-section-svg" data-lo={view.lo.toFixed(2)} data-hi={view.hi.toFixed(2)}>
      {#each view.yticks as t}
        <line x1={PAD.l} x2={W - PAD.r} y1={view.y(t)} y2={view.y(t)} class="grid" />
        <text x={PAD.l - 4} y={view.y(t) + 3} class="tick end">{t.toFixed(1)}</text>
      {/each}
      {#each view.xticks as t}
        <text x={view.x(t)} y={H - PAD.b + 12} class="tick mid">{Math.round(t)}</text>
      {/each}
      <line x1={PAD.l} x2={W - PAD.r} y1={view.y(section.water_level_m)} y2={view.y(section.water_level_m)} class="water" />
      <path d={view.terrain} class="terrain" data-testid="cs-terrain" />
      <path d={view.face} class="face" style:stroke={colour} data-testid="cs-face" />
      {#each view.daylight as d}
        <circle cx={view.x(d.offset_m)} cy={view.y(d.level_m)} r="3" class="daylight" style:stroke={colour} data-testid="cs-daylight" data-offset={d.offset_m.toFixed(2)} data-level={d.level_m.toFixed(2)}><title>Daylight point: the cut meets the ground {Math.abs(d.offset_m).toFixed(1)} m {d.offset_m < 0 ? "left" : "right"} of the line, at {d.level_m.toFixed(2)} m</title></circle>
      {/each}
      <line
        x1={view.x(-section.half_width_m)}
        x2={view.x(section.half_width_m)}
        y1={view.y(section.design_invert_m)}
        y2={view.y(section.design_invert_m)}
        class="invert"
        style:stroke={colour}
      />
      <text x={W / 2} y={H - 2} class="tick mid">Offset from the design line (m): left − · right +</text>
    </svg>
    <dl class="facts" data-testid="cross-facts">
      <dt>Chainage</dt><dd data-testid="cs-chainage">{km(section.chainage_m)}</dd>
      <dt>Water level</dt><dd>{section.water_level_m.toFixed(2)} m</dd>
      <dt>Invert</dt><dd>{section.design_invert_m.toFixed(2)} m</dd>
      <dt>Dredge</dt><dd>{section.dredge_area_m2.toFixed(0)} m²</dd>
      <dt>Dry cut</dt><dd>{section.excav_area_m2.toFixed(0)} m²</dd>
      <dt>Face</dt><dd>{section.max_face_h_m.toFixed(1)} m</dd>
    </dl>
  {:else if !busy}
    <p class="slot" data-testid="no-cross-section">Run volumes to see the cross-section.</p>
  {/if}
</div>

<style>
  .cross { position: relative; display: flex; gap: var(--sp-2); align-items: flex-start; flex-wrap: wrap; height: 100%; padding: var(--sp-1); overflow: auto; }
  svg { flex: 1 1 200px; min-width: 200px; max-height: 100%; font-family: var(--font-ui); }
  .grid { stroke: var(--rule); stroke-width: 1; }
  .tick { font-size: 9px; fill: var(--ink-3); }
  .tick.end { text-anchor: end; }
  .tick.mid { text-anchor: middle; }
  .terrain { fill: none; stroke: var(--ink); stroke-width: var(--lw-thick); }
  .face { fill: none; stroke-width: var(--lw-thick); stroke-dasharray: 6 3; }
  .invert { stroke-width: var(--lw-design); }
  .daylight { fill: var(--surface); stroke-width: var(--lw-thick); }
  .water { stroke: var(--ink); stroke-width: var(--lw-thin); stroke-dasharray: 8 4; }
  .facts { margin: 0; display: grid; grid-template-columns: auto auto; gap: 2px var(--sp-2); font-size: var(--fs-11); align-content: start; }
  dt { color: var(--ink-3); }
  dd { margin: 0; text-align: right; color: var(--ink); font-variant-numeric: tabular-nums; white-space: nowrap; }
  .slot { color: var(--ink-3); font-style: italic; margin: var(--sp-2); }
  .status { position: absolute; top: var(--sp-1); left: var(--sp-1); margin: 0; padding: 2px var(--sp-1); font-size: var(--fs-11); color: var(--ink-3); background: var(--surface); }
  .refusal { margin: var(--sp-2); color: var(--warn); max-width: 60ch; }
</style>
