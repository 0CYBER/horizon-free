use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `player`'s client crossed `passage` from `from` (its feet before), and the server is
    /// about to follow (`guest_passage_cross`): the move to `from` passed and both ends are
    /// tracked. Set `cancel` and the crossing is refused (`Vetoed`); the player goes back
    /// to `from`. Server-side.
    pub(in crate::game) fn native_player_passage_cross(
        &mut self,
        player: PlayerRef,
        passage: mod_api::PassageId,
        from: mod_api::Vec3,
        cancel: &mut bool,
    ) {
        self.fire_player_passage_cross(player, passage, from, cancel);
    }
}
