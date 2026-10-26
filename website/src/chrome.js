// What every page with the site's top bar shares: the light and dark switch and the phone menu.
// Used by Home, Releases, Benchmarks and Docs.
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

// The page follows the system; the toggle picks the other appearance and remembers it.
export function setUpTheme() {
  const root = document.documentElement;
  const button = document.querySelector(".theme-toggle");
  if (!button) return;
  const label = button.querySelector("[data-theme-label]");
  const system = matchMedia("(prefers-color-scheme: dark)");
  const dark = () => (root.dataset.theme ? root.dataset.theme === "dark" : system.matches);
  const show = () => (label.textContent = dark() ? "Light" : "Dark");
  button.addEventListener("click", () => {
    root.dataset.theme = dark() ? "light" : "dark";
    try {
      localStorage.setItem("theme", root.dataset.theme);
    } catch {}
    show();
  });
  system.addEventListener("change", show);
  show();
}

// The phone menu: a full-screen sheet; the page behind it is inert while it is open.
export function setUpMenu() {
  const bar = document.querySelector(".nav");
  const button = document.querySelector(".menu-button");
  const menu = document.getElementById("menu");
  const content = document.querySelector("[data-menu-content]") || document.getElementById("main");
  if (!bar || !button || !menu) return;
  const label = button.querySelector(".menu-label");
  const items = [...menu.querySelectorAll("nav a")];
  items.forEach((a, i) => a.style.setProperty("transition-delay", `${60 + i * 35}ms`));
  let closing;
  const setOpen = (open) => {
    clearTimeout(closing);
    button.setAttribute("aria-expanded", String(open));
    label.textContent = open ? "Close" : "Menu";
    bar.classList.toggle("is-menu", open);
    content.inert = open;
    document.body.style.overflow = open ? "hidden" : "";
    if (open) {
      menu.hidden = false;
      requestAnimationFrame(() => requestAnimationFrame(() => menu.classList.add("is-open")));
      items[0].focus({ preventScroll: true });
    } else {
      menu.classList.remove("is-open");
      closing = setTimeout(() => (menu.hidden = true), reducedMotion ? 0 : 320);
    }
  };
  button.addEventListener("click", () => setOpen(button.getAttribute("aria-expanded") !== "true"));
  items.forEach((a) => a.addEventListener("click", () => setOpen(false)));
  addEventListener("keydown", (e) => {
    if (e.key === "Escape" && button.getAttribute("aria-expanded") === "true") {
      setOpen(false);
      button.focus();
    }
  });
  matchMedia("(min-width: 1280px)").addEventListener("change", (e) => e.matches && setOpen(false));
}

// GitHub answers, cached for ten minutes in the tab.
export async function cached(key, load) {
  try {
    const hit = JSON.parse(sessionStorage.getItem(key) || "null");
    if (hit && Date.now() - hit.at < 10 * 60 * 1000) return hit.value;
  } catch {}
  const value = await load();
  try {
    sessionStorage.setItem(key, JSON.stringify({ at: Date.now(), value }));
  } catch {}
  return value;
}

export async function get(url, as = "json") {
  const res = await fetch(url, { headers: as === "json" ? { Accept: "application/vnd.github+json" } : {} });
  if (!res.ok) throw new Error(`${res.status}`);
  return as === "json" ? res.json() : res.text();
}

export const slim = (r) => ({
  tag_name: r.tag_name,
  name: r.name,
  published_at: r.published_at,
  html_url: r.html_url,
  prerelease: r.prerelease,
  body: r.body || "",
  assets: (r.assets || []).map((a) => ({ name: a.name, browser_download_url: a.browser_download_url, size: a.size })),
});
