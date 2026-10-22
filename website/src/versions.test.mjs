// node --test website/src/ (npm test in website/)
import { test } from "node:test";
import assert from "node:assert/strict";
import { compareTags, isGame, isLauncher, launcherVersion, newestFirst } from "./versions.js";

test("0.2 tags follow every tag from before", () => {
  const order = ["v2026.10.19.15", "v2026.10.21", "v0.2.0-beta.1", "v0.2.0", "v0.2.1-beta", "v0.2.1-beta.2",
    "v0.2.1-beta.11", "v0.2.1-rc.1", "v0.2.1", "v0.2.9", "v0.2.10", "v0.3.0-1", "v0.3.0-alpha", "v0.3.0", "v1.0.0"];
  assert.deepEqual([...order].reverse().sort(compareTags), order);
  assert.deepEqual(["launcher-v0.16.0", "launcher@0.2.0", "launcher-v0.17.0", "launcher@0.2.1"].sort(newestFirst),
    ["launcher@0.2.1", "launcher@0.2.0", "launcher-v0.17.0", "launcher-v0.16.0"]);
  assert.equal(compareTags("v2026.10.19", "v2026.10.19.0"), 0);
});

test("game and launcher tags never mix", () => {
  for (const tag of ["v0.2.0", "v0.3.0-rc.1", "v2026.10.21", "v2026.10.19.15"]) assert.ok(isGame(tag), tag);
  for (const tag of ["launcher@0.2.0", "launcher-v0.16.0", "v0.2", "vnext", "v2026.10.21-x"]) assert.ok(!isGame(tag), tag);
  assert.equal(launcherVersion("launcher@0.2.0"), "0.2.0");
  assert.equal(launcherVersion("launcher-v0.16.0"), "0.16.0");
  assert.ok(!isLauncher("v0.2.0") && !isLauncher("launcher-vnext"));
});
