# The release

Every SDK is the release `sdk-<build id>` of this repository, at
`https://github.com/horizon-rust/horizon/releases/download/sdk-<build id>/<file>`. Its files:

{{#sdk-assets}}

The compiled slices exist once per [host](06-hosts.md); a file named `…-server.tar.xz` is the dedicated server's.
Everything else exists once per build. `.tar.xz` files are tar archives compressed with xz; `surface.json` is the
file itself.

A Nightly carries a copy of its build's `sdk.json` in its own signed sums. The launcher learns which build a game is
from the copy in that game's release, and keeps the answer, so it is known offline afterwards.
When a second Nightly displaces a dated release of the same day, the three files needed to verify the first
build (`sdk.json`, `SHA256SUMS` and `SHA256SUMS.v2.sig`) stay in `mod-index-nightly-<date>-<short commit>`.
The launcher checks that copy against the original Nightly version before using its SDK.

This describes the restored publication contract, not a claim that every main commit already has an SDK.
Publication follows code-owner review and green checks/parity on the exact main SHA. A build without its signed,
complete `sdk-<build id>` release is unavailable for native builds; the launcher does not rebuild the game to fill
the gap. Maintainers can exercise the complete route without publishing with
`gh workflow run prebuilt-game.yml --ref main` (dry run by default).

## Which slices a build needs

The launcher fetches only the compiled slices the instance's mods need, decided once for the whole set of enabled
mods. The routes and their slices come from the same input list the launcher and the packager read:

{{#sdk-routes}}

Each slice unpacks into its own folder. Using a kept slice again checks only its files' names and sizes against a
stamp written after its verified unpack; it does not hash them again.

## `sdk.json`

The index. Readers ignore fields they don't know and refuse a `schema` they don't know: adding a field keeps the
number, removing or redefining one changes it.

| Field | What it is |
|---|---|
| `schema` | The index's schema number (1). |
| `build.commit` | The build id: the game's full commit sha. |
| `build.game` | The vanilla version the build implements (`26.3`). |
| `build.date` | The commit's date, `YYYY-MM-DD`, UTC. |
| `build.nightly` | The Nightly name that ships this build, when one does. |
| `package_format` | The prebuilt package's layout number. A reader takes only the number it was made for. |
| `toolchain.rust` | The Rust release everything in the SDK was compiled with. |
| `content_api` | The content manifest's version: the `api` key of `horizon-mod.toml`. |
| `surface.hash` | `surface.json`'s own hash: equal hashes, equal surface. |
| `surface.previous` | The build id of the SDK published before this one, which it is compared with. Absent on the first. |
| `surface.source` | That comparison in one word, its worst change: `identical`, `additive`, `conditional` or `breaking` ([classes](02-surface.md#what-a-change-is)). |
| `surface.changes` | The ids of the records that changed, breaking ones first. |
| `surface.notes` | The change notes attached to them, each `{ ids, why, migrate }` ([notes](02-surface.md#change-notes)). |
| `mixins.rev`, `mixins.patch` | The pinned horizon-mixins tool's revision and the SHA-256 of the patch carried on it. |
| `assets` | Every file of the release but `sdk.json` and the sums, by name: `sha256`, `size`, `kind`, `codec` (`none` or `xz`), and for a compiled slice its `target` and `side` (`client` or `server`). |
| `inputs` | The files each route needs, named before any download, build or packaging starts: its SDK slices, the programs beside the game and the checkout files, how to make each, and what each slice must hold. |

## How a download is checked

The mod tool and the launcher check an SDK the same way, whatever the [base](03-in-a-project.md#mirrors-and-offline-use)
it came from, and refuse it by name at the first step that fails:

1. `SHA256SUMS` carries Horizon's signature for the tag `sdk-<build id>`. That signature never passes for a game
   release, and a game release's never passes for an SDK.
2. `sdk.json` is the file those sums list, names this build, has a schema and package format the reader knows, and
   lists only assets of known kinds and codecs under plain file names.
3. Every other file has the size and SHA-256 `sdk.json` gives, checked before it is decoded. An archive unpacks into
   a temporary folder, renamed into place only when every entry unpacked, and is refused past an unpacked size
   limit or for any entry that is not a plain file or folder inside it.

An SDK newer than the launcher reads (its schema, package format or Rust) asks for a launcher update; one made with
an older Rust than the launcher compiles with asks for a newer game build.
