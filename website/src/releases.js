// The patch notes: the latest game release in full, the ones before it folded. Shared by
// build.mjs (baked into the page) and main.js (refreshed from GitHub).
import { markdown, escape } from "./markdown.js";

const REPO = "ludovic111/horizon";

// "Horizon 0.2.3: Cave culling, weather" (before 0.2, "Horizon 2026.10.13: …") reads as the version and its line.
function parts(r) {
  const version = r.tag_name.replace(/^v/, "");
  const name = r.name || "";
  const line = name.includes(": ") ? name.slice(name.indexOf(": ") + 2) : name.replace(/^Horizon\s*/, "").replace(version, "").trim();
  return { version, line: line.charAt(0).toUpperCase() + line.slice(1), date: (r.published_at || "").slice(0, 10) };
}

export function releasesHtml(list) {
  if (!list.length) return "";
  const [latest, ...older] = list;
  const l = parts(latest);
  const head = `<article class="release-latest">
          <header class="release-head">
            <p class="eyebrow">Latest release<span aria-hidden="true"> · </span><time datetime="${l.date}">${l.date}</time></p>
            <h3 class="release-version">${escape(l.version)}</h3>
            ${l.line ? `<p class="release-line">${escape(l.line)}</p>` : ""}
            <a class="link-arrow" href="${escape(latest.html_url)}">Downloads and full notes<span aria-hidden="true">→</span></a>
          </header>
          <div class="prose release-body">${markdown(latest.body || "")}</div>
        </article>`;
  const rest = older
    .map((r) => {
      const o = parts(r);
      return `<details class="release">
            <summary><span class="release-tag">${escape(o.version)}</span><span class="release-sub">${escape(o.line)}</span><time datetime="${o.date}">${o.date}</time></summary>
            <div class="prose">${markdown(r.body || "")}<p><a href="${escape(r.html_url)}">Downloads and full notes on GitHub</a></p></div>
          </details>`;
    })
    .join("");
  return `${head}
        <div class="release-older">
          ${rest ? `<p class="release-older-title">Earlier releases</p>${rest}` : ""}
          <p class="release-more"><a class="link-arrow" href="https://github.com/${REPO}/releases">Every release on GitHub<span aria-hidden="true">→</span></a></p>
        </div>`;
}
