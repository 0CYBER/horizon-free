use super::*;

/// The hook point after the client replaces its player.
impl Game<'_> {
    /// After the client replaces its player on respawn or dimension change
    /// (NeoForge's `ClientPlayerNetworkEvent.Clone`). Client-side.
    pub(in crate::game) fn native_client_player_clone(
        &mut self,
        old_id: i32,
        new_id: i32,
        uuid: u128,
        died: bool,
        dimension_changed: bool,
    ) {
        // The new LocalPlayer has no collision exclusions; a mod sets them again in the hook.
        if let Some(body) = self.player.as_mut() {
            body.collision_exclusions.clear();
        }
        self.fire_client_player_clone(old_id, new_id, uuid, died, dimension_changed);
    }
}
