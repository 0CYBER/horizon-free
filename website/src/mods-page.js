// /mods and /mods/<id>: the registry's list (kind filter, search, order) and a project's page. The pages are baked
// from the index at build time; this fetches the published index again so a project merged since shows at once,
// the ratings (crates/horizon-mod-registry), and on a project's page the signed-in player's own rating to change.
import { API, INDEX_URL, KINDS, RATINGS, SORTS, listHtml, matches, projectHtml, rateHtml, ratingOf, readIndex, sorted, starsHtml } from "./mods.js";
import { cached, setUpMenu, setUpTheme } from "./chrome.js";
import { escape } from "./markdown.js";

const root = document.getElementById("mods");
const baked = readIndex(JSON.parse(document.getElementById("mods-index").textContent || "null")) || [];
let projects = baked;
// Every project's ratings, once the service answered.
let ratings = null;
let kind = "all";
let text = "";
let order = "downloads";
// On a project's page: the signed-in account ({login, csrf}) or null, and its own rating of the project.
let me;
let mine = null;

const id = (location.pathname.match(/^\/mods\/([a-z0-9_]+)\/?$/) || [])[1];

function showList() {
  const list = root.querySelector("#mod-list");
  if (!list) return;
  const shown = sorted(projects.filter((p) => matches(p, kind, text)), order, ratings);
  list.innerHTML = listHtml(shown, ratings);
  root.querySelector("[data-mod-count]").textContent = `${shown.length} of ${projects.length}`;
  root.querySelectorAll("[data-kind-filter]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.kindFilter === kind)));
}

function showRate(p, message = "") {
  const rate = root.querySelector("#mod-rate");
  if (!rate || me === undefined) return;
  rate.innerHTML = rateHtml(p, me, mine) + (message ? `<p class="mod-error">${escape(message)}</p>` : "");
}

function showProject() {
  const p = projects.find((x) => x.id === id);
  const page = root.querySelector(".mod-page, .mod-missing");
  if (!page) return;
  if (p) {
    page.outerHTML = projectHtml(p, ratings ? ratingOf(ratings, p.id) : undefined);
    document.title = `${p.name} · Horizon`;
    showRate(p);
  } else {
    page.outerHTML = `<div class="mod-missing"><h1 class="display page-title">Not in the registry</h1><p class="lede">No project is called ${id} here. It may not be published yet, or it was taken down.</p><a class="button secondary" href="/mods">Every mod</a></div>`;
  }
}

function render() {
  if (id) showProject();
  else showList();
}

// The signed-in account and its own rating, for the page's rating control.
async function signIn() {
  try {
    const r = await fetch(`${API}/me`, { credentials: "same-origin" });
    me = r.ok ? await r.json() : null;
    if (me) {
      const own = await fetch(`${API}/${id}/rating`, { credentials: "same-origin" });
      mine = own.ok ? (await own.json()).stars : null;
    }
  } catch {
    me = null;
  }
  const p = projects.find((x) => x.id === id);
  if (p) showRate(p);
}

async function rate(stars) {
  const p = projects.find((x) => x.id === id);
  if (!p || !me) return;
  root.querySelectorAll(".mod-star").forEach((b) => (b.disabled = true));
  let message = "";
  try {
    const r = await fetch(`${API}/${id}/rating`, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json", "X-CSRF-Token": me.csrf },
      body: JSON.stringify({ stars }),
    });
    const json = await r.json().catch(() => ({}));
    if (r.ok) {
      mine = json.stars;
      ratings = { ...ratings, [id]: { average: json.average, count: json.count } };
      const line = root.querySelector("[data-mod-rating]");
      if (line) line.innerHTML = `${starsHtml(ratingOf(ratings, id))} · `;
    } else {
      message = json.error || `The service answered ${r.status}.`;
    }
  } catch {
    message = "The service didn't answer. Try again in a minute.";
  }
  showRate(p, message);
}

if (id) {
  root.addEventListener("click", (e) => {
    const star = e.target.closest?.("[data-stars]");
    if (star) rate(Number(star.dataset.stars));
  });
  signIn();
} else {
  root.querySelectorAll("[data-kind-filter]").forEach((b) =>
    b.addEventListener("click", () => {
      kind = b.dataset.kindFilter;
      showList();
    }),
  );
  root.querySelector("#mod-search")?.addEventListener("input", (e) => {
    text = e.target.value;
    showList();
  });
  root.querySelector("#mod-sort")?.addEventListener("change", (e) => {
    order = e.target.value;
    showList();
  });
  const query = new URLSearchParams(location.search);
  if (KINDS.some(([k]) => k === query.get("kind"))) kind = query.get("kind");
  if (SORTS.some(([k]) => k === query.get("sort"))) order = query.get("sort");
  const select = root.querySelector("#mod-sort");
  if (select) select.value = order;
}

setUpTheme();
setUpMenu();
render();
cached("horizon:mod-index", () => fetch(INDEX_URL).then((r) => (r.ok ? r.json() : Promise.reject(new Error(`${r.status}`)))))
  .then((index) => {
    const fresh = readIndex(index);
    if (fresh) {
      projects = fresh;
      render();
    }
  })
  .catch(() => {});
fetch(RATINGS)
  .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`${r.status}`))))
  .then((json) => {
    if (json && typeof json.projects === "object") {
      ratings = json.projects;
      render();
    }
  })
  .catch(() => {});
