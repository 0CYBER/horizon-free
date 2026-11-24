use super::*;

impl Game<'_> {
    /// Draw at one listed player's latency icon slot, or cancel its vanilla sprite.
    pub(in crate::game) fn native_tab_list_latency_draw(
        &self,
        player_uuid: u128,
        player_name: &str,
        latency_ms: i32,
        bounds: [i32; 4],
        draw: &mut HudBatch,
        canceled: &mut bool,
    ) {
        self.fire_tab_list_latency_draw(player_uuid, player_name, latency_ms, bounds, draw, canceled);
    }
}
