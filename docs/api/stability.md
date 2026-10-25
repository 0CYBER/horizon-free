# API stability and versioning

The rules every Horizon API follows ([the APIs](README.md)): how each one is versioned, what the stability levels
promise, how something is deprecated and removed, and the tests that keep old mods, scripts and tools working.

## Versions

- **Each API has its own version**, separate from the game's release (`v0.2.14`, [versioning.md](../versioning.md))
  and from Minecraft's. A game release says which API versions it serves.
- **Semantic versioning**, `MAJOR.MINOR.PATCH`. A **line** is what stays compatible: `0.1`, `0.2`, … before 1.0,
  then `1`, `2`, ….
  - Before 1.0: an addition makes the next patch (`0.1.0` → `0.1.1`); a breaking change starts the next line
    (`0.2.0`).
  - From 1.0: an addition makes the next minor (`1.0` → `1.1`); a breaking change starts the next major (`2.0`).
- **Where the version lives:**

| API | The version | A user names the line they need |
| --- | --- | --- |
| Mods | the WIT package (`horizon:mods@0.1.0`) | `api = "0.1"` in `horizon-mod.toml` |
| Agent control | the control protocol's version (planned: a `hello` reply and MCP `serverInfo`) | `horizon-ctl --api 1` (planned) |
| Server control API | the path (`/api/v1/`) | the path |
| `server.toml` | the file's keys; a key is never renamed in place | nothing: old files keep loading |
| Launcher commands | the command names and their `--json` output | nothing |
| Data formats | a `version` or `schema` field in the file | the reader rejects versions newer than it knows |

## What counts as a breaking change

Breaking: removing or renaming anything; changing a type, a default, a unit or a meaning; making something optional
required; refusing input that was accepted; a new case in a WIT `enum` or `variant` the game sends to mods (an older
component cannot decode it); a JSON field that changes type.

Not breaking: a new function, interface, capability, command, endpoint, tool or `server.toml` key; a new optional
argument or JSON field (readers must ignore fields they don't know); a new WIT case the mod *sends* (the game decodes
every case it ever served); faster, or fixing behaviour to what the reference already says.

## Stability levels

Every part of an API has one level, shown in its reference and its surface page.

| | Experimental | Preview | Stable |
| --- | --- | --- | --- |
| Who may use it | opt-in only (a flag or a manifest field naming it); never in a release default | anyone; marked "preview" where players see it (a mod's row on the Mods page) | anyone |
| May change | in any release, without notice | only at a new line, after a deprecation | only at a new major, after a deprecation |
| Deprecation window | none | at least 30 days, with one release line serving old and new | at least 180 days, with the old major served beside the new one |
| Needs | a design note | a generated reference, a guide section, an example, compatibility tests | the same, plus one line of real use without a break |

- **Promotion.** Experimental → preview when it has what the table asks. Preview → stable when real users (a published
  mod, a host's script, an agent's tool) used it for a whole line and nothing broke. The maintainers' orchestrator
  proposes the promotion with the evidence (the compatibility tests, the complete docs, that line of use) and Ludovic
  approves it.
- **Horizon-only by design.** An API is stable only for what Horizon itself does; a vanilla concept it exposes (a block
  id, a command, a `server.properties` key) changes when vanilla changes it, with a version port
  ([version-ports.md](../version-ports.md)), and the changelog says so.

## Deprecation and removal

1. The pull request that deprecates marks it where it is defined (a `/// Deprecated: use X; goes in 0.3.` doc comment,
   WIT's `@deprecated`, a `deprecated` field in a tool's schema) and adds the replacement in the same release.
2. The reference shows it as deprecated, and the surface page's changelog says what replaces it and when it goes.
3. Using it logs one warning per mod, session or client, naming the replacement; it keeps working.
4. It is removed only after the window, and only at the next line (preview) or major (stable).
5. A line or major that is no longer served is refused with a clear message (`needs API 0.1; this Horizon serves 0.3
   and 0.4`), never loaded half-way.

## Compatibility tests

Every preview and stable API has tests that run old users against the current code, in CI:

| API | The test |
| --- | --- |
| Mods | a test mod per served line, built from source against that line's WIT and SDK, loaded by today's runtime with expected output (`crates/horizon-mods/tests/`); a line is served as long as its test mod passes |
| Agent control | a transcript per protocol version: commands and the reply keys each must have |
| Server | `api::ROUTES` against `route()` (generated reference); a `server.toml` from each release line parses to the same settings; golden replies per endpoint |
| Launcher | the generated reference; golden `--json` output per command |
| Data formats | a sample file of every version a reader accepts, read by today's reader |

The generated references are the API's own diff: a pull request that changes an API changes its reference, and the
reviewer applies the rules above to that diff.

## Changelog

Each surface page ends with a changelog: one line per change, newest first, with the API version and the pull request.
Deprecations and breaking changes are in bold. The release notes of the game release that ships a change link it.

## Today

| Part | Level | Version |
| --- | --- | --- |
| Mod API: `log`, `game`, `chat`, `hud`, `storage`, `events` | preview | 0.1.0 |
| Content mods (`registries`: data-driven blocks and items) | preview | manifest `api = "0.2"` |
| Server mods: world `horizon-server-mod`, `players`, `server-events` | experimental | manifest `api = "0.3"` |
| Script mods (Lua run by an interpreter mod, [mods.md](mods.md#ways-in)) | not started | |
| `cargo horizon-mod` (`new`, `build`, `run`), `horizon-mod-sdk` | preview | 0.1.0 |
| Shader API | not started | |
| `horizon-ctl` commands and MCP tools | preview | none yet (unversioned) |
| The game's own control-socket commands outside `horizon-ctl`'s table (test hooks) | experimental | |
| `horizon-server` commands, `server.toml` | stable | `horizon-server --version` |
| Server control API | stable | `/api/v1` |
| `horizon-launcher` commands and MCP tools | preview | none yet (unversioned) |
| Benchmark report | stable | `"version": 1` |
| Telemetry batch and consent | stable | `"schema": 1`, consent `2` |
| `horizon-graphics.txt`, `horizon-extras.txt`, `horizon-mods.txt` | preview | none |
| `SHA256SUMS` and `SHA256SUMS.sig` | stable | |
