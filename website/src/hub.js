// The hub (/hub): the roadmap from website/data/roadmap.json (generated from roadmap/roadmap.toml by
// tools/roadmap.py), with each item's issues and pull requests in their live GitHub state. Shared by build.mjs
// (baked into the page) and hub-page.js (refreshed from GitHub in the browser). Every item says the same thing
// twice: `plain` (.hub-plain) for players and `dev` (.hub-dev) for contributors; the page's Simple / Technical
// switch shows one of them.
import { escape, inline } from "./markdown.js";

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

export function itemHtml(item, roadmap, live) {
  const done = item.status === "shipped";
  const state = done ? "" : progress(item, live);
  return `<article class="hub-item" id="item-${escape(item.id)}" data-area="${escape(item.area)}" data-item="${escape(item.id)}">
  <p class="hub-item-meta"><span class="badge">${escape(areaName(roadmap, item.area))}</span>${done ? `<span class="badge badge-game">${escape(item.shipped_in)}</span>` : ""}${item.ongoing ? `<span class="hub-ongoing">Ongoing</span>` : ""}</p>
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
  const row = (item, i, meta) => `<li class="row"><a href="#item-${escape(item.id)}" data-goto="roadmap"><span class="row-index">${String(i + 1).padStart(2, "0")}</span><span class="hub-row-body"><span class="hub-row-title">${escape(item.title)}</span><span class="hub-plain">${inline(item.plain)}</span><span class="hub-dev">${inline(item.dev)}</span></span><span class="row-meta">${escape(meta(item))}</span></a></li>`;
  return `<div class="hub-today">
  <p class="hub-plain hub-lead">${inline(o.today_plain)}</p>
  <p class="hub-dev hub-lead">${inline(o.today_dev)}</p>
</div>
<section class="hub-block" aria-labelledby="h-focus">
  <h2 class="title" id="h-focus">Being done now</h2>
  <ol class="index hub-rows">${focus.map((item, i) => row(item, i, (x) => progress(x, live))).join("")}</ol>
</section>
<section class="hub-block" aria-labelledby="h-next">
  <h2 class="title" id="h-next">Next</h2>
  <ol class="index hub-rows">${next.map((item, i) => row(item, i, (x) => areaName(roadmap, x.area))).join("")}</ol>
</section>
<section class="hub-block" aria-labelledby="h-shipped">
  <h2 class="title" id="h-shipped">Recently shipped</h2>
  <ol class="index hub-rows">${shipped.map((item, i) => row(item, i, (x) => `In ${x.shipped_in}`)).join("")}</ol>
</section>
<section class="hub-block hub-help" aria-labelledby="h-help">
  <h2 class="title" id="h-help">How to help</h2>
  <p class="hub-plain">${inline(o.help_plain)}</p>
  <p class="hub-dev">${inline(o.help_dev)}</p>
  <p class="hub-actions"><a class="button primary" href="#contribute" data-goto="contribute">Contribute</a><a class="button secondary" href="#help-wanted" data-goto="roadmap">Help wanted</a></p>
</section>`;
}

// The Contribute tab's list of good first tasks.
export function helpWantedHtml(roadmap) {
  return roadmap.items
    .filter((i) => i.status === "help-wanted")
    .map((item, i) => {
      const first = (item.links || []).map(number).find(Boolean);
      const href = first ? issueUrl(first) : `#item-${item.id}`;
      return `<li class="row"><a href="${escape(href)}"><span class="row-index">${String(i + 1).padStart(2, "0")}</span><span class="hub-row-body"><span class="hub-row-title">${escape(item.title)}</span><span class="hub-plain">${inline(item.plain)}</span><span class="hub-dev">${inline(item.dev)}</span></span><span class="row-meta">${first ? `#${first}` : ""}</span></a></li>`;
    })
    .join("");
}
