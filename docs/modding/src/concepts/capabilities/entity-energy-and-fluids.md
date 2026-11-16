# Entity energy and fluids

Look up an entity's handlers with `entity.capability(host, &ENTITY_ENERGY, side)`
or `entity.capability(host, &ENTITY_FLUIDS, side)`. Register providers with
`r.entity_capability(&ENTITY_ENERGY, "mymod:robot", |host, robot, side| …)`
and the equivalent call for `ENTITY_FLUIDS`. Neither key has a game-provided handler.

| Key | Handler | NeoForge counterpart |
| --- | --- | --- |
| `ENTITY_ENERGY` | `EnergyHandler` | `Capabilities.Energy.ENTITY` |
| `ENTITY_FLUIDS` | `ResourceHandler<FluidResource>` | `Capabilities.Fluid.ENTITY` |

The context is `Option<Direction>`: `Some(direction)` identifies the side reached
by the lookup, and `None` requests no particular side. Your provider receives this
context and can return a different handler for each side. `ENTITY_AUTOMATION`
uses the same context; `ENTITY_ITEMS` uses `()`.

Use `move_energy` or `move_resources` inside a transaction to transfer between
entity, block and item handlers.
