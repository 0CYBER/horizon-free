import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { boardFrom, boardHtml, creditFrom, creditHtml, fixesIn, itemOf, journalFrom, journalHtml, journalJson, overviewHtml, progress, roadmapHtml, slimIssue, suggestItemsHtml } from "./hub.js";

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
  const week = { week: "2030-01", date: "2029-12-31", title: "A week", plain: "p", dev: "d" };
  const release = { tag_name: "v0.2.2", name: "0.2.2", published_at: "2030-01-02T00:00:00Z", html_url: "u", body: "" };
  const journal = journalHtml(journalFrom([release], [], { ...roadmap, journal: [week] }), roadmap);
  const board = boardHtml(boardFrom([], roadmap), roadmap);
  const page = readFileSync(new URL("./hub.html", import.meta.url), "utf8") + roadmapHtml(roadmap) + overviewHtml(roadmap) + board + journal;
  const ids = [...page.matchAll(/ id="([^"]+)"/g)].map((m) => m[1]);
  assert.deepEqual(ids.filter((id, i) => ids.indexOf(id) !== i), []);
  for (const item of roadmap.items) assert.ok(ids.includes(item.id), item.id);
});

test("the overview lists the focus items and never nests links", () => {
  const html = overviewHtml(roadmap);
  for (const item of roadmap.items.filter((i) => i.focus)) assert.ok(html.includes(`href="#${item.id}"`), item.id);
  for (const row of html.match(/<li class="row">[\s\S]*?<\/li>/g)) assert.equal(row.split("<a ").length, 2, row);
});

test("the board places issues and pull requests as Kira's board does", () => {
  const now = Date.parse("2030-01-10T00:00:00Z");
  const list = [
    issue(1),
    issue(2),
    issue(3, { assignees: [{ login: "x" }] }),
    pr(10, { body: "Fixes #1" }),
    pr(11, { body: "Fixes #2", draft: true }),
    pr(12, { labels: [{ name: `roadmap:${roadmap.items[0].id}` }] }),
    pr(13, { draft: true }),
    pr(14, { state: "closed", pull_request: { merged_at: "2030-01-09T00:00:00Z" } }),
    pr(15, { state: "closed", pull_request: { merged_at: "2029-12-01T00:00:00Z" } }),
  ].map(slimIssue);
  const board = boardFrom(list, roadmap, now);
  const ids = (column) => board[column].map((c) => c.n);
  assert.deepEqual(ids("todo"), []);
  assert.deepEqual(ids("progress"), [2, 3, 13]);
  assert.deepEqual(ids("review"), [1, 12]);
  assert.deepEqual(ids("done"), [14]);
  assert.deepEqual(board.review[0].prs, [10]);
  assert.equal(board.review[1].item, roadmap.items[0].id);
  assert.equal(itemOf(slimIssue(pr(16, { body: "Fixes #1438" })), roadmap), "end-city-banners");
});

test("the journal groups a release's changes by roadmap item and lists what awaits a release", () => {
  const releases = [
    { tag_name: "v0.2.2", name: "Horizon 0.2.2: Rich presence", published_at: "2030-01-02T00:00:00Z", html_url: "u2", body: "### New\n- Discord Rich Presence (#1787)\n\n### Fixed\n- A fix (#5, fixes #4)\n" },
    { tag_name: "launcher@0.2.1", name: "Horizon launcher 0.2.1", published_at: "2030-01-01T00:00:00Z", html_url: "u1", body: "### Fixed\n- Window (#6)\n" },
  ];
  const list = [
    pr(1788, { state: "closed", pull_request: { merged_at: "2030-01-03T00:00:00Z" } }),
    pr(7, { state: "closed", pull_request: { merged_at: "2030-01-03T00:00:00Z" }, labels: [{ name: "skip-release" }] }),
  ].map(slimIssue);
  const entries = journalFrom(releases, list, { ...roadmap, journal: [] });
  assert.deepEqual(entries.map((e) => e.id), ["next-release", "v0.2.2", "launcher@0.2.1"]);
  assert.deepEqual(entries[0].changes.map((c) => [c.prs, c.item]), [[[1788], "discord-presence"]]);
  assert.deepEqual(entries[1].changes.map((c) => [c.section, c.text, c.item]), [["new", "Discord Rich Presence", "discord-presence"], ["fixed", "A fix", null]]);
  const html = journalHtml(entries, roadmap);
  assert.ok(html.includes('href="#discord-presence"'));
  assert.ok(html.includes("And 1 fix."));
});

test("journal.json is the flat feed Kira reads: merges, releases, shipped items, weeks, ids that never change", () => {
  const releases = [
    { tag_name: "v0.2.48", name: "Horizon 0.2.48", published_at: "2030-01-02T00:00:00Z", html_url: "https://github.com/ludovic111/horizon/releases/tag/v0.2.48", body: "### New\n- Packs (#1)\n" },
    { tag_name: "nightly-1", name: "Nightly", published_at: "2030-01-03T00:00:00Z", html_url: "u", body: "" },
  ];
  const list = [
    pr(1788, { state: "closed", pull_request: { merged_at: "2030-01-04T00:00:00Z" }, html_url: "https://github.com/ludovic111/horizon/pull/1788" }),
    pr(7, { state: "closed", pull_request: { merged_at: "2030-01-04T00:00:00Z" }, labels: [{ name: "skip-release" }] }),
    pr(8, { state: "closed", pull_request: { merged_at: "2030-01-04T00:00:00Z" }, labels: [{ name: "skip-release" }, { name: "roadmap:lean-code" }] }),
    pr(9),
  ].map(slimIssue);
  const week = { week: "2030-01", date: "2029-12-31", title: "A week", plain: "p", dev: "d" };
  const feed = journalJson(releases, list, { ...roadmap, journal: [week] }, "2030-01-05T00:00:00Z");
  const byId = new Map(feed.entries.map((e) => [e.id, e]));
  // A merged pull request, with its roadmap item; skip-release ones only when they move an item; open ones never.
  assert.deepEqual(byId.get("pr-1788"), { id: "pr-1788", kind: "merge", date: "2030-01-04T00:00:00Z", title: "PR 1788", url: "https://github.com/ludovic111/horizon/pull/1788", number: 1788, item: "discord-presence", item_title: "Discord Rich Presence" });
  assert.ok(!byId.has("pr-7") && byId.get("pr-8").item === "lean-code" && !byId.has("pr-9"));
  // Game and launcher releases only.
  assert.equal(byId.get("release-v0.2.48").tag, "v0.2.48");
  assert.ok(!byId.has("release-nightly-1"));
  // Every shipped item, dated by its release when the feed has it, with its post and the announce flag.
  const shipped = roadmap.items.filter((i) => i.status === "shipped");
  for (const item of shipped) {
    const e = byId.get(`shipped-${item.id}`);
    assert.equal(e.kind, "shipped");
    assert.equal(e.url, `https://playhorizon.dev/hub#${item.id}`);
    assert.ok(e.post.length <= 240 && typeof e.announce === "boolean", item.id);
  }
  assert.equal(byId.get("shipped-server-resource-packs").date, "2030-01-02T00:00:00Z");
  assert.equal(byId.get("weekly-2030-01").date, "2030-01-06T23:59:59.000Z");
  // Newest first; every entry has what Kira needs, and links only where Kira shows them.
  assert.deepEqual(feed.entries.map((e) => e.date), feed.entries.map((e) => e.date).sort().reverse());
  for (const e of feed.entries) {
    assert.ok(e.id && ["merge", "release", "weekly", "shipped"].includes(e.kind) && e.date && e.title, e.id);
    assert.match(e.url, /^https:\/\/(github\.com|playhorizon\.dev)\//, e.id);
  }
});

test("items still to come take votes and suggestions; shipped ones don't", () => {
  const html = roadmapHtml(roadmap);
  for (const item of roadmap.items) {
    assert.equal(html.includes(`data-vote="${item.id}"`), item.status !== "shipped", item.id);
    assert.equal(suggestItemsHtml(roadmap).includes(`value="${item.id}"`), item.status !== "shipped", item.id);
  }
  // Hidden until the hub's server answers.
  assert.ok(html.includes('aria-pressed="false" hidden>'));
});

test("the contributors carry the Discord name of those who agreed on Kira", () => {
  const github = [{ login: "Alex", html_url: "https://github.com/Alex", avatar_url: "", contributions: 3 }, { login: "sam", html_url: "https://github.com/sam", avatar_url: "", contributions: 1 }];
  const kira = { contributors: [{ github: "alex", discord: "Alex <b>" }, { github: 5 }, null] };
  const credit = creditFrom(github, kira);
  assert.deepEqual(credit.map((c) => c.discord), ["Alex <b>", ""]);
  const html = creditHtml(credit);
  assert.ok(html.includes("3 commits · Alex &lt;b&gt; on Discord") && html.includes("1 commit</span>"));
  assert.deepEqual(creditFrom(github, "not json").map((c) => c.discord), ["", ""]);
});
