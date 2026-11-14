# Controlling spawns

Use spawn hooks to accept or reject a spawn, edit a mob before finalization, or control despawning. All four hooks run on the server with a read-only host and fire on replayed ticks. Your listeners must be pure functions of their inputs. `finalize_spawn` can also change the mob it is about to finalize, through the editor it is handed, and the pack's group data; it changes nothing else.

| Moment | Hook | Answer |
|---|---|---|
| The spawn predicate for a type and position | `mob_spawn_placement_check` | `SpawnCheck::{Default, Succeed, Fail}` |
| The mob's own position and obstruction checks | `mob_spawn_position_check` | `SpawnCheck::{Default, Succeed, Fail}` |
| Just before `finalizeSpawn` | `finalize_spawn` | `Flow::Cancel`, `spawn_cancelled`, a new `difficulty`, edits to `mob`, a new `group` |
| Each tick, before a mob's `checkDespawn` | `mob_despawn` | `Despawn::{Default, Allow, Deny}` |

## Saying where a mod mob spawns

`Registrar::declare_spawn_placement(path, rule)` is NeoForge's `RegisterSpawnPlacementsEvent`: during registration a mod gives its entity type a `SpawnPlacementRule`, and the game's own spawn checks answer from it for that type, wherever they run (natural spawning, chunk generation, spawners, patrols, sieges, reinforcements).

```rust,ignore
r.declare_spawn_placement("ghoul", mod_api::SpawnPlacementRule {
    placement: mod_api::SpawnPlacementType::OnGround,
    heightmap: mod_api::Heightmap::MotionBlockingNoLeaves,
    spawnable_on: Some("mymod:ghoul_ground".to_owned()), // a block tag the block below is in
    light: mod_api::SpawnLight::Dark,              // Any, Dark (a monster's) or Bright (an animal's)
    biome_tag: Some("mymod:haunted".to_owned()),         // a biome tag the spot's biome is in
}).unwrap();
```

The placement type is `SpawnPlacements.isSpawnPositionOk`'s first check, and the rule stands in for the type's spawn predicate: every field that is set must hold, and a trial spawner ignores `light`. A type with no declaration keeps its template's rules. The game resolves the declarations once at load, so a check reads one table and never walks the mods. A tag is a namespaced name written without `#`; an id outside the mod's namespace, a bad tag or a second declaration of the same type refuses the mods at startup, and a type no content file defines refuses the resource load.

`mob_spawn_placement_check` is the separate per-check moment: it runs after the rule and is told its answer as `default_result`, so you can override it for one position.

## Walking every entity in a level

`host.level_all_entities(level)` returns the authoritative level's available
`EntityRef`s without an AABB filter, including spectator players. Unlike
`level_entities_in`, it does not append the dragon's parts: vanilla's
`ServerLevel.getAllEntities` iterates the entity lookup, not multipart hit boxes.
A remote client, stale handle or parked level returns `None`, never a partial
client-side list. Outside a scoped hook query, an unavailable lookup member makes the query return `None`.
Inside a scoped hook query, unavailable members are omitted.

The walk follows the level's entity lookup admission order, not spatial order.
Removal and re-admission moves an entity to the end. Chunk entities join section
by section, each in stored admission order. Entities that move sections retain
their admission place rather than moving to that section's end. Zero runtime IDs
use stable storage join order; unregistered entity kinds have no authoritative
lookup handle and are omitted. Cache the returned list
when you need several passes in one hook rather than repeating the host query.

## Checking a spawn

`mob_spawn_placement_check` and `mob_spawn_position_check` both get the `level` the mob would join (a `LevelRef`, so a
listener can read the blocks around `pos` through the host) and the `spawner` that asked: `Spawner::Block { pos }` for a
spawner block, `Spawner::Trial { pos }` for a trial spawner, and `None` for natural spawning, chunk generation, patrols,
sieges, reinforcements and a spawner minecart. Set `*event.result` to `SpawnCheck::Succeed` or `Fail` to force the
check; `Default` keeps vanilla's answer.

```rust,ignore
fn mob_spawn_placement_check(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobSpawnPlacementCheckEvent<'_>) {
    // spawner blocks work as usual; nothing else spawns zombies here
    if event.kind == "minecraft:zombie" && event.spawner.is_none() {
        *event.result = mod_api::SpawnCheck::Fail;
    }
}
```

The position check also gets `event.mob`, the mob about to spawn, as an `UnspawnedMobRef` with the same reads as
`finalize_spawn`'s (no editor: the hook only reads). Vanilla creates the mob before this check; Horizon creates it there
only when the hook has a listener, the same way the spawn would, and the spawn then goes on with that very mob, so what
the listener read is what joins the level. A refused check drops the mob: it never joins the level. Creating it draws
nothing from the level's random, so a listener that answers `Default` changes no draw.

```rust,ignore
fn mob_spawn_position_check(&self, host: &dyn mod_api::Host, event: &mut mod_api::MobSpawnPositionCheckEvent<'_>) {
    if event.mob.custom_name(host).is_ok_and(|name| name.is_some()) {
        *event.result = mod_api::SpawnCheck::Succeed;
    }
}
```

Where the game does not create the mob fresh, the handle answers only `kind` and `position` (the rest is
`UnspawnedMobError::Unsupported`): chunk generation's packs, which the world generator creates and finalizes after the
check, and a spawner or trial spawner whose spawn data carries entity data, which the game loads after the tick.

## Stopping a spawn or leaving a mob plain

`finalize_spawn` has two cancels. Returning `Flow::Cancel` skips finalization: the mob spawns, but with no equipment, attributes or class setup. Setting `event.spawn_cancelled = true` adds no mob at all.

```rust,ignore
fn finalize_spawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::FinalizeSpawnEvent<'_>) -> mod_api::Flow {
    if event.kind == "minecraft:zombie" && event.reason == mod_api::SpawnReason::Natural {
        event.spawn_cancelled = true;
    }
    mod_api::Flow::Continue
}
```

`finalize_spawn` fires only where vanilla calls `finalizeSpawn`, so a breeding, an infested block's silverfish or a thrown egg's chick does not tell it.

For `/summon minecraft:ender_dragon` without NBT, command placement starts at zero
rotation and consumes no random yaw draw. Skipping finalization also skips its
follow-range and handedness draws. Spawn eggs retain their random placement yaw;
an explicit NBT compound follows the load route without finalization.

## Changing the mob before it is finalized

`event.mob` is the mob itself, an `UnspawnedMobRef`: it is not in a level, so it is not an `EntityRef`, and only this
hook call can use it. Read it through the host (`kind`, `position`, `equipment`, `attribute_base`, `attribute_value`,
`tags`, `custom_name`) and change it through `event.edits`, which reaches only this mob: equipment (an API-owned `ItemStack`), an attribute's base or a permanent modifier, tags and the custom name. An edit applies at once, so the
reads and the next listener see it. Once the hook returns, the handle answers `UnspawnedMobError::Expired`, as does a
handle from another call passed to this call's editor.

```rust,ignore
fn finalize_spawn(&self, host: &dyn mod_api::Host, event: &mut mod_api::FinalizeSpawnEvent<'_>) -> mod_api::Flow {
    if event.mob.kind(host).is_ok_and(|kind| kind.path() == "zombie") {
        let health = mod_api::Identifier::parse("minecraft:max_health").unwrap();
        let _ = event.edits.set_attribute_base(event.mob, &health, 40.0);
        let _ = event.edits.add_tag(event.mob, "mymod:tough");
    }
    mod_api::Flow::Continue
}
```

Listeners run before finalization. Finalization's own rolls follow them: equipment it rolls for a
slot replaces what a listener put there, and the attribute bonuses it adds apply on top of a listener's base. Return
`Flow::Cancel` to keep exactly what the listener left, since no finalization runs. Attribute edits reach the
attributes a mob keeps itself (max health, movement speed, follow range, attack damage, knockback resistance,
spawn reinforcements, jump strength, burning time, flying speed); the others answer `Unsupported` until the mob has
joined, when a `LivingRef` writes them. The ender dragon's handle answers its kind and position, and `Unsupported`
for the rest. A mob that finalization creates (a chicken jockey's chicken, a zombie horse's rider) fires the hook
again with a handle of its own.

The hook fires on replayed ticks with the read-only host, so the editor and `group` are the only things a
listener changes, and it must change them the same way every time: a pure function of the event.

`event.group` is the pack's `SpawnGroupData` as plain data (`SpawnGroup`, variants named as vanilla saves them):
read it, change it, set it or clear it, and finalization reads what is left. Each hook call receives `None`; group data is not passed between natural pack members.
Chunk generation shares group data inside the generator, outside this hook. A group you set affects
only this mob's finalization.

See [the `finalize_spawn` contract](https://github.com/arismoko/horizon/blob/main/crates/horizon-mod-vocab/src/hooks/finalize_spawn.rs) for the spawn sites that fire the hook.

## Changing how large a spawn group grows

A natural spawn adds mobs of one type to a group until the group reaches the type's limit (`Mob.getMaxSpawnClusterSize`: 4 by default, more for horses, wolves and schooling fish, 1 for ghasts and pillagers). The entity part of the type changes that limit with `EntityPart::spawn_cluster_size`, which gets the read-only host, the mob just spawned, why and from what it spawned, and the limit vanilla would use. Return `Some(limit)` to replace it, or `None` to leave it to the next part and then to vanilla. The first part to answer wins, and the group stops growing once its size reaches the limit.

```rust,ignore
struct ZombiePacks;
#[mod_api::horizon_part]
impl mod_api::EntityPart for ZombiePacks {
    fn spawn_cluster_size(
        &self,
        _: &dyn mod_api::Host,
        _: mod_api::UnspawnedMobRef,
        _: mod_api::SpawnReason,
        _: Option<mod_api::SpawnSource>,
        vanilla: u32,
    ) -> Option<u32> {
        Some(vanilla * 2)
    }
}
```

Only natural spawning's packs ask it: vanilla has no group limit anywhere else. A type with no part keeps vanilla's limit and costs nothing. The mob is the same `UnspawnedMobRef` as `finalize_spawn`'s, finalized and not in a level, and the call runs on replayed ticks like `finalize_spawn`, so an answer must be a pure function of its inputs. Attach the part with `r.entity_part("minecraft:zombie", ZombiePacks)` as [entity parts](../../concepts/parts/entity-parts-and-mob-goals.md) describe.

## Keeping or removing a mob

`mob_despawn` fires once per tick for every mob in a loaded chunk, just before vanilla's `Mob.checkDespawn`, persistent mobs and mobs in peaceful mode included. `Allow` discards the mob, `Deny` keeps it, `Default` leaves vanilla's rules to decide. An answer other than `Default` skips all of vanilla's checks, so a `Deny` keeps a hostile mob in peaceful mode and neither answer takes the idle-despawn `nextInt(800)` draw.

```rust,ignore
fn mob_despawn(&self, _: &dyn mod_api::Host, event: &mut mod_api::MobDespawnEvent<'_>) {
    if event.kind == "minecraft:zombie" {
        *event.result = mod_api::Despawn::Deny;
    }
}
```

The wither and the ender dragon override `checkDespawn` in vanilla, so the event is not fired for them.

## Adding or removing a custom spawner

The level's custom spawners (phantoms, patrols, cats, village sieges, the wandering trader) are a list a mod edits once, in `modify_custom_spawners`. A mod's own spawner is a key plus `custom_spawner_tick`, which runs every tick while mob spawning is on:

```rust,ignore
fn modify_custom_spawners(&self, _: &dyn mod_api::Host, event: &mut mod_api::ModifyCustomSpawnersEvent<'_>) {
    // no phantoms in this world
    event.spawners.remove(&mod_api::SpawnerEntry::Builtin(mod_api::SpawnerKind::Phantom));
    event.spawners.add(mod_api::SpawnerKey::new("mymod:rat_swarm").unwrap());
}

fn custom_spawner_tick(&self, host: &mut dyn mod_api::ServerHost, event: &mut mod_api::CustomSpawnerTickEvent<'_>) {
    if event.key.as_str() == "mymod:rat_swarm" && event.spawn_enemies {
        // spawn through `host`
    }
}
```

Every mod is asked about every key, so a mod ignores keys it does not own. See [the custom spawner contracts](https://github.com/arismoko/horizon/blob/main/crates/horizon-mod-vocab/src/hooks/custom_spawners.rs).

## Spawning from a level

`LevelRef::spawn_mob(host, kind, pos, reason)` is `EntityType.spawn`: the mob is made as any of its type is, Parts, attachments and `entity_constructing` included, then `finalize_spawn` and `finalizeSpawn` run with `reason`, and it joins the level (`entity_join_level`; inside a transaction, at its root commit), not persistent. It returns the mob's handle, or `None` with nothing spawned: a kind that isn't a mob, a position that isn't finite, outside `Level.isInSpawnableBounds` or in an unloaded chunk, a level not in place, a client, or a mod that cancelled the spawn or kept the mob out. The level's spawn limits, flags and spawn rules don't apply, as they don't to `/summon`. Items, experience and block drops have their own calls (`spawn_item`, `award_experience`, `pop_resource`), and any other entity `summon`.

A level's natural spawning (`NaturalSpawner.spawnForChunk`) also has a policy a mod can change:

- `spawn_limits(host)` and `set_spawn_limits(host, limits)`: a `SpawnLimits` with each spawning category's `getMaxInstancesPerChunk`, `SpawnLimits::VANILLA` (70 monsters, 10 creatures, 15 ambient, 5 axolotls, underground water and water creatures each, 20 water ambient) until a mod sets others. A limit caps the category twice, as vanilla's does: over the level, under `limit * spawnable chunks / 289`, and around each player near the chunk, under `limit`. A limit of 0 stops the category.
- `spawn_flags(host)` and `set_spawn_flags(host, flags)`: a `SpawnFlags` with `monsters` and `friendlies`, both on until a mod turns one off. Off keeps that side out of the level's natural spawning; neither turns monsters on against the `spawn_monsters` rule or anything on with `spawn_mobs` off.
- `can_spawn_entities_in_chunk(host, [x, z])`: `ServerLevel.canSpawnEntitiesInChunk`, the question natural spawning asks of a spawn position outside the chunk it started in. Horizon's spawner answers it as "the chunk is loaded", and so does this call.

```rust,ignore
fn level_load(&self, host: &mut dyn mod_api::ServerHost, event: &mut mod_api::LevelLoadEvent<'_>) {
    let Some(level) = host.level(event.dimension) else { return };
    // twice the monsters, no bats
    level.set_spawn_limits(host, mod_api::SpawnLimits { monster: 140, ambient: 0, ..mod_api::SpawnLimits::VANILLA });
}
```

Only natural spawning reads the policy: spawners, trial spawners, patrols, sieges, phantoms, a new chunk's creatures and custom spawners keep their own rules. A change counts from the level's next spawn pass, and mobs already there stay. The policy is not saved. It survives parking and reopening a level within the same world session, but closing the world resets it. Set it in `level_load`. Reads work on any host and answer `None` on a client or for a stale handle; the writes need a `ServerHost` and return `false` there.
