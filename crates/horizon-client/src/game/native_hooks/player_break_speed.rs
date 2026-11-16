use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A player's destroy speed for the block at `pos` may be changed or canceled.
    pub(in crate::game) fn native_player_break_speed(
        &mut self,
        player: u32,
        pos: [i32; 3],
        state: u32,
        original_speed: f32,
        new_speed: &mut f32,
        canceled: &mut bool,
    ) {
        self.fire_player_break_speed(player, pos, state, original_speed, new_speed, canceled);
    }
}
