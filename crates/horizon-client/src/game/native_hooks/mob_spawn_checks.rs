use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `MobSpawnEvent.SpawnPlacementCheck`: the entity type `kind` may spawn
    /// at `pos` in `level` for `reason` (light, ground, biome:
    /// `SpawnPlacements.checkSpawnRules`). `default_result` is what vanilla's
    /// predicate answered; `spawner` is the block that asked, if one did;
    /// `result` starts as `Default` (vanilla's answer stands). Server-side.
    pub(in crate::game) fn native_mob_spawn_placement_check(
        &self,
        call: mod_api::__private::MobSpawnPlacementCheckCall<'_>,
    ) {
        self.fire_mob_spawn_placement_check(call);
    }
    /// `MobSpawnEvent.PositionCheck`: the freshly created mob of type `kind`
    /// may stand at `pos` in `level` for `reason` (`Mob.checkSpawnRules` and
    /// `checkSpawnObstruction`). `spawner` is the block that asked, if one
    /// did; `mob` is the mob itself, lent for the call. `result` starts as
    /// `Default`: vanilla's two checks decide; `Succeed` or `Fail` skips them.
    /// Server-side.
    pub(in crate::game) fn native_mob_spawn_position_check(
        &self,
        call: mod_api::__private::MobSpawnPositionCheckCall<'_>,
    ) {
        self.fire_mob_spawn_position_check(call);
    }
}
