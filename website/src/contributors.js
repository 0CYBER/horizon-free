// The contributors: GitHub's list for the repository, without bots, the owner first. Shared by
// build.mjs (baked into the page) and main.js (refreshed from GitHub).
import { escape } from "./markdown.js";

const OWNER = "ludovic111";
const AVATARS = "https://avatars.githubusercontent.com/";

// Reduces GitHub's contributor objects to what the page shows; bots (type Bot, or a login
// ending in [bot]) and anything without a login or a profile address are dropped.
export function contributorsFrom(list) {
  if (!Array.isArray(list)) return [];
  return list
    .filter((c) => c && typeof c.login === "string" && c.login && c.type !== "Bot" && !/\[bot\]$/i.test(c.login))
    .map((c) => ({
      login: c.login,
      html_url: typeof c.html_url === "string" ? c.html_url : `https://github.com/${c.login}`,
      avatar_url: typeof c.avatar_url === "string" ? c.avatar_url : "",
      contributions: Number(c.contributions) || 0,
    }))
    .sort((a, b) => (b.login === OWNER) - (a.login === OWNER) || b.contributions - a.contributions);
}

// GitHub serves avatars at any size: 96 px is twice what the page draws, for sharp screens.
export function avatar(url) {
  if (!url.startsWith(AVATARS)) return "";
  const u = new URL(url);
  u.searchParams.set("s", "96");
  return u.href;
}

export function contributorsHtml(list) {
  return list
    .map((c) => {
      const src = avatar(c.avatar_url);
      const count = `${c.contributions.toLocaleString("en-US")} ${c.contributions === 1 ? "commit" : "commits"}`;
      return `<li><a class="contributor" href="${escape(c.html_url)}">${src ? `<img class="contributor-avatar" loading="lazy" decoding="async" src="${escape(src)}" width="48" height="48" alt="">` : `<span class="contributor-avatar" aria-hidden="true"></span>`}<span class="contributor-name">${escape(c.login)}</span><span class="contributor-count">${count}</span></a></li>`;
    })
    .join("");
}
