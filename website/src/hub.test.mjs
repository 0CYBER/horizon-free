import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fixesIn, overviewHtml, progress, roadmapHtml, slimIssue } from "./hub.js";

const roadmap = JSON.parse(readFileSync(new URL("../data/roadmap.json", import.meta.url), "utf8"));

const pr = (number, extra = {}) => ({ number, title: `PR ${number}`, state: "open", pull_request: { merged_at: null }, user: { login: "a" }, labels: [], assignees: [], body: "", ...extra });
const issue = (number, extra = {}) => ({ number, title: `Issue ${number}`, state: "open", user: { login: "b" }, labels: [{ name: "bug" }], assignees: [], ...extra });

test("pull requests and issues reduce to their hub state", () => {
  assert.deepEqual(fixesIn("Fixes #12, #13 and closes #4. Mentions #9."), [4, 12, 13]);
  const merged = slimIssue(pr(5, { state: "closed", pull_request: { merged_at: "2030-01-01T00:00:00Z" }, body: "Fixes #3" }));
  assert.equal(merged.kind, "pr");
  assert.equal(merged.state, "merged");
  assert.deepEqual(merged.fixes, [3]);
  assert.equal(slimIssue(issue(6, { state: "closed", state_reason: "not_planned" })).state, "dropped");
  assert.deepEqual(slimIssue(issue(7)).labels, ["bug"]);
});

test("an item's progress counts its pull requests and issues", () => {
  const live = new Map([
    [1, slimIssue(pr(1))],
    [2, slimIssue(pr(2, { draft: true }))],
    [3, slimIssue(pr(3, { state: "closed", pull_request: { merged_at: "2030-01-01T00:00:00Z" } }))],
    [4, slimIssue(issue(4))],
  ]);
  const item = { links: ["#1", "#2", "#3", "#4", "#99", "docs/x.md"] };
  assert.equal(progress(item, live), "1 pull request in review · 1 in progress · 1 merged · 1 issue open");
  assert.equal(progress(item, new Map()), "");
});

test("the roadmap renders every item once, with both texts, in its status's section", () => {
  const html = roadmapHtml(roadmap);
  for (const item of roadmap.items) {
    assert.equal(html.split(`id="${item.id}"`).length, 2, item.id);
  }
  assert.ok(html.indexOf('id="now"') < html.indexOf('id="next"'));
  assert.ok(html.includes('class="hub-plain"') && html.includes('class="hub-dev"'));
  // Shipped items say which release had them.
  assert.ok(html.includes(`>${roadmap.items.find((i) => i.status === "shipped").shipped_in}<`));
});

test("an item is its own anchor (hub#<id>, as Kira links it) and takes no other id of the page", () => {
  const page = readFileSync(new URL("./hub.html", import.meta.url), "utf8") + roadmapHtml(roadmap) + overviewHtml(roadmap);
  const ids = [...page.matchAll(/ id="([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(ids.filter((id, i) => ids.indexOf(id) !== i), []);
  for (const item of roadmap.items) assert.ok(ids.includes(item.id), item.id);
});

test("the overview lists the focus items and never nests links", () => {
  const html = overviewHtml(roadmap);
  for (const item of roadmap.items.filter((i) => i.focus)) assert.ok(html.includes(`href="#${item.id}"`), item.id);
  for (const row of html.match(/<li class="row">[\s\S]*?<\/li>/g)) assert.equal(row.split("<a ").length, 2, row);
});
