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

## Changelog

- Schema 1 (2026-10-24): kinds `mod`, `modpack`, `resourcepack`, `shader`; `.hpack` format 1.
