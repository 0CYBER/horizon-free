# Horizon Free on macOS

This checkout is `0CYBER/horizon-free` at
`5e3a83d63b647af0a7114e0bc0732d66089dc959` (launcher 0.2.12).
The macOS build uses the fork's existing launcher and packaging scripts.

## Build for Apple Silicon

Install Xcode command line tools and Rust, then run from this repository:

```sh
rustup toolchain install 1.99.1 --profile minimal --no-self-update
cargo +1.99.1 build --manifest-path launcher/Cargo.toml --release --locked \
  --features gpui_platform/runtime_shaders
python3 -m venv .venv
.venv/bin/python -m pip install dmgbuild==1.6.5
DMGBUILD="$PWD/.venv/bin/dmgbuild" packaging/macos/package.sh \
  launcher/target/release/horizon-launcher 0.2.12 dist arm64
```

The macOS UI dependency needs a newer Rust compiler than the repository's
stated minimum. Runtime shaders avoid requiring Apple's optional Metal
compiler download. The command explicitly selects a compiler without changing
the default Rust toolchain.

The output is `dist/Horizon.app`, an app ZIP, and a disk image. The default
packager signs the app ad hoc; distributing a notarized app requires your own
Apple Developer ID. See `packaging/macos/README.md` for that separate process.

## Use an existing Minecraft installation

Horizon is a native Rust game with its own launcher. Open `Horizon.app` to run
it. TLauncher supplies an existing Minecraft installation for resource
preparation; it does not launch the Horizon executable as a Java profile.

In Horizon's Settings, choose **Minecraft folder** and select the installation
that contains the Minecraft version pinned by the installed Horizon build.
On macOS, the usual folder is:

```text
~/Library/Application Support/minecraft
```

Use **Command-Shift-G** in the folder picker to enter that path. The standard
layout contains `versions/26.3/26.3.jar`, `versions/26.3/26.3.json`, `libraries`,
and `assets`. Horizon checks the version metadata and client jar against its
pinned checksums. A renamed Fabric profile such as `Fabric 26.3` is not
automatically recognized by this checkout; install the matching vanilla
version or provide a local folder with the standard layout.

During verification, a checksum-matched `Fabric 26.3` jar was exposed through
`.local/minecraft-source` using links to the existing installation and verified
Mojang version metadata. That helper folder is specific to the test machine,
is ignored by Git, and is not included in a fresh clone. If it already exists
in a local setup, choose it as the Minecraft folder instead.

Install a Horizon game version, then prepare its resources and play. Minecraft
files and prepared resources stay in ignored local folders and are not
included in the app or disk image.

The launcher retains the fork's existing account behavior. Authenticated
multiplayer still depends on the server's account requirements.

## Verified on this Mac, 4 November 2026

- Release build: Rust 1.99.1, native arm64, runtime Metal shaders.
- Bundle: valid ad hoc signature; executable links only macOS system libraries.
- Distribution: app ZIP inspected; disk-image checksum verified by `hdiutil`.
- Launcher window: rendered successfully; its built-in UI benchmark completed.
- Game: installed and prepared `nightly-2026-11-04-3fb4c31` using the existing
  TLauncher client, libraries, Java 25, and assets.
- Launch: the packaged launcher opened a new disposable creative world on the
  Apple M4 Metal renderer, rendered 129 total frames (120 in the world), and
  exited successfully. The smoke report recorded `error: null` and 49 chunks.

Verification used fresh temporary game and launcher homes, with self-updates,
keychain access, telemetry, and audio disabled. Existing worlds were not used.
Audio and authenticated multiplayer were not tested.
