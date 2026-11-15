# In a mod project

A code mod's project names the mod API by the path where `cargo horizon-mod` keeps the pinned build's SDK:

```toml
[dependencies]
horizon-mod-api = { path = ".horizon/sdk/api/crates/horizon-mod-api" }
```

`horizon-mod-points` and `horizon-internals` are named the same way, by `.horizon/sdk/api/crates/<package>`. The
SDK carries no `horizon-internals` crate yet: a mod that names game internals builds through the launcher, which
puts the mod's own generated facade in its place, and not with a plain `cargo check`. Features
(`features = ["testing"]` on a dev-dependency) are kept.

`cargo horizon-mod new` writes that path, and it is the only form the launcher takes: when it builds a mod, it
refuses every other form under `E_SDK_SOURCE`, with the line to write instead:

| Written | Why it is refused |
|---|---|
| `{ git = "…", rev = "…" }` | The SDK is no longer named by git. |
| `"0.1"` or `{ version = "0.1" }` | A version names no source, so Cargo would take the crate from crates.io, where anyone can publish that name. |
| `registry`, `registry-index`, `branch`, `tag`, `rev` or `workspace` | It does not name the SDK. |
| `{ path = "…" }` anywhere else | The path must be the pinned SDK the tool fetches. |

## The pin

The build a mod is made for is its pin, always a full commit sha in the file:

- a native mod: `[native] horizon` in `horizon-mod.toml`;
- a wasm mod: `sdk` under `[package.metadata.horizon]` in `Cargo.toml`.

`cargo horizon-mod new` writes the current Nightly's build, or `--horizon`'s. `cargo horizon-mod update` moves a
native mod's pin. Wherever a build is given, these forms are accepted:

| Written | Means |
|---|---|
| `nightly` | The current Nightly's build. |
| `nightly-YYYY-MM-DD` | That day's Nightly's build. |
| 40 hex digits | That build. |
| 7 to 39 hex digits | The one SDK whose build id starts with them, among the releases listed: this repository's 100 newest on GitHub, or a folder base's entries. Another URL base takes the full sha only. |

A Nightly name is read from `<base>/<name>/sdk.json`. A Nightly whose build has no SDK says so, and asks for a build
that has one.

## `.horizon/sdk/`

In a code mod, `new`, `build`, `build --wasm`, `check`, `hooks`, `fits` and `update` fetch the pinned build's SDK
into `.horizon/sdk/` when the folder is missing or holds another build, after the [checks](01-release.md#how-a-download-is-checked).
It holds `sdk.json`, `surface.json`, the unpacked API source under `api/`, and `build`, a file naming the build. That
file is written last, so a fetch that stopped partway is fetched again by the next command.

`fits` and `update` fetch no SDK for a mod made for exactly one build: its manifest says so before one is needed. Any of these commands makes a fresh clone of a mod work, `check` included, since it fetches before Cargo
reads the path. An editor needs that first command too: until it runs, `horizon-mod-api`'s path does not exist.

`sdk diff`, `fits` and `update` read other builds' small files (`sdk.json` and `surface.json`) into
`.horizon/cache/<build id>/`, with the same checks.

`.horizon/` is in the template's `.gitignore`, and the package walker skips it: it is never packed, copied by the
launcher or sent to a registry.

## Mirrors and offline use

`HORIZON_SDK_BASE` replaces the releases' base for the SDK fetches of the tool and the launcher alike:

- a URL: a mirror with the same layout, `<base>/sdk-<build id>/<file>`;
- anything else: a folder holding `sdk-<build id>/` folders, and `nightly/` or `nightly-<date>/` folders with their
  `sdk.json` for Nightly names. The launcher also reads game releases' folders there, for their build ids.

Every SDK base is checked the same way, against Horizon's signature. The launcher's own copies are described in
[Cache and disk](../../how-to/native-mods/10-cache-and-disk.md).

The internals commands (`internals add`, `inject add`, `inject update`, and `update` on a mod with an `internals`
list or `[[inject]]` rows) read `internals.json` and `points.json` from the pinned build's signed SDK, through the
same fetch and `HORIZON_SDK_BASE`: its `build-data.tar.xz` is unpacked once into
`.horizon/cache/<build id>/build-data/`. A build with no SDK release is refused by name.
