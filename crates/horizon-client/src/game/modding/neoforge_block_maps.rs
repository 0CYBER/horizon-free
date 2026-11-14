//! NeoForge's built-in block data maps (`NeoForgeDataMaps`): `neoforge:flammable`,
//! `neoforge:oxidizables` and `neoforge:waxables`, merged from the world's
//! data packs and mods' files as a mod's own data maps are, and installed on
//! the world's behaviours for fire spread, weathering, scraping and waxing.
use super::Game;
use super::native_data_maps::{DataMapType, DataMaps, MapRegistries, WorldRegistries};
use horizon_world::level::BlockMaps;
use std::sync::Arc;

/// The three maps' ids, all on `minecraft:block`.
const MAPS: [&str; 3] = [
    "neoforge:flammable",
    "neoforge:oxidizables",
    "neoforge:waxables",
];

/// The maps of a world's tables' files; none where no pack gives a value. `rank` is a block's
/// registry id, which picks the block an inverse map gives back.
fn load(
    overlay: &horizon_world::loot::PackOverlay<'_>,
    registries: &dyn MapRegistries,
    rank: impl Fn(&str) -> u32,
) -> Option<Arc<BlockMaps>> {
    let types = MAPS.map(|id| DataMapType {
        id: id.to_owned(),
        registry: "minecraft:block".to_owned(),
    });
    let maps = DataMaps::load(&types, overlay, registries);
    let values = |id| maps.json_values("minecraft:block", id);
    let block_maps =
        BlockMaps::from_values(values(MAPS[0]), values(MAPS[1]), values(MAPS[2]), rank);
    (!block_maps.is_empty()).then(|| Arc::new(block_maps))
}

impl Game<'_> {
    /// Install the open world's tables' block maps, replacing the last
    /// load's: none for a world without tables.
    pub(in crate::game) fn install_block_maps(&self) {
        let maps = self.world_tables.as_ref().and_then(|tables| {
            let overlay = horizon_world::loot::PackOverlay {
                conditions: &tables.conditions,
                files: &tables.files,
            };
            let table = &self.res.behaviours().table;
            let rank = |block: &str| {
                table
                    .block_by_name(block)
                    .map_or(u32::MAX, |b| u32::from(b.0))
            };
            // Tables and the registry generation are installed together before this call.
            // Use the same membership boundary as their staged data-map load.
            let registries = WorldRegistries::new(
                self.ui
                    .registries
                    .by_registry
                    .iter()
                    .map(|(registry, entries)| (registry.as_str(), entries.ids.as_slice())),
                self.mods.datapack_registries.as_deref(),
                &tables.items,
                &self.options.resources,
                &overlay,
            );
            load(&overlay, &registries, rank)
        });
        self.res.behaviours().set_block_maps(maps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pack_files_give_the_block_maps() {
        let file = |value: serde_json::Value| vec![serde_json::to_vec(&value).unwrap()];
        let files = vec![
            (
                "data_maps/neoforge:block/flammable".to_owned(),
                file(json!({"values": {"minecraft:stone": {"ignite_odds": 5, "burn_odds": 20}}})),
            ),
            (
                "data_maps/neoforge:block/waxables".to_owned(),
                file(json!({"values": {"test:tin": {"waxed": "test:waxed_tin"}}})),
            ),
        ];
        let overlay = horizon_world::loot::PackOverlay {
            files: &files,
            ..horizon_world::loot::PackOverlay::NONE
        };
        let ids = ["minecraft:stone", "test:tin", "test:waxed_tin"].map(String::from);
        let items = horizon_client::items::Items::empty();
        let registries = WorldRegistries::new(
            [("minecraft:block", ids.as_slice())],
            None,
            &items,
            std::path::Path::new(""),
            &overlay,
        );
        let block_maps = load(&overlay, &registries, |_| 0).unwrap();
        assert_eq!(block_maps.flammable("minecraft:stone"), Some((5, 20)));
        assert_eq!(block_maps.waxed("test:tin"), Some("test:waxed_tin"));
        assert_eq!(block_maps.next_oxidation("test:tin"), None);
        let none = load(&horizon_world::loot::PackOverlay::NONE, &registries, |_| 0);
        assert!(none.is_none());
    }
}
