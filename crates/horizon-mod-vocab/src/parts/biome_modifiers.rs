use super::*;
/// A biome modifier declared in code: its id (`mymod:ruby_ore`) and what it does. The game
/// applies every declared modifier once, when the world's biome tables build.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: only registration builds it; the game reads it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BiomeModifierDeclaration {
    /// The modifier's id, in the declaring mod's namespace.
    pub id: String,
    /// The biomes it applies to and the change.
    pub modifier: crate::BiomeModifier,
}

impl BiomeModifierDeclaration {
    /// The path the data-file reader knows the modifier by, as if the mod shipped it.
    pub fn file(&self) -> String {
        let (namespace, name) = self.id.split_once(':').unwrap_or_default();
        format!("data/{namespace}/neoforge/biome_modifier/{name}.json")
    }
}

/// The biome modifiers mods declared, in registration order (mods in dispatch order).
pub fn biome_modifier_declarations() -> &'static [BiomeModifierDeclaration] {
    installed().map_or(&[], |installed| &installed.biome_modifiers)
}

/// Registrar operations for biome modifiers.
pub trait BiomeModifiersRegistrar {
    /// Declare a biome modifier `path` (`no_zombies` or `mymod:no_zombies`) in code: the same
    /// operations as the data files in `data/<ns>/neoforge/biome_modifier/` (add and remove
    /// features, add and remove spawns, add and remove spawn costs) on the biomes it names.
    /// The game applies it with the data-file modifiers, once, when the world's tables build:
    /// every addition first, then every removal, then every `ModifyBiome`, each in mod and
    /// registration order. A world
    /// no modifier names generates exactly as before.
    ///
    /// `ModifyBiome` is NeoForge's MODIFY phase: it sets a biome's climate values and effects
    /// colours (`horizon:modify_biome` in a file).
    /// A modifier naming a biome, feature or entity type the world lacks (another mod's, a
    /// convention tag) is skipped with a line in the log, as a data file is.
    ///
    /// An id outside the mod's namespace, a repeated id, an empty list, a name that is not
    /// namespaced, an unknown generation step, a spawner of weight 0 or with its smallest group
    /// above its largest, a spawn cost that is not finite, and a climate number that is not finite, a colour above
    /// 0xFFFFFF or a temperature modifier other than `none` or `frozen` refuse the mods, naming the mod.
    fn declare_biome_modifier(
        &mut self,
        path: &str,
        modifier: crate::BiomeModifier,
    ) -> Result<(), crate::BiomeModifierError>;
}
