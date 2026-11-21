use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A guest opened the ender chest at `pos`, once `container_opened` let the menu open;
    /// `player` is the guest's UUID. Server-side.
    pub(in crate::game) fn native_ender_chest_opened(&mut self, player: u128, pos: [i32; 3]) {
        self.fire_ender_chest_opened(player, pos);
    }
}
