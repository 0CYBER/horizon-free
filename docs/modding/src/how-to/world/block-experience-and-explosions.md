# Block experience and explosions

Attach a `BlockPart` to the blocks that own the behavior.

## Experience

`exp_drop(level, event)` answers `Some(points)` to replace the vanilla experience
sample, before enchantments. `None` leaves the answer to another attached part
and then vanilla. The query receives the dimension, state, position, optional
block entity, breaker entity handle and tool. The level host reads without writing;
`level.block_entity_data(entity)` reads the supplied entity's saved data without
identity metadata and refuses stale handles. Answers are clamped to 0 through
32,767. Player breaks supply the part's answer to the global `break_block`
event's `exp_to_drop`, after Silk Touch suppresses the starting amount (except
for spawners). An explicit global override is applied as given. `block_drops` sees the resulting amount afterwards.
Level and mob breaks use the shared resource-drop path. A canceled player break
or a break that does not harvest produces no award.

## Explosions

`on_block_exploded(host, event)` runs before the engine removes each destroyed
block. The server host permits writes. `event.block_entity` is a live handle,
and `entity.data(host)` can read it while the block remains. The event borrows
the same `ExplosionStart` description as the global start event: dimension,
center, radius, fire, interaction, damage type, source and owner.

The engine writes air after the callback, including when the callback
replaced the block. Vanilla's separate `wasExploded` behavior follows removal.
Air, spared blocks, canceled explosions and explosions that keep blocks produce
no callback. The initial explosion experience amount is zero.

`explosion_resistance(level, event)` answers `Some(resistance)` to replace the
block's own explosion resistance against `event.explosion`, as a ray of the
blast enters the block. The ray's next steps in the same block reuse the
answer, so keep it a pure function of the event. A fluid in the cell still
raises the value to 100, and the overrides vanilla applies afterwards keep
winning: a respawn anchor's water cell, a TNT minecart's rails and the blocks
under them (never asked) and a dangerous wither skull's 0.8 cap. A negative or
non-finite answer is refused with a warning and the block's own value is used.

`can_drop_from_explosion(level, event)` answers whether a destroyed block drops.
`Some(false)` skips its loot and its `spawnAfterBreak` (an infested block's
silverfish, an ore's experience); `Some(true)` drops even for TNT, the one
block vanilla keeps from dropping. The block is removed either way, and a TNT
still primes. Both queries get a read-only level host and the same
`ExplosionStart` as `on_block_exploded`; `None` leaves the answer to another
attached part and then vanilla.

Every method on this page routes through prepared per-block subscriptions.
Unsubscribed blocks build no callback context. A native `exp_drop` panic
unwinds before the engine applies its answer. A native
`on_block_exploded` panic is contained and reported as
`on_block_exploded callback failed: {error}`; block removal continues. A panic
in `explosion_resistance` or `can_drop_from_explosion` is contained the same
way, and vanilla's answer is used.

## Setting off an explosion

`level.explode(host, source, center, radius, fire, interaction, settings)` is
vanilla's full `Level.explode(source, damageSource, calculator, x, y, z, radius,
fire, interaction, smallParticles, largeParticles, blockParticles, sound)`, on
the server, in the level in place. `ExplosionSettings::default()` is the short
form; its fields are under [Damage type, calculator and effects](#damage-type-calculator-and-effects). `source` is the exploding entity (`Option<EntityRef>`). The
entity that caused the blast is never passed: it comes from the source, as
vanilla's `Explosion.getIndirectSourceEntity` has it.

| `source` | Causing entity |
| --- | --- |
| A primed TNT | Its owner (the player or mob that lit it), or none |
| A living entity (a player, a mob, the ender dragon, an armour stand) | The source itself |
| A projectile (an arrow, a fireball, a thrown trident, a firework rocket, a fishing bobber...) | Its owner when that is living, else none |
| Anything else, or `None` | None |

With a source and a causing entity the damage is `minecraft:player_explosion`,
otherwise `minecraft:explosion` (unless `settings` names its own), and the
causing entity gets the kill: a mob
killed by a blast whose source is player-lit TNT dies to that player, and one
killed by unowned TNT to no one. The source itself is not hurt or pushed by its
own blast, and the blocks it breaks roll their loot with it as "this entity". A
TNT block the blast reaches primes, owned by the causing entity.

`interaction` says what happens to blocks:

| `ExplosionInteraction` | Blocks |
| --- | --- |
| `None` | Stay |
| `Block` | Break, drops decay by `block_explosion_drop_decay` |
| `Mob` | Break when the source may grief (`entity_mob_griefing` for it; without a source, the `mob_griefing` rule), drops decay by `mob_explosion_drop_decay`; otherwise stay |
| `Tnt` | Break, drops decay by `tnt_explosion_drop_decay` |
| `Trigger` | Stay; buttons, levers, doors, trapdoors, fence gates, bells and candles react as to a wind charge |

`fire` lights about a third of the air above solid blocks in reach, whatever the
interaction, `None` included. The steps run in vanilla's order:
`explosion_start` (which may cancel it), the blocks the rays reach,
`explosion_detonate`, entity damage and knockback, the blocks, the fire, then
what players hear and see. A blast set off inside another blast's hook keeps
its own source and cause and leaves the other blast's as they were.

There is no radius limit, as in vanilla. A blast casts 1352 rays whatever its
radius; their steps grow with the radius and the distinct positions they find
roughly with its cube, so a big blast costs what it finds (a ray of a radius-48
blast takes up to about 280 steps). A radius of 0 or below finds no blocks and
hurts no one, but `explosion_start` still hears it and players still see it;
`explosion_detonate` only runs from a radius of 0.00001.

### Damage type, calculator and effects

Every `ExplosionSettings` field left `None` is vanilla's own choice.

| Field | What it does | `None` |
| --- | --- | --- |
| `damage_type` | A registered damage type (`mymod:fuel_tank`): its death message, its tags (`#is_explosion`, armour, immunity), and the type `living_damage` and `explosion_start` hear | The one derived above |
| `calculator` | `ExplosionCalculator`, vanilla's `SimpleExplosionDamageCalculator` | The source's calculator (below) |
| `small_particle` | The centre particle of a small blast (a radius under 2, or one that keeps its blocks) | `minecraft:explosion` |
| `large_particle` | The centre particle of any other blast | `minecraft:explosion_emitter` |
| `block_particles` | `ExplosionBlockParticle`s (particle, scaling, speed, weight), one picked by weight for each bit of debris; an empty list throws none | `minecraft:poof` (scaling 0.5) and `minecraft:smoke`, one each |
| `sound` | The sound event | `minecraft:entity.generic.explode` |

The calculator's fields:

- `explodes_blocks`: false finds no blocks, so nothing breaks, burns or reaches
  `explosion_detonate`'s block list.
- `damages_entities`: false leaves every entity's health alone (decorations and
  end crystals too); they are still pushed. The damage is the default formula.
- `knockback_multiplier`: how hard entities are pushed, 1 when `None`. A flying
  player (the ability, not creative mode) is pushed 0. An entity neither damaged
  nor pushed (`damages_entities` false and a multiplier of exactly 0) is not
  checked for exposure.
- `immune_blocks`: a block tag whose blocks never break. **When given it
  replaces the resistance rule entirely**: its blocks resist 3,600,000 and every
  other cell resists nothing, obsidian and fluids included, and no part's
  `explosion_resistance` is asked. Leave it `None` to keep the usual rule, with
  your parts' answers.

Without a calculator the source brings its own, as its own blast does in
vanilla:

| Source | Calculator |
| --- | --- |
| A primed TNT that went through a portal | Nether portal blocks resist nothing and never break; every other cell as usual |
| A wind charge (a player's or a breeze's) | The blocks of `#blocks_wind_charge_explosions` (barrier, bedrock) resist 3,600,000; every other cell, fluids included, nothing |
| Anything else, or `None` | The default calculator |

With a calculator given, the source's is not used, as in vanilla.

A particle is a vanilla particle type that takes no options (`minecraft:flame`,
`minecraft:end_rod`; not `minecraft:block` or `minecraft:item`). The packet
holds each block particle's weight as an `int`, so a weight, or the list's
total, past 2,147,483,647 is refused.

### Refusals

`explode` returns an `ExplodeError`, explodes nothing and tells no hook:

| `ExplodeError` | When |
| --- | --- |
| `Unavailable` | A client, a parked level or a stale handle, a `source` that is stale, names no entity or is a player in another level, a centre in a chunk that isn't loaded |
| `InvalidBlast` | A centre outside the world or not finite, a radius or knockback multiplier that isn't finite |
| `UnknownDamageType` | A damage type that isn't registered |
| `UnknownBlockTag` | An `immune_blocks` that names no block tag |
| `InvalidParticle` | A particle that isn't a particle type taking no options, a block particle with a weight of 0 or a scaling or speed that isn't finite, or a weight or total of weights past 2,147,483,647 |
| `UnknownSound` | A sound that isn't a sound event id, or a `minecraft:` one the server doesn't have |

## Entities that destroy blocks

`can_entity_destroy(host, event)` answers whether `event.entity` may destroy the
block, where vanilla asks its per-block rule: the wither's block break (any
block but air and `#wither_immune`), a dangerous wither skull's blast (the
entity is the skull, still readable; only a block it may destroy has its
resistance capped at 0.8), the dragon's wall check, a zombie breaking a door
and a zombie choosing a turtle egg to trample. `Some(true)` makes a block
vanilla spares destroyable for that entity, such as bedrock for the wither;
`Some(false)` spares one it would break, and the dragon is then in a wall. The
griefing gate runs first, so with `mob_griefing` off nothing breaks whatever
the part says, and `living_destroy_block` can still keep a block the part
allowed (the turtle-egg choice fires none). For the dragon,
`#dragon_transparent` blocks are skipped before the part is asked, and a
`#dragon_immune` block stays even when the part says yes. A ravager asks only
the griefing event. The host is read-only. The dragon asks every
tick for each block in its head, neck and body, so answer fast. A panic is
contained and vanilla's rule is used.

## Pistons

`on_destroyed_by_push_reaction(level, event)` hears a piston's move destroy the
block (a `Destroy` push reaction, such as a torch's), after the block dropped
its resources and while it is still in its cell. `event.direction` is the way
the blocks move and `event.fluid` is the fluid state the cell holds. The removal still happens: the cell becomes air afterwards.
The level host writes and schedules ticks, but a write to any cell of the move
is lost. The destroyed cells turn to air, and the pushed blocks were read
before, so each moves as it was and the cells they leave and enter end as air
or a moving block, as does the arm. Writes outside the move stay. A move that
`piston_pre` cancels and a client's replay of a move are not heard. A panic is
contained and the move goes on.
