use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A viewed level's sound reached this client (`viewed_level_sound`): where in the player's
    /// level it plays, if anywhere. Client-side.
    pub(in crate::game) fn native_viewed_level_sound(
        &mut self,
        call: mod_api::__private::ViewedLevelSoundCall<'_>,
    ) {
        self.fire_viewed_level_sound(call);
    }
}
