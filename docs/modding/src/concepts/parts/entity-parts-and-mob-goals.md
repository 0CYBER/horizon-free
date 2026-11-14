# Entity parts and mob goals

Use entity Parts in native mods. `r.entity_part` is absent from wasm builds.

`r.entity_part("minecraft:zombie", ZombieGoals)` attaches an `EntityPart` to one entity type, a mod's
own or vanilla's. Its method `register_goals` runs at every `Mob` construction of that type —
spawned or loaded, the moment vanilla's `registerGoals` fan-out ends — and the goals it adds join the
mob's two selectors beside the species' own, with the same priorities, flag locks and interruption:

```rust,ignore
struct ZombieGoals;
#[mod_api::horizon_part]
impl EntityPart for ZombieGoals {
    fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
        // goals.mob().id() is "minecraft:zombie": branch as per-species code does.
        goals.add_goal(7, Stare::default());
        goals.add_target_goal(3, Remember::default());
    }
}
```

`goals.add_goal(priority, goal)` and `goals.add_target_goal(priority, goal)` are `GoalSelector.addGoal`
on the mob's two selectors; `GoalFlags` claims `MOVE`, `LOOK`, `JUMP` and `TARGET` as vanilla's `Goal.Flag`
does, and an eligible goal with a higher priority (a lower number) interrupts a running goal
holding the same flag only when the running goal's `Goal::is_interruptable` returns true. A goal
object is per mob, built fresh at every construction, and its state is never saved: a goal that needs
memory across loads keeps it in attachments. The goal wrapper and read view are created only where a mod goal runs.

A `Goal` reads and writes through its `GoalContext`: `mob()` is a fresh handle of the mob it runs on,
`host()` the tick's read view, `random()` the mob's own random (`mob.getRandom()`, so a mod's draws
keep the mob's stream in shape). The read view answers the ticking mob itself directly, the player the
tick sees, and every other entity from the tick's snapshot — a `getEntities` of the moment the mob
list was taken out — so a read the snapshot does not carry (a mob's gear or effects, its species id)
answers `None`, and the handle docs mark the reads that carry the snapshot rule. Writes apply to the
mob itself through the engine's own paths: `move_to(pos, speed)` and `stop_navigation()`
(`PathNavigation`), `look_at(pos)` (`LookControl`), `jump()`, and `set_target(Option<LivingRef>)`
(`Mob.setTarget`, with the write handle's rules: a fox, an enderman or a zombified piglin stores it
with its setter's side effects, and a brain-backed species refuses, since its target is a brain
memory that `MobRef::set_brain_target` writes from a server host). Writes to other entities
are not part of a goal; the write handles a `ServerHost` takes do not exist in a goal tick.
Clearing a fox's target ends defending without changing its trusted entities. An enderman's
target controls its creepy flag and attacking speed modifier; clearing it also clears its stared-at
flag. A zombified piglin schedules its first anger sound and next alert when it acquires a target
from none, not when switching between targets. Targeting a player does not count as player damage.


## Brain behaviors

`EntityPart::register_behaviors` receives a `BehaviorRegistrar` whenever a brain-driven species builds or rebuilds its brain. Use `add(activity, priority, min_duration, max_duration, requirements, behavior)` to join a fresh `Behavior` to the species' supported activity. Registration returns a `BehaviorRefusal` for an unsupported activity or invalid duration. Behaviors start and stop in vanilla `Brain.tick` order: priority, activity bucket, then insertion order, with mod behaviors after vanilla behaviors in the same bucket. Their state is not saved. `BehaviorCx` provides the mob, host, random source and typed brain memory access; each species refuses memories it does not support.

## Spawn groups and shearing

The part's `spawn_cluster_size` changes how large a natural spawn group of the type grows
(`Mob.getMaxSpawnClusterSize`); [controlling spawns](../../how-to/world/spawning.md#changing-how-large-a-spawn-group-grows)
shows it.

`is_shearable` and `on_sheared` make the type shearable, as NeoForge's `IShearable` does: shears in a player's
hand and a dispenser's ask `is_shearable` first, and the part that says `Some(true)` shears the entity in
`on_sheared` and returns the drops, which the game spawns before damaging the shears. `Some(false)` keeps the
entity unsheared, and `None` leaves it to the species (a sheep keeps shearing as vanilla's does).

## Equipped durability

`living.hurt_and_break(host, slot, amount)` changes equipped durability on the server;
negative amounts repair. It returns the pre-shrink stack only on break, and `None` when
nothing breaks or the entity or slot is unavailable. Creative players' equipment is
exempt. The operation does not fire equip effects.

`mob.set_wanted_position(host, [x, y, z], speed)` writes `MoveControl` directly,
without pathfinding or requiring loaded terrain. `wanted_position(host)` and
`speed_modifier(host)` read its stored position and speed, including after the
wish is consumed. The mob moves by its own control; its AI may replace the wish
next tick. Finite negative speed is accepted as in vanilla. Non-finite coordinates
return `WantedPositionError::NonFinitePosition`; non-finite speed returns
`NonFiniteSpeed`. Refused inputs leave the old wish intact. The setter returns
`Ok(false)` (reads return `None`) for stale handles, clients, the dragon and mobs
held out of the level. These methods are server side.
