import { test } from "node:test";
import assert from "node:assert/strict";
import { CAPABILITIES, entryFrom, listHtml, matches, projectHtml, readIndex, submitHtml } from "./mods.js";

const project = (over = {}) => ({
  id: "coords_hud", kind: "mod", name: "Coordinates HUD", summary: "Where you are.", authors: ["Horizon"],
  license: "MIT", source: "https://example.org/src",
  versions: [{ version: "0.1.0", api: "0.1", side: "client", capabilities: ["game-read", "hud"], game: "26.3",
    file: { name: "coords_hud-0.1.0.hmod", url: "https://github.com/ludovic111/horizon-mods/releases/download/x/coords_hud-0.1.0.hmod", size: 438, sha256: "e".repeat(64) } }],
  ...over,
});

test("reads schema 1, keeps the kinds it knows and refuses a newer schema", () => {
  const index = { schema: 1, projects: [project(), project({ id: "map", kind: "world" }), project({ id: "Bad id" })] };
  assert.deepEqual(readIndex(index).map((p) => p.id), ["coords_hud"]);
  assert.equal(readIndex({ schema: 2, projects: [project()] }), null);
  assert.equal(readIndex(null), null);
});

test("filters by kind and every word", () => {
  const p = project();
  assert.ok(matches(p, "all", "coordinates horizon"));
  assert.ok(matches(p, "mod", ""));
  assert.ok(!matches(p, "modpack", ""));
  assert.ok(!matches(p, "all", "zoom"));
});

test("escapes what authors wrote and only links https addresses", () => {
  const evil = project({ name: "<script>x</script>", source: "javascript:alert(1)", summary: '"><img src=x onerror=alert(1)>' });
  const html = listHtml([evil]) + projectHtml(evil);
  assert.ok(!html.includes("<script>x"));
  assert.ok(!html.includes("<img src=x"));
  assert.ok(!html.includes("javascript:"));
});

test("a mod's page says what it may do, as the game does", () => {
  const html = projectHtml(project());
  assert.ok(html.includes(`May ${CAPABILITIES["game-read"]}`));
  assert.ok(html.includes("Unsigned"));
  assert.ok(projectHtml(project({ key: "RWT" })).includes("Signed"));
  assert.ok(html.includes("horizon-launcher mods install --source horizon coords_hud"));
});

test("the submit form sends what the service reads", () => {
  const html = submitHtml();
  for (const name of ["kind", "file", "id", "version", "game", "name", "summary", "license", "source", "key", "signature"]) {
    assert.ok(html.includes(`id="f-${name}"`), name);
  }
  assert.match(html, /<option value="shader" disabled>/);
  const entry = entryFrom({ id: " my_mod ", kind: "mod", name: "My mod", summary: "s", description: "", authors: "Alice, , Bob", license: "MIT", source: "https://x.test", icon: " ", key: "", signature: "", version: "1.0.0", game: "26.3", changelog: "" });
  assert.deepEqual(entry.authors, ["Alice", "Bob"]);
  assert.equal(entry.id, "my_mod");
  assert.equal(entry.icon, undefined);
  assert.equal(JSON.parse(JSON.stringify(entry)).key, undefined, "an empty key isn't sent");
});

test("a project's page links its owners' GitHub profiles, and only valid logins", () => {
  const html = projectHtml(project({ owners: ["Stink-O", "bad login", "-x", "a--b", "ok\"><script>"] }));
  assert.ok(html.includes('href="https://github.com/Stink-O"'));
  assert.ok(html.includes("@Stink-O"));
  for (const bad of ["bad login", "github.com/-x", "a--b", "<script>"]) assert.ok(!html.includes(bad), bad);
  assert.ok(!projectHtml(project()).includes("Published by"));
});
