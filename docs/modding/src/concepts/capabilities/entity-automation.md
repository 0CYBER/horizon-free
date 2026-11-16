# Entity automation

Use `entity.capability(host, &ENTITY_AUTOMATION, side)` to access the inventory an
entity exposes to hoppers and droppers. This is NeoForge's
`Capabilities.Item.ENTITY_AUTOMATION`, distinct from the broader
[`ENTITY_ITEMS`](entity-items.md) capability.

Register a provider with
`r.entity_capability(&ENTITY_AUTOMATION, "mymod:robot", |host, robot, side| …)`.
Use `r.entity_capability_before_game` to answer ahead of the game's provider for
a minecart or boat. The context is `Option<Direction>`; `None` means no particular side.

## Game providers and transfers

- Chest and hopper minecarts and chest boats expose their container slots.
- Players and chested donkeys, mules and llamas have no game-provided automation handler.
- A pushing hopper requests the side it reaches the entity from (`Up` when the
  hopper is above it). A pulling hopper requests `Down` from the entity above it.
  A dropper requests the side it drops onto.
- At a target, automation chooses randomly among the game's container entities
  and entities whose mod providers answer this capability, using vanilla's
  `getEntityContainer` selection draw.
- A mod handler takes the container's place. Automation moves one item per
  transfer in a transaction, as it does through a block's `ITEMS` handler.
