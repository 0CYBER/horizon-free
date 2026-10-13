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
(releases and patch notes) and `raw.githubusercontent.com` (the README's roadmap and
`launcher/links.json`), both on Horizon's own repository.

## Fonts

| File | Source | Licence |
| --- | --- | --- |
| `public/fonts/Geist-Regular.woff2`, `Geist-Medium.woff2`, `Geist-SemiBold.woff2`, `GeistMono-Regular.woff2` | Geist and Geist Mono, from `crates/horizon-theme/fonts` (converted from TTF to WOFF2) | SIL Open Font License 1.1, shipped as `public/fonts/OFL-Geist.txt` |
| `public/fonts/InstrumentSerif-Regular.woff2` | Instrument Serif, from `crates/horizon-theme/fonts` (converted from TTF to WOFF2) | SIL Open Font License 1.1, shipped as `public/fonts/OFL-InstrumentSerif.txt` |

The fonts are self-hosted; nothing is fetched from a font service.

## Icons and the social preview

| File | Source | Licence |
| --- | --- | --- |
| `public/favicon.svg`, `public/favicon-32.png`, `public/apple-touch-icon.png` | Horizon's logo, `docs/media/logo.svg` (an original drawing of a sunset over blocks) | MIT, Horizon's original work |
| `public/og.png` | Rendered by `scripts/shots.mjs` from `public/og.html`: the Instrument Serif wordmark over `public/media/hero-1200.webp` | MIT for the layout; the fonts and the hero as listed here |

No Minecraft logo or wordmark appears anywhere on the site, in images or as text styling.
GitHub and Discord appear only as plain text links, without their logos.

## The hero

| File | Source | Licence |
| --- | --- | --- |
| `public/media/hero.webp`, `hero-1200.webp`, `hero-portrait.webp` | `docs/media/sunset.jpg`, darkened slightly and, for phones, cropped by `scripts/assets.mjs` | A screenshot of Horizon's client, as listed under Screenshots |

The grain over the hero is drawn by the stylesheet (an SVG noise filter inlined as a data URI),
not an image file. The launcher's own background (`launcher/assets/background.jpg`, from
backgrounds.supply) is **not** used: its licence is unconfirmed. For the same reason the
launcher's screenshots, which show it, are not on the site; the launcher section shows
Horizon's own menus instead. No image from backgrounds.supply or prismal is used anywhere.

## Screenshots

All captured in Horizon's Rust client, with vanilla resources prepared from the maintainer's
own copy of Minecraft: Java Edition 26.3, and no mods or shaders. Converted to WebP at 1600
and 800 pixels wide by `scripts/assets.mjs`.

| Files in `public/media/` | Source | What it shows |
| --- | --- | --- |
| `menu-title*.webp` | `docs/media/menu-title.png` | Horizon's own title screen (no Minecraft logo or splash text) |
| `menu-worlds*.webp` | `docs/media/menu-worlds.png` | Horizon's own world list |
| `menu-options*.webp` | `docs/media/menu-options.png` | Horizon's own options screen |
| `menu-pause*.webp` | `docs/media/menu-pause.png` | Horizon's own pause screen |
| `settings*.webp` | `docs/media/settings-simple.png` | Horizon's own Video settings, Simple view |
| `hero*.webp`, `sunset*.webp`, `village*.webp`, `survival*.webp`, `nether*.webp`, `night*.webp`, `underwater*.webp` | `docs/media/*.jpg` | In-game views; none shows a Mojang logo or splash text |

Horizon's interface in these images is Horizon's original work (MIT). The block, mob and
item textures and the HUD sprites drawn in them belong to Mojang and Microsoft; they are
shown as screenshots of the game, as the
[Minecraft Usage Guidelines](https://www.minecraft.net/en-us/usage-guidelines) allow for
fan websites about the game, with the disclaimer they require. The site offers no game
files: players need their own legally obtained copy of Minecraft: Java Edition.

## Text

The parity table and the roadmap are generated from Horizon's README, and the patch notes
come from Horizon's GitHub releases: Horizon's own text, MIT.
