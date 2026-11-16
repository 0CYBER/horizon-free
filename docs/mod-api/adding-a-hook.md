# Adding a hook

A new hook needs its declaration in the area file of `crates/horizon-mod-vocab/src/hooks/` (`///` lines and
`#[hook(side = "server", contract = "cancel", out = "cancel")]` on `fn name();`; the argument list is in that folder's `mod.rs`), its `native_*` function with the placeholder body
`let _ = (every, parameter);` (all its parameters, in order), and a run of `cargo run -p xtask -- mod-api`.
Put that function in `crates/horizon-client/src/game/native_hooks/<area>.rs`, matching the declaration's
area filename. Alternatively, a named hook may live in any logical module under `game/`, including
area folders, `mod.rs` and `#[path]` modules. Its target must name that module and
`Game<'_>::native_<name>`. Exactly one matching `Game` method must exist. Outside `native_hooks/`,
the method must call `dispatch_<name>` itself: placeholders are not filled there. Remove macros,
`cfg` and `cfg_attr` from that method so injections can patch it. A refusal names the hook and
what to change. These checks run at build time and add no dispatch-time work.
The build discovers native hook modules automatically; do not add a module list entry. Shared
support lives in `shared.rs`; `fire.rs` includes generated dispatch entry points, not hook implementations.
That command only replaces the placeholder in its owning area file with the `dispatch_<hook>` call; commit
that source change and the declaration. Fills are idempotent; rerun the command after a failed write. The `horizon-mod-api-gen` build dependency generates the hook traits,
`Host` methods and dispatch code into Cargo's `OUT_DIR` when the game and renderer build, including release
builds. These generated Rust files are not kept in the source tree or committed, and no manual regeneration
is needed after a merge. Every hook needs a `mod_api` entry: the build fails naming any hook without one. A hook that stands in for a
method a vanilla block, block entity, item or menu overrides also gets a `scope` and a `key`
(`{"contract": "collect", "out": "output", "scope": "block", "key": "state"}`): the key is a
`BlockState` (routed by its prepared block key for block parts), an `&ItemStack`, a `ModBlockEntity` or an id `&str`, and
the generator also puts the method on that scope's part trait ([Parts](../modding/src/concepts/parts.md)). A body that is neither the
placeholder nor already calls the dispatch is an error, and the generator never changes it. Hooks that are
free functions outside `Game` are wired by hand: those crates can't see the mod list, so each calls a callback
the executable installs once, such as `NativeLoadHooks` in `horizon-world` and `NativeClientHooks` in the
client library (`living_breathe`, `ingredient`). A build reports a placeholder still to
fill instead of changing hand-written source; run `cargo run -p xtask -- mod-api` to fill it. Mixin injections
go at the head of the body, before the dispatch, so they still run first.

Mutable owned stack-list arguments and collection outputs use generated per-listener validation, not
a hook-specific write-back rule. Single-stack and optional-stack adapters keep their existing policy.
Dispatch retains the preceding accepted stack list and validates
each edited output before leaving the listener's ownership scope. Refusal logs the mod, hook,
output and conversion error without discarding scalar/cancel outputs. The engine wrapper captures
the active immutable item-registry owner before lending its mutable host, and caches accepted
engine conversions for final commit. Untouched listeners neither clone another checkpoint nor
revalidate unchanged stacks. Keep this policy in the generator when adding mixed-output hooks;
the general generated hook notes describe the author-visible rule once.

The `wasm-mods` job also builds `tests/mixed-output-fixture` and runs
`horizon-mod-wasm`'s `listener_outputs` target with `mixed-output-fixture` enabled.
This opt-in catalog contract is absent from normal API/wire generation: the existing
cancellable drop hooks carry `ItemDrop`, not a direct `Vec<ItemStack>`. The test runs
the same literal stack/scalar/cancel edits natively and through a compiled guest's
generated event serialization and the real `WasmMod` proxy, including warning ownership,
cancelled-listener filtering and later-listener independence. Its conversion callback
has a fixed fixture limit; Game registry conversion has separate coverage. A standalone
Wasm VM running both dispatcher and listeners is not evidence of this boundary.
