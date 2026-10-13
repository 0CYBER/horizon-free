// The page's behaviour: the download button for this platform, the latest releases, the
// roadmap and links from main, the sidebar's selection, the screenshot tabs and lightbox.
// Everything degrades to the baked-in content when GitHub can't be reached.
import { markdown, escape, section } from "./markdown.js";

const REPO = "ludovic111/horizon";
const RELEASES_PAGE = `https://github.com/${REPO}/releases?q=launcher-v&expanded=true`;
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

const releases = cached("horizon:releases", () =>
  get(`https://api.github.com/repos/${REPO}/releases?per_page=30`).then((list) =>
    list
      .filter((r) => !r.draft)
      .map((r) => ({
        tag_name: r.tag_name,
        name: r.name,
        published_at: r.published_at,
        html_url: r.html_url,
        prerelease: r.prerelease,
        body: r.body || "",
        assets: r.assets.map((a) => ({ name: a.name, browser_download_url: a.browser_download_url, size: a.size })),
      })),
  ),
).catch(() => baked.releases);

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
  const button = document.getElementById("download");
  const meta = document.getElementById("download-meta");
  const os = platform();
  const list = await releases;
  const launcher = list.find((r) => r.tag_name.startsWith("launcher-v") && !r.prerelease);
  if (!launcher) return;
  const version = launcher.tag_name.replace("launcher-v", "");
  document.querySelectorAll("[data-launcher-version]").forEach((el) => (el.textContent = `Launcher ${version}`));
  const asset = os && launcher.assets.find((a) => a.name.includes(os.key));
  const others = `<a href="${escape(launcher.html_url)}">Other platforms</a>`;
  if (asset) {
    button.href = asset.browser_download_url;
    button.textContent = "Download the launcher";
    const mb = asset.size ? ` · ${(asset.size / 1e6).toFixed(0)} MB` : "";
    meta.innerHTML = `For ${escape(os.label)} · version ${escape(version)}${mb} · ${others}`;
  } else {
    button.href = launcher.html_url || RELEASES_PAGE;
    meta.innerHTML = `Launcher ${escape(version)} for Linux x86-64, macOS on Apple Silicon and Windows x64. It runs on a computer, not on phones or tablets.`;
  }
}

// --- Patch notes ---------------------------------------------------------------------

const isGame = (tag) => /^v\d{4}\.\d{1,2}\.\d{1,2}/.test(tag);

async function setUpNotes() {
  const list = (await releases).filter((r) => isGame(r.tag_name));
  if (!list.length) return;
  const box = document.getElementById("releases");
  const shown = list.slice(0, 3);
  box.innerHTML =
    shown
      .map((r, i) => {
        const date = (r.published_at || "").slice(0, 10);
        return `<details class="release"${i === 0 ? " open" : ""}>
          <summary><span class="release-title">${escape(r.name || r.tag_name)}</span><time datetime="${date}">${date}</time></summary>
          <div class="prose">${markdown(r.body)}<p><a href="${escape(r.html_url)}">Downloads and full notes on GitHub</a></p></div>
        </details>`;
      })
      .join("") +
    (list.length > shown.length
      ? `<p class="release-more"><a href="https://github.com/${REPO}/releases">Older releases on GitHub</a></p>`
      : "");
}

// --- The roadmap and the links from main ---------------------------------------------

async function setUpRoadmap() {
  try {
    const readme = await cached("horizon:readme", () => get(`https://raw.githubusercontent.com/${REPO}/main/README.md`, "text"));
    const roadmap = section(readme, "Roadmap");
    if (roadmap) document.getElementById("roadmap").innerHTML = markdown(roadmap, { shift: 1 });
  } catch {}
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
    // An empty address hides its row, as in the launcher.
    if (!url) a.closest(".row").hidden = true;
    else if (/^https:\/\//.test(url)) a.href = url;
  });
}

// --- The sidebar ---------------------------------------------------------------------

function setUpNav() {
  const nav = document.getElementById("nav");
  const sidebar = document.querySelector(".sidebar");
  const menu = document.querySelector(".menu-button");
  const selection = nav.querySelector(".nav-selection");
  const links = [...nav.querySelectorAll("a")];
  const sections = links.map((a) => document.querySelector(a.getAttribute("href")));

  const select = (index) => {
    links.forEach((a, i) => (i === index ? a.setAttribute("aria-current", "true") : a.removeAttribute("aria-current")));
    const a = links[index];
    selection.style.height = `${a.offsetHeight}px`;
    selection.style.transform = `translateY(${a.offsetTop}px)`;
    nav.classList.add("has-selection");
  };

  let current = -1;
  const update = () => {
    const line = innerHeight * 0.35;
    let index = 0;
    sections.forEach((s, i) => {
      if (s.getBoundingClientRect().top <= line) index = i;
    });
    if (innerHeight + scrollY >= document.documentElement.scrollHeight - 2) index = sections.length - 1;
    if (index !== current) select((current = index));
  };
  let queued = false;
  addEventListener("scroll", () => {
    if (!queued) requestAnimationFrame(() => ((queued = false), update()));
    queued = true;
  }, { passive: true });
  addEventListener("resize", () => select(Math.max(current, 0)));
  update();

  const setOpen = (open) => {
    sidebar.classList.toggle("is-open", open);
    menu.setAttribute("aria-expanded", String(open));
    menu.textContent = open ? "Close" : "Menu";
    if (open) select(Math.max(current, 0));
  };
  menu.addEventListener("click", () => setOpen(!sidebar.classList.contains("is-open")));
  links.forEach((a) => a.addEventListener("click", () => setOpen(false)));
  addEventListener("keydown", (e) => {
    if (e.key === "Escape" && sidebar.classList.contains("is-open")) {
      setOpen(false);
      menu.focus();
    }
  });
}

// --- Screenshots ---------------------------------------------------------------------

function setUpTabs() {
  const list = document.querySelector(".segmented");
  const tabs = [...list.querySelectorAll('[role="tab"]')];
  const selection = list.querySelector(".segment-selection");
  const place = (tab) => {
    selection.style.width = `${tab.offsetWidth}px`;
    selection.style.height = `${tab.offsetHeight}px`;
    selection.style.transform = `translate(${tab.offsetLeft}px, ${tab.offsetTop - 2}px)`;
  };
  const choose = (tab, focus) => {
    tabs.forEach((t) => {
      const on = t === tab;
      t.setAttribute("aria-selected", String(on));
      t.tabIndex = on ? 0 : -1;
      document.getElementById(t.getAttribute("aria-controls")).hidden = !on;
    });
    place(tab);
    if (focus) tab.focus();
  };
  tabs.forEach((tab, i) => {
    tab.addEventListener("click", () => choose(tab));
    tab.addEventListener("keydown", (e) => {
      const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (step) choose(tabs[(i + step + tabs.length) % tabs.length], true);
    });
  });
  place(tabs.find((t) => t.getAttribute("aria-selected") === "true"));
  addEventListener("resize", () => place(tabs.find((t) => t.getAttribute("aria-selected") === "true")));
  document.fonts?.ready.then(() => place(tabs.find((t) => t.getAttribute("aria-selected") === "true")));
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

function setUpReveal() {
  const items = document.querySelectorAll(".reveal");
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
    { rootMargin: "0px 0px -8% 0px" },
  );
  items.forEach((el) => observer.observe(el));
}

setUpReveal();
setUpNav();
setUpTabs();
setUpLightbox();
setUpDownload();
setUpNotes();
setUpRoadmap();
setUpLinks();
