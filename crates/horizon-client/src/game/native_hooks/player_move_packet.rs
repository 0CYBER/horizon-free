use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Server player `player` (their UUID) sent a movement packet that passed validation, to `to`
    /// and `to_rotation`, with the packet's `on_ground` flag and what it carried. The guest's state
    /// is swapped in. Server-side.
    pub(in crate::game) fn native_player_move_packet(
        &mut self,
        call: mod_api::__private::PlayerMovePacketCall,
    ) {
        self.fire_player_move_packet(call);
    }
}
