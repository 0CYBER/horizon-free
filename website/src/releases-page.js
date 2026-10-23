// The Releases page: the list baked at build time, refreshed from GitHub when a newer release is out, and the
// channel filter. Without JavaScript the baked list reads as it is.
import { releasesPageHtml } from "./releases.js";
import { isGame, isLauncher, newestFirst } from "./versions.js";
import { cached, get, setUpMenu, setUpTheme, slim } from "./chrome.js";

const REPO = "ludovic111/horizon";
const list = document.getElementById("release-list");
const counter = document.querySelector("[data-release-count]");
const buttons = [...document.querySelectorAll("[data-filter]")];
let filter = "all";

const ours = (r) => !r.draft && (isGame(r.tag_name) || isLauncher(r.tag_name));

// Every release, a page of 100 at a time (GitHub's most), up to 500.
async function loadAll() {
  const all = [];
  for (let page = 1; page <= 5; page++) {
    const batch = await get(`https://api.github.com/repos/${REPO}/releases?per_page=100&page=${page}`);
    all.push(...batch.filter(ours).map(slim));
    if (batch.length < 100) break;
  }
  return all;
}

function apply() {
  const entries = [...list.querySelectorAll(".release-entry")];
  let shown = 0;
  for (const el of entries) {
    const on = filter === "all" || el.dataset.channel === filter;
    el.hidden = !on;
    if (on) shown++;
  }
  counter.textContent = `${shown} release${shown === 1 ? "" : "s"}`;
  buttons.forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.filter === filter)));
}

async function refresh() {
  // One small request first: the full list only when GitHub has something newer than the page.
  const newest = await cached("horizon:releases-newest", () => get(`https://api.github.com/repos/${REPO}/releases?per_page=5`)).catch(() => null);
  if (!newest) return;
  const top = newest.filter(ours).map((r) => r.tag_name).sort(newestFirst)[0];
  if (!top || top === list.dataset.newest) return;
  const all = await cached("horizon:releases-all", loadAll).catch(() => null);
  if (!all || !all.length) return;
  const open = new Set([...list.querySelectorAll(".release-entry[open]")].map((el) => el.id));
  list.innerHTML = releasesPageHtml(all);
  list.dataset.newest = top;
  // Keep what the reader opened open.
  open.forEach((id) => document.getElementById(id)?.setAttribute("open", ""));
  apply();
}

// A link to one release (/releases#release-v0.2.1) opens it.
function openTarget() {
  const el = location.hash && document.getElementById(location.hash.slice(1));
  if (el && el.classList.contains("release-entry")) {
    if (el.hidden) {
      filter = "all";
      apply();
    }
    el.open = true;
    el.scrollIntoView();
  }
}

buttons.forEach((b) =>
  b.addEventListener("click", () => {
    filter = b.dataset.filter;
    apply();
  }),
);
addEventListener("hashchange", openTarget);
setUpTheme();
setUpMenu();
apply();
openTarget();
refresh().then(openTarget);
