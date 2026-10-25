// /mods and /mods/<id>: the registry's list (kind filter, search) and a project's page. The pages are baked from
// the index at build time; this fetches the published index again so a project merged since shows at once.
import { INDEX_URL, KINDS, listHtml, matches, projectHtml, readIndex } from "./mods.js";
import { cached, setUpMenu, setUpTheme } from "./chrome.js";

const root = document.getElementById("mods");
const baked = readIndex(JSON.parse(document.getElementById("mods-index").textContent || "null")) || [];
let projects = baked;
let kind = "all";
let text = "";

const id = (location.pathname.match(/^\/mods\/([a-z0-9_]+)\/?$/) || [])[1];

function showList() {
  const list = root.querySelector("#mod-list");
  if (!list) return;
  const shown = projects.filter((p) => matches(p, kind, text));
  list.innerHTML = listHtml(shown);
  root.querySelector("[data-mod-count]").textContent = `${shown.length} of ${projects.length}`;
  root.querySelectorAll("[data-kind-filter]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.kindFilter === kind)));
}

function showProject() {
  const p = projects.find((x) => x.id === id);
  const page = root.querySelector(".mod-page, .mod-missing");
  if (!page) return;
  if (p) {
    page.outerHTML = projectHtml(p);
    document.title = `${p.name} · Horizon`;
  } else {
    page.outerHTML = `<div class="mod-missing"><h1 class="display page-title">Not in the registry</h1><p class="lede">No project is called ${id} here. It may not be published yet, or it was taken down.</p><a class="button secondary" href="/mods">Every mod</a></div>`;
  }
}

function render() {
  if (id) showProject();
  else showList();
}

if (!id) {
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
  const wanted = new URLSearchParams(location.search).get("kind");
  if (KINDS.some(([k]) => k === wanted)) kind = wanted;
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
