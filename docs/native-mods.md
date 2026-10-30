# Native mods (experimental)

Native mods are **mixin bundles** that change the game's own Rust source before it is compiled, with
[horizon-mixins](https://github.com/horizon-rust/horizon-mixins) (MIT, by levzzz5154, made for Horizon). A bundle
is a `horizon-mixin.toml` and a few `.rs` helpers that hook into named functions of the game (HEAD, BEFORE_CALL,
RETURN, MODIFY_ARG, OVERWRITE...; the tool's README lists them). A port such as Veinminer is three HEAD hooks and
one module. Start a mod from [horizon-native-mod-template](https://github.com/horizon-rust/horizon-native-mod-template).

Native mods are native code with the player's rights, not sandboxed. The launcher turns them on only when the player
installs one (after a notice) and builds them only behind a red warning; the registry lists one only after a review; and
they play in a game of their own, built for the instance they are installed in.

## Pinned to a Nightly

A native mod builds on one exact Horizon commit: `[compatibility] horizon_revision` in its `horizon-mixin.toml`,
and `horizon` in the `horizon-native.json` of its registry file (`.hnmod`, `horizon_mods::index::native`). Mods
pin a **Nightly's commit** (or a release's). Every Nightly ships its source and keeps its commit for good as the
tag `nightly-source/<date>-<short>` ([release-flow.md](release-flow.md#nightly-channel)), so a pinned mod still
builds after its Nightly is replaced. All the mods of one build must pin the same commit; when they don't, the
launcher says which mod pins which, and the player installs versions made for the same Nightly. A bundle with no
pin builds on the commit of the installed version the build starts from.

### Mods on the hook points

A mod that hooks only Horizon's native hook points (`crates/horizon-client/src/game/native_hooks.rs`: empty
functions the game calls at fixed places, plus helpers, kept stable across releases) can name the version it
needs instead of pinning: `"hooks": 1` in its `.hnmod`'s `horizon-native.json`, the `NATIVE_HOOKS_API` of that
file when it was made. Hook points are only ever added: each addition raises `NATIVE_HOOKS_API`, and a mod builds
on any Horizon whose version is the one it needs or a later one. Its `horizon` is then only the commit it was tested on. The launcher builds it on the commit the other mods
pin, else on the installed version's commit (the game the player plays), after checking that commit's
`NATIVE_HOOKS_API` (read from GitHub) is at least the mod's; when it isn't and nothing pins, it builds on the commit the
mod was tested on. In its own copy of the mod it sets `horizon_revision` to the commit it builds on. Horizon's
official mods (Zoom, FPS counter, Keystrokes, Minimap, Recipes) are made this way, so any of them build together
with no re-release for each Nightly. The `mods` feature (WebAssembly mods) is passed to the build only when the source has it.

API 2 adds the item panel beside container screens and its recipe pages (`native_panel_items`,
`native_panel_recipes`, and the hook `native_panel_event` for the player's clicks, searches and keys over items), and
the game's items and recipes (`native_recipes`, `native_known_recipes`, `native_place_recipe`, `native_give`): what a
recipe viewer needs, in builds with or without the `mods` feature. A WebAssembly mod's panel shows only while no native
mod shows one.

## A bundle

A bundle is a folder with a `horizon-mixin.toml`. Besides its helpers and modules (horizon-mixins' README) it can
have its own crate, data files and a post-build step (horizon-mixins 0.6.0). The test bundle
[`launcher/tests/fixtures/native-crate-demo`](../launcher/tests/fixtures/native-crate-demo) uses all of them:

```
native-crate-demo/
  horizon-mixin.toml
  frame.rs                # helper, compiled into the game
  data/greeting.txt       # the bundle's data: native_data!("crate_demo", "greeting.txt")
  engine/                 # the mod's own crate
    Cargo.toml            # [dependencies] crc32fast = "1"
    data/salt.txt         # the crate's data: include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/salt.txt"))
    src/lib.rs
    src/bin/bake.rs       # the post-build step
```

```toml
[mod]
id = "crate_demo"
version = "0.1.0"

[[injection]]
id = "frame"
hook = "frame"            # a name from native-hooks.json, instead of `target`
hook_version = 1
kind = "HEAD"
helper = "frame.rs"
bindings = [{ name = "game", ty = "&mut crate::game::Game<'_>", value = "&mut *self" }]

[[crate]]
id = "engine"
path = "engine"           # holds Cargo.toml
# dependent = "horizon-client"   (the default: the workspace package that depends on it)

[[post_build]]
crate = "engine"
bin = "bake"
args = []
```

### Named hook points

`native-hooks.json` at the root of the source names each hook point and the version of its contract (signature,
when it is called, what it does): `{"name": "hud", "version": 1, "target": "horizon_client::game::native_hooks::Game<'_>::native_hud"}`.
An injection names `hook` and `hook_version` instead of `target`, and horizon-mixins resolves it from the catalog
of the source it applies to, so a hook point can move in the source without breaking the mods that use it. A name
the catalog lacks is `E_HOOK_UNKNOWN` (the error lists the names it has), a version it lacks `E_HOOK_VERSION`. A
contract is never changed: an incompatible change adds a version. Helpers (`native_text`, `native_player`…) are not
in the catalog; they are kept stable as the hook points are. A test (`catalog_matches_hook_points`) keeps the
catalog and `native_hooks.rs` in step.

### The mod's crate

A `[[crate]]` is a Cargo package of the bundle's, staged as `native-crates/<mod>/<crate>/` and added to the
workspace's members and to the `[dependencies]` of its `dependent`, so helpers call it as
`crate_demo_engine::checksum(…)`. Cargo compiles it once and keeps it: editing a helper, or switching another mod
on or off, recompiles `horizon-client` but not the mod's crate, so the heavy code of a big mod belongs there and
only the glue in helpers. A crate can't depend on `horizon-client` (Cargo forbids the cycle) or on any path or git
crate, and has no `Cargo.lock`, `[workspace]` or build of its own.

**Dependencies come from Horizon's `Cargo.lock`.** A crate names registry crates by version requirement
(`crc32fast = "1"`, `{ version, features, default-features, optional }`); each must already be in Horizon's lock
file in a version that matches, and builds in exactly that version. The lock file is the allow list: horizon-mixins
refuses a crate the lock lacks (`E_CRATE_DEPENDENCY`) and writes the mod's crates into the staged lock. A
dependency's features (its default ones included) may use other crates: `hashbrown = "0.15"` needs
`allocator-api2` and `equivalent`, which Horizon locks for other crates while it uses hashbrown without its defaults.
horizon-mixins lets `cargo update --workspace` add those links between crates already in the lock, and refuses any
crate it would add or change, naming it, so the game still builds with `--locked`. The launcher checks it again
(every downloaded package of the staged lock as in Horizon's: name, version, source, checksum), and a lock that
still misses something fails at `cargo fetch --locked` with a message saying so. The build stops with:

```
horizon-mixins failed (exit status: 1). The end of its output, the whole of it in …/native/build.log:
E_CRATE_DEPENDENCY: crate_lock needs rand ^0.9, which isn't in Horizon's Cargo.lock (…/horizon-mixin.toml)
```

**Features are shared with the game.** Cargo builds each locked crate once, with the union of the features everyone
asks for, so a feature a mod's crate turns on also applies to the game's own use of that crate. `serde_json`'s
`preserve_order`, for example, would change the key order of every JSON file the game writes. Ask only for
features that add something (new functions, new types), never ones that change existing behaviour.

### Data files

A bundle's `data/` folder (any files) is staged as `native-data/<mod>/` at the root of the workspace. A helper or
module compiles a file in with `horizon_client::native_data!("<mod id>", "path/in/data")` (`&'static [u8; N]`,
`crate::native_data!` inside the lib), since hook points API 3. A mod's crate keeps its own data in its folder and
reads it with `include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/…"))`. The registry's `.hnmod` may carry
both (`mixins/<id>/data/…`, `mixins/<id>/<crate>/data/…`).

### Post-build steps

A `[[post_build]]` names a bin of one of the mod's crates. After the game compiles, the launcher builds it with the
same profile into the same `native/target/`, copies it out at once, then runs it (at low priority, in mod order) in
the new build's version folder. A bin's file name doesn't include its crate, so horizon-mixins refuses a step bin
named like a workspace bin (`horizon-client`) or another mod's step bin, ignoring case (`E_POST_BUILD`); the copy
keeps an older bundle from running another mod's program or replacing the game. The step runs with:

| Variable | What |
|---|---|
| `HORIZON_NATIVE_MOD` | the mod's id |
| `HORIZON_NATIVE_VERSION_DIR` | the new build's version folder (also the working directory) |
| `HORIZON_NATIVE_DATA_DIR` | `<version folder>/native-data/<mod>/`, created: what the game reads at run time with `native_built_data("<mod>")` (or `horizon_client::native_data::built`) |
| `HORIZON_NATIVE_CACHE_DIR` | `<launcher data>/native/cache/<mod>/`, created, kept from one build to the next |

Its output goes to `native/build.log`. When it fails, so does the build ("crate_demo's post-build step failed …")
and the version folder is removed. It runs on the player's computer like the rest of the mod's code: only the
launcher runs it, never horizon-mixins.

## What the launcher does

In the window: install a native mod from the Mods page. The first one shows a notice once (the first build downloads
Rust and can take 20 to 30 minutes; later builds are faster) and turns on Settings → Game → **Native mods
(experimental)**, which stays off for players who never install one; `mods install` does the same. Then **Build**. The launcher names the mods, warns in red, and builds only from "I trust these mods: build"; the
row shows each step, the error if one fails, and the build the instance plays. From the command line:
`horizon-launcher native-build [FOLDER] [--instance NAME] [--update] --trust-native-code`. FOLDER is a bundle, an
unpacked `.hnmod`, or a folder of them; without it, the native mods turned on in the instance. It is not an MCP tool:
trusting native code is the player's decision, never an agent's.

1. **Rust.** The launcher's own toolchain ([`rust_toolchain`](../launcher/src/rust_toolchain.rs)): Rust 1.99.1's
   official `rustc`, `cargo` and `rust-std` (and `rust-mingw` on Windows), each pinned by SHA-256, downloaded once
   from static.rust-lang.org into `<launcher data>/rust/`, with its own Cargo home `<launcher data>/cargo/`. No
   rustup, nothing on the system; the player's own Rust is never used. Windows uses the `windows-gnu` host, which
   links without Visual Studio. `rust-mingw` only links, so on Windows the toolchain also brings MinGW-w64
   (`rust_toolchain::MINGW`: WinLibs' GCC 16.2.0 on MinGW-w64 14.0.0, MSVCRT like `rust-mingw`, one zip pinned by
   SHA-256, its `mingw64/` without the documentation) on the build's `PATH`: `ring`, `hidapi` and wasmtime compile C,
   and `windows-sys`' `raw-dylib` imports need `dlltool` and the assembler it runs. The linker is the system's on
   macOS (the Command Line Tools: the launcher asks for `xcode-select --install`) and Linux (`cc`).
   `horizon-launcher rust status|install|remove`.
2. **horizon-mixins**, at the commit the mods name (their `.hnmod`'s `mixins`), else `native_mods::MIXINS_REV`,
   built once with that cargo (`cargo install --locked --git … --rev …`) into `native/tools/<commit>/`.
3. **The source** of the pinned commit: GitHub's archive of it, once per commit (`native/source/<commit>/`, about
   63 MB unpacked), with the commit in `.horizon-revision`. Never the player's own checkout. Links in the
   archive are left out: the tool refuses them and nothing needs them to build.
4. **Mixins.** The bundles are copied first, so the build and its key use that copy whatever happens to the
   folder meanwhile. `horizon-mixins apply` stages the modded source; the launcher copies only the files that
   changed into `native/stage/`, so Cargo, which goes by file times, recompiles only the crates the mods touch.
   With mods' crates, their dependencies are checked against Horizon's `Cargo.lock` ("The mod's crate").
5. **Compile**: `cargo build --locked --profile fast [--features horizon-client/mods] -p horizon-client`, into
   `native/target/`. `fast` is the workspace's own profile: the release's optimizations without its link-time
   ones (`lto = false`, `codegen-units = 16`, incremental), so a rebuild recompiles and links only what changed.
   A revision whose `Cargo.toml` has no `fast` builds with `release`, the release's own command (game-build.yml);
   `native-mods.json` says which (`profile`). The mods feature is passed while the revision still has it. Cargo
   downloads the locked crates from crates.io (Horizon has no git dependencies). The compiler and the
   horizon-mixins build run at the lowest priority (`rust_toolchain::low_priority`: nice 19 and the idle I/O class
   on Linux, nice 19 on macOS, below-normal priority on Windows), so the desktop and a game in play stay smooth;
   on an otherwise idle computer they still use every core. The log is `native/build.log`; a failure shows its
   last lines (a horizon-mixins diagnostic in one line).
6. **Post-build**: each mod's post-build program is built; it runs in the new version folder once that is copied
   (next step), before the version counts as installed ("Post-build steps").
7. **Install**: a new version, `native-<commit>-<key>`, a copy of the starting version's folder (resources
   included, cloned on APFS, Btrfs and ReFS) with the built game in place of its own, and a `native-mods.json`
   saying which release or Nightly it started from (`base`) and which commit, mods, horizon-mixins and Rust made
   it. The instance the mods are installed in plays it, the default **Horizon** instance included: the build
   pins its version, as picking a version does. Updates of the launcher's version leave a pinned instance where
   it is, and their clean-up never removes a native build. An instance built before this (the **Native mods**
   instance the default home's mods used to go to) keeps its worlds and its build.

## Rebuilds stay on the instance's Horizon

A build starts from the version the instance plays (`native_mods::base_for`). When that is a native build, the
next one compiles the same commit (its `native-mods.json`'s `horizon`) from the same game files, unless a mod pins
another, so a rebuild after a mod changes or is switched off compiles only what changed. A newer Horizon is taken
only when the player asks: `native-build --update` starts from the newest installed release or Nightly (a full
rebuild); `native_mods::update_for` says which one, if any, the window can offer.

The compiler runs as the release's build does, whatever the player's own Rust setup: the launcher's toolchain
drops the variables that change compiler flags or the target (`RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`,
`RUSTDOCFLAGS`, `CARGO_INCREMENTAL`, `CARGO_PROFILE_*`, `CARGO_BUILD_*` but `CARGO_BUILD_JOBS`, `CARGO_TARGET_*`,
rustup's and the wrappers'), and starts Cargo from the disk's root with `--manifest-path`. Cargo reads `.cargo/config.toml` from
the folder it starts in and every one above it, whatever `CARGO_HOME` says, so from the launcher's data folder it
would read the player's `~/.cargo/config.toml`, whose `[profile]`, `[build]` and `[target]` settings beat the
workspace and the environment; no switch turns that search off. The source's own `.cargo/config.toml`, if it
has one, is passed with `--config`.

## Cache and disk

The key hashes the commit, the horizon-mixins commit, Rust's version, the profile and features, and every file of
every bundle. The same set on the same commit is never built twice. A change gives a new build, and
`native/target/` stays, so a rebuild compiles only `horizon-client` and what the mods touched.

Measured 2026-10-30 on Linux (x86_64 desktop), Zoom 2.0.0 and FPS Counter 2.0.0 on Nightly `6f8c7a0`, the `fast`
profile, the launcher at nice 19 with `CARGO_BUILD_JOBS=10` (an empty launcher home: Rust, the crates and the
source downloaded first):

| What | Time |
|---|---|
| First build: Rust, horizon-mixins, the whole game (384 crates) | 4 min 29 s in all; 3 min 6 s of compiling the game; `native/target/` about 2 GB |
| One mod switched off, rebuild | 56 s in all, 51 s of compiling (only `horizon-client`) |
| That mod switched on again | under 1 s: the kept build plays again |
| Nothing changed, rebuild | 1 s, nothing compiled |

The test bundle `native-crate-demo` (its own crate on `crc32fast`, data files, the `frame` hook by name, a
post-build step), measured the same way on 2026-10-30 (Rust already downloaded):

| What | Time | Compiled |
|---|---|---|
| First build (horizon-mixins, the crates, the whole game) | 3 min 27 s; 2 min 24 s compiling the game | 343 crates |
| A helper's code changed | 48 s (43 s compiling) | `horizon-client` only: the mod's crate is kept |
| The mod's crate changed | 11 s (6 s compiling) | `crate-demo-engine`, then `horizon-client` |
| Nothing changed | under 1 s | nothing |

The same set with the `release` profile took 7 min 6 s for the first build and 2 min 34 s per rebuild (2 min 28 s of
compiling `horizon-client`, mostly its link-time optimization). In play, a `fast` game drew the same frames per
second as the Nightly's own `release` build (vsync-bound, about 112 FPS at 120 Hz), with about 0.1 ms more CPU per
frame (0.57 to 0.68 ms against 0.51 to 0.54 ms).

Measured before builds used `fast`, with the `release` profile, on 2026-10-28 on the Mac mini (M4 Pro, 14 cores) with Veinminer 0.3.0 on Nightly `83f15fa`,
`CARGO_BUILD_JOBS=8` while other agents' builds ran (upper bounds):

| What | Size, time |
|---|---|
| Rust 1.99.1 (`<launcher data>/rust/`) | 453 MB; 58 s to download and unpack |
| horizon-mixins (`native/tools/`) | 7 MB; built once in 19 s |
| Crates in the launcher's Cargo home (`<launcher data>/cargo/`) | 289 MB |
| Source of one commit, and the stage | 64 MB each |
| First build: the whole game | 9 min 10 s of compiling, 9 min 41 s in all; `native/target/` 1.5 GB |
| Rebuild after a helper's code changed | 2 min 46 s of compiling (only `horizon-client`), 2 min 53 s in all |
| A change horizon-mixins ignores (a comment) | nothing compiled; 31 s, mostly the version folder's copy |

Measured 2026-10-29 on Windows 11 (Ryzen 7 9800X3D, 16 threads), the same mod and Nightly, an empty `native/target/`
(horizon-mixins, the source and the crates were already downloaded):

| What | Size, time |
|---|---|
| Rust 1.99.1 and MinGW-w64 (`<launcher data>/rust/`) | 1.7 GB, MinGW-w64 875 MB of it |
| Crates in the launcher's Cargo home | 657 MB |
| First build: the whole game | 6 min 57 s of compiling, 8 min 3 s in all; `native/target/` 2.0 GB |

Measured 2026-10-28 on Linux (an x86_64 server with no Rust of its own), Zoom on Horizon `1149c92`, 3 jobs while CI
ran: `rust install` in 11 s (592 MB), the first build in 12 min 9 s. The built game ran Zoom's proof (under `xvfb-run`:
the game needs a display even with `--offscreen`).

The native builds an instance plays are kept, and the three newest no instance plays (`native_mods::prune`, by when
each was built or last used): switching a mod off and on again plays the kept build again without compiling.
`native/target/` stays for the next one.

## Trust

- **Off until the player installs one.** Settings → Game → Native mods stays off for a player who never installs a
  native mod; the first install shows the notice and turns it on. The command needs `--trust-native-code`; the window
  shows a warning in red before every build: the mods run with full access to the computer, and their ids and
  versions. The Mods page says who made each one, and "Not made or endorsed by the Horizon project" for any mod not
  Horizon's own.
- **The registry** (horizon-rust/horizon-mods) lists them as their own kind, `native-mod`, only after a maintainer
  has read the source: the `.hnmod` contains `.rs` and `.toml` files and may contain data files under
  `mixins/<id>/data/` or a crate's `data/`. The launcher checks the bundle without running anything. Native code is trusted.
  The file is pinned by SHA-256 like every registry file. An update is reviewed again; `revoked.toml` takes one down.
  A WebAssembly mod's entry may turn `native-mod` once (never back): its `.hmod` versions stay published and
  are checked as `.hmod`s, the index lists only its `.hnmod` versions, and the launcher's update keeps the
  player's `.hmod` (the game runs it until a native build has the mod, then skips it); removing the mod removes both.
- **Isolation of data.** Native mods run only in the instance they are installed in, with its worlds, options and
  logs (the default Horizon instance's, when they are installed there); other instances play their own versions.
- What this does not do: sandbox the compiled game. horizon-mixins' checks stop mistakes, not a malicious helper
  ([security notes](https://github.com/horizon-rust/horizon-mixins/blob/main/docs/security.md)).

## Multiplayer

- **Single player and LAN host:** the mods run in the host's game and its integrated server, so a server-side
  hook applies to every player of the world.
- **Joining a server:** only client-side hooks do anything; the protocol is unchanged.
- **Dedicated servers:** not built yet; the same pipeline can build the server (`--no-default-features`).

## Left to do

- Rebuild after the mods change, and offer **Update** when `update_for` finds a newer Horizon (asking first, as it
  compiles for minutes).
- A prebuilt horizon-mixins from its CI instead of building it on the player's computer.
