// Benchmark results for the site: benchmark reports (docs/benchmark-report.md) in, the summary the Benchmarks page
// reads out (public/benchmarks.json, and the telemetry server's GET /v1/benchmarks/summary in the same shape).
//
//   node scripts/benchmarks.mjs --ours fleet.json [--telemetry items.jsonl] [-o public/benchmarks.json]
//
// --ours lists our own runs (tools/perf_bench.py reports, Horizon and vanilla, one machine each); --telemetry holds
// players' `benchmark` items as the server stores them ({ install_id, day, version, data: <report> } per line).
// Players' results are grouped by GPU, CPU, render distance and game, and a group is published only with at least
// MIN_INSTALLS installs, so nobody can be picked out. install_id is used to count installs and to weigh each install
// once, and never leaves this script; neither do the day, OS version, driver, the exact RAM and VRAM, or the screen.
// A group also says how its installs split by hardware class (RAM, VRAM, drive, OS, integrated or discrete GPU),
// each class only with at least MIN_INSTALLS installs. The classes are the server's (src/classes.rs).
import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const FORMAT = "horizon-benchmark-summary";
export const MIN_INSTALLS = 3;
export const SCENES = ["village", "forest", "ocean", "mountain"];
export const FIELDS = ["avg_fps", "low_1pct_fps", "p95_ms", "world_ms", "chunk_load_ms", "rss_peak_mb", "rss_steady_mb"];

const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);
const finite = (x) => typeof x === "number" && Number.isFinite(x);
export function median(xs) {
  const v = xs.filter(finite).sort((a, b) => a - b);
  if (!v.length) return null;
  const m = v.length >> 1;
  return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2;
}
// fps to integers, milliseconds to a tenth under 10 and to integers above, megabytes to integers.
export function round(field, x) {
  if (!finite(x)) return null;
  if (field.endsWith("_ms")) return x < 10 ? Math.round(x * 10) / 10 : Math.round(x);
  return Math.round(x);
}
const trim = (s) => (typeof s === "string" ? s.replace(/\(R\)|\(TM\)|\(tm\)/g, "").replace(/\s+/g, " ").trim() : "");
const gameOf = (app) => (app?.name === "horizon-client" ? "horizon" : app?.name === "minecraft" && !(app.mods || []).some((m) => m.name !== "fabric-loader") ? "vanilla" : null);

// --- Hardware classes: coarse, so that a class holds many machines ---------------------------

export const DIMENSIONS = ["ram", "vram", "storage", "os", "gpu_kind"];
export function ramClass(gb) {
  if (!(gb > 0)) return null;
  return gb <= 5 ? "4 GB or less" : gb <= 10 ? "8 GB" : gb <= 20 ? "16 GB" : gb <= 40 ? "32 GB" : "64 GB or more";
}
export function gpuKindClass(s) {
  return s === "integrated" || s === "IntegratedGpu" ? "integrated" : s === "discrete" || s === "DiscreteGpu" ? "discrete" : null;
}
// A GPU's own memory in MiB; an integrated GPU shares the system's.
export function vramClass(mb, kind) {
  if (gpuKindClass(kind) === "integrated") return "shared";
  if (!(mb > 0)) return null;
  for (const [max, name] of [[2560, "2 GB or less"], [4608, "4 GB"], [6656, "6 GB"], [8704, "8 GB"], [12800, "12 GB"], [16896, "16 GB"]]) if (mb <= max) return name;
  return "more than 16 GB";
}
const oneOf = (list) => (s) => (list.includes(s) ? s : null);
export const storageClass = oneOf(["nvme", "ssd", "hdd"]);
export const osClass = oneOf(["linux", "macos", "windows"]);
// A report's classes, in DIMENSIONS order.
export function classesOf(report) {
  const hw = report?.hardware || {};
  const kind = hw.gpu?.kind;
  return [ramClass(finite(hw.ram_mb) ? hw.ram_mb / 1024 : 0), vramClass(hw.gpu?.vram_mb, kind), storageClass(hw.storage), osClass(hw.os), gpuKindClass(kind)];
}
// Per dimension, the classes with at least minInstalls installs, most first; dimensions without one are left out.
export function breakdowns(classes, minInstalls) {
  const out = {};
  DIMENSIONS.forEach((dim, i) => {
    const cells = [...classes[i]].map(([c, set]) => ({ class: c, installs: set.size })).filter((c) => c.installs >= minInstalls);
    cells.sort((a, b) => b.installs - a.installs || (a.class < b.class ? -1 : a.class > b.class ? 1 : 0));
    if (cells.length) out[dim] = cells;
  });
  return out;
}

// Why a report can't be compared with the others, or null when it can: the standard flight over the four scenes,
// VSync off, no frame cap, drawn on a screen.
export function rejection(report) {
  if (!report || report.format !== "horizon-benchmark-report" || report.version !== 1) return "not a version 1 benchmark report";
  if (report.kind !== "game") return "not a game run";
  if (!gameOf(report.app)) return "not Horizon or plain vanilla";
  const c = report.conditions || {};
  if (!(c.scenario === "standard" || c.scenario === "flight" || String(c.scenario || "").startsWith("perf_bench:"))) return "not the standard flight";
  if (c.vsync !== false) return "VSync on or unknown";
  if (c.fps_cap != null) return "frame rate capped";
  if (c.offscreen === true) return "offscreen";
  if (!finite(c.render_distance)) return "no render distance";
  const phases = new Set((report.frames || []).map((f) => f.phase));
  if (!SCENES.every((s) => phases.has(s))) return "not every scene";
  if (!trim(report.hardware?.gpu?.name) || !trim(report.hardware?.cpu)) return "no GPU or CPU name";
  return null;
}

// One report's figures: frame figures averaged over every flight (scene and repeat), the rest from its metrics.
export function figures(report) {
  const frames = report.frames || [];
  const m = report.metrics || {};
  const metric = (name) => (finite(m[name]?.mean) ? m[name].mean : null);
  const loads = Object.keys(m).filter((k) => /^chunks\.[^.]+\.load_ms$/.test(k)).map(metric).filter(finite);
  const avg = (key) => mean(frames.map((f) => f[key]).filter(finite));
  return {
    avg_fps: avg("avg_fps"),
    low_1pct_fps: avg("low_1pct_fps"),
    p95_ms: avg("p95_ms"),
    world_ms: metric("startup.world_ms"),
    chunk_load_ms: mean(loads),
    rss_peak_mb: metric("memory.rss_peak_mb"),
    rss_steady_mb: metric("memory.rss_steady_mb"),
    cpu_p50_ms: avg("cpu_p50_ms"),
    gpu_p50_ms: avg("gpu_p50_ms"),
  };
}

const roundAll = (fig, fields) => Object.fromEntries(fields.map((f) => [f, round(f, fig[f])]));

// Players' items -> groups of at least minInstalls installs. Each install counts once: its median first, then the
// median across installs.
export function aggregatePlayers(items, { minInstalls = MIN_INSTALLS } = {}) {
  const groups = new Map();
  for (const item of items) {
    const report = item?.data;
    if (!item?.install_id || rejection(report)) continue;
    const key = JSON.stringify([trim(report.hardware.gpu.name), trim(report.hardware.cpu), report.conditions.render_distance, gameOf(report.app)]);
    if (!groups.has(key)) groups.set(key, { installs: new Map(), reports: 0, versions: new Set(), classes: DIMENSIONS.map(() => new Map()) });
    const g = groups.get(key);
    g.reports++;
    if (item.version) g.versions.add(item.version);
    if (!g.installs.has(item.install_id)) g.installs.set(item.install_id, []);
    g.installs.get(item.install_id).push(figures(report));
    classesOf(report).forEach((c, i) => {
      if (!c) return;
      if (!g.classes[i].has(c)) g.classes[i].set(c, new Set());
      g.classes[i].get(c).add(item.install_id);
    });
  }
  const out = [];
  for (const [key, g] of groups) {
    if (g.installs.size < minInstalls) continue;
    const [gpu, cpu, render_distance, game] = JSON.parse(key);
    const perInstall = [...g.installs.values()].map((runs) => Object.fromEntries(FIELDS.map((f) => [f, median(runs.map((r) => r[f]))])));
    const medians = Object.fromEntries(FIELDS.map((f) => [f, round(f, median(perInstall.map((r) => r[f])))]));
    out.push({ gpu, cpu, render_distance, game, source: "players", installs: g.installs.size, reports: g.reports, versions: [...g.versions].sort(byVersionDesc).slice(0, 3), medians, breakdowns: breakdowns(g.classes, minInstalls) });
  }
  // Most installs first, then by GPU, CPU, render distance and game, compared as plain strings (as the server does).
  const cmp = (x, y) => (x < y ? -1 : x > y ? 1 : 0);
  return out.sort((a, b) => b.installs - a.installs || cmp(a.gpu, b.gpu) || cmp(a.cpu, b.cpu) || a.render_distance - b.render_distance || cmp(a.game, b.game));
}

// "v0.2.10" after "v0.2.9", dated tags by their numbers; good enough to list the newest three.
function byVersionDesc(a, b) {
  const n = (v) => (v.match(/\d+/g) || []).map(Number);
  const [x, y] = [n(a), n(b)];
  for (let i = 0; i < Math.max(x.length, y.length); i++) if ((x[i] ?? -1) !== (y[i] ?? -1)) return (y[i] ?? -1) - (x[i] ?? -1);
  return 0;
}

// Our own runs: one row per machine, game and render distance, with what was measured and how. An entry holds one
// report (its runs are the report's repeats) or several (one game each); figures are medians over the reports.
export function ourRows(runs) {
  const rows = [];
  for (const { report, reports, machine, note, version, capped_fps } of runs) {
    const list = reports || [report];
    for (const r of list) {
      const why = rejection(r);
      if (why) throw new Error(`${machine}: ${r?.label || "report"} can't be published: ${why}`);
    }
    const [first] = list;
    const c = first.conditions;
    if (list.some((r) => r.conditions.render_distance !== c.render_distance || gameOf(r.app) !== gameOf(first.app) || r.app.version !== first.app.version))
      throw new Error(`${machine}: reports of different games, versions or render distances in one entry`);
    const figs = list.map(figures);
    const fig = Object.fromEntries(Object.keys(figs[0]).map((k) => [k, median(figs.map((f) => f[k]))]));
    const scenes = Object.fromEntries(
      SCENES.map((s) => {
        const fs = list.flatMap((r) => r.frames.filter((f) => f.phase === s));
        const loads = list.map((r) => r.metrics?.[`chunks.${s}.load_ms`]?.mean);
        return [s, { avg_fps: round("avg_fps", mean(fs.map((f) => f.avg_fps))), low_1pct_fps: round("low_1pct_fps", mean(fs.map((f) => f.low_1pct_fps))), chunk_load_ms: round("chunk_load_ms", median(loads)) }];
      }),
    );
    rows.push({
      gpu: trim(first.hardware.gpu.name),
      cpu: trim(first.hardware.cpu),
      render_distance: c.render_distance,
      game: gameOf(first.app),
      source: "us",
      machine,
      measured: list.map((r) => (r.created || "").slice(0, 10)).sort().at(-1),
      // A release build reports its crate version; the entry can name the release it was.
      version: version || first.app.version,
      backend: first.hardware.gpu.backend || null,
      os: first.hardware.os || null,
      window: c.window || null,
      repeats: list.reduce((n, r) => n + (r.conditions.repeats || 1), 0),
      load_avg_1m: Math.max(...list.map((r) => r.conditions.load_avg_1m ?? 0)),
      ...(note ? { note } : {}),
      // The frame rate the display held this game to (a present mode VSync off doesn't lift): its average is a floor.
      ...(capped_fps ? { capped_fps } : {}),
      // A split of 0 means the backend gave no timing (Metal has no GPU timer here): unknown, not free.
      medians: { ...roundAll(fig, FIELDS), cpu_p50_ms: fig.cpu_p50_ms > 0 ? round("cpu_p50_ms", fig.cpu_p50_ms) : null, gpu_p50_ms: fig.gpu_p50_ms > 0 ? round("gpu_p50_ms", fig.gpu_p50_ms) : null },
      scenes,
    });
  }
  return rows;
}

export function summary({ ours = [], players = [], generated = new Date() } = {}) {
  return { format: FORMAT, version: 1, generated: generated.toISOString().replace(/\.\d+Z$/, "Z"), min_installs: MIN_INSTALLS, groups: [...ours, ...players] };
}

// --- The command line ----------------------------------------------------------------------

async function main(argv) {
  const arg = (name) => {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : null;
  };
  const site = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const out = arg("-o") || resolve(site, "public/benchmarks.json");
  let ours = [];
  if (arg("--ours")) {
    // [{ "machine": "Mac mini", "reports": ["path/to/report.json", ...], "version": "v…", "note": "..." }], paths
    // relative to the list; "version" names the release when the report only has the crate version.
    const listPath = resolve(arg("--ours"));
    const list = JSON.parse(await readFile(listPath, "utf8"));
    const load = async (path) => JSON.parse(await readFile(resolve(dirname(listPath), path), "utf8"));
    const runs = [];
    for (const entry of list) runs.push({ ...entry, reports: await Promise.all((entry.reports || [entry.report]).map(load)), report: undefined });
    ours = ourRows(runs);
  }
  let players = [];
  if (arg("--telemetry")) {
    const text = await readFile(resolve(arg("--telemetry")), "utf8");
    players = aggregatePlayers(text.split("\n").filter((l) => l.trim()).map((l) => JSON.parse(l)));
  }
  const result = summary({ ours, players });
  await writeFile(out, JSON.stringify(result, null, 1) + "\n");
  console.log(`${out}: ${ours.length} rows measured by us, ${players.length} player groups`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main(process.argv.slice(2));
