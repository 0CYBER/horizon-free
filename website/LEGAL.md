# The website's assets and their licences

Every file the site serves, where it comes from and the terms it is used under. The build
copies this file to `/LEGAL.md` on the site.

Horizon is not an official Minecraft product. It is not approved by or associated with
Mojang or Microsoft. Minecraft is a trademark of Mojang Synergies AB.

## Code

| File | Source | Licence |
| --- | --- | --- |
| `src/*.html`, `src/*.css`, `src/*.js`, `build.mjs`, `serve.mjs`, `scripts/*`, `Caddyfile`, `Dockerfile` | Written for this site | MIT, as the rest of Horizon ([LICENSE](https://github.com/ludovic111/horizon/blob/main/LICENSE)) |

The site loads no third-party code at runtime: no frameworks, CDNs, analytics, trackers or
cookies. The only requests to other hosts are the browser's calls to `api.github.com`
(releases and patch notes) and `raw.githubusercontent.com` (`launcher/links.json`), both on Horizon's own repository.

## Fonts

| File | Source | Licence |
| --- | --- | --- |
| `public/fonts/Geist-Regular.woff2`, `Geist-Medium.woff2`, `Geist-SemiBold.woff2`, `GeistMono-Regular.woff2` | Geist and Geist Mono, from `crates/horizon-theme/fonts` (converted from TTF to WOFF2) | SIL Open Font License 1.1, shipped as `public/fonts/OFL-Geist.txt` |
| `public/fonts/PixelifySans-Medium.woff2`, `PixelifySans-SemiBold.woff2`, `PixelifySans-Bold.woff2` | Pixelify Sans by The Pixelify Sans Project Authors (github.com/eifetx/Pixelify-Sans), from `crates/horizon-theme/fonts` (converted from TTF to WOFF2) | SIL Open Font License 1.1, shipped as `public/fonts/OFL-PixelifySans.txt` |

The fonts are self-hosted; nothing is fetched from a font service.

## Icons and the social preview

| File | Source | Licence |
| --- | --- | --- |
| `public/favicon.svg`, `public/favicon-32.png`, `public/apple-touch-icon.png` | Horizon's mark, `docs/media/brand/icon.svg` (an original drawing: an H whose crossbar is the horizon), rendered by `cargo run -p xtask -- icons` | MIT, Horizon's original work |
| `public/og.png` | Rendered by `scripts/shots.mjs` from `public/og.html`: the mark, a headline and the faint wordmark, in Geist | MIT for the layout; the fonts as listed here |

No Minecraft logo or wordmark appears anywhere on the site, in images or as text styling.
GitHub and Discord appear only as plain text links, without their logos.

## The pixel art

| Files in `public/media/` | Source | Licence |
| --- | --- | --- |
| `stone-light.png`, `stone-dark.png` | The page's stone texture, drawn in code by `scripts/art.mjs` from the page colours of `crates/horizon-theme` | MIT, Horizon's original work |
| `scene-light.png`, `scene-dark.png` | The hero's landscape (a dawn and a sunset with Horizon's sliced sun), drawn in code by `scripts/art.mjs`, the same drawing as the launcher's Home banner (`launcher/src/art.rs`) | MIT, Horizon's original work |

No Minecraft texture is used: the stone is not Minecraft's stone, only shades of the page colour.
The launcher's old background (`launcher/assets/background.jpg`, from backgrounds.supply, licence
never confirmed) is gone from the repository, so the launcher's screenshots no longer show it.

## Screenshots

All captured in Horizon's Rust client, with vanilla resources prepared from the maintainer's
own copy of Minecraft: Java Edition 26.3, at the default graphics settings and with no mods or shaders
(the game last captured on 2026-10-17, the menus on 2026-10-22). Converted to WebP at 1600 and 800 pixels wide
by `scripts/assets.mjs`.

| Files in `public/media/` | Source | What it shows |
| --- | --- | --- |
| `launcher*.webp` | `docs/media/launcher-play.png` | The Horizon launcher's Play page (no background image) |
| `settings*.webp` | `docs/media/settings-simple.png` | Video Settings, with Horizon's graphics buttons at the top |
| `sunset*.webp`, `village*.webp`, `survival*.webp`, `nether*.webp`, `night*.webp`, `underwater*.webp` | `docs/media/<name>.jpg` | In-game views of one fixed-seed world; none shows a Mojang logo or splash text |

Horizon's interface in these images is Horizon's original work (MIT). The block, mob and
item textures and the HUD sprites drawn in them belong to Mojang and Microsoft; they are
shown as screenshots of the game, as the
[Minecraft Usage Guidelines](https://www.minecraft.net/en-us/usage-guidelines) allow for
fan websites about the game, with the disclaimer they require. The site offers no game
files: players need their own legally obtained copy of Minecraft: Java Edition.

### The launcher's Servers page

| Files in `public/media/docs/` | Source | What it shows |
| --- | --- | --- |
| `servers-create.webp`, `servers-dashboard.webp`, `servers-invite.webp`, `servers-backups.webp` | Screenshots of the Horizon launcher's Servers page (a debug build on Linux, with made-up addresses, names and `@example.com` emails), converted to WebP | The Create a server dialog, a running server, a Tailscale invitation, the Backups tab |

They show only Horizon's own interface (MIT) and a default player head.

## Text

The parity table is generated from Horizon's README, and the patch notes
come from Horizon's GitHub releases: Horizon's own text, MIT. The documentation
(`docs/*.md` in this folder, served under `/docs`) is Horizon's own text, MIT.

## Contributors

The Contributors block lists the GitHub accounts that have committed to Horizon: login,
commit count and profile link, from GitHub's API. Their avatars are not stored in this repository;
the visitor's browser loads them from `avatars.githubusercontent.com`, each one the picture its
owner chose for their GitHub account. Bots are left out.
