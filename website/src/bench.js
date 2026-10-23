// The Benchmarks page's tables, from the summary (public/benchmarks.json, or the telemetry server's
// GET /v1/benchmarks/summary in the same shape). Shared by build.mjs (baked) and benchmarks-page.js (live).
import { escape } from "./markdown.js";

export const METRICS = [
  { key: "avg_fps", label: "Average FPS", kind: "fps", better: "high" },
  { key: "low_1pct_fps", label: "1% low FPS", kind: "fps", better: "high", hint: "the slowest 1% of frames, as FPS" },
  { key: "p95_ms", label: "Frame time, 95th percentile", kind: "ms", better: "low" },
  { key: "world_ms", label: "Start to in-world", kind: "ms", better: "low", hint: "process start to the spawn area loaded and drawn" },
  { key: "chunk_load_ms", label: "Chunk load after a teleport", kind: "ms", better: "low", hint: "the four scenes' average" },
  { key: "rss_steady_mb", label: "Memory while playing", kind: "mb", better: "low" },
  { key: "rss_peak_mb", label: "Memory, peak", kind: "mb", better: "low" },
];

export function format(kind, x) {
  if (x == null || !Number.isFinite(x)) return "–";
  if (kind === "fps") return x >= 100 ? Math.round(x).toLocaleString("en") : x.toFixed(x >= 10 ? 0 : 1);
  if (kind === "ms") return x >= 1000 ? `${(x / 1000).toFixed(x >= 10000 ? 0 : 1)} s` : `${x < 10 ? x.toFixed(1) : Math.round(x)} ms`;
  if (kind === "mb") return x >= 1000 ? `${(x / 1024).toFixed(1)} GB` : `${Math.round(x)} MB`;
  return String(x);
}

// "2.4x faster", "1.3x slower", "2.0x less" (memory), from Horizon's side.
export function compare(metric, horizon, java) {
  if (horizon == null || java == null || !horizon || !java) return null;
  const ratio = metric.better === "high" ? horizon / java : java / horizon;
  if (Math.abs(ratio - 1) < 0.05) return { text: "about the same", wins: "tie" };
  const word = metric.kind === "mb" ? (ratio > 1 ? "less" : "more") : ratio > 1 ? "faster" : "slower";
  const r = ratio > 1 ? ratio : 1 / ratio;
  return { text: `${r.toFixed(r >= 10 ? 0 : 1)}x ${word}`, wins: ratio > 1 ? "horizon" : "java" };
}

// The game's average frame rate sat at a display cap (`capped_fps`): the real one is at least that.
export const capped = (row) => Boolean(row?.capped_fps && row.medians?.avg_fps >= row.capped_fps * 0.97);

// compare() between two rows, saying "at least" where Horizon's average sat at a cap.
export function compareRows(metric, h, j) {
  const c = compare(metric, h?.medians?.[metric.key], j?.medians?.[metric.key]);
  if (!c || metric.key !== "avg_fps" || !capped(h)) return c;
  return c.wins === "horizon" ? { ...c, text: `at least ${c.text}` } : { text: "capped, can't tell", wins: "none" };
}

export const cell = (metric, row) => {
  const v = format(metric.kind, row?.medians?.[metric.key]);
  return metric.key === "avg_fps" && capped(row) ? `${v} <span class="capped" title="Held to the display's ${row.capped_fps} Hz: the real figure is at least this">(cap)</span>` : v;
};

const machines = (groups) => {
  const by = new Map();
  for (const g of groups.filter((g) => g.source === "us")) {
    if (!by.has(g.machine)) by.set(g.machine, []);
    by.get(g.machine).push(g);
  }
  return by;
};

const pick = (rows, game, rd) => rows.find((r) => r.game === game && r.render_distance === rd);

// One machine we measured: Horizon and Java side by side at each render distance.
export function machineHtml(name, rows) {
  const first = rows[0];
  const distances = [...new Set(rows.map((r) => r.render_distance))].sort((a, b) => a - b);
  const horizon = rows.find((r) => r.game === "horizon");
  const java = rows.find((r) => r.game === "vanilla");
  const head = distances.map((rd) => `<th scope="colgroup" colspan="3">Render distance ${rd}</th>`).join("");
  const sub = distances.map(() => `<th scope="col">Horizon</th><th scope="col">Java</th><th scope="col"><span class="sr-only">Horizon against Java</span></th>`).join("");
  const body = METRICS.map((m) => {
    const cells = distances
      .map((rd) => {
        const h = pick(rows, "horizon", rd);
        const j = pick(rows, "vanilla", rd);
        const c = compareRows(m, h, j);
        return `<td class="num${c?.wins === "horizon" ? " wins" : ""}">${cell(m, h)}</td><td class="num${c?.wins === "java" ? " wins" : ""}">${cell(m, j)}</td><td class="vs vs-${c?.wins || "none"}">${c ? escape(c.text) : ""}</td>`;
      })
      .join("");
    return `<tr><th scope="row">${escape(m.label)}${m.hint ? `<span class="metric-hint">${escape(m.hint)}</span>` : ""}</th>${cells}</tr>`;
  }).join("");
  const javaWins = METRICS.flatMap((m) =>
    distances.filter((rd) => compareRows(m, pick(rows, "horizon", rd), pick(rows, "vanilla", rd))?.wins === "java").map((rd) => `${m.label.toLowerCase()} at ${rd}`),
  );
  // Per scene too: an average that Horizon wins can hide a scene where Java is faster.
  const sceneName = { village: "the village", forest: "the forest", ocean: "the ocean", mountain: "the mountain" };
  for (const rd of distances) {
    const h = pick(rows, "horizon", rd);
    const j = pick(rows, "vanilla", rd);
    for (const [scene, js] of Object.entries(j?.scenes || {})) {
      const hs = h?.scenes?.[scene];
      if (!hs?.avg_fps || !js?.avg_fps || (h.capped_fps && hs.avg_fps >= h.capped_fps * 0.97)) continue;
      if (js.avg_fps > hs.avg_fps * 1.05) javaWins.push(`average FPS over ${sceneName[scene] || scene} at ${rd} (${format("fps", js.avg_fps)} against ${format("fps", hs.avg_fps)})`);
    }
  }
  const facts = [
    ["CPU", first.cpu],
    ["GPU", first.gpu],
    ["Horizon", horizon ? `${horizon.version} · ${horizon.backend || "?"}` : "not measured"],
    ["Java", java ? `${java.version} (vanilla) · ${java.backend || "?"}` : "not measured"],
    ["Measured", `${first.measured}${first.repeats ? ` · ${first.repeats} runs of each` : ""}${first.window ? ` · ${first.window.join("x")} window` : ""}`],
  ];
  return `<article class="machine" id="machine-${escape(name.toLowerCase().replace(/[^a-z0-9]+/g, "-"))}">
          <header class="machine-head">
            <p class="eyebrow"><span class="badge badge-us">Measured by us</span></p>
            <h3 class="machine-name">${escape(name)}</h3>
            <dl class="machine-facts">${facts.map(([k, v]) => `<div><dt>${k}</dt><dd>${escape(String(v))}</dd></div>`).join("")}</dl>
            ${rows.find((r) => r.note) ? `<p class="machine-note">${escape(rows.find((r) => r.note).note)}</p>` : ""}
          </header>
          <div class="table-wrap"><table class="bench-table">
            <thead><tr><td></td>${head}</tr><tr><td></td>${sub}</tr></thead>
            <tbody>${body}</tbody>
          </table></div>
          ${javaWins.length ? `<p class="java-wins"><strong>Where Java is ahead:</strong> ${escape(javaWins.join("; "))}.</p>` : `<p class="java-wins">Horizon is ahead on every figure here.</p>`}
        </article>`;
}

export function machinesHtml(summary) {
  const by = machines(summary?.groups || []);
  if (!by.size) return `<p class="lede">No measurements are published yet.</p>`;
  return [...by].map(([name, rows]) => machineHtml(name, rows)).join("\n");
}

// The hardware breakdowns (the server's classes), in words.
export const DIMENSIONS = [
  { key: "ram", label: "Memory", word: (c) => `${c} RAM` },
  { key: "vram", label: "Graphics memory", word: (c) => (c === "shared" ? "shared graphics memory" : `${c} graphics memory`) },
  { key: "storage", label: "Drive", word: (c) => ({ nvme: "NVMe", ssd: "SSD", hdd: "hard disk" })[c] || c },
  { key: "os", label: "System", word: (c) => ({ linux: "Linux", macos: "macOS", windows: "Windows" })[c] || c },
  { key: "gpu_kind", label: "Graphics", word: (c) => (c === "integrated" ? "integrated GPU" : c === "discrete" ? "separate GPU" : c) },
];

// A group's breakdowns in one line: "32 GB RAM (3) · NVMe (3) · Linux (3)". Empty without any.
export function breakdownLine(breakdowns) {
  return DIMENSIONS.flatMap((d) => (breakdowns?.[d.key] || []).map((c) => `${d.word(c.class)} (${c.installs})`)).join(" · ");
}

// GET /v1/aggregate's breakdowns: players' play sessions per hardware class. Empty without any.
export function hardwareHtml(aggregate) {
  const b = aggregate?.breakdowns || {};
  const dims = DIMENSIONS.filter((d) => b[d.key]?.length);
  if (!dims.length) return "";
  const rows = dims
    .flatMap((d) =>
      b[d.key].map(
        (c, i) =>
          `<tr>${i === 0 ? `<th scope="rowgroup" rowspan="${b[d.key].length}">${escape(d.label)}</th>` : ""}<th scope="row">${escape(d.word(c.class))}</th><td class="num">${c.installs}</td><td class="num">${Math.round(c.hours).toLocaleString("en")}</td><td class="num">${format("ms", c.p50_ms)}</td><td class="num">${format("ms", c.p99_ms)}</td></tr>`,
      ),
    )
    .join("");
  return `<div class="table-wrap"><table class="bench-table players-hardware">
            <thead><tr><th scope="col">By</th><th scope="col">Class</th><th scope="col">Installs</th><th scope="col">Hours played</th><th scope="col">Frame time, median</th><th scope="col">Frame time, 99th percentile</th></tr></thead>
            <tbody>${rows}</tbody>
          </table></div>`;
}

// Players' groups: one row each, at least `min_installs` installs.
export function playersHtml(summary) {
  const groups = (summary?.groups || []).filter((g) => g.source === "players");
  if (!groups.length)
    return `<p class="empty">No player results yet. A GPU and CPU pair shows here once at least ${summary?.min_installs || 3} different installs have shared a benchmark with it.</p>`;
  const rows = groups
    .map(
      (g) => `<tr><th scope="row">${escape(g.gpu)}<span class="metric-hint">${escape(g.cpu)}</span></th><td class="num">${g.render_distance}</td><td>${g.game === "vanilla" ? "Java" : "Horizon"}</td>${["avg_fps", "low_1pct_fps", "p95_ms", "world_ms", "rss_steady_mb"].map((k) => `<td class="num">${format(METRICS.find((m) => m.key === k).kind, g.medians?.[k])}</td>`).join("")}<td><span class="badge badge-players">From ${g.installs} players</span>${breakdownLine(g.breakdowns) ? `<span class="metric-hint">${escape(breakdownLine(g.breakdowns))}</span>` : ""}</td></tr>`,
    )
    .join("");
  return `<div class="table-wrap"><table class="bench-table players-table">
            <thead><tr><th scope="col">GPU and CPU</th><th scope="col">Render distance</th><th scope="col">Game</th><th scope="col">Average FPS</th><th scope="col">1% low</th><th scope="col">p95</th><th scope="col">To in-world</th><th scope="col">Memory</th><th scope="col">Source</th></tr></thead>
            <tbody>${rows}</tbody>
          </table></div>`;
}
