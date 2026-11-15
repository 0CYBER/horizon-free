# Block-state facts

A `BlockState` is an API-owned value pairing a `BlockKey` with a block-local canonical ordinal, not a numeric engine state ID or a live object. Read facts without a world position on either side through `host.block_states()` (import `HostBlockStates`). The service reads the game's loaded tables. Fact queries and matching allocate nothing; `definition` copies the property data for the caller. Property operations on `BlockState` borrow explicit immutable block metadata; `block()` and `ordinal()` need no host:

| Method | Vanilla |
|---|---|
| `replaceable(state) -> Option<bool>` | `BlockState.canBeReplaced()` |
| `destroy_speed(state) -> Option<f32>` | `BlockBehaviour.Properties.destroyTime` (`getDestroySpeed`): stone 1.5, -1 unbreakable |
| `sound(state) -> Option<BlockSoundType>` | `BlockState.getSoundType()`: volume, pitch and the break, step, place, hit and fall events |
| `has_block_entity(state) -> Option<bool>` | `BlockState.hasBlockEntity()` |
| `map_color(state) -> Option<u8>` | `BlockState.getMapColor`: the `MapColor.id` a map item colours a column by, 0 for none |
| `states_of(block) -> Option<BlockStateRange>` | `StateDefinition.getPossibleStates`, as owned values in canonical local-ordinal order |
| `definition(block)` | Property names and legal serialized values |
| `matches(state, block, constraints)` | Partial property match with typed invalid-property and invalid-value errors |

Use `StateConstraint` for each partial constraint; `StateMatchError` identifies an invalid property or value by index. `StateProperty` describes a property's legal values. These types are shared by state-definition services.

Queries return `None` for an unknown state or block, including an invalid local ordinal, and on a host with no game behind it (a mod's test double). The ordinal order is defined by the complete canonical property constraints, not by the engine's numeric enumeration.

## Fluid-state facts

Import `HostFluidStates` to reach `host.fluid_states()` on either owning side.
`definition(key)` returns `Some(FluidDefinition::World { default, properties })` for a world fluid,
`Some(FluidDefinition::TankOnly)` for a registered tank-only fluid, and `None` without a loaded registry.
The properties and typed default come from the loaded preparation, not a second vanilla table.
These metadata reads are available in setup, reload and world callbacks after the fluid keys resolve,
not during registration or configuration.

`validate(key, constraints)` checks partial `StateConstraint` values in input order without matching
any particular state. It returns `Result<(), FluidStateError>`: `UnknownProperty(index)`,
`InvalidValue(index)` or `Duplicate(index)` identifies the first offending input.
`NoWorldState` means the fluid is tank-only or the host has no definition; use `definition` to distinguish them.

`type_properties(key)` returns a `FluidTypeProperties` record, or `None` without a loaded registry or for
a key it doesn't hold; `host.fluid_key(name)` is `None` for an unknown name. The record is the same in every
level: `description_id`, `light_level`, `fall_distance_modifier`, and whether the fluid can push entities, be
swum in, drown, extinguish, hydrate and carry boats. Flowing forms answer as their source. The empty fluid and
tank-only fluids emit no light, do nothing to an entity and have a fall modifier of 1. Anything that depends on
a level, such as lava's current or source conversion, isn't in the record.
The record also carries three `Fluid` facts: `explosion_resistance` (100 for water, lava and a mod's
world fluid, 0 for the empty or a tank-only fluid), `randomly_ticks` (lava only) and `drip_particle`
(`minecraft:dripping_water` or `minecraft:dripping_lava`, none for the rest).

`legacy_block(state)` is `FluidState.createLegacyBlock`: the water, lava or mod-fluid block that holds a
fluid state (a source is level 0, a flowing amount is `8 - amount`, falling adds 8) and air for the empty
fluid. It is `None` for a state no fluid can be in: a tank-only fluid, an amount above 8, a zero amount
for anything but the empty fluid, or a source that isn't full.

What depends on a position is on `LevelRef`, not in this service: `fluid_tick_delay(host, fluid, pos)`
(water 5, lava 30 and 10 in the Nether, a mod fluid's as its `fluid_rules` parts change it there),
`fluid_height`, `fluid_height_for_camera` and `fluid_can_be_replaced_with(host, pos, fluid, direction)`.
`fluid_tick(server, pos)` runs a fluid's flow step immediately; `schedule_fluid_tick` queues one.

`FluidState::matches(constraints)` is a pure local check of carried vanilla fields. Unknown properties
or noncanonical serialized values return false. A falling source remains a source; a flowing state
of level 8 remains flowing. Empty constraints match any state. Validate separately for indexed errors.
Neither matching nor validation allocates; explicitly copying a definition may allocate.
Unresolved saved tank identities remain `FluidIdentity::Unresolved(handle)`, not world states or tank-only keys.
