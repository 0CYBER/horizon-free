//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `data_maps` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call `ItemKey::data`,
// `data_map_value` and `data_map_entries`
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait DataMapsBackend {
    /// Side: both.
    /// Entry `entry` (its full registry id, `minecraft:pig`) of registry `registry`: its value
    /// in data map `map`, before the map decodes it: the merged JSON the server's data packs
    /// and mods' files gave it, as NBT. `Ok(None)` where it has none.
    #[access(PublishedResources)]
    fn data_map_value(
        &self,
        registry: &str,
        entry: &str,
        map: &str,
    ) -> Result<Option<Tag>, DataMapError>;
    /// Side: both.
    /// Every entry data map `map` of registry `registry` gives a value, with that value as
    /// `data_map_value` answers it, in id order (`IRegistryExtension.getDataMap`).
    #[access(PublishedResources)]
    fn data_map_entries(
        &self,
        registry: &str,
        map: &str,
    ) -> Result<Vec<(String, Tag)>, DataMapError>;
}
