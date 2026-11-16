use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player's level is about to change through
    /// `Player.giveExperienceLevels`: once per level gained or lost by points,
    /// and for the anvil, `/experience add ... levels` and `PlayerRef::give_levels`.
    /// Change `levels`, or set `cancel` to leave the level unchanged. The
    /// player's state is swapped in, so `native_acting_player`, `native_tell`
    /// and the item helpers reach them. Server-side.
    pub(in crate::game) fn native_player_xp_level_change(
        &mut self,
        player: u128,
        name: &str,
        levels: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_player_xp_level_change(player, name, levels, cancel);
    }
}
