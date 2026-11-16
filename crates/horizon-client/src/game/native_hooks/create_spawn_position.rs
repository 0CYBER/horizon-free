use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before a new server level selects its initial world spawn
    /// (NeoForge's `LevelEvent.CreateSpawnPosition`).
    pub(in crate::game) fn native_create_spawn_position(
        &mut self,
        position: &mut [i32; 3],
        rotation: &mut [f32; 2],
        cancel: &mut bool,
    ) {
        self.fire_create_spawn_position(position, rotation, cancel);
    }
}
