# Native mods

Native mod docs live in the modding book: [native mods](modding/src/how-to/native-mods.md), [trait mods, Parts and attachments](modding/src/concepts/parts.md) and the [hook notes](https://github.com/horizon-rust/horizon/tree/main/crates/horizon-mod-vocab/src/hooks).

## Living death version 2

`Hooks::living_death` receives a mutable `LivingDeathEvent` on the server only.
Its fields are `entity`, `kind`, `source: &HurtSource`,
`weapon: Option<&ItemStack>` and `cancel`. This is a clean contract break:
the old top-level `damage_type` and `attacker` fields and callback signature
are removed, with no compatibility callback.

The source is the accepted killing blow seen by `living_incoming_damage`:
damage type, real causing entity ID (including players), direct entity ID
(including arrows), and original optional source position. The optional weapon
is that blow's complete stack, including damage and component overrides, not
a later read of the attacker's hand.
`/damage` keeps the selected registered trident type rather than relabeling it
as thrown damage. Its explicit `by` (direct) and `from` (causing) entities remain
independent, including a locally bound causing player. A damage type alone does
not manufacture an arrow-family direct entity.

Cancellation suppresses death loot, experience and death messages without
writing health. A listener that sets health to 5 leaves the entity alive at 5;
an unhealed body at health <= 0 is removed after 20 ticks without redispatch
or loot. Cancellation alone does not heal or force health to 1.
The cancelled continuation belongs to the living body: full-body teleports preserve
its original removal deadline, rather than restarting it or retrying death.
A later positive `set_health` revives a cancelled dying body, clears that deadline
and permits a subsequent genuine death; transaction abort restores the prior
health and continuation together. Saved mobs retain the optional Horizon-owned
`horizon:death_canceled` bit alongside their real `DeathTime`; ordinary loading
resumes a saved dying body without running its death callback or loot again.

### Portable contract regression

`crates/horizon-mod-wasm/tests/fixture/src/lending.rs` supplies the same
`Lender::living_death` listener and `check_death` assertions to the native
`collect_positions_copy_in_and_out_natively_as_in_the_real_guest` test and
the compiled-guest `real_member` test. They dispatch a player/arrow source
with distinct causing/direct IDs and a nonintegral position, and check the
bow's identity, damage and complete custom-component patch, both optional
weapon branches, cancellation, and an unchanged borrowed weapon. Their scope is
the portable record boundary, not Game death timing or loot behavior.

Build the matching real guest and standard module, then run both selectors:

```sh
cargo build -p cargo-horizon-mod
# From crates/horizon-mod-wasm/tests/fixture, with ROOT set to the repository:
"$ROOT/target/debug/cargo-horizon-mod" horizon-mod build --wasm
"$ROOT/target/debug/cargo-horizon-mod" horizon-mod wasm-std "$ROOT/target/horizon-wasm-std.wasm"
# From the repository root:
cargo test -p horizon-mod-wasm --features listener-fixture --test native_listener collect_positions_copy_in_and_out_natively_as_in_the_real_guest -- --exact
HORIZON_WASM_FIXTURE="$PWD/crates/horizon-mod-wasm/tests/fixture/target/horizon-mod/wasm_fixture.hmod" HORIZON_WASM_STD="$PWD/target/horizon-wasm-std.wasm" cargo test -p horizon-mod-wasm --features listener-fixture --test real_member real_member -- --ignored --exact
```

### Victim ownership and hot path

One immutable accepted-hit record follows the mob or player body; there is no
separate death-weapon table. Mob reconstruction moves it through the existing
non-Clone vehicle transfer token, along with the processed death attempt; the
real `DeathTime` NBT codec preserves the original removal deadline. A full player
teleport restore moves the record through the existing body-replacement boundary,
and respawn discards it.
Each body stores an optional shared pointer; snapshots share the immutable
record rather than copying its source name and stack. Records are captured only
for listened fatal deaths/loot or accepted projectiles needed by `loot_drops`.
Without those listeners, no accepted record or saved weapon is allocated.
Armor's existing deferred event carries the actual cause, direct entity and
position as scalars; synchronous equipment wear borrows the paused hit's name.
The attack scope retains a projectile's weapon only when incoming damage,
death or loot listeners need it. No idle-tick sweep or second source map is added.

Prepared-resource Game regressions live in
`game::native_hooks::death_source_tests`; the player cases additionally require
loopback sockets. `game::lan_actions::guest_body::tests::accepted_hit_moves_through_teleport_restore_but_not_respawn`
covers the ownership transfer. These are source regressions, not a record of a
completed run.

The clean-room contract references are the public
[NeoForge 26.3 LivingDeathEvent Javadoc](https://aldak0.ru/javadoc/26.3.x/net/neoforged/neoforge/event/entity/living/LivingDeathEvent.html)
and [ArmorHurtEvent Javadoc](https://aldak0.ru/javadoc/26.3.x/net/neoforged/neoforge/event/entity/living/ArmorHurtEvent.html),
plus the pinned game's `LivingEntity.die`, `LivingEntity.tickDeath` and
`DamageSources` factories. NeoForge's LGPL source and patches were not read.

## Prebuilt SDK layout

The v4 prebuilt archive separates target artifacts (`deps/`) from host proc-macro
libraries (`host/`). Cargo builds with an explicit `--target <rustc host triple>`,
so host build dependencies cannot introduce a second metadata variant into the
target SDK. Only host proc-macro dynamic libraries are shipped, not their host
dependency metadata. Consumers pass both folders as `-L dependency` paths; named
extern and facade lookups use only `deps/`. `native/` still holds native linker
inputs. The identity manifest hashes files in all three folders.

Target compilation retains `-C prefer-dynamic` (and `-C rpath` on macOS), while
Cargo's explicit target keeps those flags off host proc-macros. Multiple versions
of a target dependency are legitimate: uniqueness is required only when selecting
a named extern or publishing an internals facade, never across the whole SDK.

To rebuild and prove the archive with the launcher's pinned Rust toolchain active:

```sh
python3 tools/prebuilt_game.py build prebuilt
python3 tools/prebuilt_game.py smoke prebuilt
python3 tools/prebuilt_game.py exports prebuilt
python3 tools/native_dylib_probe.py --sdk prebuilt --output target/native-dylib-probe
```

For a server archive, add `--server` to the build command. Use a fresh
`CARGO_TARGET_DIR` when collecting a clean-build proof; all commands must use the
same compiler as `launcher/src/rust_toolchain.rs`.

## Upstream service contracts carried by this sync

The upstream additions below describe behavior to preserve in the typed `Host` and
handle API, not additional `native_*` entry points or a bundle API counter. Use the
book and generated API reference for the available signatures; pending hook fragments
must be converted to the hook catalog before being advertised as available.

- Tamed-mob death messages are offered after construction and only when an online
  owner would receive them with `show_death_messages` enabled. Listeners may replace
  or suppress the message in dispatch order. The server-only named-mob log is not
  a second player-message hook.
- Dedicated-server settings describe the live listener: account authentication,
  bound port and whether Velocity forwarding is active. They never expose the
  forwarding secret, and are unavailable before listening or on integrated/LAN
  servers. Secure-profile enforcement and hardcore are not part of this read.
- Vehicle and passenger reads use entity handles (`entity_vehicle` and
  `entity_passengers` on `Host`), not parallel numeric-ID helpers. Passenger order
  follows vanilla, with the controlling passenger first. These are read-only.
- UUID lookup from a read-only context must not swap levels. A mutable lookup can
  reach other loaded levels; entity NBT reads there must refuse unloaded levels,
  missing entities and client-side use rather than load a level implicitly.
- Map reads return the 128 × 128 row-major color bytes and scale, dimension and
  locked metadata. Reading saved data must not install a loaded map as a side
  effect; unknown maps and client-side world reads return no value.
- Living-entity sight uses eye-to-eye COLLIDER shapes, no fluids, the source's
  collision context and vanilla's 128-block limit. Unknown entities, different
  levels and unavailable server levels must not report visibility. The typed
  `living_has_line_of_sight` and `level_line_of_sight` reads replace raw-ID helper
  proposals; the point-to-point form has no entity collision context.
- Camera submersion (main's `native_camera_submersion(camera, fog_type)`, API 646): a client
  listener runs when the camera's fluid submersion type is evaluated
  (`Camera.getFluidInCamera`), before fog and underwater screen overlays are prepared.
  Setting `fog_type` to `FogType::Atmospheric` clears underwater, lava and powder-snow fog
  and overlays (Freecam uses it). Hook entry: `camera_submersion` v1, client, `listen`.
- Client entity update (main's `native_client_entity_update(viewer, id, nbt) -> bool`,
  `host.client_entity_update`): like `native_client_entity_set` for a few fields. It reads
  `nbt` as `set` does but sends only the data of the keys `nbt` has (`transformation` sends
  its four parts); a key at its default value is sent, so a field can go back to it. A
  display's `start_interpolation` goes with it, so a changed transformation eases over
  `interpolation_duration`. `set` still sends every field. False when `id` isn't this
  viewer's client-side entity, when the viewer isn't on, and outside a player's hook.

These are contract notes, not runtime verification of the converted services.

## Living absorption

Living handles expose `absorption(host)` on both sides and
`set_absorption(host, amount)` on the server. Resolution follows health: armor
stands, dragons and stale handles return `None` on reads. A non-finite write
returns `false` and changes nothing; accepted amounts clamp to
`0..=max_absorption` for that entity. Player and mob writes roll back with the
transaction. This is the absorption amount, not the absorption status effect.
