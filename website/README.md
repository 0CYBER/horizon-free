# Horizon's website

Live at **https://playhorizon.dev**.

One static page in Horizon's design language (`docs/design-system.md`), with the tokens of
`crates/horizon-theme` (generated into `src/tokens.css` by `cargo run -p xtask -- tokens`):
Geist and Geist Mono, cool neutrals in a light and a dark appearance (the system's, or the
toggle's) and one violet accent, numbered index lists with one-pixel rules, and short, eased
motion. A headline and the download, then a scroll through what it is, the screenshots, the parity grid, the launcher, the patch
notes, the community links and the contributors. Plain HTML, CSS and JavaScript; no framework and
no runtime dependencies.

## Layout

```
build.mjs          renders dist/ (no dependencies)
serve.mjs          a tiny static server for local checks
src/index.html     the page, with {{placeholders}} build.mjs fills
src/404.html       the not-found page
src/privacy.html   what the launcher can send when players allow it, retention, deletion (/privacy)
src/tokens.css     the design tokens, generated from crates/horizon-theme (do not edit)
src/style.css      every style, on top of the tokens
src/main.js        download buttons, patch notes, links, contributors, nav and menu, tabs, lightbox, motion
src/markdown.js    a small Markdown renderer (escapes everything), shared by build and page
src/releases.js    the patch notes' markup, shared by build and page
src/contributors.js the contributors' filtering and markup, shared by build and page
public/            fonts, screenshots, icons, og.png, robots.txt, sitemap.xml: copied as is
scripts/assets.mjs rebuilds public/ from the repo (fonts to WOFF2, images to WebP)
scripts/shots.mjs  Playwright screenshots at every width, and og.png
Dockerfile, Caddyfile, railway.json   the Railway service
LEGAL.md           every asset, its source and licence
shots/             (gitignored) full-page screenshots at 360–1920 px, a 1250×700 window, the open phone menu
```

## What comes from where

- **Built in** (`build.mjs`): the parity table, its score, harness count and audit date,
  all from `README.md`; the Discord and Sponsor links from
  `launcher/links.json`; the latest releases and the contributors (`/repos/ludovic111/horizon/contributors`,
  bots left out, the owner first), when GitHub answers; without an answer the build still succeeds and
  the Contributors block and its menu entries stay hidden until the page loads them. Locally these come from
  the checkout; on Railway, which builds only this folder, from `main` on GitHub. The build
  fails if the README has no parity table, so a broken README never deploys.
- **Fetched by the page**: the releases (the download button picks the newest
  `launcher-v*` release's asset for the visitor's OS, and the three latest game releases'
  notes are rendered), the contributors (avatars come from `avatars.githubusercontent.com`),
  and `links.json` from `main`. Each
  falls back to what was built in, and the download button to the releases page.

## Run it locally

```sh
cd website
node build.mjs && node serve.mjs     # http://localhost:4173
```

Asset and screenshot scripts need the dev dependencies (sharp, wawoff2, Playwright):

```sh
npm install
GITHUB_TOKEN=$(gh auth token) node build.mjs   # optional token: bakes the releases past the API rate limit
npm run assets     # after a font, screenshot or logo changes in the repo
npm run shots      # shots/*.png (gitignored; SHOTS=dir to write elsewhere) and public/og.png
```

The screenshots are not committed: full-page PNGs at a dozen widths weigh about 17 MB.
Running text stays under 80 characters a line; on 360–430 px phones the narrowest paragraphs
fall to about 26–35, which is what that width allows. `npm run shots` also reports horizontal overflow, tap targets under 44 px on touch widths
and characters per line, for 360 to 1920 px wide and a 1250×700 window.

## Railway

The service deploys the `website` branch and answers on https://playhorizon.dev. Its
`*.up.railway.app` address redirects there with a 301 (the Caddyfile), so search engines index
one URL; the page also names it as canonical, in its Open Graph tags and in `sitemap.xml`.
Railway's healthcheck reaches `$PORT` under its own host name and is served normally.

Create a service from the GitHub repo with **root directory `website`**. `railway.json`
selects the Dockerfile: a Node stage runs `build.mjs` (reading the README from `main`), and
a Caddy stage serves `dist/` on `$PORT` with compression, cache headers, a strict
Content-Security-Policy and the 404 page. Its watch patterns also redeploy when the README
or `launcher/links.json` change on main. No variables are needed; an optional
`GITHUB_TOKEN` (read-only, public repos) lets the build bake the releases even when
GitHub's anonymous rate limit is hit.

## Rules the site keeps

- The disclaimer, "Not an official Minecraft product. Not approved by or associated with
  Mojang or Microsoft.", is under the headline, in the phone menu, the footer and the 404 page.
- No Minecraft logo or wordmark; "Minecraft" appears only descriptively ("for Minecraft:
  Java Edition 26.3"). No game files are offered or linked.
- Parity is worded as the README words it: 100% of what the harnesses compare.
- No cookies, analytics, trackers or third-party fonts and scripts.
- Motion stops under `prefers-reduced-motion`; nothing hijacks the scroll.
