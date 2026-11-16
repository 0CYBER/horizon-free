# Your first native mod

A native mod is a Rust crate that implements hooks from the mod API. The launcher compiles it on your machine
against the exact Horizon build an instance plays, then runs that instance with your mod in it. In this page you
make a mod that greets every player who joins.

You need the Horizon launcher (`horizon-launcher`) with an instance, and the `cargo horizon-mod` tool, which you
install with Rust's `cargo` ([as in the content mod tutorial](first-content-mod.md#1-install-the-mod-tool)). The
native build itself uses the launcher's own pinned Rust toolchain, not yours.

## 1. Make the project

```sh
cargo horizon-mod new greeter --template native
cd greeter
```

This writes a flat project, pinned to the build of the current Nightly (or to `--horizon <build>`), and fetches
that build's [SDK](../reference/sdk.md):

```text
greeter/
├── horizon-mod.toml     # the manifest; [native] pins the exact Horizon build
├── Cargo.toml           # depends on horizon-mod-api in .horizon/sdk/, never on the game
├── src/lib.rs           # the hooks
├── assets/
├── data/
└── .horizon/sdk/        # the pinned SDK: fetched by the tool, never committed or packed
```

After a fresh clone, run any `cargo horizon-mod` command once before opening the project in an editor: it fetches
`.horizon/sdk/` again.

`cargo horizon-mod update` moves the pin to a newer Horizon later.

## 2. Read the hook

`src/lib.rs` implements one method of `mod_api::Hooks`:

```rust,ignore
{{#include ../../examples/src/greeter.rs:mod}}
```

- Each method of `Hooks` is one moment in the game. You implement only the ones you need; the game calls
  your mod only for those.
- `#[mod_api::horizon_mod]` marks the one `impl Hooks` at the crate root and subscribes the mod to exactly the
  methods it implements.
- The `host` is your handle on the game. A server hook gets a `ServerHost`, a client hook a `ClientHost`, so a call
  on the wrong side does not compile.
- The `event` carries the moment's data. Values you may change are writable fields of the event.

[Writing hooks](writing-hooks.md) explains these rules in full.

## 3. Build and play

```sh
horizon-launcher native-build . --instance "My Instance" --trust-native-code
```

The launcher downloads the prebuilt game for the pinned Horizon build, compiles your crate against it, and makes
that build the one the instance plays. `--trust-native-code` records that you allow native code on this
install: a native mod runs with your rights, outside any sandbox. Start the instance and join a world; the
greeting appears in chat.

After an edit, run `native-build` again. The [build log](../how-to/native-mods/05-build-log.md) shows the compiler's
output when a build fails.

## Next

- [Writing hooks](writing-hooks.md): the crate root, the hook factory, events and hosts.
- [Hooks and the Host](../concepts/hooks-and-the-host.md) and [Parts](../concepts/parts.md): how a mod reaches
  the game.
- [Your first WebAssembly mod](first-wasm-mod.md): the same code, sandboxed.
- [Native mods: building and shipping](../how-to/native-mods.md): what the launcher does, trust, multiplayer and
  servers.
