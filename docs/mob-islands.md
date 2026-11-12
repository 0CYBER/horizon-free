# Parallel mob tick: reach and acceptance requirements

Design: [#14457](https://github.com/horizon-rust/horizon/issues/14457).
Implementation: `crates/horizon-client/src/game/creatures/mob_islands.rs`.
Parallel ticking is off by default. Design approval and normal implementation review are separate gates.

## Reach is a correctness contract

Two mobs may run in different groups only if neither turn can read or write state whose serial
ordering depends on the other. Bounds must cover the entire turn, including positions reached
after movement or teleportation, nearby queries at those positions, pushes, and brain events.
An initial resting, unloaded, sitting or leashed state is not proof of independence: earlier
turns can change another mob before its turn.

When changing a mob's movement, locomotion attributes or effects, teleportation, query range,
remembered entity references, or cross-mob writes:

1. Audit `reaches`, `jump`, `reads_villages`, ID links and the production fallback checks.
2. Extend the conservative bounds, or keep a serial fallback for every affected tick until
   those bounds are established. Never use an unsupported local bound to keep a tick parallel.
3. Add or extend a behavioral regression with independently placed interacting mobs. Check
   that the planner joins them or that the production entry point actually falls back.
4. For a supported parallel interaction, compare grouped and serial outcomes and assert that
   multiple groups really ran. For a fallback, check production and verify, including that
   verify does not execute a cloned reference tick. A serial-only test cannot prove grouping.

Verify logs discrepancies and retains the grouped result. It is diagnostic coverage, not a
production recovery mechanism or a substitute for the reach audit.

## Locomotion bound

Initial velocity does not bound a mob's movement during its turn: `MoveControl.tick` sets the
mob's `speed` from a goal's speed modifier and the movement or flying speed attribute, and
`LivingEntity.travel` accelerates by it before the mob moves. The `2 * |v| + SLACK` term is the
reach of a mob whose locomotion is its type's (`default_locomotion`). Any other mob, changed
attributes or a Speed, Slowness, Jump Boost or Dolphin's Grace effect, whatever its role or NoAI
state, adds `locomotion(m)` to its reach. A value that isn't finite keeps the tick serial.

Derivation. In one turn the mob moves once (`do_move`), by its velocity after the turn changed it;
friction scales the velocity only after the move. So the displacement is at most `|v0|` (the
velocity it starts with, pushes included: the existing `2 * |v| + SLACK` term) plus everything the
turn added before the move:

- `moveRelative(amount, input)` adds `input * amount` with the input normalized when longer than
  1, so it adds at most `|amount|`. Its `amount` is, by travel mode: on the ground
  `getFrictionInfluencedSpeed`, which is `speed` (Horizon's `friction_speed`: `speed` for a
  friction of 0.6 or less, `speed * 0.216 / friction³ < speed` above); in the air the flying speed,
  `0.02` or a tenth of `speed` for a ridden mount; in water `0.02 + (speed - 0.02) * efficiency`,
  at most `max(0.02, speed)`; flying mobs the type's air speed or `speed` (allay, nautilus,
  dolphin); fish and other swimmers constants of at most 0.1 or `speed`. So one call adds at most
  `push = max(speed, air speed, 0.1)`. A turn makes at most two: travel's, and one of a jump (a
  rabbit's hop, a mob's push forward in water) or a swimmer's climb (`0.1 * speed`).
- A jump sets the vertical velocity to the jump power: the jump strength times the block's jump
  factor (at most 1), plus 0.1 per Jump Boost level. A sprinting jump adds 0.2 horizontally, a
  ridden horse's jump 0.4 forward, a ridden camel's dash `22.2222 * movement speed` along its look
  and 0.6 up (`Camel.executeRidersJump`).
- `speed` is at most `max(|speed now|, modifier * max(walk, fly))`: the move control's value from
  this turn, or the one it left last turn when it doesn't run. `modifier` is the largest of the
  current move and navigation modifiers and `MAX_SPEED_MODIFIER` = 4.0. Audit of the vanilla
  values: goals and behaviours pass constants of at most 2.25 (panics, tempts, avoids), a llama
  following a caravan grows its own up to 3.6, a husk's spear charge on a camel husk uses 4.0
  (`SpearUseGoal`). Content mods carry no goal speeds; native mods' goals run only while every
  mod is quiet, which excludes registering goals.
- `walk` is the movement speed with every modifier that only lowers it left out and every one
  that can rise in the turn at its highest: the larger of the base and the type's speed, plus an
  enderman's 0.15 attack bonus, times the speed bonus, the sprint's 1.3, each Speed effect's
  `1 + 0.2 * level` (Slowness left out: it can expire before the move) and a witch's Swiftness
  (1.2, it can finish drinking in its turn); at least the effective value. `fly` is the larger of
  the effective and the type's flying speed.

So `locomotion = 2 * push + dash + 0.4 + 0.2 + jump`. Dolphin's Grace only changes the water's
friction, which applies after the move. Leaps whose velocity doesn't depend on the attributes (a
frog's or goat's long jump, a breeze's) are the same for a changed mob as for its type, and stay
in the default term. Ordinary effects give small bounds (Speed II on a cow: about 4 blocks); a
speed of 96 gives a body hundreds of blocks wide, which joins most islands: right, but serial.

The resource-backed regression gives a stationary cow a movement speed of 8: it joins the next herd,
96 blocks on, and not the one after, runs into it over 20 ticks while two groups run, and matches
the serial mobs and level random in production and verify.

## Owner teleports

`TamableAnimal.tryToTeleportToOwner` (`FollowOwnerGoal`, `TamableAnimalPanicGoal`) puts a pet on
a block within 3 of its owner's on each horizontal axis and 1 vertically, feet at the block's
bottom centre. The owner is a player, and players are read-only during the mob loop, so the plan
knows where each stands: the host's player and the guests a mob can see. For each player the pet's
owner can be (`tame::owned_by`), the destination box (the pet's body over those blocks), grown by
its motion, is a second place of the pet's island (`Away`), with its reads around it. The pet
joins the mobs near its owner and no mob between. A pet whose owner isn't in this level gets no
such place: it cannot teleport. The resource-backed regression teleports a wolf, a cat and a
parrot from one herd to their owner in the next, with the third herd in a group of its own.

## Teleport consumables

A mob that finishes an item whose consumable has `ConsumeEffect::TeleportRandomly` (chorus fruit;
a fox eats it in `Fox.aiStep`) lands within half the `diameter` of where it stood on each axis
(`Entity.randomTeleport`), then drops to the first ground below, as deep as the world goes. Its
reach adds `diameter / 2 + 1` to its motion, and its body reaches down to `i32::MIN`. The check
reads the item, never the species. The regression puts a fox 85 blocks above one herd; it joins
that herd only, eats, lands, and matches the serial result with three groups.

## Raids

A raider's hurt order comes from a process-wide counter (`raider::next_hurt_seq`), and `raids.rs`
compares it only among one raid's raiders. A raid's raiders share an island, so their order is the
serial one. The numbers themselves differ from the serial run's; verify compares mobs without
them, as it does `section_seq`. The regression puts one raider in each of two far herds.

## Wither and warden

Neither reaches the whole level. A wither's side heads keep their targets by id and look them up
in the level's mobs (`wither::target_pos`): those ids are links. A new head target is picked
within 20 blocks, inside its reads. A warden keeps its suspects, memories, noted attackers and
pending vibration as mob ids, read and angered at any distance: every id in its state is a link.
The suspects it keeps by UUID resolve only to the host's player during the loop, serially as in
groups (`LoopLookup`).

## Native mods and the profiler

What a mod can run in a mob's turn: `native_living_change_target`, `native_living_heal`,
`native_living_breathe`, `native_living_drown`, `native_living_fall`,
`native_living_death_message`, `native_register_goals` (its goals), the incoming damage hooks,
`native_living_damage_pre`, `native_living_knockback`, a content mod effect's ticks,
`native_living_jump` and `native_living_damage_post`, and mixins into them. A trait mod's
overridden methods can't be seen, so each mod declares it with `native_mob_phase_quiet`; one
that doesn't keeps the loop serial. While every mod is quiet, grouped turns skip the hooks that
hand something back (no mod listens) and keep the two notifications, which the server thread tells
at the join, ordered by turn. A mob carrying a content mod's effect keeps the tick serial while
mods are loaded. The entity tick Pre's cancellations are heard before the loop and passed to
every group.

The tick profiler samples on whichever thread runs a mob: each helper samples for whoever samples
on the server thread, into its own sums, which join the server thread's when it ends. A sampled
mob's time on a helper includes its waits for earlier mobs' level random.

```sh
cargo test -p horizon-client --lib game::creatures::mob_islands:: -- --include-ignored --test-threads=1
```

Set `HORIZON_TEST_RESOURCES` to prepared resources and isolate game homes as `CLAUDE.md`
requires. Do not set `HORIZON_TEST_SEEDS` when proving grouped execution: it forces fallback.

## Other ways a turn moves a mob

Audited by grepping position writes, `teleport`, `set_pos` and `move_to` under `mobs/` and
`game/creatures/`:

- Enderman and shulker teleports (`jump`) and the hurt-time teleports in `ai.rs`: margins.
- `tame::teleport_to_owner`: the owner's destination box above.
- Teleporting meals: the consumable reach above.
- Motion: `travel`, vex `noPhysics` travel, turtle and bat adjustments: the velocity reach and
  the locomotion bound.
- Villager `start_sleeping`/`stop_sleeping` and the first-tick bed position: the villager claim box
  (its bed and points of interest).
- `riders::stop_riding` and `conversion::single`: placed at the vehicle or at the converted mob,
  both ids linked in the reach.
- Block landing (`landing.rs`), resizing (`living.rs`): sub-block adjustments.
- `MobEvent::Push` and `SetVelocity` (ravager roar, hoglin/piglin knockback, entity pushing,
  frog tongue): they change velocity, not position, on a target in the actor's own reads, so the
  target is in its island. Open: a ravager's fling is `4 / d` and unbounded for centres closer than
  about 0.03 blocks, so a mob pressed against a ravager could move farther than its reach in the
  same tick.

## Before merge

- Pass `pr_ready.py` on the exact proposed head and review the port-ledger warnings. Resolve
  failing tests or establish their cause; missing directories are not a passing result.
- Refresh the affected parity and real server-thread/TCP guest proof on that head. Record
  actual grouped execution and verify counts; fallback-only parity is separate evidence.
- Supply serial-versus-parallel measurements of tick time, including median and tail latency,
  CPU time and planning overhead. Include separated herds, RNG-heavy mobs, crowded worlds
  and pets, raids and changed speeds, on representative worker counts. Record hardware, configuration,
  mob counts, warmup, duration and commit. Show that gains justify thread creation and waits
  and that fallback overhead is acceptable before accepting this as a performance change.

These are acceptance requirements, not a claim that the draft has already satisfied them.
