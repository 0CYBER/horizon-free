use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `ModifyCustomSpawnersEvent`: the level `dimension` builds its custom spawners. `spawners`
    /// starts as vanilla's list; a mod takes spawners out or adds its own by key.
    pub(in crate::game) fn native_modify_custom_spawners(
        &self,
        dimension: &str,
        spawners: &mut mod_api::CustomSpawners,
    ) {
        self.fire_modify_custom_spawners(dimension, spawners);
    }
    /// `CustomSpawner.tick` for a mod's spawner `key` in the level `dimension`, with
    /// `spawn_enemies` as `ServerLevel.isSpawningMonsters`.
    pub(in crate::game) fn native_custom_spawner_tick(
        &mut self,
        dimension: &str,
        key: mod_api::SpawnerKey,
        spawn_enemies: bool,
    ) {
        self.fire_custom_spawner_tick(dimension, key, spawn_enemies);
    }
}
