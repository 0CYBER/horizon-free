use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `FinalizeSpawnEvent`: the mob of type `kind` at `pos` is about to be
    /// finalized (`Mob.finalizeSpawn`) for `reason`. `cancel` skips the
    /// finalization (the mob still spawns, uninitialised); `spawn_cancelled`
    /// stops the spawn. `difficulty` is what finalization will read; `mob` is
    /// the lent mob, changed through `edits`, and `group` the pack's group
    /// data finalization reads. Server-side.
    pub(in crate::game) fn native_finalize_spawn(
        &self,
        call: mod_api::__private::FinalizeSpawnCall<'_>,
    ) {
        self.fire_finalize_spawn(call);
    }
}
