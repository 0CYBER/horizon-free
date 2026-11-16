pub use content::registry_entry::RegistryEntry;
pub use content::registry_keys::UnknownFluid;
mod __private {
    pub use crate::content::registry_keys::{
        resolve_block_entity_types, resolve_enchantments, resolve_entity_types, resolve_menus,
    };
    pub use crate::wire::wire_types::registry_entry_key_slot;
    // hidden: load-owned key resolution, used by the game at the registry boundaries
    pub use crate::content::registry_keys::{
        block_definitions, block_key, effect_index, resolve_block_keys, resolve_effects,
    };
    pub use crate::content::registry_keys::{
        block_entity_type_key, block_entity_type_name, block_id as block_key_id, dimension_id,
        dimension_key, effect_id as effect_key_id, enchantment_key_id, entity_type_key_id,
        item_id as item_key_id, menu_key_id,
    };
    pub use crate::content::registry_keys::{
        block_keys, effect_key, effect_key_by_index, enchantment_keys, entity_type_keys, fluid_id,
        fluid_key, fluid_key_utf16, item_key, item_key_by_identifier, item_keys, menu_keys,
        resolve_fluids, resolve_items, world_fluid_key,
    };
}
