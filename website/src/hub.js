// The hub (/hub): the roadmap from website/data/roadmap.json (generated from roadmap/roadmap.toml by
// tools/roadmap.py), with each item's issues and pull requests in their live GitHub state. Shared by build.mjs
// (baked into the page) and hub-page.js (refreshed from GitHub in the browser). Every item says the same thing
// twice: `plain` (.hub-plain) for players and `dev` (.hub-dev) for contributors; the page's Simple / Technical
// switch shows one of them.
import { escape, inline } from "./markdown.js";
import { parts, plainItem, sections } from "./releases.js";
import { isGame, isLauncher } from "./versions.js";
import { avatar } from "./contributors.js";

export const REPO = "ludovic111/horizon";
const issueUrl = (n) => `https://github.com/${REPO}/issues/${n}`;
const fileUrl = (path) => `https://github.com/${REPO}/blob/main/${path}`;

// The issues a pull request closes when merged: "Fixes #12", "closes #3, resolves #4" (as Kira's board reads them).
export function fixesIn(text) {
  const out = new Set();
  for (const m of String(text ?? "").matchAll(/\b(?:fix(?:e[sd])?|close[sd]?|resolve[sd]?)\s*:?\s+((?:#\d+(?:\s*(?:,|and)\s*)?)+)/gi)) {
    for (const n of m[1].matchAll(/#(\d+)/g)) out.add(Number(n[1]));
  }
  return [...out].sort((a, b) => a - b);
}

// A GitHub issue or pull request (from /issues, which lists both) reduced to what the hub shows.
export function slimIssue(raw) {
  const pr = Boolean(raw.pull_request);
  const merged = pr && raw.pull_request.merged_at;
  return {
    n: raw.number,
    title: String(raw.title || ""),
    kind: pr ? "pr" : "issue",
    state: merged ? "merged" : raw.state === "open" ? "open" : raw.state_reason === "not_planned" ? "dropped" : "closed",
    draft: Boolean(raw.draft),
    user: raw.user?.login || "",
    labels: (raw.labels || []).map((l) => (typeof l === "string" ? l : l.name)).filter(Boolean),
    assigned: (raw.assignees || []).length > 0,
    fixes: pr ? fixesIn(raw.body) : [],
    at: merged || raw.closed_at || raw.updated_at || "",
    url: raw.html_url || issueUrl(raw.number),
  };
}

const number = (link) => Number((String(link).match(/^#(\d+)$/) || [])[1]) || null;

// "#12 in review", "#13 merged": a link's state, when GitHub told us.
function linkState(issue) {
  if (!issue) return null;
  if (issue.kind === "pr") return issue.state === "open" ? (issue.draft ? "in progress" : "in review") : issue.state;
  return issue.state === "open" ? "open" : "closed";
}

function linkHtml(link, live) {
  const n = number(link);
  if (!n) {
    const url = link.startsWith("https://") ? link : fileUrl(link);
    return `<a class="hub-link" href="${escape(url)}">${escape(link.replace(/^https:\/\//, ""))}</a>`;
  }
  const issue = live.get(n);
  const state = linkState(issue);
  const label = issue ? `${issue.kind === "pr" ? "Pull request" : "Issue"} #${n}: ${issue.title}` : `#${n}`;
  return `<a class="hub-link${state ? ` is-${state.replace(" ", "-")}` : ""}" href="${issueUrl(n)}" title="${escape(label)}">#${n}${state ? ` <span>${state}</span>` : ""}</a>`;
}

// The technical text with its #N and the item's own paths as links.
function devHtml(item) {
  let text = item.dev.replace(/(^|[^\w[/`])#(\d+)\b/g, (_, before, n) => `${before}[#${n}](${issueUrl(n)})`);
  for (const link of item.links || []) {
    if (!number(link) && !link.startsWith("https://")) text = text.replace(link, `[${link}](${fileUrl(link)})`);
  }
  return inline(text);
}

// "2 pull requests in review · 3 merged · 1 issue open", from the item's links.
export function progress(item, live) {
  const known = (item.links || []).map(number).filter(Boolean).map((n) => live.get(n)).filter(Boolean);
  const pulls = known.filter((i) => i.kind === "pr");
  const count = (list, word) => (list.length ? `${list.length} ${word}` : "");
  const parts = [
    count(pulls.filter((p) => p.state === "open" && !p.draft), pulls.filter((p) => p.state === "open" && !p.draft).length === 1 ? "pull request in review" : "pull requests in review"),
    count(pulls.filter((p) => p.state === "open" && p.draft), "in progress"),
    count(pulls.filter((p) => p.state === "merged"), "merged"),
    count(known.filter((i) => i.kind === "issue" && i.state === "open"), known.filter((i) => i.kind === "issue" && i.state === "open").length === 1 ? "issue open" : "issues open"),
  ].filter(Boolean);
  return parts.join(" · ");
}

const areaName = (roadmap, id) => roadmap.areas.find((a) => a.id === id)?.name || id;

// An item's vote button: hidden until the page has the counts from the hub's server (hub-page.js).
export function voteHtml(item) {
  return `<button type="button" class="button secondary small hub-vote" data-vote="${escape(item.id)}" aria-pressed="false" hidden><span class="hub-vote-label">Want it</span><span class="hub-vote-count" data-votes></span></button>`;
}

export function itemHtml(item, roadmap, live) {
  const done = item.status === "shipped";
  const state = done ? "" : progress(item, live);
  return `<article class="hub-item" id="${escape(item.id)}" data-area="${escape(item.area)}" data-item="${escape(item.id)}">
  <p class="hub-item-meta"><span class="badge">${escape(areaName(roadmap, item.area))}</span>${done ? `<span class="badge badge-game">${escape(item.shipped_in)}</span>` : ""}${item.ongoing ? `<span class="hub-ongoing">Ongoing</span>` : ""}${done ? "" : voteHtml(item)}</p>
  <h3 class="hub-item-title">${escape(item.title)}</h3>
  <p class="hub-plain">${inline(item.plain)}</p>
  <p class="hub-dev">${devHtml(item)}</p>
  ${state ? `<p class="hub-progress" data-progress>${escape(state)}</p>` : ""}
  <p class="hub-links">${(item.links || []).map((l) => linkHtml(l, live)).join("")}</p>
</article>`;
}

const SECTION_LEDE = {
  now: "Being worked on, with pull requests open or just merged.",
  next: "Decided, and up after what is being done now.",
  later: "Decided, further out.",
  "help-wanted": "Good first contributions: a clear vanilla behaviour, an issue with the steps, a place to start.",
  shipped: "Out in a release. The journal has every change.",
};

// The Roadmap tab: one section per status, the items in the file's order (areas by priority).
export function roadmapHtml(roadmap, live = new Map()) {
  const order = new Map(roadmap.areas.map((a, i) => [a.id, i]));
  return roadmap.statuses
    .map(({ id, name }) => {
      const items = roadmap.items.filter((i) => i.status === id);
      if (!items.length) return "";
      // Shipped stays newest first; the others go by the areas' priority.
      if (id !== "shipped") items.sort((a, b) => order.get(a.area) - order.get(b.area));
      return `<section class="hub-section" id="${id}" data-status="${id}" aria-labelledby="h-${id}">
  <div class="hub-section-head"><h2 class="title" id="h-${id}">${escape(name)}</h2><p class="meta">${escape(SECTION_LEDE[id] || "")}</p></div>
  <div class="hub-items">
${items.map((item) => itemHtml(item, roadmap, live)).join("\n")}
  </div>
</section>`;
    })
    .join("\n");
}

// The area filter's buttons: every area that has an item.
export function areaFilterHtml(roadmap) {
  const used = roadmap.areas.filter((a) => roadmap.items.some((i) => i.area === a.id));
  return [`<button type="button" class="button secondary small" data-area-filter="all" aria-pressed="true">All</button>`]
    .concat(used.map((a) => `<button type="button" class="button secondary small" data-area-filter="${escape(a.id)}" aria-pressed="false">${escape(a.name)}</button>`))
    .join("\n");
}

// The Overview tab: where Horizon is, the 3 to 5 things in focus, what is next, what shipped last, how to help.
export function overviewHtml(roadmap, live = new Map()) {
  const o = roadmap.overview;
  const focus = roadmap.items.filter((i) => i.focus);
  const next = roadmap.items.filter((i) => i.status === "next");
  const shipped = roadmap.items.filter((i) => i.status === "shipped").slice(0, 3);
  const row = (item, i, meta) => `<li class="row"><a href="#${escape(item.id)}" data-goto="roadmap"><span class="row-index">${String(i + 1).padStart(2, "0")}</span><span class="hub-row-body"><span class="hub-row-title">${escape(item.title)}</span><span class="hub-plain">${inline(item.plain)}</span><span class="hub-dev">${inline(item.dev)}</span></span><span class="row-meta">${escape(meta(item))}</span></a></li>`;
  return `<div class="hub-today">
  <p class="hub-plain hub-lead">${inline(o.today_plain)}</p>
  <p class="hub-dev hub-lead">${inline(o.today_dev)}</p>
</div>
<section class="hub-block" aria-labelledby="h-ov-focus">
  <h2 class="title" id="h-ov-focus">Being done now</h2>
  <ol class="index hub-rows">${focus.map((item, i) => row(item, i, (x) => progress(x, live))).join("")}</ol>
</section>
<section class="hub-block" aria-labelledby="h-ov-next">
  <h2 class="title" id="h-ov-next">Next</h2>
  <ol class="index hub-rows">${next.map((item, i) => row(item, i, (x) => areaName(roadmap, x.area))).join("")}</ol>
</section>
<section class="hub-block" aria-labelledby="h-ov-shipped">
  <h2 class="title" id="h-ov-shipped">Recently shipped</h2>
  <ol class="index hub-rows">${shipped.map((item, i) => row(item, i, (x) => `In ${x.shipped_in}`)).join("")}</ol>
</section>
<section class="hub-block hub-help" aria-labelledby="h-ov-help">
  <h2 class="title" id="h-ov-help">How to help</h2>
  <p class="hub-plain">${inline(o.help_plain)}</p>
  <p class="hub-dev">${inline(o.help_dev)}</p>
  <p class="hub-actions"><a class="button primary" href="#contribute" data-goto="contribute">Contribute</a><a class="button secondary" href="#help-wanted" data-goto="roadmap">Help wanted</a><a class="button secondary" href="#report">Report a bug</a><a class="button secondary" href="#suggest">Suggest something</a></p>
</section>`;
}

// The Contribute tab's list of good first tasks.
export function helpWantedHtml(roadmap) {
  return roadmap.items
    .filter((i) => i.status === "help-wanted")
    .map((item, i) => {
      const first = (item.links || []).map(number).find(Boolean);
      const href = first ? issueUrl(first) : `#${item.id}`;
      return `<li class="row"><a href="${escape(href)}"><span class="row-index">${String(i + 1).padStart(2, "0")}</span><span class="hub-row-body"><span class="hub-row-title">${escape(item.title)}</span><span class="hub-plain">${inline(item.plain)}</span><span class="hub-dev">${inline(item.dev)}</span></span><span class="row-meta">${first ? `#${first}` : ""}</span></a></li>`;
    })
    .join("");
}

// --- The work board --------------------------------------------------------------------------------------------

export const COLUMNS = [
  { id: "todo", name: "To do", about: "Open and free: pick one, and say so with /claim on Discord." },
  { id: "progress", name: "In progress", about: "Claimed, assigned, or a draft pull request." },
  { id: "review", name: "In review", about: "A pull request is ready for review." },
  { id: "done", name: "Done", about: "Closed or merged in the last seven days." },
];
const DONE_DAYS = 7;

// The roadmap item an issue or pull request moves: its roadmap:<id> label, else the item linking it, else (for a
// pull request) the item linking an issue it fixes.
export function itemOf(issue, roadmap) {
  const label = issue.labels.find((l) => l.startsWith("roadmap:"));
  const ids = new Set(roadmap.items.map((i) => i.id));
  if (label && ids.has(label.slice(8))) return label.slice(8);
  const linking = (n) => roadmap.items.find((i) => (i.links || []).includes(`#${n}`))?.id;
  return linking(issue.n) || issue.fixes.map(linking).find(Boolean) || null;
}

// The board from GitHub's open issues and pull requests and those closed lately, placed as Kira's work board places
// them: an issue goes to review when an open, ready pull request fixes it, to progress when a draft one does or it is
// claimed or assigned; a pull request that fixes no open issue is a card of its own.
export function boardFrom(list, roadmap, now = Date.now()) {
  const columns = Object.fromEntries(COLUMNS.map((c) => [c.id, []]));
  const open = list.filter((i) => i.state === "open");
  const openIssues = new Set(open.filter((i) => i.kind === "issue").map((i) => i.n));
  const fixing = (n) => open.filter((p) => p.kind === "pr" && p.fixes.includes(n));
  const card = (i, prs = []) => ({ n: i.n, kind: i.kind, title: i.title, user: i.user, url: i.url, item: itemOf(i, roadmap) || prs.map((p) => itemOf(p, roadmap)).find(Boolean) || null, prs: prs.map((p) => p.n) });
  for (const i of open) {
    if (i.kind === "issue") {
      const prs = fixing(i.n);
      const claimed = i.assigned || i.labels.some((l) => /^(in progress|claimed)$/i.test(l));
      const column = prs.some((p) => !p.draft) ? "review" : prs.length || claimed ? "progress" : "todo";
      columns[column].push(card(i, prs));
    } else if (!i.fixes.some((n) => openIssues.has(n))) {
      columns[i.draft ? "progress" : "review"].push(card(i));
    }
  }
  const since = now - DONE_DAYS * 864e5;
  const seen = new Set();
  for (const i of list) {
    if (seen.has(i.n) || !["merged", "closed"].includes(i.state) || !(Date.parse(i.at) >= since)) continue;
    seen.add(i.n);
    columns.done.push(card(i));
  }
  columns.done.sort((a, b) => b.n - a.n);
  return columns;
}

const SHOWN = 20; // cards per column before "Show more"

function cardHtml(c, roadmap, more) {
  const item = c.item && roadmap.items.find((i) => i.id === c.item);
  return `<li class="hub-card" data-item="${escape(c.item || "")}"${more ? " data-more" : ""}><a href="${escape(c.url)}"><span class="hub-card-ref">${c.kind === "pr" ? "PR" : "Issue"} #${c.n}${c.prs.length ? ` · ${c.prs.map((n) => `PR #${n}`).join(", ")}` : ""}</span><span class="hub-card-title">${escape(c.title)}</span><span class="hub-card-meta">${item ? `<span class="hub-card-item">${escape(item.title)}</span>` : ""}${c.user ? `<span>${escape(c.user)}</span>` : ""}</span></a></li>`;
}

export function boardHtml(columns, roadmap) {
  return COLUMNS.map(({ id, name, about }) => {
    const cards = columns[id];
    return `<section class="hub-column" data-column="${id}" aria-labelledby="col-${id}">
  <h3 id="col-${id}"><span>${escape(name)}</span><span class="hub-count" data-count>${cards.length}</span></h3>
  <p class="meta">${escape(about)}</p>
  <ol class="hub-cards-list">${cards.map((c, i) => cardHtml(c, roadmap, i >= SHOWN)).join("")}</ol>
  ${cards.length > SHOWN ? `<button type="button" class="button secondary small hub-more" data-more-button>Show ${cards.length - SHOWN} more</button>` : ""}
</section>`;
  }).join("\n");
}

// The board's roadmap filter: every item that has a card.
export function boardFilterHtml(columns, roadmap) {
  const used = new Set(Object.values(columns).flat().map((c) => c.item).filter(Boolean));
  return [`<option value="">Every card</option>`, `<option value="-">Cards of a roadmap item</option>`]
    .concat(roadmap.items.filter((i) => used.has(i.id)).map((i) => `<option value="${escape(i.id)}">${escape(i.title)}</option>`))
    .join("");
}

// --- The journal -----------------------------------------------------------------------------------------------

export const JOURNAL_URL = "https://playhorizon.dev/hub#journal";
const ENTRIES = 40;

// The journal, newest first: each release with its changes grouped by roadmap item, the weekly summaries
// (roadmap/journal/), and what merged since the newest game release. releases: GitHub's release objects; list: the
// hub's issues and pull requests (their roadmap:<id> labels place changes the roadmap doesn't link).
export function journalFrom(releases, list, roadmap) {
  const prs = new Map(list.filter((i) => i.kind === "pr").map((i) => [i.n, i]));
  const placed = (numbers) => {
    for (const n of numbers) {
      const known = prs.get(n);
      const id = known ? itemOf(known, roadmap) : roadmap.items.find((i) => (i.links || []).includes(`#${n}`))?.id;
      if (id) return id;
    }
    return null;
  };
  const ours = releases.filter((r) => !r.draft && (isGame(r.tag_name) || isLauncher(r.tag_name)));
  const entries = ours.map((r) => {
    const changes = sections(r.body)
      .filter((x) => /^(new|what's new|fixed)/i.test(x.title))
      .flatMap((x) => x.items.map((text) => ({ section: /^fixed/i.test(x.title) ? "fixed" : "new", text })))
      .map((c) => {
        const numbers = [...c.text.matchAll(/#(\d+)/g)].map((m) => Number(m[1]));
        return { ...c, text: plainItem(c.text), prs: numbers, item: placed(numbers) };
      });
    return {
      id: r.tag_name,
      kind: isLauncher(r.tag_name) ? "launcher" : "release",
      date: r.published_at || "",
      title: r.name || r.tag_name,
      plain: parts(r).line,
      url: r.html_url,
      changes,
      shipped: roadmap.items.filter((i) => i.status === "shipped" && i.shipped_in === r.tag_name).map((i) => i.id),
    };
  });
  // Merged but in no release's notes yet: a release is cut from main a while after its last merge, so the newest
  // releases' notes, not their dates, say what they hold. Pull requests players don't see (skip-release) never are.
  const games = ours.filter((r) => isGame(r.tag_name)).map((r) => r.published_at).sort().reverse();
  const since = games[Math.min(4, games.length - 1)] || "";
  const noted = new Set(entries.flatMap((e) => e.changes.flatMap((c) => c.prs)));
  const pending = list
    .filter((i) => i.kind === "pr" && i.state === "merged" && i.at > since && !noted.has(i.n) && !i.labels.includes("skip-release"))
    .sort((a, b) => b.at.localeCompare(a.at));
  if (pending.length) {
    entries.push({
      id: "next-release",
      kind: "merged",
      date: pending[0].at,
      title: "Merged, in the next release",
      plain: `${pending.length} change${pending.length === 1 ? "" : "s"} merged since the last release.`,
      url: `https://github.com/${REPO}/pulls?q=is%3Apr+is%3Amerged`,
      changes: pending.map((p) => ({ section: "merged", text: p.title, prs: [p.n], item: itemOf(p, roadmap) })),
      shipped: [],
    });
  }
  for (const w of roadmap.journal || []) {
    // A week's summary sits after its last day's releases.
    const end = new Date(Date.parse(w.date) + 7 * 864e5 - 1).toISOString();
    entries.push({ id: `week-${w.week}`, kind: "week", date: end, title: w.title, plain: w.plain, dev: w.dev, url: `${JOURNAL_URL.replace("#journal", "")}#week-${w.week}`, changes: [], shipped: [] });
  }
  return entries.sort((a, b) => b.date.localeCompare(a.date)).slice(0, ENTRIES);
}

const HUB = "https://playhorizon.dev/hub";
const FEED = 300;

// /data/journal.json, which Kira reads for #updates and X (kira's src/journal.js): one flat entry per merged pull
// request, release, shipped roadmap item and weekly summary, newest first. An entry's id never changes: Kira posts
// each id once. Shipped items carry `post` (at most 240 characters, tools/roadmap.py checks) and `announce`.
export function journalJson(releases, list, roadmap, generated = new Date().toISOString()) {
  const items = new Map(roadmap.items.map((i) => [i.id, i]));
  const ours = releases.filter((r) => !r.draft && (isGame(r.tag_name) || isLauncher(r.tag_name)));
  const published = new Map(ours.map((r) => [r.tag_name, r.published_at || ""]));
  const merges = list
    .filter((i) => i.kind === "pr" && i.state === "merged")
    .map((pr) => ({ pr, item: itemOf(pr, roadmap) }))
    // What players see, and anything that moves a roadmap item.
    .filter(({ pr, item }) => item || !pr.labels.includes("skip-release"))
    .map(({ pr, item }) => ({
      id: `pr-${pr.n}`,
      kind: "merge",
      date: pr.at,
      title: pr.title,
      url: pr.url,
      number: pr.n,
      ...(item ? { item, item_title: items.get(item)?.title } : {}),
    }));
  const released = ours.map((r) => ({
    id: `release-${r.tag_name}`,
    kind: "release",
    date: r.published_at || "",
    title: r.name || r.tag_name,
    plain: parts(r).line,
    url: r.html_url,
    tag: r.tag_name,
  }));
  const shipped = roadmap.items
    .filter((i) => i.status === "shipped")
    .map((i) => ({
      id: `shipped-${i.id}`,
      kind: "shipped",
      date: published.get(i.shipped_in) || generated,
      title: i.title,
      plain: i.plain,
      url: `${HUB}#${i.id}`,
      item: i.id,
      item_title: i.title,
      shipped_in: i.shipped_in,
      post: i.post || i.plain,
      announce: Boolean(i.announce),
    }));
  // A week's summary is dated at the end of its week, after that week's releases.
  const weekly = (roadmap.journal || []).map((w) => ({
    id: `weekly-${w.week}`,
    kind: "weekly",
    date: new Date(Date.parse(w.date) + 7 * 864e5 - 1000).toISOString(),
    title: w.title,
    plain: w.plain,
    dev: w.dev,
    url: `${HUB}#week-${w.week}`,
  }));
  const entries = [...merges, ...released, ...shipped, ...weekly].sort((a, b) => b.date.localeCompare(a.date) || a.id.localeCompare(b.id));
  return { format: "horizon-journal", version: 2, generated, url: `${HUB}#journal`, entries: entries.slice(0, FEED) };
}

const KIND = { release: "Game", launcher: "Launcher", merged: "Merged", week: "This week" };

function changesHtml(entry, roadmap) {
  if (!entry.changes.length) return "";
  const line = (c) => `<li>${inline(c.text)}${c.prs.length ? ` ${c.prs.map((n) => `<a class="hub-ref" href="${issueUrl(n)}">#${n}</a>`).join(" ")}` : ""}</li>`;
  // Simple: what is new, then how many fixes. Technical: every change, under the roadmap item it moves.
  const added = entry.changes.filter((c) => c.section !== "fixed");
  const fixes = entry.changes.length - added.length;
  const simple = `<div class="hub-plain">${added.length ? `<ul class="hub-changes">${added.map((c) => `<li>${inline(c.text)}</li>`).join("")}</ul>` : ""}${fixes ? `<p class="meta">${added.length ? "And " : ""}${fixes} fix${fixes === 1 ? "" : "es"}.</p>` : ""}</div>`;
  const groups = new Map();
  for (const c of entry.changes) groups.set(c.item || "", [...(groups.get(c.item || "") || []), c]);
  const order = [...groups.keys()].sort((a, b) => (a === "") - (b === ""));
  const dev = `<div class="hub-dev">${order
    .map((id) => {
      const item = roadmap.items.find((i) => i.id === id);
      return `<p class="hub-group">${item ? `<a href="#${escape(id)}">${escape(item.title)}</a>` : "Other changes"}</p><ul class="hub-changes">${groups.get(id).map(line).join("")}</ul>`;
    })
    .join("")}</div>`;
  return simple + dev;
}

export function journalHtml(entries, roadmap) {
  return entries
    .map((e) => {
      const shipped = e.shipped.map((id) => roadmap.items.find((i) => i.id === id)).filter(Boolean);
      const items = [...new Set(e.changes.map((c) => c.item).filter(Boolean))].map((id) => roadmap.items.find((i) => i.id === id)).filter(Boolean);
      return `<article class="hub-entry hub-entry-${e.kind}" id="${e.kind === "week" ? escape(e.id) : `journal-${escape(e.id.replace(/[^\w.-]/g, "-"))}`}">
  <p class="hub-entry-meta"><span class="badge${e.kind === "release" ? " badge-game" : e.kind === "launcher" ? " badge-launcher" : ""}">${KIND[e.kind]}</span><time datetime="${escape(e.date)}">${escape(e.date.slice(0, 10))}</time></p>
  <h3 class="hub-entry-title">${e.url ? `<a href="${escape(e.url)}">${escape(e.title)}</a>` : escape(e.title)}</h3>
  ${e.kind === "week" ? `<p class="hub-plain">${inline(e.plain)}</p><p class="hub-dev">${inline(e.dev.replace(/(^|[^\w[/`])#(\d+)\b/g, (_, b, n) => `${b}[#${n}](${issueUrl(n)})`))}</p>` : ""}
  ${shipped.length ? `<p class="hub-shipped">Shipped: ${shipped.map((i) => `<a href="#${escape(i.id)}">${escape(i.title)}</a>`).join(", ")}</p>` : ""}
  ${items.length ? `<p class="hub-entry-items hub-plain">${items.map((i) => `<a class="hub-link" href="#${escape(i.id)}">${escape(i.title)}</a>`).join("")}</p>` : ""}
  ${changesHtml(e, roadmap)}
</article>`;
    })
    .join("\n");
}

// --- Contribute: the suggestion form's items and the contributors ------------------------------------------------

// The suggestion form's "About" choices: the items still to come.
export function suggestItemsHtml(roadmap) {
  const open = roadmap.items.filter((i) => i.status !== "shipped");
  return [`<option value="">Something new</option>`]
    .concat(open.map((i) => `<option value="${escape(i.id)}">${escape(i.title)}</option>`))
    .join("");
}

// The contributors, from GitHub (contributors.js's list), with the Discord name of those who linked their accounts
// on Kira and agreed to be credited (Kira's /contributors.json: { contributors: [{ github, discord }] }).
export function creditFrom(github, kira) {
  const discord = new Map(
    (Array.isArray(kira?.contributors) ? kira.contributors : [])
      .filter((c) => c && typeof c.github === "string" && typeof c.discord === "string")
      .map((c) => [c.github.toLowerCase(), c.discord]),
  );
  return github.map((c) => ({ ...c, discord: discord.get(c.login.toLowerCase()) || "" }));
}

export function creditHtml(list) {
  return list
    .map((c) => {
      const src = avatar(c.avatar_url);
      const count = `${c.contributions.toLocaleString("en-US")} ${c.contributions === 1 ? "commit" : "commits"}`;
      return `<li><a class="contributor" href="${escape(c.html_url)}">${src ? `<img class="contributor-avatar" loading="lazy" decoding="async" src="${escape(src)}" width="48" height="48" alt="">` : `<span class="contributor-avatar" aria-hidden="true"></span>`}<span class="contributor-name">${escape(c.login)}</span><span class="contributor-count">${count}${c.discord ? ` · ${escape(c.discord)} on Discord` : ""}</span></a></li>`;
    })
    .join("");
}
