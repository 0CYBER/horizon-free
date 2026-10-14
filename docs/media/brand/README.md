# Horizon's mark and icons

**The mark.** An H whose crossbar is the horizon: the line runs edge to edge in Horizon's violet,
the stems above it in full strength and their reflection below at a third. It is drawn on whole
units of a 16-unit grid, so it is sharp at 16 px and holds at 1024 px. `docs/design-system.md`
places it in the design system; `candidates/` keeps the two alternatives that were shown with it.

## Masters (edit these)

| File | What |
|------|------|
| `icon.svg` | the app icon on the 16 grid: the full square with 22 % corners (Windows, Linux, favicons) |
| `icon-macos.svg` | the same tile on Apple's 1024 grid (824 px tile, 185 px corners, transparent margin) |
| `avatar.svg` | full-bleed square for services that round or crop the image (GitHub, Discord) |
| `avatar-kira.svg` | the bot's avatar: the mark on violet, so Kira reads apart from the project |
| `mark.svg`, `mark-dark.svg` | the mark alone, for light and for dark backgrounds |
| `social/card.html`, `social/banner.html` | the GitHub social preview and the Discord banner, as pages |

## Generated (don't edit)

`cargo run -p xtask -- icons` (needs `rsvg-convert` from librsvg) renders the masters into:

| Output | Used by |
|--------|---------|
| `packaging/icons/hicolor/<size>/apps/horizon.png` (16–512) and `scalable/apps/horizon.svg` | Linux: copy the tree into `~/.local/share/icons/hicolor` (or `/usr/share/icons/hicolor`) |
| `packaging/linux/horizon-launcher.desktop`, `horizon.desktop` (hand-written) | Linux: the launcher installs both and the hicolor icons itself, under `$XDG_DATA_HOME` (`~/.local/share`), with its own path in `Exec` (`launcher/src/desktop.rs`; `horizon-launcher desktop-entry --remove` takes them out). Wayland matches the windows' app ids `horizon-launcher` and `horizon` |
| `packaging/icons/horizon.ico` | Windows: embedded in `horizon-client.exe`, its tools and `horizon-launcher.exe` as icon resource 1 (`build.rs` of each, Windows builds only) |
| `packaging/icons/horizon.icns` | macOS: `Horizon.app/Contents/Resources/horizon.icns`, packaged with the .dmg by `packaging/macos/package.sh` |
| `crates/horizon-theme/icons/horizon-64.png` | the game window's icon at run time (X11 and Windows title bars, `window::icon`) |
| `website/public/favicon.svg`, `favicon-32.png`, `apple-touch-icon.png` | the website |
| `icon-1024.png`, `icon-macos-1024.png` | previews, store listings |
| `social/github-avatar-500.png` | GitHub: the organisation or account avatar |
| `social/discord-icon-512.png` | Discord: the server icon |
| `social/kira-avatar-512.png` | Discord: Kira's avatar |

The two pages in `social/` are screenshots, at their exact size, with Playwright from the
website's dev dependencies:

```sh
cd website && npm install
node -e 'const {chromium}=require("playwright");(async()=>{const b=await chromium.launch();for(const[f,w,h,o]of[["card",1280,640,"github-social-1280x640"],["banner",960,540,"discord-banner-960x540"]]){const p=await b.newPage({viewport:{width:w,height:h}});await p.goto("file://"+require("path").resolve("../docs/media/brand/social/"+f+".html"));await p.waitForTimeout(300);await p.screenshot({path:"../docs/media/brand/social/"+o+".png"})}await b.close()})()'
```

## Drawn in code

| Where | What |
|-------|------|
| `crates/horizon-client/src/title_edition.rs` | the R, U and S of "RUST EDITION" under the classic title screen's logo: Horizon's own glyphs (MIT), rectangles in that file. The banner is rebuilt at load time from the player's own `textures/gui/title/edition.png` (its "EDITION" and "T" carried over, its shades sampled), so no Mojang pixels are in the repository; a resource pack's banner is left as drawn |

## Uploading (the accounts are Ludovic's)

- **GitHub avatar**: github.com/settings/profile (or the organisation's settings), *Edit* the
  picture, upload `social/github-avatar-500.png`.
- **GitHub social preview**: the repository's *Settings › General › Social preview › Edit*,
  upload `social/github-social-1280x640.png`.
- **Discord server icon and banner**: *Server Settings › Server Profile*: icon
  `social/discord-icon-512.png`; banner `social/discord-banner-960x540.png` (the banner needs
  boost level 2).
- **Kira's avatar**: discord.com/developers/applications › Kira › *Bot* › icon, upload
  `social/kira-avatar-512.png` (or *General Information › App Icon* for the application too).
