# Support modules: `[[native.module]]`

A support module puts shared Rust code in the function-replacement mirror of a game module. Its code
can use that game module's private items, and your mixin helpers can call it. The launcher compiles the
mirror against the prebuilt game; it does not edit or rebuild the original game source.

Use one for code that several helpers share and that needs the game's private items. Shared code that needs
only public items goes in your mod's own crate.

## Example

Declare `fnr_runtime` in the mirror of the autosave module. The full manifest and a helper that calls
it are on [Bridges](4-bridges.md#example).

```toml
[[native.module]]
id = "runtime"
file = "crates/horizon-client/src/game/world/autosave.rs"
name = "fnr_runtime"
source = "src/runtime.rs"
```

`src/runtime.rs` holds the module's items, with no `mod` wrapper:

```rust,ignore
pub fn bumped(ticks: i64) -> i64 {
    ticks + 1
}
```

A helper calls it by its path in the game crate: `crate::game::autosave::fnr_runtime::bumped(*ticks)`. That
file is the module `game::autosave`, because `world/` is an area ([Naming the target](2-mixins.md#naming-the-target)).

Inside the module, `super::` is the game module that received it, such as `super::Game`. Make the items that
helpers call `pub`. The module may call your mod's crate; your mod's crate cannot call the module.

## Fields

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | The row's name, unique in the mod. |
| `file` | yes | The game source file whose module is mirrored, relative to the repository root: `crates/horizon-client/src/…`. |
| `name` | yes | The module's Rust name. Pick one the destination module does not use, with a prefix of your own, such as `fnr_`. |
| `source` | yes | Your file under `src/` with the module's items. |
| `visibility` | no | The module's visibility: `"pub(crate)"` (the default, enough for your helpers) or `"pub"`. |

## What you will see

A module builds with the mixins, so the build log's `Function replacement:` line covers it.

## Errors

The `horizon-mod.toml:` errors come when the manifest is read; the coded ones come at build time.

| Message | Fix |
| --- | --- |
| ``E_MIXIN_MODULE_DESTINATION: <mod>:<row>: <file> is not a file of the game crate (`horizon_client`); …`` | Name a file under `crates/horizon-client/src/`. |
| `E_MIXIN_MODULE_SOURCE: <mod>:<row>: <parse error>` | Fix the Rust error shown in `source`. |
| `horizon-mod.toml: module file must be a plain path` | Write `file` as a relative path without `..`. |
| `horizon-mod.toml: module name must be an identifier` | Use a Rust identifier. |
| `horizon-mod.toml: module source must be a file under src/` | Move the file under `src/`. |
| `horizon-mod.toml: module visibility must be pub or pub(crate)` | Use one of the two, or leave it out. |
| `horizon-mod.toml: empty or duplicate module id` | Give the row an id no other row uses. |
