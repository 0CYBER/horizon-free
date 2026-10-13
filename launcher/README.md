# Horizon launcher

A small desktop app that installs Horizon builds, prepares them from your own copy of
Minecraft, and starts the client or a dedicated server. Built with [GPUI](https://www.gpui.rs),
Zed's UI framework; it has its own Cargo workspace and is not part of the game's builds.

![Play](docs/play.png)

## What it does

- **Versions**: lists the published releases of ludovic111/horizon, installs the build for
  this platform into its own folder, and removes it again.
- **Play**: prepares resources as `PLAY.txt` describes (your launcher's 26.3 install, the
  26.3 server jar from that version's own json, Python 3.10+ and Java 25), then starts the
  client, or a server (`--dedicated --nogui`) in a folder you choose, with a live console.
- **Patch notes**: each release's notes, newest first, with a link to GitHub Sponsors.
- **What's next**: the README's roadmap, from main.

Data lives in the platform's data directory (`~/.local/share/horizon-launcher` on Linux,
`~/Library/Application Support/horizon-launcher` on macOS, `%APPDATA%\horizon-launcher` on
Windows): `versions/<tag>/`, `server-jars/`, `server/` and `settings.json`.

## Building

    cargo build --release

On NixOS, `nix-shell` in this folder provides the libraries GPUI links to (Wayland, X11,
xkbcommon, Vulkan, fontconfig). macOS needs Xcode's command line tools. The `launcher`
workflow builds macOS and Windows binaries as artifacts.

Geist, Geist Mono and Instrument Serif (SIL Open Font License, `assets/fonts/OFL-*.txt`) are
bundled so the type is the same on every platform.

Background image (`assets/background.jpg`): backgrounds.supply (free preview), licence to confirm before release.

## Screenshots

![Server](docs/server.png)
![Versions](docs/versions.png)
![Patch notes](docs/patch-notes.png)
