use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A village siege found a spawn position on the server. Set `canceled`
    /// to stop the siege. There is one siege per dimension.
    /// 389.
    pub(in crate::game) fn native_village_siege(
        &mut self,
        dimension: &str,
        player: u128,
        attempted_pos: [f64; 3],
        canceled: &mut bool,
    ) {
        self.fire_village_siege(dimension, player, attempted_pos, canceled);
    }
}
