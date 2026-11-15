//! The `biome_colors` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0111

pub trait Hooks {
    /// Change the colours of `biome` (`minecraft:plains`) on this client: the
    /// grass, foliage (leaves, vines), dry foliage (leaf litter) and water
    /// tints, 0xRRGGBB, as its biome file and the colormaps set them. Asked
    /// for every biome when a mod calls `native_refresh_biome_colors`, which
    /// rebuilds the terrain with them; a season mod answers from its season
    /// and calls that when the season turns. The dark forest's modifier
    /// still applies to the grass set here; the swamp's noise colours
    /// replace it.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (NeoForge adds ColorResolvers with RegisterColorHandlersEvent.ColorResolvers; it does not retint a biome's own colours)"
    )]
    fn biome_colors();
}
