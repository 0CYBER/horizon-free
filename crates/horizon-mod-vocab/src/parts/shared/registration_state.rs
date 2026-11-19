//! Registration state, separate from dispatch operations.
use super::*;

/// What every mod's [`Hooks::register`](crate::Hooks::register) attached.
#[derive(Default)]
pub(crate) struct Installed {
    pub(crate) menu_recipe_books: HashMap<String, crate::parts::menus::MenuBookDeclaration>,
    pub(crate) recipe_book_types: HashMap<String, String>,
    pub(crate) game_rule_categories: Vec<String>,
    pub(crate) game_rules: Vec<crate::GameRuleDecl>,
    pub(crate) custom_stats: Vec<String>,
    pub(crate) game_rule_slots: Vec<u32>,
    pub(crate) keys: Parts,
    pub(crate) tags: TagParts,
    pub(crate) block_entity_types: Vec<BlockEntityTypeDecl>,
    pub(crate) block_entity_transforms: HashMap<String, Arc<dyn crate::BlockEntityTransform>>,
    pub(crate) entity_attributes: Vec<EntityAttributeDeclaration>,
    pub(crate) world_fluids: Vec<(String, crate::WorldFluid)>,
    pub(crate) entity_attribute_modifications: Vec<EntityAttributeModification>,
    pub(crate) attributes: Vec<super::attributes::AttributeDeclaration>,
    pub(crate) attribute_widenings: Vec<super::attributes::AttributeBoundsWidening>,
    pub(crate) general_entity_types: Vec<crate::parts::entity_types::GeneralEntityTypeDeclaration>,
    pub(crate) spawn_placements: Vec<SpawnPlacementDeclaration>,
    pub(crate) entity_part_states: Vec<crate::entity::part_state::EntityPartStateDeclaration>,
    pub(crate) biome_modifiers: Vec<BiomeModifierDeclaration>,
    pub(crate) biome_regions: Vec<BiomeRegionDeclaration>,
    pub(crate) datapack_registries:
        Vec<crate::world::datapack_registry::DatapackRegistryDeclaration>,
    pub(crate) registry_keys: horizon_mod_runtime::registry_keys::Requests,
    pub(crate) effects: OnceLock<horizon_mod_runtime::registry_keys::Effects>,
    pub(crate) resolved_block_entity_types: OnceLock<horizon_mod_runtime::registry_keys::Names>,
    pub(crate) fluids: OnceLock<horizon_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_items: OnceLock<horizon_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_game_events: OnceLock<horizon_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_entity_types: OnceLock<horizon_mod_runtime::registry_keys::Names>,
    pub(crate) resolved_blocks: OnceLock<horizon_mod_runtime::block_definitions::Blocks>,
    pub(crate) particles: OnceLock<horizon_mod_runtime::registry_keys::Particles>,
    /// The ticket controllers the mods registered; a [`TicketController`]'s slot indexes it.
    pub(crate) controllers: Vec<ControllerInfo>,
    pub(crate) arm_poses: Vec<crate::client::arm_pose::PoseInfo>,
    pub(crate) structure_types: Vec<crate::world::structure_type::StructureTypeInfo>,
    pub(crate) dimensions: Vec<crate::world::dimension::DimensionInfo>,
    pub(crate) chunk_generators: Vec<crate::world::chunk_generator::ChunkGeneratorInfo>,
    pub(crate) biome_sources: Vec<crate::world::biome_source::BiomeSourceInfo>,
    pub(crate) density_function_types: Vec<crate::world::density_function::DensityFunctionTypeInfo>,
    /// Loading warnings, by owning mod id, in registration order.
    pub(crate) warnings: Vec<(String, crate::LoadingMessage)>,
    pub(crate) hook_bits: Vec<bool>,
    pub(crate) tag_hook_bits: Vec<bool>,
}
