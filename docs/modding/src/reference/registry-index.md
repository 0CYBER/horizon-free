# The registry index and `.hpack`

Use the registry to distribute content, WebAssembly and native mods (`.hmod`), modpacks (`.hpack`),
resource packs, data packs, shaders-folder packs and Horizon shader packs (`.hshader`).
Submit your project page and version file in a pull request on horizon-mods ([Publishing your mod](../how-to/publish.md)).
Versions are pinned by SHA-256. The launcher and website read schema 1 of `index.json`.
The format is defined by `horizon_mod_package::index` and `horizon_mod_package::modpack`.

## Repository page fields

Place `horizon-registry.toml` at your repository root in the newest release tag
to supply `name`, `summary`, `description`, `authors` and `license`. Other keys
are refused. The registry entry takes precedence; this file fills fields the
entry leaves out.

## `index.json`

```json
{
  "schema": 1,
  "generated": "2026-10-24T18:00:00Z",
  "projects": [{
    "id": "coords_hud", "kind": "mod", "name": "Coordinates HUD", "summary": "One line.",
    "description": "Markdown.", "authors": ["Horizon"], "owners": ["ludovic111"],
    "license": "MIT", "source": "https://github.com/...", "icon": "https://...png", "key": "RWT...", "downloads": 1234, "stars": 56, "updated": "2026-10-24",
    "categories": ["utility"], "tags": ["hud"], "screenshots": ["https://...png"],
    "versions": [{
      "version": "0.1.0", "published": "2026-10-24", "changelog": "",
      "api": "0.2", "side": "client", "capabilities": ["registries"], "game": "26.3",
      "file": { "name": "coords_hud-0.1.0.hmod", "url": "https://github.com/horizon-rust/horizon-mods/releases/download/...",
                "size": 438, "sha256": "e113...", "signature": "untrusted comment: ...\n...\ntrusted comment: ...\n...\n" }
    }]
  }],
  "revoked": [{ "id": "bad_mod", "sha256": "1111...", "reason": "Sent chat messages it didn't declare.", "date": "2026-10-24" }]
}
```

| Field | Meaning |
| --- | --- |
| `schema` | 1. A reader refuses a higher number with "update Horizon" rather than misread it. |
| `projects[].id` | `[a-z0-9_]{1,64}`, unique; a mod's is its manifest's `id`. |
| `kind` | `mod` (a `.hmod` without `[native]`: content or WebAssembly), `native-mod` (a `.hmod` with `[native]`), `modpack`, `resourcepack`, `shader` (Horizon `.hshader`), `shaderpack` (a `shaders/`-folder zip), `datapack` (a data-pack zip). Other kinds (worlds, themes) may come without a new schema: a reader keeps a kind it doesn't know and doesn't offer it. |
| `summary`, `description` | Required summary of at most 160 characters for lists; Markdown for the project's page, required for a listed project and 50–10,000 characters after trimming. |
| `owners` | GitHub logins allowed to publish updates. |
| `license`, `source` | An SPDX expression and an `https://` address of the source code: both required. |
| `icon` | Required for a listed project. An HTTPS URL in the generated index, from the registry's `icons/<id>.png` (not a remote image input). |
| `categories` | 1–3 from the kind’s category list for a listed project. |
| `tags` | Up to 10 lowercase tags. |
| `screenshots` | Optional, up to 8 HTTPS PNG or JPEG URLs in the generated index, from files kept in the registry under `screenshots/<id>/`. |
| `key` | The author's [minisign](https://jedisct1.github.io/minisign/) public key. When set, every version must be signed with it; an update signed with another key is refused. Absent: the project is unsigned and the launcher says so. |
| `versions` | Newest first, at least one. |
| `downloads` | How many times its files were downloaded, every version together: GitHub's count of each release asset, summed across release assets. Absent when no file has been downloaded. Read-only: never in an entry. |
| `stars` | GitHub stars of the project's source repository, refreshed by the registry. Absent when zero or the source is not on GitHub. Read-only: never in an entry. The launcher shows the count and a button that opens the repository. |
| `updated` | Its newest version's `published` (the latest of `versions[].published`), set by the registry, so a reader can sort by it beside `downloads` without walking every version. Absent when no version has a date. Read-only: never in an entry. |
| `api` | A mod's API line (its manifest's `api`); a native mod's pinned revisions (`native-1/<horizon>/<mixins>`); a modpack's manifest format (`1`). |
| `side`, `capabilities` | A mod's, read from its manifest during validation. |
| `game` | The Minecraft version it is for. |
| `requires` | A modpack's mods: `{ id, version, sha256 }`, each a version in this index with that hash. |
| `file` | A plain name with the kind's extension (`.hmod`, `.hpack`, `.zip`, `.hshader`), an `https://` URL (the registry's release `https://github.com/horizon-rust/horizon-mods/releases/download/<id>-<version>/<name>`, or an asset of the project's own repository, `https://github.com/<owner>/<repo>/releases/download/<tag>/<name>`: the launcher accepts exactly these shapes), the size (at most 64 MiB), the lowercase hex SHA-256, and the `.minisig` text when the project has a key. |
| `revoked` | Files taken down: one file by `sha256`, or every version of `id` without it. The launcher refuses to install them and turns installed copies off with the reason. |

Readers ignore fields they don't know, so an optional field can be added to schema 1. Renaming or removing a field, or
changing what one means, makes schema 2.

The registry's CI holds the whole index to these rules. A reader keeps what passes them: a version that breaks one is
left out with its reason, a project left with no version goes too, a modpack version whose mod was left out goes with
it, and an id listed twice leaves out every project with that id. The rest of the index still loads; only text that is
not an index, or a newer schema, fails as a whole.

Each version may list `native_binaries`, an array of records with `build` (the
exact game build ID, at most 128 bytes with no ASCII control characters), `target` (the canonical Rust target triple), and `file`.
The file uses the index's existing name, URL, size, SHA-256 and minisign signature
fields; the signature is required. Records live only in the registry, never in
`horizon-mod.toml`. Duplicate build/target pairs are rejected. Lookup returns a
typed absence when the exact pair is unavailable, so source preparation can
handle it without selecting another build or architecture. These signatures are
registry-produced metadata, independent of project package keys. The launcher
builds native mods locally with its pinned Rust toolchain against the prebuilt
game; registry-built native binaries are not used.

## Before a file is installed

`index::check_file` runs on the downloaded file, before it reaches the game folder:

1. Its size and SHA-256 are the ones the entry lists, and it isn't revoked.
2. With a project key: its signature verifies with that key (minisign's prehashed and legacy signatures both).
3. What the file says matches the entry: a mod's `horizon-mod.toml` (id, version, API, side, capabilities) and its
   content's fence ([making a content mod](../how-to/content-mods.md#content-mods-api-02)); for native code, the same manifest's `[native]`
   (game, the exact Horizon and horizon-mixins revisions) and checked root Cargo project; for WebAssembly, its `[wasm]` table and `mod.wasm`; a modpack's manifest (id, version, format, game,
   mods); a resource pack's `pack.mcmeta`; a shader pack's `horizon_shader_api::check` and manifest (id, version,
   shader API).
4. A kind this build can't install is refused with "this Horizon can't install a … yet".

## `.hpack`

A zip holding `horizon-modpack.toml` and, optionally, `overrides/`. The mods are not inside: the launcher downloads
each from the index and checks it as above.

```toml
format = 1
id = "cozy"                 # [a-z0-9_]{1,64}: the profile's folder
name = "Cozy"
version = "1.0.0"
description = "Coordinates and a calm render distance."
authors = ["Horizon"]
game = "26.3"

[[mods]]
id = "coords_hud"
version = "0.1.0"
sha256 = "f891bde88f4b841fd35c4f3305bc8904b671ebe9d5ad28d5aecc57e8b23fc3ce"
on = true                   # optional, default true: turned on in the profile's horizon-mods.txt
```

Unknown keys are refused. `overrides/` may hold only `options.txt`, `horizon-graphics.txt`, `horizon-extras.txt`,
`resourcepacks/<name>.zip` and `horizon-mods/.horizon-mod-data/<a listed mod>/...` (a mod's own saved settings,
a native mod's saved settings), each at most 64 MiB and 256 MiB in all; anything else refuses the pack. The launcher copies
them into the profile's game folder.

## Submitting a project

Submit a pull request on [horizon-rust/horizon-mods](https://github.com/horizon-rust/horizon-mods) ([Publishing your mod](../how-to/publish.md)); the
submission form can't take `.hmod` files made with this SDK yet. Your page needs:

- A description of 50–10,000 characters after trimming.
- 1–3 categories from your project kind’s list and up to 10 lowercase tags.
- A square PNG icon, 64–1024 pixels and at most 256 KiB, stored in the registry.
- Optionally, up to 8 PNG or JPEG screenshots stored in the registry.

The generated index supplies HTTPS URLs for images. Unknown entry keys are refused.
A project’s kind cannot change. If you set a minisign key, sign every version with it.
For a repository-linked project, the submission service directs you to publish a GitHub
release in that repository instead of accepting a file update.

Check your package before submitting it:

```sh
cargo horizon-mod check my_mod.hmod
```

Use `--key` and `--signature` to check its signature. The command validates package
metadata and contents without running your mod’s code.

## Native dependency tools

Run these commands in your native mod’s project folder.

| Command | Behaviour |
| --- | --- |
| `cargo horizon-mod internals sync` | Explicitly checks a native mod's library through the launcher against an isolated copy of its pinned game source, then replaces top-level `internals` with compiler-discovered paths, kinds and hashes. Handles globs and macros, removes unused declarations, preserves retained entries and prints additions/removals. Requires a trusted mod and `horizon-launcher` on `PATH` (or `HORIZON_LAUNCHER` set to its executable). Never runs automatically on build. |
| `cargo horizon-mod internals add <path>…` | Adds each declared internal and its tool-written hash to top-level `internals` in your mod's `horizon-mod.toml`. Without support modules, reads `internals.json` from the build data of the native mod's pinned SDK; with `[[native.module]]` tables it asks the launcher for a discovery catalog of the pinned source with those modules applied (trusted native code required). Unknown paths, shape-only entries and paths available only under a `cfg` are refused before the manifest is changed. Existing declarations are refreshed, not duplicated. |
| `cargo horizon-mod update [--horizon <build>]` | Moves the mod's pin to the current Nightly's build (or the build given: `nightly-<date>` or a sha): a native mod's `[native] horizon`, a wasm mod's `sdk` under `[package.metadata.horizon]`. The next command fetches that build's SDK into `.horizon/sdk/`. `Cargo.toml` names the SDK by path, so its dependency does not change. For a native mod, for every declared internal, prints `missing`, `changed` (old signature → new signature), or `unchanged`, then refreshes the available hashes. Missing or gated declarations keep their old hash and must be removed or replaced before building. Build names: [the pin](sdk/03-in-a-project.md#the-pin). |

The native-mod commands run in the mod's project folder and preserve the manifest's comments and
layout. `sync` performs a metadata-only `cargo check` on the current host, using default features;
Rust build scripts and procedural macros run with your rights, but the game does not run. It keeps
a separate per-pin source and check cache in the launcher's `native/internals-sync/<commit>/` folder,
never in the normal game or prebuilt target directories. The pin's source must be cached or
downloadable; an unavailable source is `E_INTERNAL_SOURCE_UNAVAILABLE`, not a fallback to another
pin. Compiler or catalog errors leave the manifest untouched. Discovery may prepare source and
metadata-check game dependencies; it does not compile or link a game executable. This separate
check cache is not a native mod build route. `[[native.state]]` is refused with
`E_MIXIN_STATE_LAYOUT`; use attachments for per-object mod data.

Without support modules, `internals add` and `update` read `internals.json` from the build data (`build-data.tar.xz`) of each pin's SDK release, `sdk-<build id>`, fetched and
checked against its signed `SHA256SUMS` as every SDK is, from `HORIZON_SDK_BASE` when it is set. A build with no
SDK release is refused by name; there is no other source. No game or mod code is run. With support modules, they run
`horizon-launcher native-build --catalog` (which needs trusted native code), stage the pinned source with the
modules, and keep the discovery catalog under `target/horizon-mod/catalog-snapshots/`; the game and the mod do not
run. Each build's data is kept in
`.horizon/cache/<build id>/build-data/`, so `update` can show the old signatures after that release expires.
If a needed catalog is neither cached nor downloadable, the command fails without changing the pin
or declarations. A mod with no declared internals needs no catalogs to update. Quote paths containing
Rust lifetimes or other shell metacharacters, for example:

```sh
cargo horizon-mod internals add 'horizon_client::game::Game' "horizon_client::game::maps::Game<'_>::dimension_id"
cargo horizon-mod internals sync
cargo horizon-mod update
```
