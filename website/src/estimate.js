// Estimates for hardware nobody has measured yet, from the nearest measured machine and the spec table
// (src/hardware.js). The method, which the Benchmarks page explains:
//
// - GPU factor: the geometric mean of the two GPUs' FP32 throughput ratio and memory bandwidth ratio.
// - CPU factor: the ratio of the two CPUs' single-thread scores (boost clock x generation factor).
// - Frames: when the measured run split each frame into CPU and GPU time, the slower of the two is scaled by its
//   own factor; otherwise the frame rate is scaled by the geometric mean of both factors. Whatever the split, the
//   true figure lies between the measured one scaled by the smaller and by the larger factor, widened by 15% for
//   the roughness of the table: that is the range shown.
// - World and chunk load times are CPU work (generation, lighting, meshing): scaled by the CPU factor.
// - Memory is not scaled: it depends on the world and settings, not on speed.
// - Memory size: players' groups say how their installs split by RAM and by the GPU's own memory (breakdowns). A
//   base whose players have neither the picked RAM nor the picked GPU's memory class counts as further away, and
//   the estimate says what the base's players had, and when the game's peak memory is a large share of the RAM.
import { findCpu, findGpu, ramClass, singleThread, vramClass } from "./hardware.js";

const MARGIN = 0.15;

export function factors(fromGpu, fromCpu, toGpu, toCpu) {
  const gpu = Math.sqrt((toGpu.tflops / fromGpu.tflops) * (toGpu.gbps / fromGpu.gbps));
  const cpu = singleThread(toCpu) / singleThread(fromCpu);
  return { gpu, cpu };
}

const distance = (f) => Math.abs(Math.log(f.gpu)) + Math.abs(Math.log(f.cpu));
// Added to the distance per memory class (RAM, VRAM) the base's players don't have: about a 20% speed gap.
const MEMORY_MISMATCH = 0.2;

// The classes a row's installs have in a breakdown, or null when it has none (our own rows, older summaries).
const classesIn = (row, dim) => (row.breakdowns?.[dim]?.length ? row.breakdowns[dim].map((c) => c.class) : null);

function mismatch(row, gpu, ramGb) {
  let d = 0;
  const vram = classesIn(row, "vram");
  if (vram && !vram.includes(vramClass(gpu))) d += MEMORY_MISMATCH;
  const ram = classesIn(row, "ram");
  if (ram && ramGb && !ram.includes(ramClass(ramGb))) d += MEMORY_MISMATCH;
  return d;
}

// What to say about memory: the base's players' RAM and VRAM when they differ from the pick, and the game's peak
// memory against the picked RAM.
export function memoryNotes(row, gpu, ramGb) {
  const notes = [];
  const list = (dim) => row.breakdowns[dim].map((c) => `${c.class} (${c.installs})`).join(", ");
  const vram = classesIn(row, "vram");
  if (vram && gpu && !vram.includes(vramClass(gpu))) notes.push(`The measured players' graphics cards have ${list("vram")} of their own memory; this one has ${gpu.vram_gb ? `${gpu.vram_gb} GB` : "none (it shares the system's)"}.`);
  const ram = classesIn(row, "ram");
  if (ram && ramGb && !ram.includes(ramClass(ramGb))) notes.push(`The measured players have ${list("ram")} of memory, not ${ramGb} GB.`);
  const peak = row.medians?.rss_peak_mb;
  if (ramGb && peak && peak / 1024 > ramGb * 0.4)
    notes.push(`${row.game === "vanilla" ? "Java" : "Horizon"} used up to ${(peak / 1024).toFixed(1)} GB here: with ${ramGb} GB, close other programs, or the system may swap and stutter.`);
  return notes;
}

// The rows (benchmarks.json groups) usable as a base: a known GPU and CPU.
function known(rows) {
  return rows.map((row) => ({ row, gpu: findGpu(row.gpu), cpu: findCpu(row.cpu) })).filter((r) => r.gpu && r.cpu);
}

// The measured row for exactly this GPU and CPU, players' first when there are some (more installs), or null.
export function measured(rows, gpu, cpu, rd, game) {
  const hits = known(rows).filter((r) => r.gpu === gpu && r.cpu === cpu && r.row.render_distance === rd && r.row.game === game);
  hits.sort((a, b) => (b.row.installs || 0) - (a.row.installs || 0));
  return hits.map((r) => r.row);
}

// The nearest measured machine for a target GPU and CPU, for one game and render distance.
export function nearest(rows, gpu, cpu, rd, game, { ramGb } = {}) {
  let best = null;
  for (const r of known(rows)) {
    if (r.row.render_distance !== rd || r.row.game !== game) continue;
    const f = factors(r.gpu, r.cpu, gpu, cpu);
    const d = distance(f) + mismatch(r.row, gpu, ramGb);
    if (!best || d < best.d) best = { ...r, f, d };
  }
  return best;
}

export function estimate(rows, gpu, cpu, rd, game, { ramGb } = {}) {
  const base = nearest(rows, gpu, cpu, rd, game, { ramGb });
  if (!base) return null;
  const m = base.row.medians;
  const { gpu: g, cpu: c } = base.f;
  const lo = Math.min(g, c) * (1 - MARGIN);
  const hi = Math.max(g, c) * (1 + MARGIN);
  let scale = Math.sqrt(g * c);
  if (m.cpu_p50_ms && m.gpu_p50_ms) {
    const worst = Math.max(m.cpu_p50_ms, m.gpu_p50_ms);
    scale = worst / Math.max(m.cpu_p50_ms / c, m.gpu_p50_ms / g);
  }
  scale = Math.min(hi, Math.max(lo, scale));
  const fps = (x) => (x == null ? null : { value: x * scale, low: x * lo, high: x * hi });
  // A base held to its display's refresh rate gives only a floor for the average.
  const avg = fps(m.avg_fps);
  if (avg && base.row.capped_fps && m.avg_fps >= base.row.capped_fps * 0.97) Object.assign(avg, { value: null, high: null, atLeast: true });
  const time = (x, f) => (x == null ? null : { value: x / f, low: x / (f * (1 + MARGIN)), high: x / (f * (1 - MARGIN)) });
  return {
    base: base.row,
    factors: base.f,
    avg_fps: avg,
    low_1pct_fps: fps(m.low_1pct_fps),
    p95_ms: m.p95_ms == null ? null : { value: m.p95_ms / scale, low: m.p95_ms / hi, high: m.p95_ms / lo },
    world_ms: time(m.world_ms, c),
    chunk_load_ms: time(m.chunk_load_ms, c),
    rss_steady_mb: m.rss_steady_mb == null ? null : { value: m.rss_steady_mb },
    rss_peak_mb: m.rss_peak_mb == null ? null : { value: m.rss_peak_mb },
    notes: memoryNotes(base.row, gpu, ramGb),
  };
}
