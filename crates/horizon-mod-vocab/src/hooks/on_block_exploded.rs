//! Per-block notice before explosion removal.
pub trait Hooks {
    /// An explosion destroys this block, before removal and while its live block
    /// entity is readable. Writes are allowed; the engine still replaces the cell
    /// with air afterwards, including a replacement written by the part.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.onBlockExploded"
    )]
    fn on_block_exploded();
}
