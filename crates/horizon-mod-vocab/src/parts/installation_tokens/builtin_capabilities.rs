//! Read the built-in declarations derived from the public capability keys.
/// Built-in block capability identities and handler types, in their exported order.
pub fn builtin_block_capabilities() -> Vec<(&'static str, &'static str)> {
    crate::parts::capabilities::providers_of::builtin_block_capabilities()
}

/// Type-only item default coverage from the same bucket owner used by `ITEM_FLUIDS`.
pub fn builtin_item_capability_registered(name: &str, item: &str) -> bool {
    name == crate::ITEM_FLUIDS.name && crate::block::fluid_capability::builtin_bucket(item)
}

/// The engine's immutable type-only default-provider coverage, without a world lookup.
pub fn game_capability_registered(by: crate::ProvidedBy, name: &str, id: &str) -> bool {
    crate::world::phase::game_capability_registered(by, name, id)
}

/// The capability another built-in default adapts, if any.
pub fn block_capability_dependency(name: &str) -> Option<&'static str> {
    crate::item::storage::default_source(name)
}

/// Whether the game's block entity type accepts this block, from the normal type owner.
pub fn block_entity_covers(kind: &str, block: &str) -> bool {
    crate::world::phase::block_entity_covers(kind, block)
}

/// The player inventory default's type domain, from its behavior owner.
pub fn builtin_player_inventory_type(kind: &str) -> bool {
    crate::entity::entity_capability::builtin_player_inventory_type(kind)
}
