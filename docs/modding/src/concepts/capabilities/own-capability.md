# A mod's own capability

You define your own capability, as Mekanism defines chemicals: a resource type and a key, published from
its root crate ([Dependencies between mods](../dependencies.md)) for other mods to use.

```rust,ignore
// The chemistry mod's root crate
#[derive(Clone, PartialEq)]
pub struct Chemical(pub String);
pub const CHEMICALS: CapabilityKey<dyn ResourceHandler<Chemical>> = CapabilityKey::new("chem_api:chemical");
```

Declare the defining mod under `[dependencies]` to build against the same `Chemical` and `CHEMICALS`. Your providers,
`r.block_entity_capability(&chem_api::CHEMICALS, &vat, …)` (where `vat` is a declared type key or
`r.block_entity_type_key("chem_pipe:vat")`), answer the same capability, and
`level.capability(host, &chem_api::CHEMICALS, pos, side)` finds either mod's handler. `move_resources` moves a
chemical as it moves items, in the caller's transaction. The game has no answer of its own for a mod's key, so only
the mods' providers do, and a lookup with none registered costs one table check. A handler that keeps its amount in
its block entity's saved data (`BlockEntityRef::merge_data`) is covered by the game's journal, so an aborted
transaction puts it back. `ItemCapabilityKey::new` and `EntityCapabilityKey::new` define item and entity
capabilities the same way.

A key is known by its name, which must be namespaced (`chem_api:chemical`), and a name has one handler type. Two
mods that each declare their own `Chemical` under one name, rather than sharing the defining mod's, would never see
each other's handlers. So the game refuses that at startup, naming both mods and both types, and it refuses a key whose name has no
namespace. Both checks run once, when the mods register.

Storage for a holder without a block entity (an entity, a player, a chunk, the level) comes stock, kept in an
attachment the mod declares, as NeoForge mods keep a `ItemStacksResourceHandler` or `SimpleEnergyHandler` in one:
`AttachedItems::new(&SLOTS, holder, size, capacity, valid)` (returns `None` for more than 4096
indices) and `AttachedFluids` (`AttachedStacks<R>`, a
`ResourceHandler<R>` over an `AttachmentType<StackList<R>>`, `single` for one index) and
`AttachedEnergy::new(&ENERGY, holder, capacity, max_insert, max_extract)` over an `AttachmentType<u32>`. Give the
item or fluid attachment the codec `ITEM_STACKS` or `FLUID_STACKS` to save it. Every write is an attachment write, so
the transaction journal, which keeps each holder's attachment map, undoes it on an abort; sync follows the attachment
type's `sync` flag. A mod's own resource type implements `StackedResource` to be held. `capacity` stays the physical
limit of an in-range index while `valid` refuses only inserts (extraction never asks it), indices past the handler's
`size` that the attachment keeps are never deleted by a smaller view, and the codecs refuse a saved `Slot` past 4096
indices or an amount past what a signed `Int` saves, rather than honoring or wrapping it.
