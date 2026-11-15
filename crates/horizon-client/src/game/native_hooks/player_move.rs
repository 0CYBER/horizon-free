use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Server player `player` (their UUID) would move from `from` to `to` (or turn from
    /// `from_rotation` to `to_rotation`); set `cancel` to veto the report. Before it is
    /// committed. Server-side.
    pub(in crate::game) fn native_player_move(
        &mut self,
        player: u128,
        from: [f64; 3],
        from_rotation: [f32; 2],
        to: [f64; 3],
        to_rotation: [f32; 2],
        cancel: &mut bool,
    ) {
        self.fire_player_move(player, from, from_rotation, to, to_rotation, cancel);
    }
}
