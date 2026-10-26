// Builds the site into dist/: src/index.html with the README's parity table and
// the community links and contributors baked in, the documentation (docs/*.md, one page each, listed in
// docs/nav.json), plus public/ and fingerprinted CSS and JS. No dependencies.
//
// Railway builds only website/, so the README and links.json come from the repo when they
// are next to it (a local checkout) and from main on GitHub otherwise. The documentation lives in website/, except the
// API portal (docs/api/), read the same way as the README. The latest releases and
// the contributors are baked in when GitHub answers, so the page reads well before (or without) JavaScript.
import { readFile, writeFile, mkdir, rm, cp } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname, posix } from "node:path";
import { fileURLToPath } from "node:url";
import { markdown, inline, section, escape } from "./src/markdown.js";
import { recentHtml, releasesPageHtml } from "./src/releases.js";
import { machinesHtml, playersHtml } from "./src/bench.js";
import { CPUS, GPUS, singleThread } from "./src/hardware.js";
import { isGame, isLauncher, launcherVersion, newestFirst } from "./src/versions.js";
import { contributorsFrom, contributorsHtml } from "./src/contributors.js";
import { areaFilterHtml, boardFilterHtml, boardFrom, boardHtml, creditFrom, creditHtml, helpWantedHtml, journalFrom, journalHtml, journalJson, overviewHtml, roadmapHtml, slimIssue, suggestItemsHtml } from "./src/hub.js";
import { INDEX_URL, KINDS, SUBMIT, listHtml, projectHtml, readIndex, submitHtml } from "./src/mods.js";

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

// Horizon's registry (/mods): the index its repository publishes, or HORIZON_MOD_INDEX (a file or an address) to
// try a test registry. Without an answer the pages are built empty and load the index themselves.
let modProjects = [];
let modIndex = null;
try {
  const from = process.env.HORIZON_MOD_INDEX || INDEX_URL;
  const text = from.startsWith("https://")
    ? await fetch(from, { signal: AbortSignal.timeout(15000) }).then((r) => (r.ok ? r.text() : Promise.reject(new Error(`${r.status}`))))
    : await readFile(from, "utf8");
  modIndex = JSON.parse(text);
  modProjects = readIndex(modIndex) ?? (console.warn("mod index: a schema this site doesn't read"), []);
} catch (e) {
  console.warn(`mod index: ${e.message}; the pages will load it themselves`);
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

// The hub's roadmap (data/roadmap.json, generated from roadmap/roadmap.toml by tools/roadmap.py) and the live state
// of the issues and pull requests its items link: the open ones and those closed lately, then (with a token) the
// older ones in one GraphQL request. Without GitHub the hub shows the links without their state.
const roadmap = JSON.parse(await readFile(join(site, "data", "roadmap.json"), "utf8"));
const linked = new Set(roadmap.items.flatMap((i) => (i.links || []).map((l) => Number((l.match(/^#(\d+)$/) || [])[1])).filter(Boolean)));
let hubLive = [];
try {
  const urls = [1, 2, 3].map((p) => `https://api.github.com/repos/${REPO}/issues?state=open&per_page=100&page=${p}`);
  urls.push(`https://api.github.com/repos/${REPO}/issues?state=closed&sort=updated&direction=desc&per_page=100`);
  for (const url of urls) {
    const res = await fetch(url, { headers: githubHeaders, signal: AbortSignal.timeout(15000) });
    if (!res.ok) throw new Error(`${res.status} from GitHub`);
    hubLive.push(...(await res.json()).map(slimIssue));
  }
  const missing = [...linked].filter((n) => !hubLive.some((i) => i.n === n));
  if (missing.length && process.env.GITHUB_TOKEN) {
    const fields = missing.map((n) => `n${n}: issueOrPullRequest(number: ${n}) { ... on Issue { number title state stateReason url } ... on PullRequest { number title state isDraft mergedAt url } }`).join(" ");
    const res = await fetch("https://api.github.com/graphql", {
      method: "POST",
      headers: githubHeaders,
      body: JSON.stringify({ query: `query { repository(owner: "ludovic111", name: "horizon") { ${fields} } }` }),
      signal: AbortSignal.timeout(15000),
    });
    const nodes = Object.values((await res.json()).data?.repository || {}).filter(Boolean);
    hubLive.push(
      ...nodes.map((x) =>
        slimIssue({
          number: x.number, title: x.title, html_url: x.url, draft: x.isDraft,
          state: x.state === "OPEN" ? "open" : "closed", state_reason: x.stateReason === "NOT_PLANNED" ? "not_planned" : null,
          pull_request: "isDraft" in x ? { merged_at: x.mergedAt } : undefined,
        }),
      ),
    );
  }
} catch (e) {
  console.warn(`hub: ${e.message}; the page will load the states itself`);
}
const hubLinked = new Map(hubLive.filter((i) => linked.has(i.n)).map((i) => [i.n, i]));
const board = boardFrom(hubLive, roadmap);
const journal = journalFrom(releases, hubLive, roadmap);

// The hub's server (crates/horizon-telemetry-server: votes, bug reports, suggestions), and Kira's list of the
// contributors who linked their Discord account and agreed to be credited (kira's /contributors.json).
const hubApi = process.env.HUB_API_URL || "https://telemetry.playhorizon.dev";
let kiraCredit = {};
try {
  const res = await fetch(process.env.KIRA_CONTRIBUTORS_URL || "https://kira-production-500e.up.railway.app/contributors.json", { signal: AbortSignal.timeout(10000) });
  if (!res.ok) throw new Error(`${res.status} from Kira`);
  kiraCredit = await res.json();
} catch (e) {
  console.warn(`hub: contributors' Discord names: ${e.message}`);
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
const modules = ["markdown.js", "versions.js", "chrome.js", "hardware.js", "estimate.js", "releases.js", "contributors.js", "bench.js", "hub.js", "main.js", "releases-page.js", "benchmarks-page.js", "docs-page.js", "hub-page.js", "mods.js", "mods-page.js", "mods-submit.js"];
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

// One top bar and phone menu for Home, Releases, Benchmarks and every Docs page.
const navSections = [["/#about", "About"], ["/#screenshots", "In game"], ["/#parity", "Parity"], ["/#launcher", "Launcher"], ["/#recent", "New", "Recently added"], ["/#community", "Community"]];
const navPages = [["/hub", "Hub"], ["/mods", "Mods"], ["/releases", "Releases"], ["/benchmarks", "Benchmarks"], ["/docs/", "Docs", "Documentation"]];
const navLink = ([href, label, menuLabel], current, menu = false) =>
  `<a href="${href}"${href === current ? ' aria-current="page"' : ""}${href === "/#contributors" ? ` data-contributors${contributors.length ? "" : " hidden"}` : ""}>${menu && menuLabel ? menuLabel : label}</a>`;
const siteNav = (current) => `    <header class="nav" id="top">
      <div class="nav-inner">
        <a class="nav-mark" href="/" aria-label="Horizon, home"><svg class="mark" viewBox="0 0 16 16" aria-hidden="true"><path class="mark-stem" d="M4 3h2v4H4zM10 3h2v4h-2z"/><path class="mark-low" d="M4 8h2v4H4zM10 8h2v4h-2z"/><path class="mark-line" d="M1 7h14v1H1z"/></svg>Horizon</a>
        <nav class="nav-links" id="nav-links" aria-label="Sections and pages">
${[...navSections, ...navPages].map((link) => `          ${navLink(link, current)}`).join("\n")}
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
      <nav aria-label="Sections and pages">
${[...navSections, ["/#contributors", "Contributors"], ...navPages].map((link) => `        ${navLink(link, current, true)}`).join("\n")}
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
          <nav class="footer-links" aria-label="Elsewhere"><a href="/">Home</a><a href="/hub">Hub</a><a href="/mods">Mods</a><a href="/releases">Releases</a><a href="/benchmarks">Benchmarks</a><a href="/docs/">Docs</a><a href="/docs/how-horizon-is-built">How it's built</a><a href="https://github.com/${REPO}">GitHub</a><a href="/privacy">Privacy</a><a href="/LEGAL.md">Assets</a></nav>
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
  DOCS_JS: `/assets/${names["docs-page.js"]}`,
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
  HUB_JS: `/assets/${names["hub-page.js"]}`,
  HUB_OVERVIEW: overviewHtml(roadmap, hubLinked),
  HUB_ROADMAP: roadmapHtml(roadmap, hubLinked),
  HUB_AREAS: areaFilterHtml(roadmap),
  HUB_HELP_WANTED: helpWantedHtml(roadmap),
  HUB_BOARD: hubLive.length ? boardHtml(board, roadmap) : `<p class="lede">The board could not be loaded here.</p>`,
  HUB_BOARD_FILTER: boardFilterHtml(board, roadmap),
  HUB_JOURNAL: journalHtml(journal, roadmap) || unavailable,
  HUB_DATA: JSON.stringify({ roadmap, live: [...hubLinked.values()], api: hubApi }).replace(/</g, "\\u003c"),
  HUB_SUGGEST_ITEMS: suggestItemsHtml(roadmap),
  HUB_VERSION: escape(game[0]?.tag_name || ""),
  HUB_CREDIT: creditHtml(creditFrom(contributors, kiraCredit)),
  YEAR: String(new Date().getUTCFullYear()),
};

// --- The documentation ---------------------------------------------------------------------
// website/docs/<slug>.md: "# Title", a one-paragraph lede, then the body. docs/nav.json orders them in groups.
// The API portal is the repository's docs/api/ (the pages and their generated references): nav.json lists it as
// `api` (its README, /docs/api) and `api/<page>` (/docs/api/<page>). Its links are written for GitHub, so links
// between its pages become the site's and every other relative link goes to the file on GitHub.

const docsNav = JSON.parse(await readFile(join(site, "docs", "nav.json"), "utf8"));
const docSlugs = new Set(docsNav.flatMap((g) => g.pages));
const apiSource = (slug) => `docs/api/${slug === "api" ? "README" : slug.slice(4)}.md`;
function apiLinks(text, from) {
  return text.replace(/\]\(([^)\s]+)\)/g, (all, url) => {
    if (url.startsWith("https://playhorizon.dev/")) return `](${url.slice(23)})`;
    if (/^([a-z]+:|#|\/)/i.test(url)) return all;
    const [file, id] = url.split("#");
    const path = posix.normalize(posix.join(posix.dirname(from), file));
    const page = path.match(/^docs\/api\/([\w-]+)\.md$/)?.[1];
    const slug = page === "README" ? "api" : page && `api/${page}`;
    const to = docSlugs.has(slug) ? `/docs/${slug}` : `https://github.com/${REPO}/blob/main/${path}`;
    return `](${to}${id ? `#${id}` : ""})`;
  });
}
const docs = [];
for (const { group, pages: slugs } of docsNav) {
  for (const slug of slugs) {
    const api = slug === "api" || slug.startsWith("api/");
    const file = api ? apiSource(slug) : `website/docs/${slug}.md`;
    let text = (api ? (await repoFile(file))[0] : await readFile(join(site, "docs", `${slug}.md`), "utf8")).replace(/\r\n?/g, "\n");
    if (api) text = apiLinks(text.replace(/^<!--[\s\S]*?-->\n/, ""), file);
    const m = text.match(/^# (.+)\n\n((?:[^\n]+\n)+)\n/);
    if (!m) throw new Error(`${file}: needs "# Title", a blank line, a lede paragraph and a blank line`);
    docs.push({ slug, file, group, title: m[1].replace(/`/g, "").trim(), lede: m[2].trim().replace(/\n/g, " "), body: text.slice(m[0].length), path: slug === "index" ? "/docs/" : `/docs/${slug}` });
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

await mkdir(join(dist, "docs", "api"), { recursive: true });
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
      SITE_NAV: siteNav("/docs/"),
    },
  };
});

const pages = ["index.html", "404.html", "privacy.html", "releases.html", "benchmarks.html", "hub.html"];
const currentPath = { "releases.html": "/releases", "benchmarks.html": "/benchmarks", "hub.html": "/hub" };
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
// The registry: /mods (the list) and /mods/<id> (a page per project), one template.
const modsTemplate = await readFile(join(site, "src", "mods.html"), "utf8");
const modsList = `        <p class="kicker"><span>Mods</span>Made for Horizon, reviewed</p>
        <h1 class="display page-title">Horizon mods</h1>
        <p class="lede">Mods, modpacks and resource packs made for Horizon. Each one was reviewed before it was listed, and the launcher checks every file before it installs it. Mods run in a sandbox and can only do what their page says.</p>
        <p class="mod-actions"><a class="button primary" href="${SUBMIT}">Submit yours</a> <a class="button secondary" href="/docs/make-a-mod">Make a mod</a></p>
        <div class="mod-tools">
          <input class="mod-search" id="mod-search" type="search" placeholder="Search" aria-label="Search the registry" autocomplete="off">
          <div class="release-filter" role="group" aria-label="Show">
            <button type="button" class="button secondary small" data-kind-filter="all" aria-pressed="true">All</button>
${KINDS.map(([k, label]) => `            <button type="button" class="button secondary small" data-kind-filter="${k}" aria-pressed="false">${label}</button>`).join("\n")}
          </div>
        </div>
        <p class="meta release-count" data-mod-count>${modProjects.length} of ${modProjects.length}</p>
        <div class="mod-list" id="mod-list" aria-live="polite">
${listHtml(modProjects)}
        </div>`;
const modPages = [
  { file: "mods.html", path: "/mods", title: "Mods", description: "Mods, modpacks and resource packs made for Horizon, reviewed, each with what it may do and how to install it.", main: modsList },
  { file: "mods/submit.html", path: "/mods/submit", title: "Submit to the registry", description: "Submit a mod, a modpack or a resource pack made for Horizon: it is checked, reviewed, then listed here and in the launcher.", main: submitHtml(), js: "mods-submit.js" },
  ...modProjects.filter((p) => p.id !== "submit").map((p) => ({ file: `mods/${p.id}.html`, path: `/mods/${p.id}`, title: p.name, description: p.summary || p.name, main: projectHtml(p) })),
];
await mkdir(join(dist, "mods"), { recursive: true });
const modsData = JSON.stringify(modIndex && modProjects.length ? { schema: modIndex.schema, projects: modProjects } : null).replace(/</g, "\\u003c");
for (const page of modPages) {
  const own = {
    ...values,
    SITE_NAV: siteNav("/mods"),
    MODS_TITLE: escape(page.title),
    MODS_DESCRIPTION: escape(page.description),
    MODS_PATH: page.path,
    MODS_JS: `/assets/${names[page.js || "mods-page.js"]}`,
    MODS_MAIN: page.main,
    MODS_DATA: modsData,
    MODS_BAKED_AT: escape(modIndex?.generated || ""),
  };
  let html = modsTemplate.replace(/\{\{(\w+)\}\}/g, (all, key) => {
    if (!(key in own)) throw new Error(`mods.html: unknown placeholder ${all}`);
    return own[key];
  });
  html = html.replace(/href="\/(favicon\.svg|favicon-32\.png|apple-touch-icon\.png)"/g, (all, icon) => `href="/${icon}?v=${iconVersions[icon]}"`);
  await writeFile(join(dist, page.file), html);
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

// The roadmap for other readers (Kira, scripts): /data/roadmap.json, as tools/roadmap.py wrote it.
await mkdir(join(dist, "data"), { recursive: true });
await cp(join(site, "data", "roadmap.json"), join(dist, "data", "roadmap.json"));
// The journal for Kira's #updates and its X posts: /data/journal.json, rebuilt with every deploy.
await writeFile(join(dist, "data", "journal.json"), JSON.stringify(journalJson(releases, hubLive, roadmap), null, 1) + "\n");

// The pages' inline scripts (they mark the page as scripted and apply the saved theme before CSS
// applies) and the inlined stylesheet: the Content-Security-Policy in the Caddyfile allows them by hash.
const allHtml = (await Promise.all([...pages, ...docPageFiles, ...modPages.map((p) => p.file)].map((page) => readFile(join(dist, page), "utf8")))).join("\n");
const hashes = (re) => [...new Set([...allHtml.matchAll(re)].map((m) => `'sha256-${createHash("sha256").update(m[1]).digest("base64")}'`))].join(" ");
await writeFile(join(dist, ".csp-script"), hashes(/<script>([\s\S]*?)<\/script>/g));
await writeFile(join(dist, ".csp-style"), hashes(/<style>([\s\S]*?)<\/style>/g));

const urls = ["/", "/hub", "/mods", ...modPages.slice(1).map((p) => p.path), "/releases", "/benchmarks", ...docs.map((d) => d.path), "/privacy"];
await writeFile(
  join(dist, "sitemap.xml"),
  `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${urls.map((u) => `  <url><loc>https://playhorizon.dev${u}</loc></url>`).join("\n")}\n</urlset>\n`,
);

console.log(
  `built dist/ from ${readmeFrom}: ${body.length} subsystems, ${percent}, ${harnesses} harnesses, ` +
    `${ours.length} releases (${game.length} game)${launcher ? `, launcher ${launcher.tag_name}` : ""}, ${benchSummary.groups.length} benchmark rows, ${contributors.length} contributors, ${docs.length} docs pages, ${modProjects.length} registry projects, ${roadmap.items.length} roadmap items (${hubLinked.size}/${linked.size} links live), ${Object.values(board).flat().length} board cards, ${journal.length} journal entries`,
);
