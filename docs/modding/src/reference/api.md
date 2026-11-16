# The mod API reference

The complete reference for the mod API is the rustdoc of the `horizon-mod-api` crate (library name `mod_api`):
every hook of `Hooks`, every event, Part trait, handle and `Host` method, with its side, its NeoForge 26.3
counterpart where it has one, and whether a mod may cancel it.

Build it from a Horizon checkout:

```sh
cargo doc -p horizon-mod-api --no-deps --open
```

## Finding your way in the reference

| Start at | For |
| --- | --- |
| `Hooks` | Every moment a mod can implement, one method each |
| `BlockPart`, `ItemPart`, `BlockEntityPart`, `MenuPart`, `EntityPart` | Per-object behaviour ([Parts](../concepts/parts.md)) |
| `Host`, `ServerHost`, `ClientHost` | What a hook can ask of the game, split by side |
| `Registrar` | What a mod declares in `Hooks::register`: keys, Parts, settings |
| `AttachmentType` | Data a mod keeps on objects ([Attachments](../concepts/attachments.md)) |
| `Wire` | Values that cross between the game and a WebAssembly mod ([Wire types](../concepts/wire-types.md)) |

An item marked **Not on wasm** is unavailable to [WebAssembly mods](../how-to/wasm-mods.md).

[Hook notes](hook-notes.md) holds longer explanations that go with groups of hooks, and
[NeoForge counterparts](neoforge-counterparts.md) lists each hook's NeoForge event or method.
