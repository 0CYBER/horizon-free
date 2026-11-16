use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// NeoForge's `PermissionsChangedEvent`: the operator list is about to give player
    /// `player` (`name`) the permission level `new` in place of `old`, through `PlayerList.op`
    /// (`/op`, the console's `op`, the management API's `operators/add` and `operators/set`)
    /// or `PlayerList.deop` (`/deop` and the same on the console and the management API;
    /// `new` is then `All`). Set `cancel` and the list keeps the old level. The player need
    /// not be online: `player_entity_id` is their server entity id when they are. Clearing
    /// the list (`operators/clear`) is not a call to either, as in vanilla. Dedicated
    /// servers only; a single-player world has no operator list. Server-side.
    pub(in crate::game) fn native_permissions_changed(
        &mut self,
        player: u128,
        player_entity_id: Option<i32>,
        name: &str,
        old: PermissionLevel,
        new: PermissionLevel,
        cancel: &mut bool,
    ) {
        self.fire_permissions_changed(player, player_entity_id, name, old, new, cancel);
    }
}
