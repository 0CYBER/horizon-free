//! The `can_drop_from_explosion` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, whether the block `state` at `pos`, destroyed by
    /// `explosion`, drops (`IBlockExtension.canDropFromExplosion`), asked where
    /// `BlockBehaviour.onExplosionHit` asks `Block.dropFromExplosion`. `Some(false)` skips
    /// what the drop runs: the block's `spawnAfterBreak` (an infested block's silverfish, an
    /// ore's experience) and its loot; `Some(true)` runs them, even for TNT; `None` keeps
    /// vanilla's answer, where only TNT drops nothing. The block is removed either way, and a
    /// TNT still primes (`TntBlock.wasExploded` is not part of the answer). A panicking part
    /// is contained and vanilla's answer is used. The level is read-only. With no part
    /// attached to the state, the cost is one table lookup. Diverges from upstream: new hook.
    #[notes = r#"## Block drops from explosions

- **`BlockPart::can_drop_from_explosion(level, event)`** (`CanDropFromExplosionEvent { pos, state, explosion }`): whether the block an explosion destroys drops (NeoForge's `IBlockExtension.canDropFromExplosion`), asked once per destroyed block where `BlockBehaviour.onExplosionHit` asks `Block.dropFromExplosion`, before anything drops. `Some(false)` skips the drop: no loot, and none of the block's `spawnAfterBreak` (an infested block's silverfish, an ore's or a spawner's experience). `Some(true)` drops, even for TNT, which vanilla keeps from dropping. `None` leaves it to another part and then vanilla. The block is removed either way, `on_block_exploded` still hears it, and a TNT still primes: `TntBlock.wasExploded` is a separate moment. A panicking part is contained with a warning and vanilla's answer is used. Authoritative servers only; the level is read-only. With no part attached to the state, one table lookup per destroyed block. Diverges from upstream: new hook."#]
    #[hook(
        side = "server",
        contract = "replace",
        out = "drops",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::can_drop_from_explosion::GameLevel<'_>::native_can_drop_from_explosion",
        neoforge = "IBlockExtension.canDropFromExplosion"
    )]
    fn can_drop_from_explosion();
}
