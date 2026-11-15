//! Extend block entity validity after content registration.
pub trait Hooks {
    /// Extend existing types' valid blocks (NeoForge `BlockEntityTypeAddBlocksEvent`).
    /// Fired after content declarations, before worlds load. `types` contains every
    /// registered type. Append `(type, block)` pairs to `additions`; this never changes
    /// the block's factory or ticker. Filter `types` to implement predicate selection.
    /// Use `event.modify(kind, blocks)` for a direct addition, or
    /// `event.modify_matching(predicate, blocks)` for all matching registered keys.
    /// Duplicate additions are harmless; unknown keys or incompatible classes fail startup.
    /// Existing sign-template memberships remain valid without a listener.
    /// Blocks must share the common vanilla superclass of the type's existing blocks.
    #[notes = r#"## Add blocks to block entity types

`Hooks::block_entity_type_add_blocks` runs once after content declarations and before
worlds load, with a `RegistrationContext`. Retain block and type keys from `register`.
Call `event.modify(kind, &[block])`, or use `event.modify_matching(|kind| ..., &[block])`
to extend every matching type. The predicate sees all registered type keys.

Validity is many-to-many. An addition allows an existing entity of that type to remain
on the block; it does not select the block's factory. A declared native type still
owns creation, and a content block otherwise inherits its original vanilla template.
A vault template associated with `minecraft:vault` creates, ticks and saves a vanilla
vault, without declaring another entity type. Signs retain their template defaults.

As in NeoForge's public `BlockEntityTypeAddBlocksEvent` javadoc, a block must inherit
the common superclass of the targeted type's original valid blocks. Invalid keys or
classes fail startup, duplicate additions are idempotent, and a resource reload cannot
change the frozen validity sets or wire IDs. No listener means no added memberships.
"#]
    #[hook(
        side = "both",
        context = "registration",
        contract = "collect",
        out = "additions",
        neoforge = "BlockEntityTypeAddBlocksEvent"
    )]
    fn block_entity_type_add_blocks();
}
