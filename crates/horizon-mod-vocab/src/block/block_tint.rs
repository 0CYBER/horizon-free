//! Block tint sources, as a client mod registers them in `register_block_tint_sources` (NeoForge's
//! `RegisterColorHandlersEvent.BlockTintSources`, vanilla's `BlockColors.register`).
//!
//! A model face with `tintindex` i is multiplied by layer i of its block's list. Vanilla's own
//! sources ([`VanillaTint`], vanilla's `BlockTintSources`) are the game's, evaluated as it evaluates
//! its own blocks. A mod's source is its own [`BlockTintSource`], held by the game and asked on the
//! meshing workers: like a structure type on the worldgen workers, it gets no `Host` and keeps no
//! mutable state, since two workers can mesh neighbouring sections at once.
use crate::{BiomeColors, BlockMetadata, BlockState, Direction, FluidState, LightLayer};
use std::sync::Arc;

/// The colour calculations block tints blend over the biomes around a block (vanilla's
/// `BiomeColors` resolvers, `ColorResolver`): vanilla's four, and the ones mods register in
/// `register_color_resolvers`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the game blends every resolver, so a new one must be handled there
#[derive(horizon_mod_api_macros::Wire)]
pub enum ColorResolver {
    /// `BiomeColors.GRASS_COLOR_RESOLVER`: `Biome.getGrassColor`.
    Grass,
    /// `BiomeColors.FOLIAGE_COLOR_RESOLVER`: `Biome.getFoliageColor`.
    Foliage,
    /// `BiomeColors.DRY_FOLIAGE_COLOR_RESOLVER`: `Biome.getDryFoliageColor`.
    DryFoliage,
    /// `BiomeColors.WATER_COLOR_RESOLVER`: `Biome.getWaterColor`.
    Water,
    /// A mod's own resolver, as [`ColorResolvers::register`] returned it.
    Mod(ModColorResolver),
}

/// A mod's colour resolver, as the game keeps it: what [`ColorResolvers::register`] returns, to
/// pass to [`BlockAndTintGetter::block_tint`]. Process-local; do not persist it.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ModColorResolver(pub(crate) u32);

/// Construct a mod colour resolver's handle at the hidden integration boundary.
// hidden: only the game's registry hands out a resolver's handle.
#[doc(hidden)]
pub fn mod_color_resolver(index: u32) -> ModColorResolver {
    ModColorResolver(index)
}

/// The index the game keeps `resolver` under, at the hidden integration boundary.
// hidden: only the game's mesher reads it.
#[doc(hidden)]
pub fn mod_color_resolver_index(resolver: ModColorResolver) -> u32 {
    resolver.0
}

/// A biome as a colour resolver reads it: vanilla's `Biome` in `ColorResolver.getColor`.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct ResolverBiome {
    /// `Biome.getBaseTemperature`.
    pub temperature: f32,
    /// `ClimateSettings.downfall`.
    pub downfall: f32,
    /// Its grass, foliage, dry foliage and water colours as this client holds them, after
    /// `biome_colors`.
    pub colors: BiomeColors,
}

/// A mod's colour resolver: vanilla's `ColorResolver`, which the game blends over the biomes
/// around a block as it blends its own (`ClientLevel.calculateBlockTint`, over the biome blend
/// radius) and caches per section while it meshes. Held by the game and asked on the meshing
/// workers, so it keeps no mutable state and gets no `Host`.
pub trait BiomeColorResolver: Send + Sync + 'static {
    /// `getColor(biome, x, z)`: the colour of `biome` (`minecraft:plains`) at block column
    /// `(x, z)`, 0xRRGGBB.
    fn color(&self, biome: &str, info: &ResolverBiome, x: f64, z: f64) -> u32;
}

/// The colour resolver registry `register_color_resolvers` hands out: NeoForge's
/// `RegisterColorHandlersEvent.ColorResolvers`.
pub trait ColorResolvers {
    /// `register(resolver)`: adds `resolver` to the ones the game blends and caches, and returns
    /// the [`ColorResolver`] a block tint source passes to
    /// [`BlockAndTintGetter::block_tint`].
    fn register(&mut self, resolver: Arc<dyn BiomeColorResolver>) -> ColorResolver;
}

// no wire: process-local listener or callback storage; trait objects are invoked through host dispatch, not serialized.
/// The game's [`ColorResolvers`]: the registered resolvers, each at the index its handle names.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
pub struct ColorResolverRegistry {
    resolvers: Vec<Arc<dyn BiomeColorResolver>>,
}

impl ColorResolvers for ColorResolverRegistry {
    fn register(&mut self, resolver: Arc<dyn BiomeColorResolver>) -> ColorResolver {
        let index = u32::try_from(self.resolvers.len()).expect("at most u32::MAX colour resolvers");
        self.resolvers.push(resolver);
        ColorResolver::Mod(ModColorResolver(index))
    }
}

impl ColorResolverRegistry {
    /// The resolvers, each at the index its handle names.
    pub fn into_resolvers(self) -> Arc<[Arc<dyn BiomeColorResolver>]> {
        self.resolvers.into()
    }
}

/// What a tint source reads of the world around the block it colours: vanilla's
/// `BlockAndTintGetter`, over the region a section is meshed from (`RenderSectionRegion`: the
/// section's chunk and its eight neighbours). A position outside that region reads as air, no
/// fluid and the light of an unloaded chunk. Block entities are not part of the region.
pub trait BlockAndTintGetter {
    /// `BlockGetter.getBlockState`: the state at `pos`.
    fn block_state(&self, pos: [i32; 3]) -> BlockState;
    /// `BlockGetter.getFluidState`: the fluid at `pos`, the empty fluid where there is none.
    /// None outside the region, as `LevelHost::fluid_state` reads an unloaded position.
    fn fluid_state(&self, pos: [i32; 3]) -> Option<FluidState>;
    /// `BlockAndLightGetter.getBrightness`: the stored light of one layer at `pos`, 0 to 15.
    fn brightness(&self, layer: LightLayer, pos: [i32; 3]) -> i32;
    /// `BlockAndLightGetter.getRawBrightness(pos, darkening)`: the brighter of the block light
    /// and the sky light less `sky_darken`.
    fn raw_brightness(&self, pos: [i32; 3], sky_darken: i32) -> i32;
    /// `BlockAndLightGetter.canSeeSky`: the sky light at `pos` is full.
    fn can_see_sky(&self, pos: [i32; 3]) -> bool;
    /// `cardinalLighting().byFace(face)`: how much a face toward `face` is shaded, 0 to 1 (the
    /// Nether's flat lighting in the Nether).
    fn shade(&self, face: Direction) -> f32;
    /// `getBlockTint(pos, resolver)`: the resolver's colour at `pos` blended over the biome blend
    /// radius, as opaque ARGB. A mod's resolver blends as vanilla's do.
    fn block_tint(&self, pos: [i32; 3], resolver: ColorResolver) -> u32;
    /// `LevelHeightAccessor.getMinY`: the lowest block Y.
    fn min_y(&self) -> i32;
    /// `LevelHeightAccessor.getHeight`: the build height.
    fn height(&self) -> i32;
}

/// A mod's block tint source: vanilla's `BlockTintSource`. Colours are ARGB, and a face it colours
/// is multiplied by their RGB, as vanilla's block renderer does. Implemented once, held by the game, asked on the meshing workers, so it
/// keeps no mutable state.
pub trait BlockTintSource: Send + Sync + 'static {
    /// `color(state)`: the colour without a world, where the block is drawn away from a level (a
    /// block in a minecart, a mod's drawn block). Also the default of the two below.
    fn color(&self, state: &BlockState, metadata: &dyn BlockMetadata) -> u32;
    /// `colorInWorld(state, level, pos)`: the colour of the block at `pos`, as the chunk mesher
    /// draws it. The mesher asks once per block and tint index each time it meshes the section.
    fn color_in_world(
        &self,
        state: &BlockState,
        metadata: &dyn BlockMetadata,
        level: &dyn BlockAndTintGetter,
        pos: [i32; 3],
    ) -> u32 {
        let _ = (level, pos);
        self.color(state, metadata)
    }
    /// `colorAsTerrainParticle(state, level, pos)`: the colour of the bits the block breaks into
    /// and of falling dust.
    fn color_as_terrain_particle(
        &self,
        state: &BlockState,
        metadata: &dyn BlockMetadata,
        level: &dyn BlockAndTintGetter,
        pos: [i32; 3],
    ) -> u32 {
        self.color_in_world(state, metadata, level, pos)
    }
    /// `relevantProperties()`: the property names the colour depends on. Vanilla groups states
    /// for shared baked models by them (`ModelGroupCollector`); Horizon bakes every state, so the
    /// game does not read it.
    fn relevant_properties(&self) -> Vec<String> {
        Vec::new()
    }
}

/// One of vanilla's tint sources (`BlockTintSources`), as the game colours its own blocks with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: vanilla's closed set, each evaluated by the game
#[derive(horizon_mod_api_macros::Wire)]
pub enum VanillaTint {
    /// `constant(color)`: one ARGB colour everywhere.
    Constant(u32),
    /// `constant(colorInHand, colorInWorld)`: `hand` without a world, `world` in it (the lily pad).
    ConstantSplit {
        /// The colour without a world.
        hand: u32,
        /// The colour in the world and of its particles.
        world: u32,
    },
    /// `grass()`: the biome's grass colour.
    Grass,
    /// `grassBlock()`: the biome's grass colour, but none on its particles.
    GrassBlock,
    /// `doubleTallGrass()`: grass colour, the upper half's taken at the lower half.
    DoubleTallGrass,
    /// `sugarCane()`: grass colour in the world, none without one.
    SugarCane,
    /// `foliage()`: the biome's foliage colour.
    Foliage,
    /// `dryFoliage()`: the biome's dry foliage colour.
    DryFoliage,
    /// `water()`: the biome's water colour in the world, none without one.
    Water,
    /// `waterParticles()`: no colour in the world; the water colour on particles.
    WaterParticles,
    /// `redstone()`: the colour of the state's `power` (0 to 15).
    Redstone,
    /// `stem()`: the colour of the state's `age` (0 to 7).
    Stem,
}

/// One layer of a block's tint list: one of vanilla's sources or a mod's own.
#[derive(Clone)]
// exhaustive: mods build one; the game evaluates both kinds
#[derive(horizon_mod_api_macros::Wire)]
pub enum TintLayer {
    /// A vanilla source, evaluated by the game as for its own blocks.
    Vanilla(VanillaTint),
    /// A mod's source.
    Source(Arc<dyn BlockTintSource>),
}

impl From<VanillaTint> for TintLayer {
    fn from(tint: VanillaTint) -> Self {
        Self::Vanilla(tint)
    }
}

impl<T: BlockTintSource> From<Arc<T>> for TintLayer {
    fn from(source: Arc<T>) -> Self {
        Self::Source(source)
    }
}

impl std::fmt::Debug for TintLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vanilla(tint) => f.debug_tuple("Vanilla").field(tint).finish(),
            Self::Source(_) => f.write_str("Source"),
        }
    }
}

/// The block tint registry `register_block_tint_sources` hands out: NeoForge's
/// `RegisterColorHandlersEvent.BlockTintSources`.
pub trait BlockTintSources {
    /// `register(sources, blocks...)`, vanilla's `BlockColors.register`: `layers[i]` colours the
    /// faces of `blocks` whose `tintindex` is i, and a face whose index is past the list is not
    /// tinted. Blocks are ids, a mod's (`mymod:cable`) or vanilla's (`minecraft:oak_leaves`); a
    /// later registration for a block replaces the earlier one, vanilla's own included.
    fn register(&mut self, layers: Vec<TintLayer>, blocks: Vec<String>);
}

/// The game's [`BlockTintSources`]: the registrations in order.
#[derive(Default)]
// exhaustive: its field is private; the game builds it
#[derive(horizon_mod_api_macros::Wire)]
pub struct BlockTintRegistry {
    registrations: Vec<(Vec<TintLayer>, Vec<String>)>,
}

impl BlockTintSources for BlockTintRegistry {
    fn register(&mut self, layers: Vec<TintLayer>, blocks: Vec<String>) {
        self.registrations.push((layers, blocks));
    }
}

impl BlockTintRegistry {
    /// Each block's layers, the last registration for a block winning, in block-id order.
    pub fn into_layers(self) -> std::collections::BTreeMap<String, Arc<[TintLayer]>> {
        let mut by_block = std::collections::BTreeMap::new();
        for (layers, blocks) in self.registrations {
            let layers: Arc<[TintLayer]> = layers.into();
            for block in blocks {
                by_block.insert(block, layers.clone());
            }
        }
        by_block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Cold;
    impl BiomeColorResolver for Cold {
        fn color(&self, _: &str, info: &ResolverBiome, _: f64, _: f64) -> u32 {
            if info.temperature < 0.15 {
                0xFF_FFFF
            } else {
                0
            }
        }
    }

    #[test]
    fn each_registered_resolver_gets_its_own_handle_and_slot() {
        let mut registry = ColorResolverRegistry::default();
        let first = registry.register(Arc::new(Cold));
        let second = registry.register(Arc::new(Cold));
        assert_ne!(first, second);
        let resolvers = registry.into_resolvers();
        assert_eq!(resolvers.len(), 2);
        let ColorResolver::Mod(second) = second else {
            panic!("a registered resolver is a mod's");
        };
        assert_eq!(mod_color_resolver_index(second), 1);
    }

    struct Fixed(u32);
    impl BlockTintSource for Fixed {
        fn color(&self, _: &BlockState, _: &dyn BlockMetadata) -> u32 {
            self.0
        }
    }

    #[test]
    fn a_later_registration_for_a_block_replaces_the_earlier_one() {
        let mut registry = BlockTintRegistry::default();
        registry.register(
            vec![VanillaTint::Grass.into()],
            vec!["mymod:a".into(), "mymod:b".into()],
        );
        registry.register(
            vec![
                Arc::new(Fixed(0x12_3456)).into(),
                VanillaTint::Foliage.into(),
            ],
            vec!["mymod:b".into()],
        );
        let layers = registry.into_layers();
        assert!(matches!(
            &*layers["mymod:a"],
            [TintLayer::Vanilla(VanillaTint::Grass)]
        ));
        assert!(matches!(
            &*layers["mymod:b"],
            [
                TintLayer::Source(_),
                TintLayer::Vanilla(VanillaTint::Foliage)
            ]
        ));
    }
}
