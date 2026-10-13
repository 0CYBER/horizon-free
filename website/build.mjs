// Builds the site into dist/: src/index.html with the README's parity table, roadmap and
// the community links baked in, plus public/ and fingerprinted CSS and JS. No dependencies.
//
// Railway builds only website/, so the README and links.json come from the repo when they
// are next to it (a local checkout) and from main on GitHub otherwise. The latest releases
// are baked in when GitHub answers, so the page reads well before (or without) JavaScript.
import { readFile, writeFile, mkdir, rm, cp } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { markdown, inline, section, escape } from "./src/markdown.js";

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

let releases = [];
try {
  const res = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=30`, {
    // An optional token (a Railway variable, or `gh auth token` locally) lifts the rate limit.
    headers: { Accept: "application/vnd.github+json", ...(process.env.GITHUB_TOKEN ? { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {}) },
    signal: AbortSignal.timeout(15000),
  });
  if (res.ok) releases = (await res.json()).filter((r) => !r.draft);
  else console.warn(`releases: ${res.status} from GitHub; the page will load them itself`);
} catch (e) {
  console.warn(`releases: ${e.message}; the page will load them itself`);
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
const parityRows = body
  .map(([name, score, gaps]) => {
    // "None: what matches" reads as the evidence; anything else is a gap and says so.
    const none = gaps.match(/^None:\s*(.*)$/);
    const detail = none
      ? `<span class="gap-none">No known gaps.</span> ${inline(none[1].charAt(0).toUpperCase() + none[1].slice(1))}`
      : inline(gaps);
    return `<tr>
          <th scope="row">${inline(name)}</th>
          <td class="score"><span class="pill${Number(score) === 1 ? " is-full" : ""}">${escape(score)}</span><span class="score-label">${escape(scoreLabel(score))}</span></td>
          <td>${detail}</td>
        </tr>`;
  })
  .join("\n");

// --- The roadmap and the latest notes -------------------------------------------------

const roadmap = section(readme, "Roadmap");
if (!roadmap) throw new Error("README: no Roadmap section");

const isGame = (tag) => /^v\d{4}\.\d{1,2}\.\d{1,2}/.test(tag);
const isLauncher = (tag) => tag.startsWith("launcher-v");
const game = releases.filter((r) => isGame(r.tag_name)).slice(0, 3);
const launcher = releases.find((r) => isLauncher(r.tag_name) && !r.prerelease);

// --- The page --------------------------------------------------------------------------

const fingerprint = (text) => createHash("sha256").update(text).digest("hex").slice(0, 10);

await rm(dist, { recursive: true, force: true });
await mkdir(join(dist, "assets"), { recursive: true });
await cp(join(site, "public"), dist, { recursive: true });
await cp(join(site, "LEGAL.md"), join(dist, "LEGAL.md"));

const md = await readFile(join(site, "src/markdown.js"), "utf8");
const mdName = `markdown.${fingerprint(md)}.js`;
await writeFile(join(dist, "assets", mdName), md);
const js = (await readFile(join(site, "src/main.js"), "utf8")).replace("./markdown.js", `./${mdName}`);
const jsName = `main.${fingerprint(js)}.js`;
await writeFile(join(dist, "assets", jsName), js);
const css = await readFile(join(site, "src/style.css"), "utf8");
// The stylesheet is inlined into the page, so the first paint needs no second request.
const cssName = `style.${fingerprint(css)}.css`;
await writeFile(join(dist, "assets", cssName), css);
const inlineCss = css.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\s*\n\s*/g, "\n").trim();

const baked = {
  links,
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

const notesHtml = game.length
  ? game.map((r, i) => releaseHtml(r, i === 0)).join("\n")
  : `<p class="section-lede">The release notes could not be loaded here.</p><a class="button secondary" href="https://github.com/${REPO}/releases">Read them on GitHub</a>`;

function releaseHtml(r, open) {
  const date = (r.published_at || "").slice(0, 10);
  return `<details class="release"${open ? " open" : ""}>
          <summary><span class="release-title">${escape(r.name || r.tag_name)}</span><time datetime="${date}">${date}</time></summary>
          <div class="prose">${markdown(r.body || "")}<p><a href="${escape(r.html_url)}">Downloads and full notes on GitHub</a></p></div>
        </details>`;
}

const values = {
  CSS: `/assets/${cssName}`,
  STYLE: inlineCss,
  JS: `/assets/${jsName}`,
  PERCENT: percent,
  HARNESSES: harnesses,
  AUDITED: audited,
  PARITY_INTRO: markdown(intro),
  PARITY_ROWS: parityRows,
  ROADMAP: markdown(roadmap, { shift: 1 }),
  NOTES: notesHtml,
  LAUNCHER_VERSION: launcher ? escape(launcher.tag_name.replace("launcher-v", "")) : "",
  DISCORD: escape(links.discord || DEFAULT_LINKS.discord),
  SPONSOR: escape(links.sponsor || DEFAULT_LINKS.sponsor),
  BAKED: JSON.stringify(baked).replace(/</g, "\\u003c"),
  YEAR: String(new Date().getUTCFullYear()),
};

for (const page of ["index.html", "404.html"]) {
  let html = await readFile(join(site, "src", page), "utf8");
  html = html.replace(/\{\{(\w+)\}\}/g, (all, key) => {
    if (!(key in values)) throw new Error(`${page}: unknown placeholder ${all}`);
    return values[key];
  });
  await writeFile(join(dist, page), html);
}

// The page's inline script (it marks the page as scripted before CSS applies) and its
// inlined stylesheet: the Content-Security-Policy in the Caddyfile allows them by hash.
const indexHtml = await readFile(join(dist, "index.html"), "utf8");
const hashes = (re) => [...new Set([...indexHtml.matchAll(re)].map((m) => `'sha256-${createHash("sha256").update(m[1]).digest("base64")}'`))].join(" ");
await writeFile(join(dist, ".csp-script"), hashes(/<script>([\s\S]*?)<\/script>/g));
await writeFile(join(dist, ".csp-style"), hashes(/<style>([\s\S]*?)<\/style>/g));

console.log(
  `built dist/ from ${readmeFrom}: ${body.length} subsystems, ${percent}, ${harnesses} harnesses, ` +
    `${game.length} game releases baked${launcher ? `, launcher ${launcher.tag_name}` : ""}`,
);
