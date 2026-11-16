//! The `block_entity_view_distance` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// How far from the camera the mod block entity of block `block` draws, in blocks
    /// (vanilla's `BlockEntityRenderer.getViewDistance`): 64 unless a part answers. Set
    /// `distance` for your own block: further for something seen from afar (a beacon-like
    /// beam, a large display), nearer for small detail. It is asked once for each mod block,
    /// the first time this client draws block entities, and kept: never per frame. A value
    /// that isn't finite and above 0 is ignored. Beyond NeoForge: the renderer's own method,
    /// no event.
    #[hook(
        side = "client",
        contract = "collect",
        out = "distance",
        scope = "block_entity",
        key = "block",
        neoforge = "none (vanilla BlockEntityRenderer.getViewDistance; NeoForge adds no event or extension for it)"
    )]
    fn block_entity_view_distance();
}
