# The registry index and `.hpack`

Horizon's registry lists what is made for Horizon and reviewed: mods (`.hmod`), modpacks (`.hpack`), resource packs
and, once their format exists, Horizon shader packs. Submissions are pull requests in
[ludovic111/horizon-mods](https://github.com/ludovic111/horizon-mods); merging one publishes it. The launcher and the
website read one file, `index.json`, which that repository's CI builds. The Rust types and every check below are
`horizon_mods::index` and `horizon_mods::modpack` (feature `index`); the sample is
`crates/horizon-mods/tests/index/index-v1.json`, read by the tests with the files it lists.

Level: preview ([stability.md](stability.md)). Schema 1.

## `index.json`

```json
{
  "schema": 1,
  "generated": "2026-10-24T18:00:00Z",
  "projects": [{
    "id": "coords_hud", "kind": "mod", "name": "Coordinates HUD", "summary": "One line.",
    "description": "Markdown.", "authors": ["Horizon"], "owners": ["ludovic111"],
    "license": "MIT", "source": "https://github.com/...", "icon": "https://...png", "key": "RWT...",
    "versions": [{
      "version": "0.1.0", "published": "2026-10-24", "changelog": "",
      "api": "0.1", "side": "client", "capabilities": ["game-read", "hud", "chat"], "game": "26.3",
      "file": { "name": "coords_hud-0.1.0.hmod", "url": "https://github.com/ludovic111/horizon-mods/releases/download/...",
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
| `kind` | `mod`, `modpack`, `resourcepack`, `shader` (Horizon's own shader packs, not Iris or OptiFine packs). Other kinds (data packs, worlds, themes) may come without a new schema: a reader keeps a kind it doesn't know and doesn't offer it. |
| `summary`, `description` | One line for lists; Markdown for the project's page. |
| `owners` | GitHub logins allowed to publish updates. |
| `license`, `source` | An SPDX expression and an `https://` address of the source code: both required. |
| `icon` | `https://`, optional. |
| `key` | The author's [minisign](https://jedisct1.github.io/minisign/) public key. When set, every version must be signed with it; an update signed with another key is refused. Absent: the project is unsigned and the launcher says so. |
| `versions` | Newest first, at least one. |
| `api` | A mod's API line (its manifest's `api`); a modpack's manifest format (`1`). |
| `side`, `capabilities` | A mod's, read from its manifest by CI, never typed by hand. |
| `game` | The Minecraft version it is for. |
| `requires` | A modpack's mods: `{ id, version, sha256 }`, each a version in this index with that hash. |
| `file` | A plain name with the kind's extension (`.hmod`, `.hpack`, `.zip`), an `https://` URL, the size (at most 64 MiB), the lowercase hex SHA-256, and the `.minisig` text when the project has a key. |
| `revoked` | Files taken down: one file by `sha256`, or every version of `id` without it. The launcher refuses to install them and turns installed copies off with the reason. |

Readers ignore fields they don't know, so an optional field can be added to schema 1. Renaming or removing a field, or
changing what one means, makes schema 2.

## Before a file is installed

`index::check_file` runs on the downloaded file, before it reaches the game folder:

1. Its size and SHA-256 are the ones the entry lists, and it isn't revoked.
2. With a project key: its signature verifies with that key (minisign's prehashed and legacy signatures both).
3. What the file says matches the entry: a mod's `horizon-mod.toml` (id, version, API, side, capabilities) and its
   content's fence ([mods.md](../mods.md#content-mods-api-02)); a modpack's manifest (id, version, format, game,
   mods); a resource pack's `pack.mcmeta`.
4. A kind this build can't install is refused with "this Horizon can't install a … yet".

The game still checks a mod's component imports against its capabilities when it starts it, as for any mod.

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
sha256 = "e1135345692f6dd8fd4c51920151944b82333e17049c9bb7d756f27b8deefc53"
on = true                   # optional, default true: turned on in the profile's horizon-mods.txt
```

Unknown keys are refused. `overrides/` may hold only `options.txt`, `horizon-graphics.txt`, `horizon-extras.txt`,
`resourcepacks/<name>.zip` and `horizon-mods/.horizon-mod-data/<a listed mod>/...` (a mod's own saved settings,
capability `storage`), each at most 64 MiB and 256 MiB in all; anything else refuses the pack. The launcher copies
them into the profile's game folder.

## The registry repository

[ludovic111/horizon-mods](https://github.com/ludovic111/horizon-mods) holds one `mods/<id>.toml` per project, written by
the submission form or by hand, and `revoked.toml`. What an entry can't vouch for is read from its file by the kind's
own check (`index::inspect`): a mod's API, side and capabilities, a modpack's mods, the size. Types and rules:
`horizon_mods::index::registry`.

```toml
id = "coords_hud"                # the file's name, and the manifest's id
kind = "mod"                     # mod | modpack | resourcepack; shader once its format exists
name = "Coordinates HUD"
summary = "One line, at most 160 characters."
description = """Markdown."""    # optional
authors = ["Horizon"]
owners = ["ludovic111"]          # GitHub logins who may publish updates
license = "MIT"
source = "https://github.com/..."
icon = "icons/coords_hud.png"    # optional: a PNG of at most 256 KiB in the repository, or https://
key = "RWT..."                   # optional: the minisign public key every version is signed with

[[versions]]                     # newest first
version = "0.1.0"                # letters, digits, '.', '-', '_': the release tag is <id>-<version>
published = "2026-10-24"
game = "26.3"
file = "coords_hud-0.1.0.hmod"   # the name players download
sha256 = "e113..."               # pins the reviewed file
signature = """untrusted comment: ..."""   # when the project has a key
changelog = ""                   # optional
```

```toml
# revoked.toml
[[revoked]]
id = "bad_mod"
sha256 = "1111..."               # optional: without it, every version
reason = "Sent chat messages it didn't declare."
date = "2026-10-24"
```

The submission form's service (`crates/horizon-mod-registry`) writes these entries: it signs the creator in with GitHub, runs the file's checks, uploads the file to the repository's `staging` release as `<sha256>.<extension>` and opens the pull request. Only an entry's `owners` publish updates through it.

Unknown keys are refused. Its CI runs `cargo-horizon-mod` (`crates/cargo-horizon-mod`):

| Command | What it does |
| --- | --- |
| `cargo horizon-mod check <file> [--json]` | A submission's checks: its kind's (above), its signature with `--key` and `--signature`, and for a mod a trial run in the sandbox (40 ticks, 120 frames, a chat line, each command). |
| `cargo horizon-mod registry check <base> <head> --files <dir>` | `check` on each file `head` lists and `base` doesn't: a pull request's new files. |
| `cargo horizon-mod registry list <checkout>` | `<sha256> <id> <version> <file>` per version: the files to download. |
| `cargo horizon-mod registry build <checkout> --files <dir> --out index.json` | Reads every entry and its file (`<dir>/<sha256>`), lists every problem, and writes the index with URLs `<release>/<id>-<version>/<file>`. |
| `cargo horizon-mod registry review <base> <head> --author <login>` | A pull request's changes: a published file never changes and is never removed (take it down in `revoked.toml`), a kind never changes; changes by a non-owner and to owners or keys are pointed out for the reviewers. |

Kinds open to submissions: `mod`, `modpack`, `resourcepack`. `shader` is in the schema and stays closed until
Horizon's own shader pack format exists (Iris and OptiFine packs are for the Modrinth browser); its check will compile
the pack in a headless Horizon. A new kind gets its own check before it opens.

## Changelog

- Schema 1 (2026-10-24): kinds `mod`, `modpack`, `resourcepack`, `shader`; `.hpack` format 1.
