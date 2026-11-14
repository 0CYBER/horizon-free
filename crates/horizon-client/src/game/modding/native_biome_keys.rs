//! Typed biome identities belong to the same installed snapshot as data maps.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn biome_key(&self, id: &str) -> Option<mod_api::BiomeKey> {
        self.loaded_data_maps().ok()?.biome_keys.key(id)
    }
    pub(in crate::game) fn biome_key_id(&self, key: mod_api::BiomeKey) -> String {
        let Ok(maps) = self.loaded_data_maps() else {
            return String::new();
        };
        maps.biome_keys
            .id(key)
            .expect("stale or unknown biome key")
            .to_owned()
    }
}
