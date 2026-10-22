// The page's behaviour: the download buttons for this platform, the latest releases, the
// links and contributors, the navigation, the menus' tabs, the lightbox and the motion.
// Everything degrades to the baked-in content when GitHub can't be reached.
import { escape } from "./markdown.js";
import { releasesHtml } from "./releases.js";
import { contributorsFrom, contributorsHtml } from "./contributors.js";
import { isGame, isLauncher as isLauncherTag, launcherVersion, newestFirst } from "./versions.js";

const REPO = "ludovic111/horizon";
const RELEASES_PAGE = `https://github.com/${REPO}/releases?q=launcher&expanded=true`;
const baked = JSON.parse(document.getElementById("baked").textContent);
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;

// --- GitHub --------------------------------------------------------------------------

async function cached(key, load) {
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

async function get(url, as = "json") {
  const res = await fetch(url, { headers: as === "json" ? { Accept: "application/vnd.github+json" } : {} });
  if (!res.ok) throw new Error(`${res.status}`);
  return as === "json" ? res.json() : res.text();
}

const slim = (r) => ({
  tag_name: r.tag_name,
  name: r.name,
  published_at: r.published_at,
  html_url: r.html_url,
  prerelease: r.prerelease,
  body: r.body || "",
  assets: r.assets.map((a) => ({ name: a.name, browser_download_url: a.browser_download_url, size: a.size })),
});
const isLauncher = (r) => isLauncherTag(r.tag_name) && !r.prerelease;

// The newest 100 releases (GitHub's most per page). Game betas come out several times a day, so the stable game
// release and the newest launcher release can both be older than all of them. The stable game release is then
// GitHub's "latest" (betas and launcher releases are never marked latest); the launcher release is looked up by its
// tag (the newest launcher@ tag with a published release, or launcher-v before 0.2: docs/versioning.md).
async function loadReleases() {
  const list = (await get(`https://api.github.com/repos/${REPO}/releases?per_page=100`)).filter((r) => !r.draft).map(slim);
  if (!list.some((r) => isGame(r.tag_name) && !r.prerelease)) {
    const latest = await get(`https://api.github.com/repos/${REPO}/releases/latest`).catch(() => null);
    if (latest && !latest.draft) list.push(slim(latest));
  }
  if (list.some(isLauncher)) return list;
  let tags = [];
  for (const prefix of ["launcher@", "launcher-v"]) {
    if (tags.length) break;
    const refs = await get(`https://api.github.com/repos/${REPO}/git/matching-refs/tags/${prefix}`).catch(() => []);
    tags = refs.map((r) => r.ref.replace("refs/tags/", "")).filter(isLauncherTag).sort(newestFirst);
  }
  for (const tag of tags.slice(0, 3)) {
    const release = await get(`https://api.github.com/repos/${REPO}/releases/tags/${tag}`).catch(() => null);
    if (release && !release.draft) return [...list, slim(release)];
  }
  return list;
}

const releases = cached("horizon:releases", loadReleases).catch(() => baked.releases);

async function setUpContributors() {
  let list = baked.contributors || [];
  try {
    list = contributorsFrom(await cached("horizon:contributors", () => get(`https://api.github.com/repos/${REPO}/contributors?per_page=100`)));
  } catch {}
  if (list.length) document.getElementById("contributors-list").innerHTML = contributorsHtml(list);
  // With nobody to show, the block and its menu entries go, as an empty link's card does.
  document.querySelectorAll("[data-contributors]").forEach((el) => (el.hidden = !list.length));
}

// --- Download ------------------------------------------------------------------------

// Apple Silicon first: Safari and Firefox say "Intel Mac OS X" on every Mac.
const MACS = [
  { key: "macos-arm64", label: "macOS on Apple Silicon", other: "Apple Silicon" },
  { key: "macos-x64", label: "macOS on an Intel Mac", other: "Intel Mac" },
];

// Only Chromium's client hints tell an Intel Mac apart.
async function intelMac() {
  try {
    return (await navigator.userAgentData.getHighEntropyValues(["architecture"])).architecture === "x86";
  } catch {
    return false;
  }
}

function platform() {
  const ua = navigator.userAgent;
  const hint = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  if (/iPhone|iPad|iPod|Android/i.test(ua) || (navigator.maxTouchPoints > 1 && /Mac/.test(hint))) return null;
  if (/Win/i.test(hint) || /Windows/.test(ua)) return { key: "windows-x64", label: "Windows x64" };
  if (/Mac/i.test(hint) || /Mac OS X/.test(ua)) return MACS[0];
  if (/Linux|X11/i.test(hint) || /Linux/.test(ua)) return { key: "linux-x86_64", label: "Linux x86-64" };
  return null;
}

async function setUpDownload() {
  let os = platform();
  if (os === MACS[0] && (await intelMac())) os = MACS[1];
  const list = await releases;
  const launcher = list.filter(isLauncher).sort((a, b) => newestFirst(a.tag_name, b.tag_name))[0];
  if (!launcher) return;
  const version = launcherVersion(launcher.tag_name);
  const find = (key) => launcher.assets.find((a) => a.name.includes(key));
  const asset = os && find(os.key);
  // On a Mac, the other processor's build too: the browser may have guessed wrong.
  const mac = MACS.includes(os) && MACS.find((m) => m !== os);
  const otherMac = mac && find(mac.key);
  const others = `${otherMac ? `<a href="${escape(otherMac.browser_download_url)}">${escape(mac.other)}</a> · ` : ""}<a href="${escape(launcher.html_url)}">Other platforms</a>`;
  document.querySelectorAll("[data-download]").forEach((a) => (a.href = asset ? asset.browser_download_url : launcher.html_url || RELEASES_PAGE));
  const mb = asset && asset.size ? ` · ${(asset.size / 1e6).toFixed(0)} MB` : "";
  const meta = asset
    ? `For ${escape(os.label)} · launcher ${escape(version)}${mb} · ${others}`
    : `Launcher ${escape(version)} for Linux x86-64, macOS on Apple Silicon and Windows x64. It runs on a computer, not on phones or tablets.`;
  document.querySelectorAll("[data-download-meta]").forEach((p) => (p.innerHTML = meta));
}

// --- Patch notes and the links from main ---------------------------------

async function setUpNotes() {
  const list = (await releases).filter((r) => isGame(r.tag_name) && !r.prerelease).sort((a, b) => newestFirst(a.tag_name, b.tag_name));
  if (list.length) document.getElementById("releases").innerHTML = releasesHtml(list.slice(0, 3));
}

async function setUpLinks() {
  let links = baked.links;
  try {
    const remote = await cached("horizon:links", () => get(`https://raw.githubusercontent.com/${REPO}/main/launcher/links.json`, "text").then(JSON.parse));
    links = { ...links };
    for (const key of Object.keys(links)) if (typeof remote[key] === "string") links[key] = remote[key].trim();
  } catch {}
  document.querySelectorAll("[data-link]").forEach((a) => {
    const url = links[a.dataset.link];
    // An empty address hides its card, as the launcher hides its button.
    if (!url) a.hidden = true;
    else if (/^https:\/\//.test(url)) a.href = url;
  });
}

// --- Navigation ----------------------------------------------------------------------

function setUpNav() {
  const bar = document.querySelector(".nav");
  // The sections on this page (the nav also links to other pages, such as /server).
  const links = [...document.querySelectorAll('.nav-links a[href^="#"]')];
  const sections = links.map((a) => document.querySelector(a.getAttribute("href")));

  let current = null;
  const update = () => {
    bar.classList.toggle("is-scrolled", scrollY > 24);
    const line = innerHeight * 0.4;
    let index = -1;
    sections.forEach((s, i) => {
      if (s.getClientRects().length && s.getBoundingClientRect().top <= line) index = i;
    });
    if (index !== current) {
      current = index;
      links.forEach((a, i) => (i === index ? a.setAttribute("aria-current", "true") : a.removeAttribute("aria-current")));
    }
  };
  onScroll(update);
  update();

  // The phone menu: a full-screen sheet; the page behind it is inert while it is open.
  const button = document.querySelector(".menu-button");
  const menu = document.getElementById("menu");
  const main = document.getElementById("main");
  const label = button.querySelector(".menu-label");
  const items = [...menu.querySelectorAll("nav a")];
  items.forEach((a, i) => a.style.setProperty("transition-delay", `${60 + i * 35}ms`));
  let closing;
  const setOpen = (open) => {
    clearTimeout(closing);
    button.setAttribute("aria-expanded", String(open));
    label.textContent = open ? "Close" : "Menu";
    bar.classList.toggle("is-menu", open);
    main.inert = open;
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
  matchMedia("(min-width: 1120px)").addEventListener("change", (e) => e.matches && setOpen(false));
}

// One rAF-throttled scroll listener for everything that follows the scroll.
const scrollers = [];
function onScroll(fn) {
  scrollers.push(fn);
}
let queued = false;
const run = () => ((queued = false), scrollers.forEach((fn) => fn()));
addEventListener("scroll", () => !queued && ((queued = true), requestAnimationFrame(run)), { passive: true });
addEventListener("resize", () => requestAnimationFrame(run));

// --- The menus' tabs ------------------------------------------------------------------

function setUpTabs() {
  const list = document.querySelector(".tabs");
  const tabs = [...list.querySelectorAll('[role="tab"]')];
  const pill = list.querySelector(".tab-pill");
  const selected = () => tabs.find((t) => t.getAttribute("aria-selected") === "true");
  const place = (tab) => {
    pill.style.width = `${tab.offsetWidth}px`;
    pill.style.height = `${tab.offsetHeight}px`;
    pill.style.transform = `translate(${tab.offsetLeft}px, ${tab.offsetTop}px)`;
  };
  const choose = (tab, focus) => {
    tabs.forEach((t) => {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      const panel = document.getElementById(t.getAttribute("aria-controls"));
      panel.classList.toggle("is-active", on);
      panel.inert = !on;
      panel.setAttribute("aria-hidden", String(!on));
    });
    place(tab);
    if (focus) tab.focus();
  };
  tabs.forEach((tab, i) => {
    tab.addEventListener("click", () => choose(tab));
    tab.addEventListener("keydown", (e) => {
      const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (step) choose(tabs[(i + step + tabs.length) % tabs.length], true);
      if (e.key === "Home") choose(tabs[0], true);
      if (e.key === "End") choose(tabs[tabs.length - 1], true);
    });
  });
  choose(selected());
  addEventListener("resize", () => place(selected()));
  document.fonts?.ready.then(() => place(selected()));
}

function setUpLightbox() {
  const dialog = document.querySelector(".lightbox");
  const big = dialog.querySelector("img");
  const caption = dialog.querySelector(".lightbox-caption");
  document.querySelectorAll(".zoom").forEach((button) => {
    button.addEventListener("click", () => {
      const img = button.querySelector("img");
      big.src = img.currentSrc.replace("-800.webp", ".webp");
      big.alt = img.alt;
      caption.textContent = img.alt;
      dialog.showModal();
    });
  });
  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) dialog.close();
  });
}

// --- Light and dark ------------------------------------------------------------------

// The page follows the system; the toggle picks the other appearance and remembers it.
function setUpTheme() {
  const root = document.documentElement;
  const button = document.querySelector(".theme-toggle");
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

// --- Motion ---------------------------------------------------------------------------

function setUpReveal() {
  // Siblings that enter together come in one after another.
  const stagger = (els, step) => els.forEach((el, i) => el.style.setProperty("--d", `${i * step}ms`));
  document.querySelectorAll(".facts, .features, .cards, .mosaic, .hero-index").forEach((group) => stagger([...group.children], 40));
  document.querySelectorAll(".hero .container, .section-head, .parity-count, .parity-copy, .launcher-copy").forEach((group) => stagger([...group.querySelectorAll(":scope > .reveal")], 60));
  stagger([...document.querySelectorAll(".system")], 40);

  const count = document.querySelector("[data-digits]");
  count.innerHTML = [...count.textContent].map((c) => `<span>${escape(c)}</span>`).join("");

  const items = document.querySelectorAll(".reveal, .reveal-media, .systems");
  if (reducedMotion || !("IntersectionObserver" in window)) {
    items.forEach((el) => el.classList.add("is-visible"));
    return;
  }
  const observer = new IntersectionObserver(
    (entries) =>
      entries.forEach((e) => {
        if (e.isIntersecting) {
          e.target.classList.add("is-visible");
          observer.unobserve(e.target);
        }
      }),
    { rootMargin: "0px 0px -10% 0px" },
  );
  items.forEach((el) => observer.observe(el));
}

setUpReveal();
setUpTheme();
setUpNav();
setUpTabs();
setUpLightbox();
setUpDownload();
setUpNotes();
setUpLinks();
setUpContributors();
