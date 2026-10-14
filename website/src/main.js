// The page's behaviour: the download buttons for this platform, the latest releases, the
// links and contributors, the navigation, the menus' tabs, the lightbox and the motion.
// Everything degrades to the baked-in content when GitHub can't be reached.
import { escape } from "./markdown.js";
import { releasesHtml } from "./releases.js";
import { contributorsFrom, contributorsHtml } from "./contributors.js";

const REPO = "ludovic111/horizon";
const RELEASES_PAGE = `https://github.com/${REPO}/releases?q=launcher-v&expanded=true`;
const baked = JSON.parse(document.getElementById("baked").textContent);
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)").matches;
const finePointer = matchMedia("(hover: hover) and (pointer: fine)").matches;

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
const isLauncher = (r) => r.tag_name.startsWith("launcher-v") && !r.prerelease;
const versionKey = (tag) => tag.replace("launcher-v", "").split(".").map(Number);
const newer = (a, b) => versionKey(b).reduce((d, n, i) => d || n - (versionKey(a)[i] || 0), 0);

// The newest 30 releases. Game releases come out several times a day, so the newest launcher release can be
// older than all of them: then it is looked up by its tag (the newest launcher-v tag with a published release).
async function loadReleases() {
  const list = (await get(`https://api.github.com/repos/${REPO}/releases?per_page=30`)).filter((r) => !r.draft).map(slim);
  if (list.some(isLauncher)) return list;
  const refs = await get(`https://api.github.com/repos/${REPO}/git/matching-refs/tags/launcher-v`).catch(() => []);
  const tags = refs.map((r) => r.ref.replace("refs/tags/", "")).filter((t) => /^launcher-v\d+\.\d+\.\d+$/.test(t)).sort(newer);
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

function platform() {
  const ua = navigator.userAgent;
  const hint = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  if (/iPhone|iPad|iPod|Android/i.test(ua) || (navigator.maxTouchPoints > 1 && /Mac/.test(hint))) return null;
  if (/Win/i.test(hint) || /Windows/.test(ua)) return { key: "windows-x64", label: "Windows x64" };
  if (/Mac/i.test(hint) || /Mac OS X/.test(ua)) return { key: "macos-arm64", label: "macOS on Apple Silicon" };
  if (/Linux|X11/i.test(hint) || /Linux/.test(ua)) return { key: "linux-x86_64", label: "Linux x86-64" };
  return null;
}

async function setUpDownload() {
  const os = platform();
  const list = await releases;
  const launcher = list.find((r) => r.tag_name.startsWith("launcher-v") && !r.prerelease);
  if (!launcher) return;
  const version = launcher.tag_name.replace("launcher-v", "");
  const asset = os && launcher.assets.find((a) => a.name.includes(os.key));
  const others = `<a href="${escape(launcher.html_url)}">Other platforms</a>`;
  document.querySelectorAll("[data-download]").forEach((a) => (a.href = asset ? asset.browser_download_url : launcher.html_url || RELEASES_PAGE));
  const mb = asset && asset.size ? ` · ${(asset.size / 1e6).toFixed(0)} MB` : "";
  const meta = asset
    ? `For ${escape(os.label)} · launcher ${escape(version)}${mb} · ${others}`
    : `Launcher ${escape(version)} for Linux x86-64, macOS on Apple Silicon and Windows x64. It runs on a computer, not on phones or tablets.`;
  document.querySelectorAll("[data-download-meta]").forEach((p) => (p.innerHTML = meta));
}

// --- Patch notes and the links from main ---------------------------------

const isGame = (tag) => /^v\d{4}\.\d{1,2}\.\d{1,2}/.test(tag);

async function setUpNotes() {
  const list = (await releases).filter((r) => isGame(r.tag_name));
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
  const links = [...document.querySelectorAll(".nav-links a")];
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

// --- Motion ---------------------------------------------------------------------------

function setUpReveal() {
  // Siblings that enter together come in one after another.
  const stagger = (els, step) => els.forEach((el, i) => el.style.setProperty("--d", `${i * step}ms`));
  document.querySelectorAll(".facts, .features, .cards, .mosaic").forEach((group) => stagger([...group.children], 90));
  document.querySelectorAll(".section-head, .parity-count, .parity-copy, .launcher-copy").forEach((group) => stagger([...group.querySelectorAll(":scope > .reveal")], 70));
  stagger([...document.querySelectorAll(".system")], 45);

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

// The statement lights up in reading order as it scrolls through the view.
function setUpStatement() {
  const fill = document.querySelector(".fill");
  if (reducedMotion) return;
  const block = fill.parentElement;
  const update = () => {
    const r = block.getBoundingClientRect();
    if (r.bottom < -200 || r.top > innerHeight + 200) return;
    const p = Math.min(1, Math.max(0, (innerHeight * 0.85 - r.top) / (innerHeight * 0.4 + r.height)));
    fill.style.setProperty("--p", (p * 104).toFixed(1));
  };
  onScroll(update);
  update();
}

// The hero's landscape moves slower than the page, and its text drifts up as it leaves.
function setUpParallax() {
  if (reducedMotion) return;
  const hero = document.querySelector(".hero");
  const layer = hero.querySelector(".hero-parallax");
  const content = hero.querySelector(".hero-content");
  onScroll(() => {
    const y = scrollY;
    if (y > hero.offsetHeight) return;
    layer.style.transform = `translate3d(0, ${(y * 0.3).toFixed(1)}px, 0)`;
    content.style.transform = `translate3d(0, ${(y * -0.08).toFixed(1)}px, 0)`;
    content.style.opacity = String(Math.max(0, 1 - y / (hero.offsetHeight * 0.7)).toFixed(3));
  });
}

// Buttons lean a little towards the pointer.
function setUpMagnetic() {
  if (reducedMotion || !finePointer) return;
  document.querySelectorAll(".magnetic").forEach((el) => {
    el.addEventListener("pointermove", (e) => {
      const r = el.getBoundingClientRect();
      const x = (e.clientX - r.left - r.width / 2) * 0.18;
      const y = (e.clientY - r.top - r.height / 2) * 0.3;
      el.style.transform = `translate3d(${x.toFixed(1)}px, ${y.toFixed(1)}px, 0)`;
    });
    el.addEventListener("pointerleave", () => (el.style.transform = ""));
  });
}

setUpReveal();
setUpStatement();
setUpParallax();
setUpMagnetic();
setUpNav();
setUpTabs();
setUpLightbox();
setUpDownload();
setUpNotes();
setUpLinks();
setUpContributors();
