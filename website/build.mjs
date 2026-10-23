// Builds the site into dist/: src/index.html with the README's parity table and
// the community links and contributors baked in, the documentation (docs/*.md, one page each, listed in
// docs/nav.json), plus public/ and fingerprinted CSS and JS. No dependencies.
//
// Railway builds only website/, so the README and links.json come from the repo when they
// are next to it (a local checkout) and from main on GitHub otherwise. The documentation lives in website/. The latest releases and
// the contributors are baked in when GitHub answers, so the page reads well before (or without) JavaScript.
import { readFile, writeFile, mkdir, rm, cp } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { markdown, inline, section, escape } from "./src/markdown.js";
import { recentHtml, releasesPageHtml } from "./src/releases.js";
import { machinesHtml, playersHtml } from "./src/bench.js";
import { CPUS, GPUS, singleThread } from "./src/hardware.js";
import { isGame, isLauncher, launcherVersion, newestFirst } from "./src/versions.js";
import { contributorsFrom, contributorsHtml } from "./src/contributors.js";

const site = dirname(fileURLToPath(import.meta.url));
const dist = join(site, "dist");
const REPO = "ludovic111/horizon";
const RAW = `https://raw.githubusercontent.com/${REPO}/main/`;
const DEFAULT_LINKS = { discord: "https://discord.gg/HcTeMZxe5E", sponsor: "https://github.com/sponsors/ludovic111" };

async function repoFile(path) {
  const local = join(site, "..", path);
  if (existsSync(local)) return [await readFile(local, "utf8"), "the checkout"];
  const res = await fetch(RAW + path, { signal: AbortSignal.timeout(15000) });
  if (!res.ok) throw new Error(`${path}: ${res.status} from GitHub`);
  return [await res.text(), "main on GitHub"];
}

// The README: required, a site with a stale or missing parity table is worse than no deploy.
const [readme, readmeFrom] = await repoFile("README.md");
let links = { ...DEFAULT_LINKS };
try {
  const parsed = JSON.parse((await repoFile("launcher/links.json"))[0]);
  for (const key of Object.keys(links)) if (typeof parsed[key] === "string") links[key] = parsed[key].trim();
} catch (e) {
  console.warn(`links.json: ${e.message}; using the built-in links`);
}

// An optional token (a Railway variable, or `gh auth token` locally) lifts the rate limit.
const githubHeaders = { Accept: "application/vnd.github+json", ...(process.env.GITHUB_TOKEN ? { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {}) };
// Every release (the Releases page lists them all), a page of 100 at a time.
let releases = [];
try {
  let res;
  for (let page = 1; page <= 5; page++) {
    res = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=100&page=${page}`, { headers: githubHeaders, signal: AbortSignal.timeout(15000) });
    if (!res.ok) break;
    const batch = await res.json();
    releases.push(...batch.filter((r) => !r.draft));
    if (batch.length < 100) break;
  }
  if (!res.ok) console.warn(`releases: ${res.status} from GitHub; the pages will load them themselves`);
  // Pre-releases of the old Alpha channel could push the newest release out of the newest 100: it is GitHub's
  // "latest" (pre-releases never are).
  if (res.ok && !releases.some((r) => isGame(r.tag_name) && !r.prerelease)) {
    const latest = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: githubHeaders, signal: AbortSignal.timeout(15000) });
    if (latest.ok) releases.push(await latest.json());
  }
} catch (e) {
  console.warn(`releases: ${e.message}; the page will load them itself`);
}

let contributors = [];
try {
  const res = await fetch(`https://api.github.com/repos/${REPO}/contributors?per_page=100`, {
    headers: { Accept: "application/vnd.github+json", ...(process.env.GITHUB_TOKEN ? { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {}) },
    signal: AbortSignal.timeout(15000),
  });
  if (res.ok) contributors = contributorsFrom(await res.json());
  else console.warn(`contributors: ${res.status} from GitHub; the page will load them itself`);
} catch (e) {
  console.warn(`contributors: ${e.message}; the page will load them itself`);
}

// --- The parity table ------------------------------------------------------------------

const status = section(readme, "Project status") ?? "";
const parityHeading = (status.match(/^###\s+(Java parity:\s*[\d.]+%)/m) || [])[1];
if (!parityHeading) throw new Error("README: no '### Java parity: N%' heading under 'Project status'");
const percent = parityHeading.match(/[\d.]+%/)[0];
const parityBody = section(readme, parityHeading) ?? "";
const rows = parityBody
  .split("\n")
  .filter((l) => /^\|/.test(l) && !/^\|\s*-/.test(l))
  .map((l) =>
    l
      .replace(/^\|/, "")
      .replace(/\|\s*$/, "")
      .split("|")
      .map((c) => c.trim()),
  );
const [, ...body] = rows; // the first row is the header
if (body.length < 5) throw new Error("README: the parity table has fewer than five rows");
const harnesses = (readme.match(/all (\d[\d,]*) (?:parity )?harnesses/) || [])[1];
if (!harnesses) throw new Error("README: no 'all N harnesses' count");
const audited = (parityBody.match(/Last audited\s+(\d{4}-\d{2}-\d{2})/) || [])[1] ?? "";
const intro = status.split(/^###/m)[0].trim();

const scoreLabel = (s) => ({ 0: "Absent", 0.25: "Started", 0.5: "Playable", 0.75: "Mostly complete", 1: "Complete" })[Number(s)] ?? s;
const check = `<svg class="check" viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7"/></svg>`;
const parityCells = body
  .map(([name, score, gaps], i) => {
    // "None: what matches" reads as the evidence; anything else is a gap and says so.
    const none = gaps.match(/^None:\s*(.*)$/);
    const full = Number(score) === 1;
    const detail = none ? inline(none[1].charAt(0).toUpperCase() + none[1].slice(1)) : `<span class="gap">Gaps:</span> ${inline(gaps)}`;
    const state = full ? `${check}Matches vanilla` : `${escape(score)} · ${escape(scoreLabel(score))}`;
    return `<li class="system${full ? " is-full" : ""}">
            <p class="system-head"><span class="system-index">${String(i + 1).padStart(2, "0")}</span><span class="system-state">${state}</span></p>
            <h3>${inline(name)}</h3>
            <p class="system-detail">${detail}</p>
          </li>`;
  })
  .join("\n");
const matching = body.filter(([, score]) => Number(score) === 1).length;
const parityHeadline = matching === body.length ? "Every subsystem matches vanilla." : `${matching} of ${body.length} subsystems match vanilla.`;

// --- The releases ----------------------------------------------------------------------

// Newest first by tag: every 0.2 tag is newer than the dated ones (src/versions.js). One channel since 2026-10-23
// (docs/release-flow.md, "One channel"); the old Alpha channel's pre-releases stay listed.
const byTag = (a, b) => newestFirst(a.tag_name, b.tag_name);
const ours = releases.filter((r) => isGame(r.tag_name) || isLauncher(r.tag_name)).sort(byTag);
const game = ours.filter((r) => isGame(r.tag_name));
const launcher = releases.filter((r) => isLauncher(r.tag_name) && !r.prerelease).sort(byTag)[0];

// --- The benchmarks ----------------------------------------------------------------------

// public/benchmarks.json: our measurements (scripts/benchmarks.mjs from data/benchmarks/fleet.json) and, once the
// telemetry server has some, players' groups. The page shows it baked and asks the server for newer player results.
const benchSummary = JSON.parse(await readFile(join(site, "public", "benchmarks.json"), "utf8"));

// --- The page --------------------------------------------------------------------------

const fingerprint = (text) => createHash("sha256").update(text).digest("hex").slice(0, 10);

await rm(dist, { recursive: true, force: true });
await mkdir(join(dist, "assets"), { recursive: true });
await cp(join(site, "public"), dist, { recursive: true });
await cp(join(site, "LEGAL.md"), join(dist, "LEGAL.md"));

// The icons keep their names (browsers and other sites ask for /favicon.svg and the
// rest), so the pages link them with their content's hash: a new mark reaches everyone
// at once instead of after the week the server lets browsers keep the old one.
const iconVersions = {};
for (const icon of ["favicon.svg", "favicon-32.png", "apple-touch-icon.png"]) {
  iconVersions[icon] = fingerprint(await readFile(join(site, "public", icon)));
}
// /favicon.ico, which browsers request on their own: the 32-pixel PNG in an ICO
// container (one image entry; ICO has held PNG data since Windows Vista).
const png32 = await readFile(join(site, "public", "favicon-32.png"));
const ico = Buffer.alloc(22);
ico.writeUInt16LE(0, 0); // reserved
ico.writeUInt16LE(1, 2); // type: icon
ico.writeUInt16LE(1, 4); // one image
ico.writeUInt8(32, 6); // width
ico.writeUInt8(32, 7); // height
ico.writeUInt16LE(1, 10); // colour planes
ico.writeUInt16LE(32, 12); // bits per pixel
ico.writeUInt32LE(png32.length, 14);
ico.writeUInt32LE(22, 18); // the image follows the header and its entry
await writeFile(join(dist, "favicon.ico"), Buffer.concat([ico, png32]));

// The page's modules, fingerprinted; each import of a shared module is rewritten to its name.
const names = {};
const rewrite = (code) => code.replace(/"\.\/(\w+\.js)"/g, (all, file) => (names[file] ? `"./${names[file]}"` : all));
// Dependencies before what imports them.
const modules = ["markdown.js", "versions.js", "chrome.js", "hardware.js", "estimate.js", "releases.js", "contributors.js", "bench.js", "main.js", "releases-page.js", "benchmarks-page.js"];
for (const file of modules) {
  const code = rewrite(await readFile(join(site, "src", file), "utf8"));
  names[file] = `${file.replace(".js", "")}.${fingerprint(code)}.js`;
  await writeFile(join(dist, "assets", names[file]), code);
}
const jsName = names["main.js"];
// The design tokens come first: src/tokens.css is generated from crates/horizon-theme
// (`cargo run -p xtask -- tokens`), so the page shares the launcher's and the game's values.
const css = (await readFile(join(site, "src/tokens.css"), "utf8")) + "\n" + (await readFile(join(site, "src/style.css"), "utf8"));
// The stylesheet is inlined into the page, so the first paint needs no second request.
const cssName = `style.${fingerprint(css)}.css`;
await writeFile(join(dist, "assets", cssName), css);
const inlineCss = css.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\s*\n\s*/g, "\n").trim();

const baked = {
  links,
  contributors,
  releases: releases.slice(0, 12).map((r) => ({
    tag_name: r.tag_name,
    name: r.name,
    published_at: r.published_at,
    html_url: r.html_url,
    prerelease: r.prerelease,
    body: isGame(r.tag_name) ? r.body : "",
    assets: r.assets.map((a) => ({ name: a.name, browser_download_url: a.browser_download_url, size: a.size })),
  })),
};

const unavailable = `<p class="lede">The release notes could not be loaded here.</p><a class="button secondary" href="https://github.com/${REPO}/releases">Read them on GitHub</a>`;

// The top bar and the footer of the pages beside the home page (Releases, Benchmarks).
const navLinks = [["/#about", "About"], ["/releases", "Releases"], ["/benchmarks", "Benchmarks"], ["/docs/", "Docs"]];
const siteNav = (current) => `    <header class="nav is-scrolled" id="top">
      <div class="nav-inner">
        <a class="nav-mark" href="/" aria-label="Horizon, home"><svg class="mark" viewBox="0 0 16 16" aria-hidden="true"><path class="mark-stem" d="M4 3h2v4H4zM10 3h2v4h-2z"/><path class="mark-low" d="M4 8h2v4H4zM10 8h2v4h-2z"/><path class="mark-line" d="M1 7h14v1H1z"/></svg>Horizon</a>
        <nav class="nav-links" aria-label="Pages">
${navLinks.map(([href, label]) => `          <a href="${href}"${href === current ? ' aria-current="page"' : ""}>${label}</a>`).join("\n")}
        </nav>
        <div class="nav-end">
          <button class="theme-toggle" type="button" aria-label="Switch between light and dark"><span data-theme-label>Dark</span></button>
          <a class="nav-github" href="https://github.com/${REPO}">GitHub</a>
          <a class="button primary small" href="/docs/install">Download</a>
          <button class="menu-button" type="button" aria-expanded="false" aria-controls="menu"><span class="menu-label">Menu</span><span class="menu-icon" aria-hidden="true"></span></button>
        </div>
      </div>
    </header>
    <div class="menu" id="menu" hidden>
      <nav aria-label="Pages">
        <a href="/">Home</a>
${navLinks.map(([href, label]) => `        <a href="${href}">${label}</a>`).join("\n")}
      </nav>
      <div class="menu-foot">
        <a class="button secondary large" href="https://github.com/${REPO}">GitHub</a>
        <p>Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.</p>
      </div>
    </div>`;
const siteFooter = `    <footer class="footer">
      <div class="container">
        <div class="footer-bar">
          <span class="footer-mark">Made in the open, in Rust.</span>
          <nav class="footer-links" aria-label="Elsewhere"><a href="/">Home</a><a href="/releases">Releases</a><a href="/benchmarks">Benchmarks</a><a href="/docs/">Docs</a><a href="https://github.com/${REPO}">GitHub</a><a href="/privacy">Privacy</a><a href="/LEGAL.md">Assets</a></nav>
        </div>
      </div>
      <p class="footer-word" aria-hidden="true">horizon</p>
    </footer>`;

const fixed = (x, n) => Number(x).toFixed(n);
const specTable =
  `<table class="bench-table spec"><thead><tr><th scope="col">Graphics card</th><th scope="col">FP32 TFLOPS</th><th scope="col">Bandwidth, GB/s</th><th scope="col">Memory, GB</th></tr></thead><tbody>` +
  GPUS.map((g) => `<tr><th scope="row">${escape(g.name)}</th><td class="num">${fixed(g.tflops, 1)}</td><td class="num">${g.gbps}</td><td class="num">${g.vram_gb || "shared"}</td></tr>`).join("") +
  `</tbody></table><table class="bench-table spec"><thead><tr><th scope="col">Processor</th><th scope="col">Boost, GHz</th><th scope="col">Generation factor</th><th scope="col">Single-thread score</th></tr></thead><tbody>` +
  CPUS.map((c) => `<tr><th scope="row">${escape(c.name)}</th><td class="num">${fixed(c.ghz, 2)}</td><td class="num">${fixed(c.ipc, 2)}</td><td class="num">${fixed(singleThread(c), 2)}</td></tr>`).join("") +
  `</tbody></table>`;
const options = (list, selected) => list.map((x) => `<option${x.name === selected ? " selected" : ""}>${escape(x.name)}</option>`).join("");
const playersGroups = benchSummary.groups.filter((g) => g.source === "players");

const values = {
  CSS: `/assets/${cssName}`,
  STYLE: inlineCss,
  JS: `/assets/${jsName}`,
  PERCENT: percent,
  HARNESSES: harnesses,
  AUDITED: audited,
  PARITY_INTRO: markdown(intro.replace("The table below is the project's own audit", "Below is the project's own audit")),
  PARITY_CELLS: parityCells,
  PARITY_HEADLINE: parityHeadline,
  SUBSYSTEMS: String(body.length),
  RECENT: recentHtml(game) || unavailable,
  SITE_NAV: "",
  SITE_FOOTER: siteFooter,
  RELEASES_JS: `/assets/${names["releases-page.js"]}`,
  RELEASE_LIST: releasesPageHtml(ours) || unavailable,
  RELEASE_COUNT: ours.length ? `${ours.length} releases` : "",
  NEWEST_TAG: escape(ours[0]?.tag_name || ""),
  BENCH_JS: `/assets/${names["benchmarks-page.js"]}`,
  MACHINES: machinesHtml(benchSummary),
  PLAYERS: playersHtml(benchSummary),
  PLAYERS_SOURCE: `${playersGroups.length ? `${playersGroups.length} groups` : "No groups yet"}, as of ${benchSummary.generated.slice(0, 10)}. The page asks the telemetry server for newer ones.`,
  GPU_OPTIONS: options(GPUS, "NVIDIA GeForce RTX 4060"),
  CPU_OPTIONS: options(CPUS, "AMD Ryzen 5 5600X"),
  SPEC_TABLE: specTable,
  BENCH_DATA: JSON.stringify(benchSummary).replace(/</g, "\\u003c"),
  LAUNCHER_META: `${launcher ? `Launcher ${escape(launcherVersion(launcher.tag_name))} for` : "For"} Linux x86-64, macOS on Apple Silicon and Windows x64.`,
  DISCORD: escape(links.discord || DEFAULT_LINKS.discord),
  SPONSOR: escape(links.sponsor || DEFAULT_LINKS.sponsor),
  CONTRIBUTORS: contributorsHtml(contributors),
  CONTRIBUTORS_HIDDEN: contributors.length ? "" : " hidden",
  BAKED: JSON.stringify(baked).replace(/</g, "\\u003c"),
  YEAR: String(new Date().getUTCFullYear()),
};

// --- The documentation ---------------------------------------------------------------------
// website/docs/<slug>.md: "# Title", a one-paragraph lede, then the body. docs/nav.json orders them in groups.

const docsNav = JSON.parse(await readFile(join(site, "docs", "nav.json"), "utf8"));
const docs = [];
for (const { group, pages: slugs } of docsNav) {
  for (const slug of slugs) {
    const file = `${slug}.md`;
    const text = (await readFile(join(site, "docs", file), "utf8")).replace(/\r\n?/g, "\n");
    const m = text.match(/^# (.+)\n\n((?:[^\n]+\n)+)\n/);
    if (!m) throw new Error(`docs/${file}: needs "# Title", a blank line, a lede paragraph and a blank line`);
    docs.push({ slug, file, group, title: m[1].trim(), lede: m[2].trim().replace(/\n/g, " "), body: text.slice(m[0].length), path: slug === "index" ? "/docs/" : `/docs/${slug}` });
  }
}
const plain = (md) => md.replace(/`/g, "").replace(/\[([^\]]+)\]\([^)]*\)/g, "$1").replace(/\*\*/g, "");
const anchors = new Map(); // path -> ids, to check every link between the pages
const problems = [];
const renderedDocs = docs.map((d) => {
  const headings = [];
  const html = markdown(d.body.replaceAll("{{HARNESSES}}", harnesses).replaceAll("{{PERCENT}}", percent), { site: true, headings });
  anchors.set(d.path, new Set(headings.map((h) => h.id)));
  return { ...d, html, headings };
});
for (const d of renderedDocs) {
  for (const [, link] of d.html.matchAll(/<a href="((?:\/docs|#)[^"]*)"/g)) {
    const [path, id] = link.split("#");
    const to = path || d.path;
    const known = anchors.get(to);
    if (!known) problems.push(`${d.file}: ${link} is not a docs page`);
    else if (id && !known.has(id)) problems.push(`${d.file}: ${link} has no such heading`);
  }
  for (const [, src] of d.html.matchAll(/<img src="(\/media\/[^"]+)"/g)) {
    if (!existsSync(join(site, "public", src))) problems.push(`${d.file}: image ${src} is not in public/`);
  }
}
if (problems.length) throw new Error(`docs: ${problems.length} broken link(s)\n${problems.join("\n")}`);

const docNav = (current) =>
  docsNav
    .map(({ group, pages: slugs }) => {
      const items = slugs
        .map((slug) => {
          const d = docs.find((x) => x.slug === slug);
          return `<li><a href="${d.path}"${slug === current.slug ? ' aria-current="page"' : ""}>${escape(d.title)}</a></li>`;
        })
        .join("");
      return `          <p class="docs-nav-group">${escape(group)}</p>\n          <ul>${items}</ul>`;
    })
    .join("\n");
const pagerLink = (d, rel) => (d ? `<a class="docs-pager-${rel}" href="${d.path}"><span>${rel === "next" ? "Next" : "Previous"}</span>${escape(d.title)}</a>` : "");

await mkdir(join(dist, "docs"), { recursive: true });
const docPageFiles = [];
const docPages = renderedDocs.map((d, i) => {
  const toc = d.headings.filter((h) => h.level === 2);
  return {
    file: join("docs", d.slug + ".html"),
    values: {
      DOC_TITLE: escape(d.title),
      DOC_DESC: escape(plain(d.lede).slice(0, 220)),
      DOC_PATH: d.path,
      DOC_GROUP: escape(d.group),
      DOC_LEDE: inline(d.lede, true),
      DOC_NAV: docNav(d),
      DOC_TOC: toc.length >= 4 ? `        <nav class="docs-toc" aria-label="On this page"><p>On this page</p><ul>${toc.map((h) => `<li><a href="#${h.id}">${escape(h.text)}</a></li>`).join("")}</ul></nav>` : "",
      DOC_BODY: d.html,
      DOC_PAGER: pagerLink(renderedDocs[i - 1], "prev") + pagerLink(renderedDocs[i + 1], "next"),
      DOC_FILE: d.file,
    },
  };
});

const pages = ["index.html", "404.html", "privacy.html", "releases.html", "benchmarks.html"];
const currentPath = { "releases.html": "/releases", "benchmarks.html": "/benchmarks" };
for (const page of pages) {
  let html = await readFile(join(site, "src", page), "utf8");
  const own = { ...values, SITE_NAV: siteNav(currentPath[page]) };
  html = html.replace(/\{\{(\w+)\}\}/g, (all, key) => {
    if (!(key in own)) throw new Error(`${page}: unknown placeholder ${all}`);
    return own[key];
  });
  html = html.replace(/href="\/(favicon\.svg|favicon-32\.png|apple-touch-icon\.png)"/g, (all, icon) => `href="/${icon}?v=${iconVersions[icon]}"`);
  await writeFile(join(dist, page), html);
}
const docTemplate = await readFile(join(site, "src", "docs.html"), "utf8");
for (const { file, values: own } of docPages) {
  let html = docTemplate.replace(/\{\{(\w+)\}\}/g, (all, key) => {
    const value = key in own ? own[key] : values[key];
    if (value === undefined) throw new Error(`docs.html: unknown placeholder ${all}`);
    return value;
  });
  html = html.replace(/href="\/(favicon\.svg|favicon-32\.png|apple-touch-icon\.png)"/g, (all, icon) => `href="/${icon}?v=${iconVersions[icon]}"`);
  await writeFile(join(dist, file), html);
  docPageFiles.push(file);
}

// The pages' inline scripts (they mark the page as scripted and apply the saved theme before CSS
// applies) and the inlined stylesheet: the Content-Security-Policy in the Caddyfile allows them by hash.
const allHtml = (await Promise.all([...pages, ...docPageFiles].map((page) => readFile(join(dist, page), "utf8")))).join("\n");
const hashes = (re) => [...new Set([...allHtml.matchAll(re)].map((m) => `'sha256-${createHash("sha256").update(m[1]).digest("base64")}'`))].join(" ");
await writeFile(join(dist, ".csp-script"), hashes(/<script>([\s\S]*?)<\/script>/g));
await writeFile(join(dist, ".csp-style"), hashes(/<style>([\s\S]*?)<\/style>/g));

const urls = ["/", "/releases", "/benchmarks", ...docs.map((d) => d.path), "/privacy"];
await writeFile(
  join(dist, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.map((u) => `  <url><loc>https://playhorizon.dev${u}</loc></url>`).join("\n")}\n</urlset>\n`,
);

console.log(
  `built dist/ from ${readmeFrom}: ${body.length} subsystems, ${percent}, ${harnesses} harnesses, ` +
    `${ours.length} releases (${game.length} game)${launcher ? `, launcher ${launcher.tag_name}` : ""}, ${benchSummary.groups.length} benchmark rows, ${contributors.length} contributors, ${docs.length} docs pages`,
);
