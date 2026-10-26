import { test } from "node:test";
import assert from "node:assert/strict";
import { CAPABILITIES, count, entryFrom, listHtml, matches, projectHtml, rateHtml, ratingOf, readIndex, sorted, starsHtml, submitHtml } from "./mods.js";

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

test("downloads read short, as in the launcher, and a missing count is 0", () => {
  assert.equal(count(999), "999");
  assert.equal(count(12_345), "12.3k");
  assert.equal(count(51_404_735), "51.4M");
  assert.ok(listHtml([project({ downloads: 12_345 })]).includes("12.3k downloads"));
  assert.ok(listHtml([project({ downloads: 1 })]).includes("1 download<"));
  assert.ok(projectHtml(project({ downloads: "9999<script>" })).includes("0 downloads"));
});

test("ratings show their mean and count, and nothing a broken answer holds", () => {
  const ratings = { coords_hud: { average: 4.26, count: 12 }, zoom: { average: "<b>", count: 3 } };
  assert.deepEqual(ratingOf(ratings, "coords_hud"), { average: 4.26, count: 12 });
  assert.equal(ratingOf(ratings, "zoom"), null);
  assert.equal(ratingOf(ratings, "__proto__"), null);
  const stars = starsHtml(ratingOf(ratings, "coords_hud"));
  assert.ok(stars.includes(`<span class="on">★★★★</span>★`), stars);
  assert.ok(stars.includes("4.3 (12)"));
  assert.ok(starsHtml(null).includes("No ratings yet"));
  assert.ok(listHtml([project()], ratings).includes("4.3 (12)"));
  // Before the service answers (or without it), no stars at all.
  assert.ok(!listHtml([project()]).includes("No ratings yet"));
  assert.ok(!projectHtml(project()).includes("No ratings yet"));
  assert.ok(projectHtml(project(), null).includes("No ratings yet"));
});

test("the list orders by downloads, by rating or by name", () => {
  const a = project({ id: "a", name: "Alpha", downloads: 5 });
  const b = project({ id: "b", name: "Beta", downloads: 50 });
  const c = project({ id: "c", name: "Gamma" });
  const ratings = { a: { average: 4.5, count: 2 }, c: { average: 4.5, count: 9 } };
  const ids = (by) => sorted([a, b, c], by, ratings).map((p) => p.id);
  assert.deepEqual(ids("downloads"), ["b", "a", "c"]);
  assert.deepEqual(ids("rating"), ["c", "a", "b"]);
  assert.deepEqual(ids("name"), ["a", "b", "c"]);
});

test("rating: a sign-in link signed out, stars signed in, none for the project's owners", () => {
  const p = project({ owners: ["Alice"] });
  assert.ok(rateHtml(p, null, null).includes('href="/api/mods/auth/login?back=/mods/coords_hud">Sign in with GitHub to rate'));
  assert.ok(rateHtml(p, { login: "alice" }, null).includes("You publish it"));
  const mine = rateHtml(p, { login: "bob" }, 3);
  assert.equal((mine.match(/data-stars=/g) || []).length, 5);
  assert.equal((mine.match(/mod-star lit/g) || []).length, 3);
  assert.ok(mine.includes('data-stars="3" aria-pressed="true"'));
  assert.ok(rateHtml(p, { login: "bob" }, null).includes("Rate it"));
});
