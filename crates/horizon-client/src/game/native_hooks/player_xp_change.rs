use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server player's experience points are about to change through
    /// `Player.giveExperiencePoints` (NeoForge's `PlayerXpEvent.XpChange`): an
    /// orb pickup after mending, an advancement reward, or `/xp add ... points`.
    /// Change the signed `amount` (in points) or set `cancel` to give nothing.
    /// An orb or an advancement reward gives nothing for a negative amount.
    /// Experience bottles and furnaces make orbs, so they come through the
    /// orb path. `native_player_exp_change` is separate and still fires first
    /// for an orb pickup. Server-side. API 394.
    pub(in crate::game) fn native_player_xp_change(
        &mut self,
        player: u128,
        player_entity_id: i32,
        name: &str,
        amount: &mut i32,
        cancel: &mut bool,
    ) {
        self.fire_player_xp_change(player, player_entity_id, name, amount, cancel);
    }
}
