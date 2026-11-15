//! Typed biome identities belong to the same installed snapshot as data maps.
use super::Game;
impl Game<'_> {
    pub(in crate::game) fn biome_key(&self, id: &str) -> Option<mod_api::BiomeKey> {
        self.loaded_data_maps().ok()?.biome_keys.key(id)
    }
    pub(in crate::game) fn biome_key_id(
        &self,
        key: mod_api::BiomeKey,
    ) -> Result<String, mod_api::BiomeKeyError> {
        let maps = self
            .loaded_data_maps()
            .map_err(|_| mod_api::BiomeKeyError::Unavailable)?;
        maps.biome_keys.id(key).map(str::to_owned)
    }
}
