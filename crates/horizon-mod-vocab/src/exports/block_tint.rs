pub use block::block_tint::{
    BiomeColorResolver, BlockAndTintGetter, BlockTintRegistry, BlockTintSource, BlockTintSources,
    ColorResolver, ColorResolverRegistry, ColorResolvers, ModColorResolver, ResolverBiome,
    TintLayer, VanillaTint,
};
mod __private {
    /// A mod colour resolver's handle and index, for the game's registry and mesher.
    pub use crate::block::block_tint::{mod_color_resolver, mod_color_resolver_index};
}
