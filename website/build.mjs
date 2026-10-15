// Builds the site into dist/: src/index.html with the README's parity table and
// the community links and contributors baked in, plus public/ and fingerprinted CSS and JS. No dependencies.
//
// Railway builds only website/, so the README and links.json come from the repo when they
// are next to it (a local checkout) and from main on GitHub otherwise. The latest releases and
// the contributors are baked in when GitHub answers, so the page reads well before (or without) JavaScript.
import { readFile, writeFile, mkdir, rm, cp } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createHash } from "node:crypto";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { markdown, inline, section, escape } from "./src/markdown.js";
import { releasesHtml } from "./src/releases.js";
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

let releases = [];
try {
  const res = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=100`, {
    // An optional token (a Railway variable, or `gh auth token` locally) lifts the rate limit.
    headers: { Accept: "application/vnd.github+json", ...(process.env.GITHUB_TOKEN ? { Authorization: `Bearer ${process.env.GITHUB_TOKEN}` } : {}) },
    signal: AbortSignal.timeout(15000),
  });
  if (res.ok) releases = (await res.json()).filter((r) => !r.draft);
  else console.warn(`releases: ${res.status} from GitHub; the page will load them itself`);
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

// --- The latest notes -------------------------------------------------

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
for (const file of ["markdown.js", "releases.js", "contributors.js", "main.js"]) {
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

const notesHtml =
  releasesHtml(game) ||
  `<p class="lede">The release notes could not be loaded here.</p><a class="button secondary" href="https://github.com/${REPO}/releases">Read them on GitHub</a>`;

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
  NOTES: notesHtml,
  LAUNCHER_META: `${launcher ? `Launcher ${escape(launcher.tag_name.replace("launcher-v", ""))} for` : "For"} Linux x86-64, macOS on Apple Silicon and Windows x64.`,
  DISCORD: escape(links.discord || DEFAULT_LINKS.discord),
  SPONSOR: escape(links.sponsor || DEFAULT_LINKS.sponsor),
  CONTRIBUTORS: contributorsHtml(contributors),
  CONTRIBUTORS_HIDDEN: contributors.length ? "" : " hidden",
  BAKED: JSON.stringify(baked).replace(/</g, "\\u003c"),
  YEAR: String(new Date().getUTCFullYear()),
};

for (const page of ["index.html", "404.html", "privacy.html"]) {
  let html = await readFile(join(site, "src", page), "utf8");
  html = html.replace(/\{\{(\w+)\}\}/g, (all, key) => {
    if (!(key in values)) throw new Error(`${page}: unknown placeholder ${all}`);
    return values[key];
  });
  html = html.replace(/href="\/(favicon\.svg|favicon-32\.png|apple-touch-icon\.png)"/g, (all, icon) => `href="/${icon}?v=${iconVersions[icon]}"`);
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
    `${game.length} game releases baked${launcher ? `, launcher ${launcher.tag_name}` : ""}, ${contributors.length} contributors`,
);
