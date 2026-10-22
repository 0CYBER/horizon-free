// The Benchmarks page: players' results from the telemetry server (the baked summary when it doesn't answer)
// and the hardware picker (a measurement when there is one, else an estimate from src/estimate.js).
import { escape } from "./markdown.js";
import { METRICS, cell, compareRows, format, playersHtml } from "./bench.js";
import { estimate, measured } from "./estimate.js";
import { findCpu, findGpu } from "./hardware.js";
import { setUpMenu, setUpTheme } from "./chrome.js";

const TELEMETRY = "https://telemetry.playhorizon.dev";
const baked = JSON.parse(document.getElementById("bench-data").textContent);
let groups = baked.groups || [];

async function live() {
  try {
    const res = await fetch(`${TELEMETRY}/v1/benchmarks/summary`, { signal: AbortSignal.timeout(3000) });
    if (!res.ok) throw new Error(String(res.status));
    const summary = await res.json();
    if (summary.format !== "horizon-benchmark-summary" || summary.version !== 1 || !Array.isArray(summary.groups)) throw new Error("format");
    // Our own rows stay the baked ones; players' come from the server.
    const players = summary.groups.filter((g) => g.source === "players" && g.installs >= (summary.min_installs || 3));
    groups = [...groups.filter((g) => g.source === "us"), ...players];
    document.getElementById("players-table").innerHTML = playersHtml({ ...summary, groups: players });
    document.querySelector("[data-players-source]").textContent = `Live from the telemetry server, aggregated ${summary.generated.slice(0, 16).replace("T", " ")} UTC.`;
    render();
  } catch {
    // The baked results stay; the line under the heading already says when they were made.
  }
}

const picker = document.getElementById("picker");
const out = document.getElementById("pick-result");

const range = (kind, e) => {
  if (!e) return "–";
  if (e.atLeast) return `at least ${format(kind, e.low)}`;
  return e.low != null ? `${format(kind, e.low)} to ${format(kind, e.high)}` : format(kind, e.value);
};

function sourceBadge(row) {
  return row.source === "us" ? `<span class="badge badge-us">Measured by us · ${escape(row.machine)}</span>` : `<span class="badge badge-players">From ${row.installs} players</span>`;
}

function measuredTable(h, j) {
  return `<table class="bench-table pick-table"><thead><tr><td></td><th scope="col">Horizon</th><th scope="col">Java</th><th scope="col"><span class="sr-only">Horizon against Java</span></th></tr></thead><tbody>${METRICS.map((m) => {
    const c = compareRows(m, h, j);
    return `<tr><th scope="row">${escape(m.label)}</th><td class="num${c?.wins === "horizon" ? " wins" : ""}">${cell(m, h)}</td><td class="num${c?.wins === "java" ? " wins" : ""}">${cell(m, j)}</td><td class="vs vs-${c?.wins || "none"}">${c ? escape(c.text) : ""}</td></tr>`;
  }).join("")}</tbody></table>`;
}

function estimateTable(h, j) {
  return `<table class="bench-table pick-table"><thead><tr><td></td><th scope="col">Horizon (estimate)</th><th scope="col">Java (estimate)</th></tr></thead><tbody>${METRICS.map(
    (m) => `<tr><th scope="row">${escape(m.label)}</th><td class="num">${range(m.kind, h?.[m.key])}</td><td class="num">${range(m.kind, j?.[m.key])}</td></tr>`,
  ).join("")}</tbody></table>`;
}

function render() {
  const gpu = findGpu(picker.gpu.value);
  const cpu = findCpu(picker.cpu.value);
  const rd = Number(picker.rd.value);
  if (!gpu || !cpu) return;
  const mh = measured(groups, gpu, cpu, rd, "horizon");
  const mj = measured(groups, gpu, cpu, rd, "vanilla");
  if (mh.length || mj.length) {
    const h = mh[0];
    const j = mj[0];
    out.innerHTML = `<p class="pick-head">${[h, j].filter(Boolean).map(sourceBadge).join(" ")}</p>${measuredTable(h, j)}${
      h && h.source === "us" && h.measured ? `<p class="meta">Measured ${escape(h.measured)} with Horizon ${escape(h.version)}. The full table is under <a href="#measured">Measured by us</a>.</p>` : ""
    }`;
    return;
  }
  const h = estimate(groups, gpu, cpu, rd, "horizon");
  const j = estimate(groups, gpu, cpu, rd, "vanilla");
  if (!h && !j) {
    out.innerHTML = `<p class="empty">Nothing measured at render distance ${rd} to estimate from yet.</p>`;
    return;
  }
  const base = (h || j).base;
  const f = (h || j).factors;
  const times = (x) => `${x >= 1 ? x.toFixed(x >= 10 ? 0 : 1) : x.toFixed(2)}x`;
  out.innerHTML = `<p class="pick-head"><span class="badge badge-estimate">Estimate</span> From ${escape(base.source === "us" ? base.machine : `${base.gpu} with ${base.cpu}`)} (${escape(base.gpu)}, ${escape(base.cpu)}): GPU factor ${times(f.gpu)}, CPU factor ${times(f.cpu)}. <a href="#method">How</a></p>${estimateTable(h, j)}<p class="meta">Ranges, not measurements: the real figure depends on drivers, the graphics API and cooling too. Memory is the measured machine's.</p>`;
}

setUpTheme();
setUpMenu();
picker.addEventListener("change", render);
render();
live();
