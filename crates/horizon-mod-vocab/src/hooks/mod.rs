//! Declarations of the mod API's hooks and draw contexts: the catalog.
//!
//! This module is never compiled (`#[cfg(any())]` in `lib.rs`): `horizon-mod-api-gen` reads these
//! files with `syn` and builds the `Hooks` trait, its dispatch, the game's entry points and the
//! rustdoc from them. Lint checks these files directly with `rustfmt`. What the game does for a mod, the host
//! methods and the handles', is not declared here but written by hand, one file per area, in `../host/<area>.rs`.
//! Each host method has one `#[access(Level)]` declaration; its default and the game's forwarder
//! are generated. Implement the same-named inherent `Game` method, without a `native_` prefix.
//! Hook entry points keep their `native_` names.
//!
//! Each hook has its own file, each declaration a bodiless `fn name();` with its rustdoc.
//! Separate files hold draw contexts and general notes. The declaration traits are:
//!
//! - `Hooks`: `#[hook(side = "...", contract = "...", ...)]`, one per moment the game tells mods about.
//! - `DrawContexts`: `#[draw_context(surface = "...", name = "...")]`, a drawing surface hooks draw on.
//! - `Notes`: `#[notes = "..."]`, prose for the hook-notes page that belongs to no one hook.
//!   A hook's own prose is a `#[notes]` beside its declaration; a host method's, a `Notes` entry.
//!
//! A hook's arguments:
//!
//! | Argument | Meaning |
//! |---|---|
//! | `side` | `"server"`, `"client"` or `"both"`; required |
//! | `contract` | `"listen"`, `"cancel"`, `"collect"` or `"replace"`; required |
//! | `out` | the event field a cancel, collect or replace hook writes; not for `"listen"` |
//! | `scope`, `key` | a Part method: the kind of Part (`"block"`, `"block_entity"`, `"item"`, `"menu"`, `"fluid"`, `"entity"`) and the hook argument naming what it is attached to |
//! | `answered_by` | `"mod"`: a global replace hook whose output also names the mod that answered, or a global collect hook whose output is `Vec<Answered<T>>`: each mod pushes `T`s into its own empty batch and dispatch names that mod on each entry |
//! | `isolate` | `true` on a global cancel hook: its dispatch runs every listener inside `catch_unwind` — one that unwinds keeps the **requested** outputs (its writes roll back), is skipped by this hook for the session, and no later listener runs on that invocation (#2414 ruling) |
//! | `host`, `context` | `"read"` or `"level"`; `"registration"`, `"data_load"`, `"loop"` or `"worldgen"` |
//! | `neoforge`, `fabric`, `paper` | the counterpart's name, or a list of them, for the rustdoc and the gap lists |
//! | `cancellable` | whether the NeoForge counterpart event is cancellable (documentation, not the contract) |
//! | `version` | the contract version an injection's `hook_version` names; left out when it is 1 |
//! | `target` | the game function that fires it; left out when it is `Game::native_<name>` in `native_hooks/<area>.rs` |
//!
//! Every hook needs `///` lines: they are the rustdoc of its trait method, and a hook without them
//! is refused. `HookId` values are process-local dispatch indexes, not persistent API identities.
//! Wasm subscriptions cross as names; native mods compile for the selected revision. Existing
//! split hooks retain their generated order through each file's `// catalog-order: N` comment.
//! These frozen ordinals only keep the split's output byte-identical; they provide no cross-build
//! compatibility. New hooks omit the comment and follow in file-name order. No shared list is
//! edited: the generator discovers the files directly.
//! Each new hook goes in a file of its own.
//! The JSON catalog (`native-hooks.json`,
//! `tools/native-hook-fragments/`) is retired and a file there fails the build.
//!
//! The game function a hook names starts as `let _ = (every, parameter);`; `cargo run -p xtask --
//! mod-api` fills in the dispatch call.
