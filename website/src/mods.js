// Horizon's registry on the site (/mods): the list with its kind filter and search, and a page per project.
// Shared by build.mjs (baked into the pages) and mods-page.js (refreshed from the published index).
// The index's format is docs/api/mod-index.md in the repository.
import { markdown, escape } from "./markdown.js";

export const INDEX_URL = "https://ludovic111.github.io/horizon-mods/index.json";
const REGISTRY = "https://github.com/ludovic111/horizon-mods";
export const SUBMIT = "/mods/submit";
// The submission form's service (crates/horizon-mod-registry), behind the site's own address.
export const API = "/api/mods";
export const SCHEMA = 1;

// The kinds the site shows, in the filter's order. Kinds a newer index adds are left out until the site knows them.
export const KINDS = [
  ["mod", "Mods"],
  ["modpack", "Modpacks"],
  ["resourcepack", "Resource packs"],
  ["shader", "Shader packs"],
];
const KIND_NAMES = { mod: "Mod", modpack: "Modpack", resourcepack: "Resource pack", shader: "Shader pack" };

// What a capability lets a mod do, as the game and the launcher say it (horizon_mods::Capability::describe).
export const CAPABILITIES = {
  "game-read": "see your position, health and the time",
  chat: "show you chat messages",
  hud: "draw text on your screen",
  storage: "save its own settings to disk",
  registries: "add blocks and items to your worlds",
};

// The index's projects this site can show, or null when it isn't an index it reads.
export function readIndex(index) {
  if (!index || typeof index !== "object" || index.schema > SCHEMA || !Array.isArray(index.projects)) return null;
  const known = new Set(KINDS.map(([k]) => k));
  return index.projects.filter((p) => known.has(p.kind) && /^[a-z0-9_]{1,64}$/.test(p.id) && p.versions?.length);
}

const https = (url) => (typeof url === "string" && url.startsWith("https://") ? url : "");
const size = (bytes) => (bytes >= 1 << 20 ? `${(bytes / (1 << 20)).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1024))} KB`);
const trust = (p) =>
  p.key ? `<span class="badge badge-signed" title="Every file is signed with the author's key">Signed</span>` : `<span class="badge badge-unsigned" title="The file is pinned by its checksum, not signed">Unsigned</span>`;
const initial = (p) => escape((p.name || p.id).trim().charAt(0).toUpperCase());
const icon = (p) =>
  https(p.icon) ? `<img class="mod-icon" src="${escape(p.icon)}" alt="" width="48" height="48" loading="lazy">` : `<span class="mod-icon" aria-hidden="true">${initial(p)}</span>`;

export function matches(p, kind, text) {
  if (kind !== "all" && p.kind !== kind) return false;
  const hay = `${p.id} ${p.name} ${p.summary} ${(p.authors || []).join(" ")}`.toLowerCase();
  return text.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w));
}

// The list: a row per project, newest version and kind on the right.
export function listHtml(projects) {
  if (!projects.length) return `<p class="lede mod-empty">Nothing here yet. <a href="${SUBMIT}">Submit yours</a>.</p>`;
  return projects
    .map(
      (p) => `<a class="mod-row" href="/mods/${p.id}" data-kind="${escape(p.kind)}" data-search="${escape(`${p.id} ${p.name} ${p.summary} ${(p.authors || []).join(" ")}`.toLowerCase())}">
  ${icon(p)}
  <span class="mod-text"><span class="mod-name">${escape(p.name)}</span><span class="mod-summary">${escape(p.summary || "")}</span></span>
  <span class="mod-side"><span class="badge">${escape(KIND_NAMES[p.kind] || p.kind)}</span><span class="mod-version">${escape(p.versions[0].version)} · ${escape((p.authors || []).join(", "))}</span></span>
</a>`,
    )
    .join("\n");
}

function installHtml(p, v) {
  const file = escape(v.file.name);
  const cli = `horizon-launcher mods install --source horizon ${p.id}`;
  const where = {
    mod: `put it in <code>horizon-mods/</code> beside <code>options.txt</code> in the game folder, then turn it on in the game: Options &gt; Mods`,
    resourcepack: `put it in <code>resourcepacks/</code> beside <code>options.txt</code>, then turn it on in Options &gt; Resource Packs`,
    modpack: `a modpack lists registry mods: install them one by one, or wait for the launcher to install modpacks`,
    shader: `Horizon's own shader packs aren't installed yet`,
  }[p.kind];
  return `<ol class="mod-steps">
  <li>In the launcher: <strong>Mods</strong>, pick <strong>Horizon</strong> beside Modrinth, find ${escape(p.name)} and press <strong>Install</strong>. It checks the file's checksum${p.key ? " and signature" : ""} first.</li>
  <li>From a terminal or an agent: <code>${escape(cli)}</code></li>
  <li>By hand: download <a href="${escape(https(v.file.url))}">${file}</a> (${size(v.file.size)}), check its SHA-256 below, and ${where}.</li>
</ol>`;
}

// GitHub's rule for a login: letters, digits and single dashes, at most 39.
const LOGIN = /^(?!-)(?!.*--)[A-Za-z0-9-]{1,39}(?<!-)$/;

// Who publishes it: the GitHub accounts the registry checked when the project was submitted.
export function ownersHtml(p) {
  const owners = (p.owners || []).filter((o) => LOGIN.test(o));
  if (!owners.length) return "";
  const links = owners
    .map((o) => `<a class="mod-owner" href="https://github.com/${o}"><img src="https://avatars.githubusercontent.com/${o}?s=48" alt="" width="24" height="24" loading="lazy">@${o}</a>`)
    .join(" ");
  return `<p class="meta mod-owners">Published by ${links}</p>`;
}

// One project's page.
export function projectHtml(p) {
  const v = p.versions[0];
  const caps = v.capabilities || [];
  const capsHtml = caps.length
    ? `<ul class="mod-caps">${caps.map((c) => `<li>May ${escape(CAPABILITIES[c] || c)}</li>`).join("")}</ul>`
    : `<p>It asks for nothing beyond writing its own log.</p>`;
  const requires = (v.requires || []).length
    ? `<h2 class="heading">Its mods</h2><ul class="mod-caps">${v.requires.map((r) => `<li><a href="/mods/${escape(r.id)}">${escape(r.id)}</a> ${escape(r.version)}</li>`).join("")}</ul>`
    : "";
  const versions = p.versions
    .map((x) => `<li><strong>${escape(x.version)}</strong> <span class="mod-version">${escape(x.published || "")} · for ${escape(x.game)}${x.api ? ` · API ${escape(x.api)}` : ""}</span>${x.changelog ? `<div class="prose">${markdown(x.changelog)}</div>` : ""}</li>`)
    .join("");
  return `<article class="mod-page" data-id="${escape(p.id)}">
  <header class="mod-head">${icon(p)}<div><p class="kicker"><span>${escape(KIND_NAMES[p.kind] || p.kind)}</span>${escape((p.authors || []).join(", "))}</p>${ownersHtml(p)}
  <h1 class="display page-title">${escape(p.name)}</h1><p class="lede">${escape(p.summary || "")}</p>
  <p class="mod-badges">${trust(p)} <span class="badge">${escape(p.license)}</span> <span class="badge">${escape(v.version)}</span>${v.side ? ` <span class="badge">${escape(v.side)}</span>` : ""}</p></div></header>
  ${p.description ? `<div class="prose">${markdown(p.description)}</div>` : ""}
  ${p.kind === "mod" ? `<h2 class="heading">What it may do</h2>${capsHtml}<p class="meta">Mods run in a sandbox with no file, network or clock access. The game refuses a mod whose code uses more than this list.</p>` : ""}
  ${requires}
  <h2 class="heading">Install</h2>
  ${installHtml(p, v)}
  <h2 class="heading">File</h2>
  <dl class="mod-file"><dt>Name</dt><dd><a href="${escape(https(v.file.url))}">${escape(v.file.name)}</a></dd><dt>Size</dt><dd>${size(v.file.size)}</dd><dt>SHA-256</dt><dd><code>${escape(v.file.sha256)}</code></dd>${p.key ? `<dt>Signing key</dt><dd><code>${escape(p.key)}</code></dd>` : ""}</dl>
  <h2 class="heading">Versions</h2>
  <ul class="mod-versions">${versions}</ul>
  <p class="mod-links">${https(p.source) ? `<a class="link-arrow" href="${escape(p.source)}">Source code<span aria-hidden="true">→</span></a>` : ""}
  <a class="link-arrow" href="${REGISTRY}/blob/main/mods/${escape(p.id)}.toml">Its entry in the registry<span aria-hidden="true">→</span></a>
  <a class="link-arrow" href="${REGISTRY}/issues/new?template=report.yml&amp;title=${escape(p.id)}">Report it<span aria-hidden="true">→</span></a></p>
</article>`;
}

// /mods/submit: the form. Its script (mods-submit.js) signs in, then sends the page and the file to the service, which
// checks the file and opens the pull request the reviewers read.
const field = (name, label, input, hint = "") =>
  `<label class="mod-field" for="f-${name}"><span class="mod-label">${label}</span>${input}${hint ? `<span class="meta">${hint}</span>` : ""}</label>`;
const text = (name, attrs = "") => `<input class="mod-search" id="f-${name}" name="${name}" ${attrs}>`;

export function submitHtml() {
  return `<p class="kicker"><span>Mods</span>Submit</p>
        <h1 class="display page-title">Submit to the registry</h1>
        <p class="lede">A mod, a modpack of registry mods, or a resource pack made for Horizon. The form checks the file and opens a pull request; a reviewer reads it, its source and the checks' report, and merging publishes it to this site and the launcher.</p>
        <p class="meta">First time? <a href="/docs/publish-a-mod">Publish a mod</a> walks through building, checking and signing the file, and <a href="/docs/make-a-mod">Make a mod</a> starts from the beginning.</p>
        <div class="mod-auth" id="submit-auth" aria-live="polite">
          <p class="meta no-script">The form needs JavaScript. You can also <a href="${REGISTRY}">open a pull request by hand</a>.</p>
        </div>
        <form class="mod-form" id="submit-form" hidden>
          <fieldset><legend class="heading">The file</legend>
          ${field("kind", "Kind", `<select class="mod-search" id="f-kind" name="kind">${KINDS.map(([k, label]) => `<option value="${k}"${k === "shader" ? " disabled" : ""}>${label.replace(/s$/, "")}${k === "shader" ? " (opens with Horizon's own shader format)" : ""}</option>`).join("")}</select>`)}
          ${field("file", "File", `<input id="f-file" name="file" type="file" accept=".hmod,.hpack,.zip" required>`, "A .hmod, a .hpack or a resource pack .zip, at most 64 MB. Run <code>cargo horizon-mod check</code> on it first.")}
          ${field("id", "Id", text("id", `required pattern="[a-z0-9_]{1,64}" placeholder="my_mod" autocomplete="off"`), "Its manifest's id: a-z, 0-9 and _. A resource pack's is yours to pick.")}
          ${field("version", "Version", text("version", `required pattern="[A-Za-z0-9][A-Za-z0-9._\\-]{0,63}" placeholder="1.0.0"`))}
          ${field("game", "For Minecraft", text("game", `required value="26.3"`))}
          ${field("changelog", "What changed", `<textarea class="mod-search" id="f-changelog" name="changelog" rows="3"></textarea>`, "Markdown.")}
          </fieldset>
          <fieldset><legend class="heading">Its page</legend>
          ${field("name", "Name", text("name", `required maxlength="64"`))}
          ${field("summary", "Summary", text("summary", `required maxlength="160"`), "One line for the list.")}
          ${field("description", "Description", `<textarea class="mod-search" id="f-description" name="description" rows="6"></textarea>`, "Markdown.")}
          ${field("authors", "Authors", text("authors", `placeholder="Alice, Bob"`), "Separated by commas.")}
          ${field("license", "Licence", text("license", `required placeholder="MIT"`), "An SPDX expression.")}
          ${field("source", "Source code", text("source", `required type="url" pattern="https://.*" placeholder="https://github.com/you/your-mod"`), "Reviewers read it: the file must be built from it.")}
          ${field("icon", "Icon", text("icon", `type="url" pattern="https://.*" placeholder="https://…/icon.png"`), "Optional, a square PNG.")}
          </fieldset>
          <fieldset><legend class="heading">Signature</legend>
          <p class="meta">Optional, and recommended: with your <a href="https://jedisct1.github.io/minisign/">minisign</a> key on the project, every later version must be signed with it, so nobody else can publish an update in your name.</p>
          ${field("key", "Public key", text("key", `placeholder="RW…" autocomplete="off"`), "A new project's. Changing a published project's key is a pull request by hand.")}
          ${field("signature", "Signature", `<input id="f-signature" name="signature" type="file" accept=".minisig">`, "The file's .minisig.")}
          </fieldset>
          <label class="mod-check"><input type="checkbox" id="f-mine" required> I made it, or I may publish it under this licence, and its source is at the address above.</label>
          <p class="mod-actions"><button class="button primary" type="submit" id="f-send">Submit for review</button></p>
          <div class="mod-result" id="submit-result" role="status" aria-live="polite"></div>
        </form>`;
}

// The service's "entry" from the form's values (strings), as it reads it.
export function entryFrom(v) {
  const optional = (x) => (x && x.trim() ? x.trim() : undefined);
  return {
    id: v.id.trim(),
    kind: v.kind,
    name: v.name.trim(),
    summary: v.summary.trim(),
    description: v.description || "",
    authors: (v.authors || "").split(",").map((a) => a.trim()).filter(Boolean),
    license: v.license.trim(),
    source: v.source.trim(),
    icon: optional(v.icon),
    key: optional(v.key),
    signature: optional(v.signature),
    version: v.version.trim(),
    game: v.game.trim(),
    changelog: v.changelog || "",
  };
}
