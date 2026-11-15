use super::*;
/// An entity type's spawn rule, declared in code (NeoForge's
/// `RegisterSpawnPlacementsEvent`): the type's id (`mymod:ghoul`) and its rule.
/// The game resolves it once, at load, into the tables its spawn checks read.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct SpawnPlacementDeclaration {
    /// The entity type the rule belongs to.
    pub id: String,
    /// Where, how bright and in which biomes the type may spawn.
    pub rule: crate::SpawnPlacementRule,
}

/// The spawn rules mods declared.
pub fn spawn_placement_declarations() -> &'static [SpawnPlacementDeclaration] {
    installed().map_or(&[], |installed| &installed.spawn_placements)
}

/// Registrar operations for spawn placements.
pub trait SpawnPlacementsRegistrar {
    /// Declare where the mod's entity type `path` may spawn (NeoForge's
    /// `RegisterSpawnPlacementsEvent`): its placement type, heightmap and a
    /// rule of a ground block tag, light and biome tag. The vanilla spawn
    /// rules (`SpawnPlacements.isSpawnPositionOk` and `checkSpawnRules`) then
    /// answer from the rule for the type, wherever the game checks a spawn:
    /// natural spawning, chunk generation, spawners, patrols, sieges and
    /// reinforcements. Code that must decide per check is the separate
    /// `mob_spawn_placement_check` hook, which still hears the rule's answer.
    ///
    /// Resolved once, at load: an id outside the mod's manifest-owned namespaces, a tag that is
    /// not a namespaced name or a second declaration of the same type refuses
    /// the mods, naming the mod; a type no content file defines refuses the
    /// resource load, naming the type.
    fn declare_spawn_placement(
        &mut self,
        path: &str,
        rule: crate::SpawnPlacementRule,
    ) -> Result<(), crate::SpawnPlacementError>;
}
