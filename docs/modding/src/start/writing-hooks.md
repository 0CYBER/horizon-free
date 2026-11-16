# Writing hooks

Every code mod, native or WebAssembly, has the same shape: one crate whose root implements `mod_api::Hooks`.
This page gives the rules that shape follows.

## The crate

- `horizon-mod.toml` is the only source of the mod's id and version. Cargo's package name and version are not
  the mod's identity.
- `Cargo.toml` depends on `horizon-mod-api` (lib name `mod_api`), never on the game's own crates. Other
  dependencies follow [The mod's crate](../how-to/native-mods/01-the-mods-crate.md).
- `src/lib.rs` holds the one marked `impl Hooks`, at the crate root.

## The mod type and its hooks

```rust,ignore
use mod_api::{self, Hooks};

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    // The hooks this mod needs.
}
```

- The mod type implements `Default`; the game makes one value of it per mod.
- `#[mod_api::horizon_mod]` generates the mod's `hooks()` factory and subscribes the mod to exactly the methods
  written in the block. Do not write a factory yourself.
- The marked impl must be at the crate root. If the build reports "cannot find function `hooks`", the marked
  impl is in a submodule.
- Hook methods are written out in the block; a method a macro would generate inside the impl is not seen.

## Events and hosts

A hook method takes the side's host and the moment's event:

```rust,ignore
fn player_logged_in(&self, host: &mut dyn ServerHost, event: &mut PlayerLoggedInEvent<'_>)
```

- The **event** carries what happened. Fields a mod may change are writable, and a hook that can be cancelled
  has a way to say so in its event or return value ([Hooks and the Host](../concepts/hooks-and-the-host.md)).
- The **host** is the game. Server hooks receive a `ServerHost`, client hooks a `ClientHost`; a hook that fires on
  both sides receives a `HostSide` to match on. A call on the wrong side does not compile.
- Handles in the event (a player, an entity, a level) have their own methods: reads take the host, writes take a
  server host.

## Order between mods

Mods run in dependency order: a mod after the mods it depends on. A hook method may carry
`#[listener(priority = <i32>, receive_cancelled)]`, both optional:

- mods run by descending `priority` (0 when omitted), then in dependency order;
- with `receive_cancelled`, the mod is still called after an earlier mod cancels the event, and can read and
  clear `event.cancelled`. It compiles only on a hook a mod may cancel.

## Per-object behaviour

A moment that belongs to one block, item, block entity, menu or entity type is a method of a **Part**, not a global
hook: the game calls it only for the objects your mod attached the Part to. See [Parts](../concepts/parts.md).
