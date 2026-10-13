# Horizon's website

One static page in the launcher's design: the tokens of `crates/horizon-theme` (palette,
spacing, radii, motion), its bundled fonts, and the launcher's sidebar, rows and buttons.
Plain HTML, CSS and JavaScript; no framework and no runtime dependencies.

## Layout

```
build.mjs          renders dist/ (no dependencies)
serve.mjs          a tiny static server for local checks
src/index.html     the page, with {{placeholders}} build.mjs fills
src/404.html       the not-found page
src/style.css      the design tokens and every style
src/main.js        download button, patch notes, roadmap, links, nav, tabs, lightbox
src/markdown.js    a small Markdown renderer (escapes everything), shared by build and page
public/            fonts, screenshots, background, icons, og.png: copied as is
scripts/assets.mjs rebuilds public/ from the repo (fonts to WOFF2, images to WebP)
scripts/shots.mjs  Playwright screenshots at every width, and og.png
Dockerfile, Caddyfile, railway.json   the Railway service
LEGAL.md           every asset, its source and licence
docs/widths/       full-page screenshots at 360–1920 px, a 1250×700 window and the open phone menu
```

## What comes from where

- **Built in** (`build.mjs`): the parity table, its score, harness count and audit date,
  and the roadmap, all from `README.md`; the Discord and Sponsor links from
  `launcher/links.json`; the latest releases, when GitHub answers. Locally these come from
  the checkout; on Railway, which builds only this folder, from `main` on GitHub. The build
  fails if the README has no parity table or roadmap, so a broken README never deploys.
- **Fetched by the page**: the releases (the download button picks the newest
  `launcher-v*` release's asset for the visitor's OS, and the three latest game releases'
  notes are rendered), the roadmap from the raw README on `main`, and `links.json`. Each
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
npm run shots      # docs/widths/*.png and public/og.png
```

Line lengths stay between 45 and 80 characters a line from 430 px up; on 360 and 390 px
phones the narrowest paragraphs (the roadmap's) fall to about 36–40, which is what 15 px text
allows in that width. `npm run shots` also reports horizontal overflow, tap targets under 44 px on touch widths
and characters per line, for 360 to 1920 px wide and a 1250×700 window.

## Railway

Create a service from the GitHub repo with **root directory `website`**. `railway.json`
selects the Dockerfile: a Node stage runs `build.mjs` (reading the README from `main`), and
a Caddy stage serves `dist/` on `$PORT` with compression, cache headers, a strict
Content-Security-Policy and the 404 page. Its watch patterns also redeploy when the README
or `launcher/links.json` change on main. No variables are needed; an optional
`GITHUB_TOKEN` (read-only, public repos) lets the build bake the releases even when
GitHub's anonymous rate limit is hit.

## The background

`public/background.webp` is Horizon's own frozen landscape, cut from the interface-free part
of `docs/media/menu-title.png`. The launcher's prismal background is not used, and the
launcher's screenshots are left out because they show it: its licence (backgrounds.supply)
is unconfirmed. The atmosphere is one replaceable file; see LEGAL.md.

## Rules the site keeps

- The disclaimer, "Not an official Minecraft product. Not approved by or associated with
  Mojang or Microsoft.", is in the hero, the footer and the 404 page.
- No Minecraft logo or wordmark; "Minecraft" appears only descriptively ("for Minecraft:
  Java Edition 26.3"). No game files are offered or linked.
- Parity is worded as the README words it: 100% of what the harnesses compare.
- No cookies, analytics, trackers or third-party fonts and scripts.
