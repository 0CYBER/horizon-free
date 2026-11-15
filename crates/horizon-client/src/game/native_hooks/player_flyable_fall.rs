use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `PlayerFlyableFallEvent`: server player `player` (`name`) landed after
    /// falling `distance` blocks at damage multiplier `multiplier` while allowed to fly (a
    /// creative or spectator player walking), so `Player.causeFallDamage` returned early
    /// and nothing was hurt. Listen only. Once for each landing, after the fall's
    /// block reaction. The player's state is swapped in. Server-side.
    pub(in crate::game) fn native_player_flyable_fall(
        &mut self,
        player: u128,
        player_entity_id: i32,
        name: &str,
        distance: f64,
        multiplier: f32,
    ) {
        self.fire_player_flyable_fall(player, player_entity_id, name, distance, multiplier);
    }
}
