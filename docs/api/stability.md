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
required; refusing input that was accepted; any change to a WIT type a function uses, a new `enum` or `variant` case
included, in either direction (the runtime links a component only when each imported function's types match exactly:
a mod built before the change no longer loads); a JSON field that changes type.

Not breaking: a new function, interface, capability, command, endpoint, tool or `server.toml` key; a new optional
argument or JSON field (readers must ignore fields they don't know); a new WIT function or type (mods built earlier
don't import it); faster, or fixing behaviour to what the reference already says.

### Mod API levels

A new WIT function doesn't break older mods, but a mod that calls it can't run on a Horizon from before it. So every
function the game offers mods has an **API level**, the one it arrived at (`crates/horizon-mods/src/api_level.rs`):
level 0 is the contract of Horizon 0.2.67 to 0.2.69, level 1 adds `render.set-minimap` and server mods' `players`, level 2 `game.raycast`, level 3 `command.run-quiet`, level 4 server mods' `world`, level 5 `recipes` (`items`, `all`, `known`), level 6 `browser` (`set-items`, `poll`, `set-recipes`) and `recipes.place` and `recipes.give`, level 7 `render.set-far-terrain`. A mod needs the level of
the newest function it imports (a component imports only the functions it calls). The level is read from the file,
never declared:

- the game refuses a mod that needs a function it doesn't have, with "made for a newer Horizon (it uses
  render.set-minimap): update Horizon to run it", instead of a linking error;
- the registry's CI writes each mod version's level into the index (`api_level`), and installers take the newest
  version the running game can run.

A pull request that adds a function lists it in `api_level.rs` at the next level and raises `LEVEL`; the
`api_reference` test fails until it does.

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
| Mod API: `log`, `game`, `chat`, `hud`, `storage`, `command`, `events` | preview | 0.1.0 |
| Mod API: `ui`, `ui-events` (a mod's screens and settings page) | experimental | manifest `experimental = ["ui"]` |
| Mod API: `render` (HUD shapes, camera shake and zoom; drawn by the game, no mod shaders) | experimental | manifest `experimental = ["render"]` |
| Mod API: `input` (claimed items' use, the mod's keys, the mouse wheel) | experimental | manifest `experimental = ["input"]` |
| Content mods (`registries`: data-driven blocks and items) | preview | manifest `api = "0.2"` |
| Server mods: world `horizon-server-mod`, `players`, `server-events`, `world`, typed `server-command`s | experimental | manifest `api = "0.3"` |
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

### The experimental mod APIs' choices

- **What a mod may take from the game.** `input.claim-wheel` is the one input a mod consumes: while it holds the
  wheel, the steps are its `wheel` events and the hotbar doesn't move. `claim-items` takes the use button only on the
  items it names. A mod's keys (`register-key`) never consume: vanilla's mapping on the same key still runs. Whether
  keys may consume too is decided before `input` becomes preview.
- **`input`'s `wheel` event** is a new case in a variant the game sends, so it breaks mods built before it (allowed
  because `input` is experimental): an `input` mod that calls `poll` needs a rebuild against the new WIT.
- **`render.set-minimap`** draws a map the game makes, not one a mod composes: reading the blocks around the player
  for a map image costs too much through hook calls, so the game keeps the tiles (vanilla's map colours) and the mod
  picks the corner, side, scale and shape. One map shows at a time, the first loaded mod's. It is a new function and a
  new type, so mods built before it keep loading.
- **`render.set-far-terrain`** turns on the game's own distant terrain, not one a mod builds: sampling the generator
  and meshing thousands of far tiles runs on the game's workers within its frame and upload budgets, which no hook
  call could match. The mod picks the distance (64 to 1024 chunks, capped lower on integrated graphics as LOD Distance
  is) and Far Detail's Reduced. The player's own LOD Distance, when on, wins; one far terrain shows at a time, the
  first loaded mod's.
