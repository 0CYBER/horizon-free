//! The `block_entity_update_tag` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0025

pub trait Hooks {
    /// What clients get of the block entity at `pos` when its `id` is
    /// yours (`BlockEntity.getUpdateTag`): set `tag` to it, read from
    /// `entity` alone. Asked on the server whenever a chunk holding it is
    /// sent and whenever its block changes, on the server thread or, for a
    /// saved chunk sent past the view distance (`lan::far`), another one; a client's copy of the block
    /// entity (made by `block_entity` there) merges it into its `nbt`.
    /// Leave it unset to send no tag of yours. A block change puts the one-shot fields of
    /// `ModBlockEntityView::set_client_update_fields` over it, and sends them alone when it is unset.
    #[hook(
        side = "server",
        contract = "replace",
        out = "tag",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntity.getUpdateTag; NeoForge adds no event or extension for it); IBlockEntityExtension.handleUpdateTag is the client half"
    )]
    fn block_entity_update_tag();
}
