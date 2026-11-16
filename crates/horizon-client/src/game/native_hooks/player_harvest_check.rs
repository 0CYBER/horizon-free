use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A player's ability to harvest the block at `pos` may be changed.
    pub(in crate::game) fn native_player_harvest_check(
        &mut self,
        player: u32,
        pos: [i32; 3],
        state: u32,
        can_harvest: &mut bool,
    ) {
        self.fire_player_harvest_check(player, pos, state, can_harvest);
    }
}
