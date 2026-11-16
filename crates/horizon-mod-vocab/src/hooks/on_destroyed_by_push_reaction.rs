//! The `on_destroyed_by_push_reaction` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, a piston's move destroys the block `state` at `pos`
    /// (`IBlockExtension.onDestroyedByPushReaction`), `direction` the way the blocks move and
    /// `fluid` the fluid state the cell holds.
    /// It is heard in `PistonBaseBlock.moveBlocks` after the block dropped its resources and
    /// while it is still in its cell, before the cell becomes air. The removal still happens:
    /// a write to `pos` is overwritten, and so is a write to any other cell of the move (the
    /// other destroyed cells, the pushed blocks and the cells they move into, the arm), since
    /// the pushed blocks were read before the destroy loop. Writes elsewhere stay; `ticks`
    /// is the move's, so scheduled ticks stay too. A cancelled move (`piston_pre`) and a
    /// client's replay of the move are not heard. A panicking part is contained. With no part
    /// attached to the state, the cost is one table lookup per destroyed block. Diverges from
    /// upstream: new hook; the removal is not replaceable here.
    #[notes = r#"## Blocks destroyed by pistons

- **`BlockPart::on_destroyed_by_push_reaction(level, event)`** (`OnDestroyedByPushReactionEvent { dimension, pos, state, direction, fluid }`): a piston's move destroys the block (a `Destroy` push reaction such as a torch's), heard in `PistonBaseBlock.moveBlocks` after the block dropped its resources and while it is still in its cell, before the engine sets air there (flags 18). `direction` is the way the blocks move: the piston's facing as it extends, the opposite as a sticky piston pulls. It is a notice (NeoForge's `IBlockExtension.onDestroyedByPushReaction`): the removal still happens. The level host writes and schedules ticks (`&mut dyn LevelWriteHost`), but a write to any cell of the move is lost: its own cell and the other destroyed cells turn to air, and the pushed blocks were read before the destroy loop, so each of them moves as it was and the cells they leave and enter end as air or a moving block, as does the arm. Writes outside the move stay. `piston_pre` is the veto for the whole move; a cancelled move and a client's replay of a move are not heard. A panicking part is contained with a warning and the move goes on. `fluid` is the fluid state the cell holds, as NeoForge passes it; the removal itself always happens, and a mod cannot replace it here. Authoritative servers only. With no part attached to the state, one table lookup per destroyed block. Diverges from upstream: new hook."#]
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::on_destroyed_by_push_reaction::GameLevel<'_>::native_on_destroyed_by_push_reaction",
        neoforge = "IBlockExtension.onDestroyedByPushReaction"
    )]
    fn on_destroyed_by_push_reaction();
}
