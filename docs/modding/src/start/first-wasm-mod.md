# Your first WebAssembly mod

A WebAssembly mod is the same Rust you write for a native mod, built to `mod.wasm` instead. The game runs it in a
sandbox, with limits on time and memory, so a player needs no compiler and no trust prompt for native code. In
this page you build the greeter from [Your first native mod](first-mod.md) as a WebAssembly mod.

You need `cargo horizon-mod` ([installed as in the content mod tutorial](first-content-mod.md#1-install-the-mod-tool))
and [rustup](https://rustup.rs): the first build installs the pinned Rust toolchain with `rust-src`.

## 1. Start from the native project

Make the project from the wasm template. It is the native project without the `[native]` table: a mod carries
native code or WebAssembly, never both, and the build writes its own `[wasm]` table. The SDK build the mod is
made for is pinned in `Cargo.toml` instead, under `[package.metadata.horizon] sdk`
([the pin](../reference/sdk/03-in-a-project.md#the-pin)).

```sh
cargo horizon-mod new greeter --template wasm
cd greeter
```

Keep the code in `src/lib.rs` as it is:

```rust,ignore
{{#include ../../examples/src/greeter.rs:mod}}
```

## 2. Build

```sh
cargo horizon-mod build --wasm
```

The tool compiles the crate with the pinned toolchain, writes `mod.wasm` beside `horizon-mod.toml`, records it in a
`[wasm]` table, and packs `target/horizon-mod/greeter.hmod`:

```toml
[wasm]
sha512 = "…"                    # mod.wasm's hash, checked when the game loads it
toolchain = "<version> <commit>" # every mod linked together is built by the same one
```

Never write `[wasm]` by hand; rebuild instead.

## 3. Play

Copy `greeter.hmod` into the game folder's `horizon-mods/` (in the launcher: **Open folder** in the instance's ⋮
menu), and turn it on with a line `mod.greeter:true` in `horizon-mods.txt` beside `options.txt`. The game loads it at start-up; join a world and the greeting appears.
If the game refuses the mod (a wrong layout, a missing dependency, a limit), the Mods screen shows why in red.

## What differs from native

Hooks, Parts, attachments, `[dependencies]` and the manifest mean the same on both targets. A WebAssembly mod
cannot use the [native route](../concepts/native-route.md) (`[native.*]` tables), and an API item marked
**Not on wasm** in the [API reference](../reference/api.md) is unavailable to it. Values cross between the game and
`mod.wasm` in one shared encoding; your own types cross when they derive [`Wire`](../concepts/wire-types.md).

[WebAssembly mods](../how-to/wasm-mods.md) covers dependencies, memory, limits and the module cache.
