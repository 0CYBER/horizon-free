# Extending block entity validity

Use `Hooks::block_entity_type_add_blocks` after retaining block and type keys from
`register`. `event.modify(kind, &[block])` extends one existing type;
`event.modify_matching(predicate, &[block])` extends all matching registered types.
The event uses a `RegistrationContext` and runs before worlds load. Duplicate additions
are harmless. Unknown keys and blocks that do not share the target's common vanilla
superclass fail startup.

A block may be valid for several entity types. Its native declaration or original
vanilla template selects creation and ticking. For example, a block declared
with `template: "minecraft:vault"` and added to `minecraft:vault` creates and saves the
vanilla vault entity. Adding another valid type does not replace that factory. Sign
templates retain their existing membership defaults. Validity is frozen with the
canonical type table and participates in the multiplayer content identity.
