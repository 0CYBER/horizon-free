// node --test website/src/ (npm test in website/)
import { test } from "node:test";
import assert from "node:assert/strict";
import { estimate, factors, measured, nearest } from "./estimate.js";
import { CPUS, GPUS, findCpu, findGpu } from "./hardware.js";

const row = (gpu, cpu, medians, extra = {}) => ({ gpu, cpu, render_distance: 12, game: "horizon", source: "us", medians, ...extra });
const mac = row("Apple M4 Pro", "Apple M4 Pro", { avg_fps: 500, low_1pct_fps: 100, p95_ms: 3, world_ms: 4000, chunk_load_ms: 1000, rss_peak_mb: 1500, rss_steady_mb: 1200 });
const laptop = row("Mesa Intel HD Graphics 620 (KBL GT2)", "Intel Core i7-7500U CPU @ 2.70GHz", { avg_fps: 100, low_1pct_fps: 20, p95_ms: 15, world_ms: 12000, chunk_load_ms: 5000, rss_peak_mb: 1400, rss_steady_mb: 1100, cpu_p50_ms: 4, gpu_p50_ms: 9 });

test("every reported name of our machines is in the table", () => {
  assert.equal(findGpu("Apple M4 Pro").name, "Apple M4 Pro (20-core GPU)");
  assert.equal(findGpu("Mesa Intel HD Graphics 620 (KBL GT2)").name, "Intel HD Graphics 620");
  assert.equal(findGpu("NVIDIA GeForce RTX 4080").name, "NVIDIA GeForce RTX 4080");
  assert.equal(findGpu("NVIDIA GeForce RTX 4080 SUPER"), null);
  assert.equal(findCpu("13th Gen Intel Core i7-13700KF").name, "Intel Core i7-13700K / KF");
  assert.equal(findCpu("Intel Core i7-7500U CPU @ 2.70GHz").name, "Intel Core i7-7500U");
  for (const g of GPUS) assert.equal(findGpu(g.name), g, g.name);
  for (const c of CPUS) assert.equal(findCpu(c.name), c, c.name);
});

test("an exact match is a measurement, not an estimate", () => {
  assert.deepEqual(measured([mac, laptop], findGpu("Apple M4 Pro"), findCpu("Apple M4 Pro"), 12, "horizon"), [mac]);
  assert.deepEqual(measured([mac], findGpu("Apple M4 Pro"), findCpu("Apple M4 Pro"), 32, "horizon"), []);
});

test("the nearest machine is the base, and the range holds the estimate", () => {
  const gpu = findGpu("Intel Iris Xe Graphics (96 EU)");
  const cpu = findCpu("Intel Core i5-1135G7");
  assert.equal(nearest([mac, laptop], gpu, cpu, 12, "horizon").row, laptop);
  const e = estimate([mac, laptop], gpu, cpu, 12, "horizon");
  const f = factors(findGpu(laptop.gpu), findCpu(laptop.cpu), gpu, cpu);
  assert.ok(f.gpu > 1 && f.cpu > 1);
  for (const key of ["avg_fps", "low_1pct_fps", "p95_ms", "world_ms", "chunk_load_ms"]) {
    const { value, low, high } = e[key];
    assert.ok(low <= value && value <= high, key);
  }
  // GPU-bound frames (9 ms GPU, 4 ms CPU) follow the GPU factor.
  assert.ok(Math.abs(e.avg_fps.value - 100 * Math.min(f.gpu, 9 / (4 / f.cpu))) < 1e-9);
  assert.ok(Math.abs(e.world_ms.value - 12000 / f.cpu) < 1e-9);
  assert.equal(e.rss_steady_mb.value, 1100);
});

test("no base, no estimate", () => {
  assert.equal(estimate([mac], findGpu("Apple M4"), findCpu("Apple M4"), 32, "horizon"), null);
  assert.equal(estimate([mac], findGpu("Apple M4"), findCpu("Apple M4"), 12, "vanilla"), null);
});

test("a base held to its display's refresh rate gives only a floor for the average", () => {
  const held = { ...mac, capped_fps: 144, medians: { ...mac.medians, avg_fps: 144 } };
  const e = estimate([held], findGpu("NVIDIA GeForce RTX 4060"), findCpu("AMD Ryzen 5 5600X"), 12, "horizon");
  assert.equal(e.avg_fps.atLeast, true);
  assert.equal(e.avg_fps.high, null);
  assert.ok(e.avg_fps.low > 0);
  // The lows sit under the cap: still a range.
  assert.ok(e.low_1pct_fps.high > e.low_1pct_fps.low);
});

test("a release title of issue numbers falls back to its notes", async () => {
  const { parts } = await import("./releases.js");
  const r = { tag_name: "v2026.10.22.07", name: "Horizon 2026.10.22.07: #1255, mob pushes, fix, #1222", body: "### Fixed\n- Fix #1222: single player runs in the frame again (#1257, fixes #1222)\n- Mob pushes: one list per mob (#1205)\n" };
  assert.equal(parts(r).line, "Single player runs in the frame again and 1 more");
  assert.equal(parts({ ...r, name: "Horizon 2026.10.22.05: Hoppers, mob tick, server tick" }).line, "Hoppers, mob tick, server tick");
});

test("players' memory classes steer the base and the notes", async () => {
  const { memoryNotes } = await import("./estimate.js");
  const { ramClass, vramClass } = await import("./hardware.js");
  assert.equal(vramClass(findGpu("NVIDIA GeForce RTX 4080")), "16 GB");
  assert.equal(vramClass(findGpu("NVIDIA GeForce RTX 3080")), "12 GB");
  assert.equal(vramClass(findGpu("Apple M4 Pro")), "shared");
  assert.equal(ramClass(16), "16 GB");
  // Two equally near groups: the one whose players have the picked GPU's memory class wins.
  const players = (vram, ram, fps) =>
    row("NVIDIA GeForce RTX 4070", "AMD Ryzen 5 7600X", { avg_fps: fps, rss_peak_mb: 3600 }, { source: "players", installs: 3, breakdowns: { vram: [{ class: vram, installs: 3 }], ram: [{ class: ram, installs: 3 }] } });
  const a = players("8 GB", "16 GB", 400);
  const b = { ...players("12 GB", "32 GB", 500), cpu: "AMD Ryzen 7 7800X3D" };
  const gpu = findGpu("NVIDIA GeForce RTX 4070 Ti");
  const cpu = findCpu("AMD Ryzen 5 7600X");
  assert.equal(nearest([a, b], gpu, cpu, 12, "horizon").row, b, "12 GB like the 4070 Ti, though the CPU differs");
  assert.equal(nearest([a, b], gpu, cpu, 12, "horizon", { ramGb: 16 }).row, a, "both classes count");
  assert.deepEqual(memoryNotes(b, gpu, 32), []);
  const notes = memoryNotes(a, gpu, 8);
  assert.equal(notes.length, 3);
  assert.match(notes[0], /8 GB \(3\) of their own memory; this one has 12 GB/);
  assert.match(notes[1], /16 GB \(3\) of memory, not 8 GB/);
  assert.match(notes[2], /Horizon used up to 3\.5 GB/);
  // Our own rows have no breakdowns: only the peak memory note.
  assert.deepEqual(memoryNotes(mac, gpu, 32), []);
  assert.equal(estimate([a], gpu, cpu, 12, "horizon", { ramGb: 8 }).notes.length, 3);
});

test("breakdowns read as words, and the hardware table shows only with data", async () => {
  const { breakdownLine, hardwareHtml } = await import("./bench.js");
  assert.equal(breakdownLine({ ram: [{ class: "32 GB", installs: 3 }], storage: [{ class: "nvme", installs: 4 }], gpu_kind: [{ class: "discrete", installs: 3 }] }), "32 GB RAM (3) · NVMe (4) · separate GPU (3)");
  assert.equal(breakdownLine(undefined), "");
  assert.equal(hardwareHtml({ breakdowns: {} }), "");
  const html = hardwareHtml({ breakdowns: { os: [{ class: "linux", installs: 5, sessions: 9, hours: 4.5, p50_ms: 6.2, p99_ms: 14 }] } });
  assert.match(html, /Linux/);
  assert.match(html, /6\.2 ms/);
});
