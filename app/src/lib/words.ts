// Plain words for the screen: pure functions, so a test can reach them without a browser.
// The engine's messages stay as they are; the app says them in the words a reader needs.

/** The last part of a path, either slash: the file name. */
export function fileNameOf(path: string): string {
  const parts = path.split(/[\\/]/).filter((p) => p !== "");
  return parts.length ? parts[parts.length - 1] : path;
}

const STAGE_ORDER = ["sizing", "alignment", "optimise", "volumes"];

/** Why the long section cannot be drawn, said once. The engine names the stage that sits just
 * upstream (`volumes is stale`), which is true but misleading beside the run banner; the cause is the
 * first stage that is out of date. `stages` is the snapshot's list, in run order. */
export function sectionReason(error: string, stages: { name: string; status: string }[] | undefined): string {
  if (!/stage cannot run: \w+ is stale/.test(error)) return error;
  const first = (stages ?? []).filter((s) => s.status === "stale").sort((a, b) => STAGE_ORDER.indexOf(a.name) - STAGE_ORDER.indexOf(b.name))[0];
  if (!first) return error;
  if (first.name === "volumes") return "No sections yet: the volumes are out of date. Run volumes.";
  return `No sections yet: the ${first.name} is out of date. Run ${first.name}, then volumes.`;
}

/** A warning from the engine about the DEM file, with the file name instead of the stored path and
 * what to do. Any other warning comes back as it was. */
export function plainWarning(w: string): string {
  const stop = /^the DEM `(.+)` is missing: the (\w+) stage cannot run without it$/.exec(w);
  if (stop) return `The DEM file ${fileNameOf(stop[1])} was not found, so the ${stop[2]} stage cannot run. Load the DEM with its Load button.`;
  const m = /^the DEM `(.+)` (is missing|has changed since its hash was recorded): the volumes may no longer hold$/.exec(w);
  if (!m) return w;
  const name = fileNameOf(m[1]);
  return m[2] === "is missing"
    ? `The DEM file ${name} was not found. Load the DEM with its Load button; until then the volumes may not hold.`
    : `The DEM file ${name} has changed since it was loaded. Load the DEM again with its Load button; until then the volumes may not hold.`;
}

/** An engine note as a sentence on the screen: a capital first letter, a full stop, and names
 * without the code marks around them. */
export function plainNote(text: string): string {
  const t = text.trim().replace(/`/g, "");
  if (t === "") return t;
  const s = t[0].toUpperCase() + t.slice(1);
  return /[.!?]$/.test(s) ? s : `${s}.`;
}

/** The value of a bank row: its size, or why it is empty. A loaded river polygon stands in for banks. */
export function bankValue(line: { coordinates: unknown[] } | null | undefined, polygonLoaded: boolean): string {
  if (line) return `${line.coordinates.length} positions`;
  return polygonLoaded ? "not needed (river polygon loaded)" : "not loaded";
}

/** A fall per metre of river, rounded to three significant figures for reading. */
export function roundedFall(fall: string): string {
  const n = Number(fall);
  if (fall.trim() === "" || !Number.isFinite(n)) return fall;
  return String(Number(n.toPrecision(3)));
}

/** What the sign of the fall means, in the document's own rule: level at chainage c = start level - fall x c. */
export const FALL_HELP =
  "Positive: the water level drops as chainage grows. Negative: it rises as chainage grows (a river that falls towards its mouth, with the mouth set at chainage 0, shows a negative number).";

/** The design line the screen draws, named with the stage that made it: the optimise stage's refined
 * line while that stage is fresh, else the alignment's own (the engine's one rule, as the table and
 * the map use it). `tableName` is the name the table gives the line. Null: no line yet. */
export function lineOnScreen(
  doc: { stages: { alignment: { results: unknown }; optimise: { results: unknown } } } | null | undefined,
  stages: { name: string; status: string }[] | undefined,
  tableName: string | null | undefined,
): string | null {
  if (!doc) return null;
  const isObj = (x: unknown): x is { id?: string } => typeof x === "object" && x !== null;
  const refined = (doc.stages.optimise.results as { design_line?: unknown } | null)?.design_line;
  const optimiseFresh = (stages ?? []).some((s) => s.name === "optimise" && s.status === "fresh");
  let line: { id?: string } | null = null;
  let stage = "";
  if (isObj(refined) && optimiseFresh) {
    line = refined;
    stage = "Optimise";
  } else if (isObj(doc.stages.alignment.results)) {
    line = doc.stages.alignment.results;
    stage = "Alignment";
  }
  if (!line) return null;
  return `${tableName ?? line.id ?? "design line"} (from ${stage})`;
}

/** True when the engine's warnings say the DEM file the design names was not found. */
export function demNotLoaded(warnings: string[] | undefined): boolean {
  return (warnings ?? []).some((w) => /^the DEM `.+` is missing:/.test(w));
}
