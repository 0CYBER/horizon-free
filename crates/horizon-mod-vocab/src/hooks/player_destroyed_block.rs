//! The `player_destroyed_block` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A player's break removed the block `state` from `pos`: the
    /// `Block.destroy(level, pos, state)` call `ServerPlayerGameMode.destroyBlock` makes only
    /// after `Level.removeBlock` succeeded, before a creative player's return and `playerDestroy`'s
    /// drops. `state` is the block the removal took out, read just before it: what
    /// `playerWillDestroy` left there (a cracked decorated pot; a disarmed tripwire, where vanilla
    /// passes `Block.destroy` the armed one) or what a `break_block` listener put there.
    /// `block_entity` is the saved data of the block entity that stood at `pos`
    /// (`BlockEntity.saveWithoutMetadata`, read with `state`), none for a block without one;
    /// the entity itself is gone from the level by now, so a live handle would be stale. The
    /// level is writable. Server-side, for every player of the world: the helpers read the
    /// acting one. Not heard for a break `break_block` canceled, nor for a break the level
    /// makes (a piston, water, an explosion).
    #[notes = r#"## Player destroyed block

- **`player_destroyed_block(host, event)`**: a player's break has removed the block. `event.pos` and `event.state` are where it stood and what the removal took out (read just before it, so after `playerWillDestroy` and `break_block`: a cracked decorated pot, a disarmed tripwire where vanilla passes `Block.destroy` the armed one, or a block a `break_block` listener put there), `event.block_entity` is the saved data of that block's block entity (`BlockEntity.saveWithoutMetadata`) or none, and the host's helpers name the acting player. It fires at the `Block.destroy` call that vanilla's `ServerPlayerGameMode.destroyBlock` makes only once `Level.removeBlock` succeeded: after the removal and before a creative player's return and `playerDestroy`'s item drops, statistics and experience. A break `break_block` cancels, or one that removes nothing, is not heard; neither is a level-side break (a piston, water, an explosion): those never go through `destroyBlock`. The block entity is read before the removal only while a mod listens, because the entity is gone afterwards and a `BlockEntityRef` would be stale. There is no NeoForge event for this moment: the name and contract are this fork's."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no event for Block.destroy; BreakEvent fires before the removal)"
    )]
    fn player_destroyed_block();
}
