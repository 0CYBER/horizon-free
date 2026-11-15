use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mob or a player is about to change the block at `pos`: a landing's
    /// trampling (farmland to dirt, a turtle egg breaking), before its
    /// neighbour updates and events. `player` is the player whose landing it
    /// was, `entity` and `kind` the mob for a mob's landing (`None` and the
    /// player's type here). `from` and `to` are engine states; dispatch lifts both to owned API values. Set `cancel`
    /// and the block stays as it was and nothing the change would have caused
    /// happens, as Bukkit's `EntityChangeBlockEvent` (@API:entity_change_block_pre@).
    pub(in crate::game) fn native_entity_change_block_pre(
        &mut self,
        call: super::fire::EntityChangeBlockPreCall<'_>,
    ) {
        self.fire_entity_change_block_pre(call);
    }
}
