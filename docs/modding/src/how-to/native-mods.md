# Native mods: building and shipping

A native mod is Rust source in a `.hmod`. The launcher compiles it on the player's machine, with its own pinned Rust
toolchain, against the prebuilt game of the exact Horizon build the mod pins in `[native]`. The game itself is never
rebuilt for a mod: the mods are compiled and linked against that prebuilt game, and the instance then plays the
result.

Native code runs with the player's rights, outside any sandbox. The first install shows a trust notice, and the
launcher builds native mods only on an install that allows native code (`--trust-native-code`, revocable in
**Settings > Storage**). A mod that must run in a sandbox is a [WebAssembly mod](wasm-mods.md).

The pages in this section follow a mod from its source to a player's game and a server:

<!-- section-pages -->

## Living knockback entry

`living_knockback` fires once per full-entry call, before resistance and motion changes.
The read-only `entry` is `KnockbackEntry::Short` when the short overload delegated the push,
otherwise `KnockbackEntry::Full`. To model doubling strength at both entries, double it
once, then double it again only on `Short`: an input of 0.4 becomes 1.6 for `Short`
and 0.8 for `Full`. There is no second hook dispatch. Two non-commuting listeners run
in dispatch order, each completing its rewrites before the next; this does not simulate
two separate passes over the listener list. Cancellation still leaves motion unchanged.

## Living death contract version 2

[`living_death` version 2](https://github.com/horizon-rust/horizon/blob/main/docs/native-mods.md#living-death-version-2) supplies
the accepted killing source and its captured weapon stack. Cancellation keeps the
health a listener leaves, suppresses loot, experience and death messages, and does
not heal the body. An unhealed cancelled body is removed at tick 20 without loot
or another death dispatch.

### Custom statistics

Declare custom statistic identities in `Hooks::register`, using the same
`Registrar` route for native and wasm mods:

```rust,ignore
fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
    registrar.register_custom_stat("steps").unwrap();
}
```

For a mod named `rewrite`, this declares `rewrite:steps` in
`minecraft:custom_stat`. An explicit `rewrite:steps` or another manifest-owned
namespace is also accepted; `minecraft`, foreign namespaces, malformed IDs and
duplicates are refused. Ignoring the returned `Result<(), String>` does not
bypass installation failure. A mod's explicit registration refusal rolls back
its declarations, and the installed inventory freezes before worlds open.

These declarations populate the integrated command/statistic inventory and
append after vanilla's IDs in LAN and client built-in registry reads, preserving
vanilla IDs. Servers declaring custom statistics require Horizon clients with
the same ordered statistic inventory; configuration rejects mismatches before
play. Register the target before rewriting an award to it; an arbitrary
identifier alone does not register a statistic.

`/scoreboard objectives add steps minecraft.custom:rewrite.steps` creates an
objective for this registered counter.
