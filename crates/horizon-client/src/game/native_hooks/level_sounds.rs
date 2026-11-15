use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `PlayLevelSoundEvent.AtPosition` before server broadcast or client playback.
    pub(in crate::game) fn native_play_level_sound_at_position(
        &mut self,
        call: mod_api::__private::PlayLevelSoundAtPositionCall<'_>,
    ) {
        self.fire_play_level_sound_at_position(call);
    }
    /// `PlayLevelSoundEvent.AtEntity` for a sound following an entity on either side.
    pub(in crate::game) fn native_play_level_sound_at_entity(
        &mut self,
        call: mod_api::__private::PlayLevelSoundAtEntityCall<'_>,
    ) {
        self.fire_play_level_sound_at_entity(call);
    }
}
