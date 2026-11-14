//! The `on_block_state_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A write through `Level.setBlock` held and the block at `pos` changed from `old` to
    /// `new` (`IBlockExtension.onBlockStateChange`), heard at the end of the write, where
    /// vanilla runs `updatePOIOnBlockStateChange`: after the block-entity removal, `on_place`,
    /// the client update, the neighbour updates and the neighbours' shape updates. The parts
    /// of `old`'s block hear it, then, when the block changed, the parts of `new`'s block;
    /// `state` is the state whose block's parts are asked. A write that `on_place` replaced
    /// with another block is not heard; the write that replaced it is. Heard on both sides:
    /// a server's writes, a client's replay of a piston's move and the blocks a client takes
    /// from the server (`LevelSide` names which). Applied local predictions are heard at once
    /// with `predicted: true`; server writes, settled corrections and all server-side writes
    /// have `predicted: false`. A held server update is unheard until acknowledgement; a
    /// confirmed prediction is not heard twice, and a denial hears predicted-to-server once.
    /// Equal-state writes and unapplied guesses are not heard. The level is read-only.
    /// A panicking part is contained. With no listener, one flag check builds nothing.
    /// Diverges from upstream: new hook.
    #[notes = r#"## Block state changes

- **`BlockPart::on_block_state_change(level, event)`** (`OnBlockStateChangeEvent { dimension, pos, state, old, new, predicted }`): a write through `Level.setBlock` held and the block at `event.pos` went from `event.old` to `event.new` (NeoForge's `IBlockExtension.onBlockStateChange`). It is heard where vanilla runs `updatePOIOnBlockStateChange`, at the end of the write: after the old block entity is removed, `on_place`, the client update, the neighbour updates and the neighbours' shape updates, so a fence beside a new stone block already shows its connection. The parts of `old`'s block hear it first, then, if the block changed, the parts of `new`'s block; `event.state` is the state whose block's parts are asked. It is a notice: the level host is read-only (`LevelSide`, which says whether this is a server or a client). Separate from the global `block_changed`, which a server hears right after the state is written, before any of the write's side effects, and also for a write whose `on_place` replaced the block; this hook skips such a write and hears the write that replaced it. Heard on both sides: every write a server makes, a client's replay of a piston's move, and on a client each block the server sends (`BlockUpdate`, `SectionBlocksUpdate`). Applied local predictions, including mining breaks and placements, are heard immediately with `predicted: true`. Server writes, settled corrections and every server-side write carry `predicted: false`. A server update for a predicted position remains unheard until acknowledgement. A confirmed prediction is not heard again; a denial is heard once from the predicted state to the server's state. Equal-state writes, unapplied guesses made while deciding an action and world generation's writes are not heard. A panicking part is contained with a warning. With no listener, one flag check builds nothing. Diverges from upstream: new hook."#]
    #[hook(
        side = "both",
        version = 2,
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::on_block_state_change::GameLevel<'_>::native_on_block_state_change",
        neoforge = "IBlockExtension.onBlockStateChange"
    )]
    fn on_block_state_change();
}
