// The release notes on the site: the home page's "Recently added" block and the Releases page (every release,
// newest first). Shared by build.mjs (baked into the pages) and the pages' scripts (refreshed from GitHub).
// Tags: the game's vX.Y.Z and the launcher's launcher@X.Y.Z from 0.2, vYYYY.MM.DD[.N] and launcher-vX.Y.Z before
// (src/versions.js). One channel (docs/release-flow.md, "One channel"): a release is the game's or the launcher's.
import { markdown, inline, escape } from "./markdown.js";
import { isLauncher, launcherVersion, newestFirst } from "./versions.js";

const REPO = "ludovic111/horizon";

export const channel = (r) => (isLauncher(r.tag_name) ? "Launcher" : "Game");
const anchor = (r) => `release-${r.tag_name.replace(/[^\w.-]/g, "-")}`;

// "Horizon 0.2.3: Cave culling, weather" (before 0.2, "Horizon 2026.10.13: …") reads as the version and its line.
export function parts(r) {
  const version = launcherVersion(r.tag_name) ?? r.tag_name.replace(/^v/, "");
  const name = (r.name || "").trim();
  let line = name.includes(": ") ? name.slice(name.indexOf(": ") + 2) : name.replace(/^Horizon( launcher)?\s*/i, "").replace(version, "").trim();
  if (junk(line)) line = fromNotes(r.body) || line;
  return { version, line: line.charAt(0).toUpperCase() + line.slice(1), date: (r.published_at || "").slice(0, 10) };
}

// A title made of issue numbers and "fix" ("#1255, mob pushes, fix, #1222") says nothing.
const junk = (line) => {
  const bits = line.split(",").map((b) => b.trim()).filter(Boolean);
  return bits.length > 1 && bits.filter((b) => /^(#\d+|fix(es)?|fixed)$/i.test(b)).length * 2 >= bits.length;
};

// The first item that isn't a bare "Fix #123:" prefix, as the line: "Mob pushes: one list per mob… and 3 more fixes".
function fromNotes(body) {
  const s = sections(body);
  const items = [...s.filter((x) => /^new|what's new/i.test(x.title)), ...s.filter((x) => /^fixed/i.test(x.title))].flatMap((x) => x.items).map(plainItem);
  const clean = items.map((i) => i.replace(/^(fix|fixes)\s+#\d+:\s*/i, "").replace(/^\w+\([^)]*\):\s*/, "")).filter(Boolean);
  if (!clean.length) return "";
  const first = clean[0].replace(/[.]$/, "");
  return clean.length > 1 ? `${first} and ${clean.length - 1} more` : first;
}

// The notes' "### Heading" sections and their top-level items, in order.
export function sections(body) {
  const out = [];
  let current = null;
  for (const raw of (body || "").replace(/\r\n?/g, "\n").split("\n")) {
    const heading = raw.match(/^#{2,3}\s+(.+?)\s*$/);
    if (heading) out.push((current = { title: heading[1], items: [] }));
    else if (current && /^[-*]\s+/.test(raw)) current.items.push(raw.replace(/^[-*]\s+/, "").trim());
  }
  return out;
}

// An item without its pull request references: "Riptide (#123, fixes #45)" -> "Riptide".
export const plainItem = (item) => item.replace(/\s*\((?:#\d+|fixes #\d+|[^()]*#\d+[^()]*)\)\s*/g, " ").replace(/\s+/g, " ").trim();

// The notes' first sentence, for a release with no title line and no sections ("Rebuilt from the latest main.").
const firstLine = (body) => ((body || "").split(/\r?\n/).find((l) => l.trim() && !/^#/.test(l)) || "").trim().replace(/[.:]$/, "");

const count = (n, one, many) => `${n} ${n === 1 ? one : many}`;

// What a release changes, in a line: "12 new, 30 fixed, 2 known issues".
export function overview(r) {
  const s = sections(r.body);
  const by = (re) => s.filter((x) => re.test(x.title)).reduce((n, x) => n + x.items.length, 0);
  const added = by(/^new|what's new/i);
  const fixed = by(/^fixed/i);
  const known = by(/^known/i);
  const bits = [];
  if (added) bits.push(count(added, "new feature", "new features"));
  if (fixed) bits.push(count(fixed, "fix", "fixes"));
  if (known) bits.push(count(known, "known issue", "known issues"));
  return { added, fixed, known, text: bits.join(", ") };
}

export function highlights(r, n = 3) {
  const added = sections(r.body).filter((x) => /^new|what's new/i.test(x.title)).flatMap((x) => x.items);
  return added.slice(0, n).map(plainItem);
}

const byTag = (a, b) => newestFirst(a.tag_name, b.tag_name);
const badge = (r) => `<span class="badge badge-${channel(r).toLowerCase()}">${channel(r)}</span>`;

// The home page: the newest releases on a line each, and what the newest ones added.
export function recentHtml(list, { rows = 5, picks = 3 } = {}) {
  const sorted = [...list].sort(byTag);
  const game = sorted.filter((r) => !isLauncher(r.tag_name));
  if (!game.length) return "";
  const lines = game
    .slice(0, rows)
    .map((r) => {
      const p = parts(r);
      return `<li class="row recent-row"><a href="/releases#${anchor(r)}"><span class="row-index"><time datetime="${p.date}">${p.date}</time></span><span class="row-name">${escape(p.version)}</span><span class="recent-line">${escape(p.line || overview(r).text || firstLine(r.body) || "Release notes")}</span><span class="row-meta">${badge(r)}</span></a></li>`;
    })
    .join("");
  // What landed last: the "New" items of the newest releases with the highest pull request numbers (a release's
  // notes list everything since the release before it, oldest first).
  const pr = (item) => Math.max(0, ...[...item.matchAll(/#(\d+)/g)].map((m) => Number(m[1])));
  const seen = new Set();
  const picked = game
    .slice(0, rows)
    .flatMap((r) => sections(r.body).filter((x) => /^new|what's new/i.test(x.title)).flatMap((x) => x.items).map((raw) => ({ raw, item: plainItem(raw), r })))
    .filter(({ item }) => !seen.has(item) && seen.add(item))
    .sort((a, b) => pr(b.raw) - pr(a.raw))
    .slice(0, picks);
  const picks_ = picked
    .map(({ item, r }) => `<li class="highlight"><p>${inline(item)}</p><a class="highlight-from" href="/releases#${anchor(r)}">${escape(parts(r).version)}</a></li>`)
    .join("");
  return `<ol class="index recent-list" aria-label="The newest releases">${lines}</ol>
          ${picks_ ? `<div class="recent-picks"><p class="eyebrow">New lately</p><ul class="highlights">${picks_}</ul></div>` : ""}
          <p class="release-more"><a class="link-arrow" href="/releases">Every release, with its full notes<span aria-hidden="true">→</span></a></p>`;
}

const ASSET_LABELS = [
  [/linux-x86_64/, "Linux x86-64"],
  [/macos-arm64/, "macOS, Apple Silicon"],
  [/macos-x86_64/, "macOS, Intel"],
  [/windows-x64/, "Windows x64"],
];
function assetLabel(name) {
  if (/^SHA256SUMS/.test(name)) return name.endsWith(".sig") ? "Checksums signature" : "Checksums";
  const os = ASSET_LABELS.find(([re]) => re.test(name));
  const what = /server/.test(name) ? "Server" : /launcher/.test(name) ? "Launcher" : /smoke-test/.test(name) ? "Smoke test" : "Game";
  return os ? `${what}, ${os[1]}` : name;
}
const size = (bytes) => (bytes >= 1e6 ? `${(bytes / 1e6).toFixed(0)} MB` : bytes >= 1e3 ? `${(bytes / 1e3).toFixed(0)} kB` : `${bytes} B`);

function releaseHtml(r, open) {
  const p = parts(r);
  const o = overview(r);
  const top = highlights(r, 4);
  const downloads = (r.assets || [])
    .map((a) => `<li><a href="${escape(a.browser_download_url)}">${escape(assetLabel(a.name))}</a><span>${escape(a.name)} · ${size(a.size || 0)}</span></li>`)
    .join("");
  return `<details class="release release-entry" id="${anchor(r)}" data-channel="${channel(r).toLowerCase()}"${open ? " open" : ""}>
            <summary><span class="release-tag">${escape(p.version)}</span><span class="release-sub">${badge(r)}${escape(p.line)}</span><time datetime="${p.date}">${p.date}</time></summary>
            <div class="release-detail">
              ${o.text || top.length ? `<div class="release-overview">
                ${o.text ? `<p class="release-counts">${escape(o.text)}</p>` : ""}
                ${top.length ? `<ul>${top.map((t) => `<li>${inline(t)}</li>`).join("")}</ul>` : ""}
              </div>` : ""}
              <div class="prose">${markdown(r.body || "") || "<p>No notes for this release.</p>"}</div>
              ${downloads ? `<div class="release-downloads"><p class="eyebrow">Downloads</p><ul>${downloads}</ul></div>` : ""}
              <p class="release-github"><a href="${escape(r.html_url)}">This release on GitHub</a></p>
            </div>
          </details>`;
}

// The Releases page: every release, newest first; the newest few open.
export function releasesPageHtml(list, { open = 3 } = {}) {
  const sorted = [...list].sort(byTag);
  if (!sorted.length) return "";
  return sorted.map((r, i) => releaseHtml(r, i < open)).join("\n");
}

export const releasesUrl = `https://github.com/${REPO}/releases`;
