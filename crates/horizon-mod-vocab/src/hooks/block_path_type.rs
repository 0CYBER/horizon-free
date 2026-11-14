//! The `block_path_type` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The optional classification of `event.state` at `event.pos`
    /// (`IBlockExtension.getBlockPathType`). Asked before every vanilla rule,
    /// including air and `is_pathfindable`. `Some` replaces the whole static
    /// classification; `None` declines to vanilla. The readonly host reads the
    /// owning level, so the same state may answer differently at different positions.
    /// No mob is supplied: engine callers in NeoForge pass null. The server's
    /// 4096-slot positional cache stores the final static classification, including
    /// Open, until a matching block-updated notification or slot collision. It
    /// survives time, reload and chunk lifetime. Ordinary writes clear only on
    /// actual change with UPDATE_CLIENTS and full status at least BLOCK_TICKING;
    /// direct block-entity notifications clear even unchanged state. Neighbour
    /// and mob-specific adjustments run afterwards, outside that cache.
    /// Diverges from upstream: new hook.
    #[hook(
        side = "server",
        contract = "replace",
        out = "path_type",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getBlockPathType"
    )]
    fn block_path_type();
}
