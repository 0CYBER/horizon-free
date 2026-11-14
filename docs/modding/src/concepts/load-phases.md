# Load phases and access

A mod's code runs in phases. What a hook may touch depends on the phase it runs in: there is no `Host` while
the mod registers, and no world until the World phase.

![Each game thread goes through Packages, Registration, Setup, Config and World, in order; a data or asset reload
enters Reloading and returns to the phase it came from.](../images/load-phases.svg)

Native and wasm hooks share the installed dependency-ordered mod slice, including registration,
config loading and reload-listener registration. The shared resolver keeps bundle-order ties,
checks present optional dependencies too, and refuses cycles for both build targets. Content-only
packages stay after code in the loaded-mod inventory; listener-specific edges use `ReloadOrder`.

Each owning thread progresses through `Packages → Registration → Setup → Config → World`.

| Phase | Available access |
| --- | --- |
| Packages | Package discovery; no game Host operations. |
| Registration | Declarations through `RegistrationContext`, with its mod-list, registry and settings reads; no Host. |
| Setup | Client setup reads, published asset/data queries and registry/settings/mod-list reads; no world handles or transactions. |
| Config | Settings reads and the mod-scoped `ConfigFiles` service; no world handles, transactions or other game operations. |
| Reloading | Published asset/data queries, registry facts, settings and mod-list reads, listener registration, and `DataLoadContext` callbacks; no world handles or transactions. |
| World | The owning game's side-typed `ServerHost` or `ClientHost`. |

`RegistrationContext::instance_dir()` reads the instance directory captured before
registration begins, as an `InstanceDirectory`: `Unix(Vec<u8>)` preserves raw Unix
path bytes, and `Windows(Vec<u16>)` preserves Windows UTF-16 code units, including
unpaired surrogates. Never convert Unix bytes with a lossy UTF-8 conversion when
constructing a native filesystem path. Wasm receives the identical host-encoded
value through Wire as data only; this does not grant filesystem access. A context
with no engine snapshot returns `RegistrationReadError::Unavailable`.

A mod that does slow work while the game starts shows it on the startup loading screen
(NeoForge's `StartupNotificationManager` and `ProgressMeter`). `RegistrationContext::progress(label,
steps)` opens a bar under the game's stage bar and returns `Result<StartupProgress, StartupProgressError>` (handle `TooManyBars` when 64 bars are already open): `step()`, `set(n)` and
`label(text)` move it, and dropping it removes the bar. The mod may keep the handle past
`register` and step it or call `bar.message(text)` from a worker thread or from `client_setup`.
`RegistrationContext::message(text)` shows a line for a few seconds. The newest four bars and the
last three lines are drawn, each cut to one line. A dedicated server logs each label, line and
tenth of a bar's steps instead. A wasm mod holds the game's screen by id, and each call crosses to
the game. A custom loading window is not part of this.

This is part of the startup progress feature, not live rendering of synchronous setup work.
Bars can be opened through `RegistrationContext`; `client_setup` has no creation context and
must retain a handle from registration. Updates become visible only while the loading screen
is actually drawing. A synchronous `client_setup` listener runs on the render thread, so its
updates cannot draw live and may appear only after it returns, if another loading frame is drawn.
Slow work can run in a mod-owned worker and step a retained bar, but neither that worker nor
the bar keeps the loading screen open. Live display of synchronous setup work and a creation
route from `client_setup` remain outside this change.
## Registration and setup

Registration runs once: `register`, `component_types`, `blocks`, `items` and `standalone_models`
receive no `Host` and read startup facts through `RegistrationContext`.
`register` also receives `&mut dyn Registrar` for writing declarations. The context's `mods()`, `registry(name)` and `setting(mod_id, key)` read
one view captured before the first of them runs, so every registration step sees the same values
whatever the dispatch order: the loaded packages, the built-in registries as `registries.json`
lists them (not what mods declare during registration) and the settings on disk when the snapshot is captured. A
mod validates the keys it declares against them from `Hooks::register`; a name that is not a
namespaced identifier, an unknown registry, an empty or path-like setting key, a package that is
not loaded, or a context the engine gave no data returns a `RegistrationReadError`. Client setup
runs once after the initial asset apply. The window's Client/Common configs load once immediately after client setup and before title,
connection, input, render or ordinary tick callbacks. This includes pack-error recovery: the failed
selection is cleared and saved during initialization, but its options screen opens only after Config
completes. A title-screen callback is already in World: world-handle constructors can return `None`,
rather than being forbidden in Setup. A dedicated
server loads Common once before its first world; Server configs load and unload per world.
A second world does not repeat registration or client setup. Config loading can read settings and
use the scoped `ConfigFiles` service to read or save its own config files. World handles,
transactions and other game operations panic with `Config` in the diagnostic.
Config file polling is not a phase transition, and Server config unloading leaves the World phase.

Attribute bounds are registration data too. `Registrar::widen_attribute_bounds` asks, from a both-side package's
`register`, join the attribute catalog once when it grows: after the attributes mods declare with
`declare_attribute`, and before entity attribute rows resolve and anything reads a definition. The bounds become
the union of every mod's asks, so the result does not depend on the dispatch order, and the default never moves
([content mods](../how-to/content-mods.md) has the call, its refusals and what saves keep).

## Reloading

Every data or asset reload enters `Reloading`, including listener registration, prepare, apply and
the reload completion notice. Initial loads can nest in Setup; runtime reloads nest in World.
Returning, including after a failed prepare, restores the entering phase. Published asset/data
queries read the published generation, not pending prepare output: completion callbacks can
refresh sprite and recipe caches after apply, including on first world open, without gaining world
handles or transactions. Their rustdoc names their load-phase availability. Server listener
registration retains `&dyn Host`, restricted to these published facts, registry facts, settings and
mod-list queries. Callbacks that take `DataLoadContext` use its restricted reads. Neither permits
opening a world handle or transaction. The first client registration and prepare run on the loader
thread; later registration runs on the render thread, prepare on its worker, and apply on the owning
game thread.

## Load contexts

`DataLoadContext` and `LoopContext` expose frozen registry and settings metadata. Use
`DataLoadOrigin` to identify the load origin and `LoopSide` to identify the owning side.
Registry entries retain registry ID order on native and wasm.

The game alone advances phases. An illegal or wrong-thread advance is refused before dispatch;
a forbidden mod call panics naming the phase, rather than returning a `Result`.
There is no separate common-setup, load-complete or intermod event: registration and the first server
reload cover common setup, client setup is `client_setup`, dedicated setup is `server_about_to_start`,
and dependencies express intermod ordering. Load complete is the edge after initial Config and before
`server_about_to_start`, not a hook.

FML's file and path APIs (`ModList` file reads, config full paths, the game directory, update URLs) are
replaced on purpose; [FML loader APIs and their Horizon routes](../reference/fml-loader-apis.md) lists each one.

## Settings snapshot limits

Settings snapshots accept regular UTF-8 files only. File and package-directory symlinks,
including internal links, are not followed. Invalid UTF-8, special files and refused entries
are absent from the snapshot rather than reported as read errors.

| Limit | Behaviour |
| --- | --- |
| 64 KiB per file, before trimming | Oversized values are omitted. |
| 1,024 captured keys per mod | Additional keys are omitted. |
| 16 MiB for the snapshot, before trimming | Values exceeding the remaining budget are omitted. |
| 4,096 enumerated names per package | A package exceeding the budget is omitted entirely. |

Capture proceeds in sorted package-ID and key-name order. The game emits one warning per
refusal class per snapshot. These limits apply to scalar settings, not mod data files.
