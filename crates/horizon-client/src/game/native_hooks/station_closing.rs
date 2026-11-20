use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A workstation `PlayerRef::open_station` opened is closing for `player` (UUID): `window`
    /// is its id, `menu` its type and `inputs` its input slots before they go back to the
    /// inventory. A listener may empty a slot to keep its stack itself; only emptied slots count
    /// (game/modding/native_stations.rs).
    pub(in crate::game) fn native_station_closing(
        &mut self,
        player: u128,
        window: i32,
        menu: &str,
        inputs: &mut Vec<Option<ItemStack>>,
    ) {
        self.fire_station_closing(player, window, menu, inputs);
    }
}
