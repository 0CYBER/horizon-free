// node --test (npm test in website/). The fixtures are shared with the telemetry server's summary test.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { aggregatePlayers, median, osClass, ourRows, ramClass, rejection, round, storageClass, summary, vramClass, gpuKindClass } from "./benchmarks.mjs";

const fixture = (name) => new URL(`./fixtures/benchmarks/${name}`, import.meta.url);
const items = readFileSync(fixture("items.jsonl"), "utf8").split("\n").filter(Boolean).map((l) => JSON.parse(l));
const expected = JSON.parse(readFileSync(fixture("expected.json"), "utf8"));

test("players' items give the expected groups", () => {
  const s = summary({ players: aggregatePlayers(items) });
  assert.equal(s.format, "horizon-benchmark-summary");
  assert.equal(s.min_installs, expected.min_installs);
  assert.deepEqual(s.groups, expected.groups);
});

test("nothing that identifies an install is published", () => {
  const text = JSON.stringify(aggregatePlayers(items, { minInstalls: 1 }));
  for (const secret of ["a1", "a2", "install_id", "secret-os-version", "secret-driver", "20394", "ram_mb", "2560", "16376", "32768", "vram_mb"]) assert.ok(!text.includes(secret), secret);
});

test("a group needs three installs, not three runs", () => {
  const one = items.filter((i) => i.install_id === "a1");
  assert.equal(aggregatePlayers([...one, ...one, ...one]).length, 0);
  assert.equal(aggregatePlayers(items, { minInstalls: 2 }).length, 2);
});

test("hardware classes are coarse, as the server's", () => {
  assert.equal(ramClass(15.6), "16 GB");
  assert.equal(ramClass(31), "32 GB");
  assert.equal(ramClass(64), "64 GB or more");
  assert.equal(ramClass(0), null);
  assert.equal(vramClass(16376, "discrete"), "16 GB");
  assert.equal(vramClass(8192, "DiscreteGpu"), "8 GB");
  assert.equal(vramClass(0, "integrated"), "shared");
  assert.equal(vramClass(0, ""), null);
  assert.equal(storageClass("nvme"), "nvme");
  assert.equal(storageClass("tape"), null);
  assert.equal(osClass("windows"), "windows");
  assert.equal(gpuKindClass("virtual"), null);
});

test("a breakdown cell needs three installs too", () => {
  const [group] = aggregatePlayers(items);
  assert.equal(group.breakdowns.ram, undefined); // 32 GB has two installs, 16 GB one
  assert.equal(group.breakdowns.storage, undefined);
  const [all] = aggregatePlayers(items, { minInstalls: 1 });
  assert.deepEqual(all.breakdowns.ram, [{ class: "32 GB", installs: 2 }, { class: "16 GB", installs: 1 }]);
  assert.deepEqual(all.breakdowns.storage, [{ class: "nvme", installs: 2 }, { class: "ssd", installs: 1 }]);
});

test("only comparable runs count", () => {
  const reasons = items.map((i) => rejection(i.data));
  assert.deepEqual(reasons.filter(Boolean), ["VSync on or unknown", "offscreen", "frame rate capped", "not every scene", "not the standard flight", "not a version 1 benchmark report"]);
});

test("medians and rounding", () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  assert.equal(median([null, NaN]), null);
  assert.equal(round("p95_ms", 3.46), 3.5);
  assert.equal(round("world_ms", 4512.4), 4512);
  assert.equal(round("avg_fps", 99.5), 100);
});

test("our runs keep the machine and how it was measured", () => {
  const [row] = ourRows([{ machine: "Desktop", report: items[0].data }]);
  assert.equal(row.source, "us");
  assert.equal(row.machine, "Desktop");
  assert.equal(row.game, "horizon");
  assert.equal(row.medians.avg_fps, 415);
  assert.equal(row.scenes.ocean.avg_fps, 420);
  assert.throws(() => ourRows([{ machine: "x", report: items[4].data }]), /VSync/);
  // Several one-game reports: medians over them.
  const [both] = ourRows([{ machine: "Desktop", reports: [items[0].data, items[1].data] }]);
  assert.equal(both.medians.avg_fps, 515);
  assert.equal(both.repeats, 2);
  assert.throws(() => ourRows([{ machine: "x", reports: [items[0].data, items[9].data] }]), /different/);
});

test("a display cap on our run is kept, and the comparison says at least", async () => {
  const [row] = ourRows([{ machine: "Mac", report: items[0].data, capped_fps: 144, version: "v1" }]);
  assert.equal(row.capped_fps, 144);
  assert.equal(row.version, "v1");
  const { compareRows, METRICS } = await import("../src/bench.js");
  const fps = METRICS[0];
  const h = { capped_fps: 144, medians: { avg_fps: 144 } };
  assert.equal(compareRows(fps, h, { medians: { avg_fps: 120 } }).text, "at least 1.2x faster");
  assert.equal(compareRows(fps, h, { medians: { avg_fps: 200 } }).wins, "none");
});
