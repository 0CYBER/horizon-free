//! The `piston_carries_block_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Whether a piston carries your block `state` at `pos` with its block
    /// entity: asked at the last step of `PistonBaseBlock.isPushable`, after
    /// the block's push reaction, for a block that is otherwise pushable,
    /// whenever a piston's structure takes the block in. Left `None`, vanilla's
    /// rule holds: a block with a block entity does not move, and any other
    /// block does. `Some(false)` keeps the block in place at that position.
    /// `Some(true)` lets a piston move it even with a block entity, which
    /// travels with the block: taken out of the world as the move starts, with
    /// its saved data and its attachments and without its removal side effects
    /// (nothing drops), and put where the block lands as a new block entity of
    /// that position, with a new ticker slot; a handle to the old position no
    /// longer resolves. If the moving block is replaced before it lands, the
    /// block entity is removed there as a broken block's is (a container drops
    /// its contents). A save mid-move keeps it in the moving piston's tag. A
    /// block entity, including a chest, moves only after its reaction permits movement
    /// and this hook answers true. While the block moves it draws as its state, without its
    /// block entity's drawing. Asked on both sides, as a client replays the
    /// move on its own copy: answer the same on both, from the arguments,
    /// without touching the world. Only attached block parts are asked; blocks
    /// without a subscribed part pay only the dispatch guards. A block that only needs to move or not, or
    /// to be popped or pushed only, is the push reaction's question, asked
    /// before this one. The order is reaction first, then this shared block-entity
    /// rule for push-pull, ignore-entity and push only; a wrong-direction push-only
    /// answer refuses before this hook. Popped destroys and drops without asking it.
    /// This hook is asked only for an otherwise-pushable block. A live owned payload
    /// is required: data-only or unmaterialized vanilla entities remain refused.
    /// Diverges from vanilla: push only never discards a block entity.
    /// NeoForge has no counterpart: its
    /// `IBlockExtension.getRelocability` is an unrelated region-copy query that
    /// pistons do not ask. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "replace",
        out = "movable",
        scope = "block",
        key = "state",
        neoforge = "none (NeoForge has no counterpart; IBlockExtension.getRelocability is unrelated)",
        target = "horizon_client::game::native_hooks::piston_carries_block_entity::GameLevel<'_>::native_piston_carries_block_entity"
    )]
    fn piston_carries_block_entity();
}
