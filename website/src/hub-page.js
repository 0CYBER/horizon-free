// The hub page (/hub): its tabs (one per #hash, so /hub#roadmap and /hub#<item id> can be linked), the
// Simple / Technical switch (remembered in localStorage), the area filter, and the live state of every
// linked issue and pull request, refreshed from GitHub's public API. Without JavaScript, every tab reads
// one after the other with what was baked at build time.
import { cached, get, setUpMenu, setUpTheme } from "./chrome.js";
import { REPO, boardFilterHtml, boardFrom, boardHtml, journalFrom, journalHtml, overviewHtml, roadmapHtml, slimIssue } from "./hub.js";

const root = document.documentElement;
const data = JSON.parse(document.getElementById("hub-data").textContent);
const tabs = [...document.querySelectorAll('.hub-tabs [role="tab"]')];
const pill = document.querySelector(".hub-tabs .tab-pill");
let area = "all";

// --- Tabs ------------------------------------------------------------------------------------------------------

const panelOf = (tab) => document.getElementById(tab.getAttribute("aria-controls"));
const place = (tab) => {
  pill.style.width = `${tab.offsetWidth}px`;
  pill.style.height = `${tab.offsetHeight}px`;
  pill.style.transform = `translate(${tab.offsetLeft}px, ${tab.offsetTop}px)`;
};
function choose(tab, { focus = false, push = false } = {}) {
  for (const t of tabs) {
    const on = t === tab;
    t.setAttribute("aria-selected", String(on));
    t.tabIndex = on ? 0 : -1;
    panelOf(t).classList.toggle("is-active", on);
  }
  place(tab);
  if (focus) tab.focus();
  if (push && location.hash !== `#${panelOf(tab).id}`) history.replaceState(null, "", `#${panelOf(tab).id}`);
}
// "#roadmap" opens its tab; "#<item id>" or "#help-wanted" opens the tab holding it and scrolls there.
function follow() {
  const target = location.hash && document.getElementById(decodeURIComponent(location.hash.slice(1)));
  const tab = tabs.find((t) => target && (panelOf(t) === target || panelOf(t).contains(target))) || tabs[0];
  choose(tab);
  if (target && target !== panelOf(tab)) {
    if (target.dataset.area && area !== "all" && target.dataset.area !== area) filter("all");
    requestAnimationFrame(() => target.scrollIntoView({ block: "start" }));
  }
}
tabs.forEach((tab, i) => {
  tab.addEventListener("click", () => choose(tab, { push: true }));
  tab.addEventListener("keydown", (e) => {
    const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
    if (step) choose(tabs[(i + step + tabs.length) % tabs.length], { focus: true, push: true });
  });
});
// Links inside the page ("Help wanted", an item of the overview) switch tabs through the hash.
addEventListener("hashchange", follow);
addEventListener("resize", () => place(tabs.find((t) => t.getAttribute("aria-selected") === "true")));
document.fonts?.ready.then(() => place(tabs.find((t) => t.getAttribute("aria-selected") === "true")));

// --- Simple / Technical ----------------------------------------------------------------------------------------

const detailButtons = [...document.querySelectorAll("[data-detail]")];
function detail(mode) {
  if (mode === "dev") root.dataset.detail = "dev";
  else delete root.dataset.detail;
  detailButtons.forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.detail === mode)));
}
detailButtons.forEach((b) =>
  b.addEventListener("click", () => {
    detail(b.dataset.detail);
    try {
      localStorage.setItem("hub:detail", b.dataset.detail);
    } catch {}
  }),
);
detail(root.dataset.detail === "dev" ? "dev" : "plain");

// --- Area filter -----------------------------------------------------------------------------------------------

const areaButtons = [...document.querySelectorAll("[data-area-filter]")];
function filter(to) {
  area = to;
  areaButtons.forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.areaFilter === area)));
  for (const section of document.querySelectorAll("[data-roadmap] .hub-section")) {
    let shown = 0;
    for (const item of section.querySelectorAll(".hub-item")) {
      item.hidden = area !== "all" && item.dataset.area !== area;
      if (!item.hidden) shown++;
    }
    section.hidden = shown === 0;
  }
}
areaButtons.forEach((b) => b.addEventListener("click", () => filter(b.dataset.areaFilter)));

// --- The board: a roadmap item's cards, and the long columns' "Show more" ----------------------------------------

const boardSelect = document.querySelector("[data-board-item]");
function showItem() {
  const want = boardSelect.value;
  for (const card of document.querySelectorAll("[data-board] .hub-card")) {
    card.hidden = want === "-" ? !card.dataset.item : want ? card.dataset.item !== want : false;
  }
  for (const column of document.querySelectorAll("[data-board] .hub-column")) {
    column.querySelector("[data-count]").textContent = column.querySelectorAll(".hub-card:not([hidden])").length;
    // A filtered column shows all its cards.
    column.classList.toggle("is-open", Boolean(want) || column.classList.contains("was-opened"));
  }
}
boardSelect?.addEventListener("change", showItem);
document.querySelector("[data-board]")?.addEventListener("click", (e) => {
  const button = e.target.closest("[data-more-button]");
  if (!button) return;
  button.closest(".hub-column").classList.add("is-open", "was-opened");
  button.remove();
});

// --- Live state from GitHub ------------------------------------------------------------------------------------

// The open issues and pull requests, and those closed or merged lately: four requests at most, kept ten minutes.
async function loadLive() {
  const list = [];
  for (let page = 1; page <= 3; page++) {
    const batch = await get(`https://api.github.com/repos/${REPO}/issues?state=open&per_page=100&page=${page}`);
    list.push(...batch.map(slimIssue));
    if (batch.length < 100) break;
  }
  const closed = await get(`https://api.github.com/repos/${REPO}/issues?state=closed&sort=updated&direction=desc&per_page=100`);
  return list.concat(closed.map(slimIssue));
}
const loadReleases = () => get(`https://api.github.com/repos/${REPO}/releases?per_page=40`);

async function refresh() {
  const fresh = await cached("horizon:hub-live", loadLive).catch(() => null);
  if (!fresh) return;
  // What the build knew about older links stays; GitHub's answer wins for the rest.
  const live = new Map([...data.live, ...fresh].map((i) => [i.n, i]));
  document.querySelector("[data-overview]").innerHTML = overviewHtml(data.roadmap, live);
  document.querySelector("[data-roadmap]").innerHTML = roadmapHtml(data.roadmap, live);
  filter(area);
  showVotes();
  const board = boardFrom(fresh, data.roadmap);
  const chosen = boardSelect.value;
  boardSelect.innerHTML = boardFilterHtml(board, data.roadmap);
  boardSelect.value = [...boardSelect.options].some((o) => o.value === chosen) ? chosen : "";
  document.querySelector("[data-board]").innerHTML = boardHtml(board, data.roadmap);
  showItem();
  const releases = await cached("horizon:hub-releases", loadReleases).catch(() => null);
  if (releases) document.querySelector("[data-journal]").innerHTML = journalHtml(journalFrom(releases, fresh, data.roadmap), data.roadmap);
  const note = document.querySelector("[data-live-note]");
  if (note) note.textContent = "Pull request and issue states are live from GitHub.";
}

// --- Votes, bug reports and suggestions: the hub's server (crates/horizon-telemetry-server) --------------------

const saved = (key, fallback) => {
  try {
    return JSON.parse(localStorage.getItem(key)) ?? fallback;
  } catch {
    return fallback;
  }
};
const save = (key, value) => {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {}
};
const post = (path, body) => fetch(`${data.api}${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });

// This browser's random id (one vote per item per browser) and the items it voted for. Nothing else identifies it.
function voter() {
  let id = saved("hub:voter", "");
  if (typeof id !== "string" || id.length < 16) {
    id = crypto.randomUUID ? crypto.randomUUID() : [...crypto.getRandomValues(new Uint8Array(16))].map((b) => b.toString(16).padStart(2, "0")).join("");
    save("hub:voter", id);
  }
  return id;
}
const mine = new Set(saved("hub:voted", []));
let counts = null; // { item: votes } once the server answered; until then the buttons stay hidden

function showVotes() {
  if (!counts) return;
  for (const button of document.querySelectorAll("[data-vote]")) {
    const id = button.dataset.vote;
    button.hidden = false;
    button.setAttribute("aria-pressed", String(mine.has(id)));
    button.querySelector(".hub-vote-label").textContent = mine.has(id) ? "Wanted" : "Want it";
    button.querySelector("[data-votes]").textContent = counts[id] ? String(counts[id]) : "";
  }
}

async function vote(id) {
  const up = !mine.has(id);
  const apply = (on, step) => {
    if (on) mine.add(id);
    else mine.delete(id);
    counts[id] = Math.max(0, (counts[id] || 0) + step);
    save("hub:voted", [...mine]);
    showVotes();
  };
  apply(up, up ? 1 : -1);
  try {
    const res = await post("/hub/vote", { item: id, voter: voter(), up });
    if (!res.ok) throw new Error(res.status);
    counts[id] = (await res.json()).votes;
    showVotes();
  } catch {
    apply(!up, up ? -1 : 1);
  }
}
document.addEventListener("click", (e) => {
  const button = e.target.closest("[data-vote]");
  if (button && counts) vote(button.dataset.vote);
});

const LABELS = { title: "the title", did: "what you did", happened: "what happened", expected: "what you expected", version: "the version", system: "OS, CPU, GPU", text: "your idea", name: "your name" };
// The server's answer in words: "did: 3 to 2000 characters" becomes "Check what you did: 3 to 2000 characters."
function trouble(code, answer) {
  if (code === 429) return "Too many sent from your connection: try again in a few minutes.";
  const said = typeof answer.error === "string" ? answer.error : "";
  const field = said.match(/^(\w+): (.+)$/);
  if (code === 400 && field) return `Check ${LABELS[field[1]] || field[1]}: ${field[2]}.`;
  return said ? `${said[0].toUpperCase()}${said.slice(1)}.` : "That didn't go through: try again in a moment.";
}

function setUpForm(form, path, thanks) {
  const status = form.querySelector("[data-status]");
  form.hidden = false;
  form.parentElement.querySelector("[data-closed]").hidden = true;
  form.addEventListener("submit", async (e) => {
    e.preventDefault();
    if (!form.reportValidity()) return;
    const button = form.querySelector('[type="submit"]');
    button.disabled = true;
    status.textContent = "Sending…";
    try {
      const res = await post(path, Object.fromEntries(new FormData(form)));
      const answer = await res.json().catch(() => ({}));
      if (res.ok) {
        thanks(status, answer);
        // A new report starts with the version filled in again; the name stays for the next one.
        for (const field of form.querySelectorAll("textarea, input:not([name='version']):not([name='name'])")) field.value = "";
      } else status.textContent = trouble(res.status, answer);
    } catch {
      status.textContent = "Couldn't reach the server: try again, or use GitHub or Discord.";
    }
    button.disabled = false;
  });
}

// The forms and the votes show once the server answers; without it the page points to GitHub and Discord.
async function connect() {
  try {
    // Revalidated: the server keeps the counts a minute, a browser's copy would miss this browser's own vote.
    const res = await fetch(`${data.api}/hub/votes`, { cache: "no-cache" });
    if (!res.ok) return;
    counts = (await res.json()).votes || {};
  } catch {
    return;
  }
  showVotes();
  setUpForm(document.querySelector('[data-form="report"]'), "/hub/report", (status, answer) => {
    const issue = typeof answer.issue === "string" && answer.issue.match(/^https:\/\/github\.com\/ludovic111\/horizon\/issues\/(\d+)$/);
    status.innerHTML = issue ? `Thanks! It's <a href="${issue[0]}">issue #${issue[1]}</a> on GitHub: follow it there.` : "Thanks! It will be on GitHub in a few minutes.";
  });
  setUpForm(document.querySelector('[data-form="suggest"]'), "/hub/suggest", (status) => {
    status.textContent = "Thanks! The maintainers read every suggestion.";
  });
}

setUpTheme();
setUpMenu();
connect();
follow();
refresh().then(() => data.roadmap.items.some((i) => `#${i.id}` === location.hash) && follow());
