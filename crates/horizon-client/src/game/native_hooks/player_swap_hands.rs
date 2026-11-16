use super::*;

impl Game<'_> {
    /// Heard before a non-spectator player's swap-hands action.
    /// The two mutable stacks start as the off-hand and main-hand stacks;
    /// the hands receive the listener's outputs unless it cancels.
    /// both slots. Set `cancel` to keep the items where they are: the player's
    /// client is sent both slots again. The player's
    /// state is swapped in, so `native_acting_player`, `native_tell` and the
    /// item helpers reach them. Once for each press, on a server: a LAN or
    /// dedicated server's players, the single-player owner and a LAN host.
    /// Not heard: the same swap made in an open inventory (hovering a slot and
    /// pressing the key, a menu click).
    pub(in crate::game) fn native_player_swap_hands(
        &mut self,
        player: u128,
        name: &str,
        to_main_hand: &mut Option<ItemStack>,
        to_off_hand: &mut Option<ItemStack>,
        cancel: &mut bool,
    ) {
        self.fire_player_swap_hands(player, name, to_main_hand, to_off_hand, cancel);
    }
}
