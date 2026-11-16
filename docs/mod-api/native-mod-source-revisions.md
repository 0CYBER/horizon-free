# Native mod source revisions

A native mod targets one exact Horizon source revision: `horizon` in its `horizon-mod.toml` `[native]` section.
The same section pins `game` and the exact `mixins` commit. Match the mod to the game revision it will run with,
and rebuild it after game updates. A source pin is not a promise that a later game or launcher will keep
building the mod unchanged.

An installed native build with a missing or invalid build record must be rebuilt from an installed
Horizon version rather than used as a source revision.

Hook points are functions the game calls at fixed places. Their names, parameters and call sites belong to the
game source and may change. Per-hook catalog versions describe individual contracts, not minimum game versions
or a cross-release compatibility range.

Hooks are declared in Rust now, not in JSON: a hook author adds a `#[hook(...)]`
declaration, with its `///` doc, to a file under `crates/horizon-mod-vocab/src/hooks/` (a new file for a new area, named
for it). A host method is not declared there: it goes in the area trait of
`crates/horizon-mod-vocab/src/host/<area>.rs`, with `#[access(WorldOnly)]`, `#[access(Metadata)]`, `#[access(ClientSetup)]` or `#[access(PublishedResources)]` on each method. The inherent `Game` method has the same name. The generator writes the access check and full-path call into Cargo's build output; do not write a game-side forwarder. The `#[horizon_mod_api_macros::host_area]` attribute generates panicking defaults for test doubles. The generator discovers area files without a shared module list. `native-hooks.json` and
`tools/native-hook-fragments/` are retired, and a file there fails the build. Collection updates
the catalog, not a global API number, source comments or documentation histories.

Hand-written API types have a selected public surface in `crates/horizon-mod-vocab/src/exports/<area>.rs`:
module declarations and re-exports stay in their source area's fragment. Engine-only re-exports sit in a
separate section of the same fragment and keep their existing paths. The generator discovers all fragments
and includes each section at its original scope. A new hand-written area adds one fragment, while
catalog-generated hook types need no export-list edit. Do not add declarations or re-export lists to
`lib.rs`.
