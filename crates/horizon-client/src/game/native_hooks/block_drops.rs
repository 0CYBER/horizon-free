use super::*;

impl GameLevel<'_> {
    /// `Block.dropResources` built the item entities `drops` for the block
    /// `state` broken at `pos` by `breaker` (a player's UUID, none for a
    /// level-side break) with `tool`, and `spawnAfterBreak` would award
    /// `experience` points: NeoForge's `BlockDropsEvent`. Edit the list or the
    /// amount, or set `cancel` for no drops, no experience and no
    /// `spawnAfterBreak` effects. Server-side, authoritative levels only.
    #[inline(always)]
    pub(in crate::game) fn native_block_drops(&self, call: super::fire::BlockDropsCall<'_>) {
        self.fire_block_drops(call);
    }
}
