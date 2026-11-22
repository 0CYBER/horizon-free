# Standalone server export

Build a dedicated-server runtime with your native mods:

```sh
horizon-launcher native-build FOLDER --server-output DIR --version TAG --trust-native-code
```

`--trust-native-code` saves your consent to run unsandboxed native code. You can omit it when
consent is saved. Optional flags include `--profile PROFILE` and `--allow-jar-downloads`.

The command exports a standalone dedicated-server engine into `DIR`, which must not exist. `FOLDER` (a mod folder or a
folder of them) is required. The export does not select, edit or launch an instance.
It uses the pinned prebuilt dedicated-server SDK and builds only the mods and native
function-replacement libraries, never the game. `--version` pins the Horizon release/Nightly
for reproducibility. The export holds the engine executable, prepared resources,
`horizon-native-mods/<id>/` projects and provenance `native-mods.json`, and prints the exported engine executable's path. Use that value for `engine.path`.
It runs on the same OS/architecture as the host and requires an installed launcher base.
It is copied outside managed cache folders. On Windows use `horizon-engine.exe`.

Wasm packages use API hooks, not engine internals. Install and enable their `.hmod` packages
in the server game folder's `horizon-mods/`.

A mod's crate depends on `horizon-mod-api` and never on the game: a `horizon-client` dependency,
under any name or target, is refused naming the mod and the key, before toolchain work. The mod API
has no sound or controller features, so server and client builds stage the same crate.

Declared jars use the same exact-hash preparation as ordinary launches. A server export refuses
missing jars by default and never asks interactively; `--allow-jar-downloads` permits the shared
preparer to download the exact files into the local cache. Cached jar bytes are **never exported**.
When moving the runtime to another machine or game home, prepare its jar cache separately; errors
name the required files and cache paths. No Java code from a jar runs.

## Run the server

Point a server at the exported engine:

    horizon-server --dir SERVER config set engine.path=/absolute/export/horizon-engine
    horizon-server --dir SERVER restart

Updates do not rebuild a pinned `engine.path`: build a new export folder, set `engine.path` to it, restart, and keep
the old folder for manual rollback. The supervisor does not auto-update or rebuild a pinned engine path.

To inspect the running server, its authenticated
[`GET /api/v1/mods`](https://github.com/horizon-rust/horizon/blob/main/docs/api/server.md) queries the engine's canonical `LoadedMods` inventory:
`mods` lists loaded native, wasm and content package ids and versions in inventory order.
`provenance` separately reports the configured program's build record, which may differ and is never
used as an inventory fallback. A stopped or unreachable engine returns HTTP 409, not an empty list.
The configuration handshake checks both-side mod IDs and versions; see [Multiplayer](11-multiplayer.md).
Native mods run unsandboxed: only build code you have read and trust.
