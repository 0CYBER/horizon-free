//! Block entities placed by world generation (structure chests, barrels and
//! template containers): turned into client containers, with loot tables
//! unpacked as `RandomizableContainer.unpackLootTable` would on first open.
use crate::{containers::BlockEntity, items::ItemStack};
use horizon_world::{
    loot::LootData,
    nbt::{Compound, JavaString, Tag},
    worldgen::generator::ProtoChunk,
};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
};

/// The `custom_data` key holding an exploration map's search until the game
/// runs it (vanilla runs `ExplorationMapFunction` when the chest is unpacked,
/// on first open; here loot is rolled with the chunk).
const PENDING_MAP: &str = "horizon:exploration_map";
const CUSTOM_DATA: &str = "minecraft:custom_data";

/// A generated chest's item, with its exploration map search kept in
/// `minecraft:custom_data` (which vanilla loads as is).
fn loot_item(s: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = ItemStack {
        item: Arc::from(s.item.as_str()),
        count: s.count.max(0) as u32,
        damage: s.damage.max(0) as u32,
        extra: None,
    };
    if let Some(map) = &s.exploration {
        let search = Tag::compound([
            ("destination", Tag::String(map.destination.as_str().into())),
            ("decoration", Tag::String(map.decoration.as_str().into())),
            ("zoom", Tag::Byte(map.zoom as i8)),
            ("search_radius", Tag::Int(map.search_radius)),
            (
                "skip_existing_chunks",
                Tag::Byte(i8::from(map.skip_existing_chunks)),
            ),
        ]);
        item.data_mut()
            .other
            .push((CUSTOM_DATA.into(), Tag::compound([(PENDING_MAP, search)])));
    }
    item
}

/// A rolled stack as an item with its enchantments (stored ones on a
/// book), potion and ominous bottle level.
pub fn loot_item_with_components(s: &horizon_world::loot::LootStack) -> ItemStack {
    let mut item = loot_item(s);
    for (id, level) in &s.enchantments {
        let level = (*level).max(0) as u32;
        if &*item.item == "minecraft:enchanted_book" {
            crate::item_data::set_level(&mut item.data_mut().stored_enchantments, id, level);
        } else {
            item.enchant(id, level);
        }
    }
    if let Some(potion) = &s.potion {
        item = item.with_potion(potion);
    }
    if let Some(level) = s.ominous_amplifier.filter(|l| *l > 0) {
        item.data_mut().ominous_amplifier = Some(level.clamp(0, 4) as u8);
    }
    item
}

/// The exploration map search an item still waits for, removed from it.
pub fn take_pending_map(item: &mut ItemStack) -> Option<horizon_world::loot::ExplorationMap> {
    let data = item.extra.as_deref_mut()?;
    let i = data
        .other
        .iter()
        .position(|(k, t)| k == CUSTOM_DATA && t.get(PENDING_MAP).is_some())?;
    let (_, mut custom) = data.other.remove(i);
    let search = match &mut custom {
        Tag::Compound(c) => c.remove(&JavaString::from(PENDING_MAP))?,
        _ => return None,
    };
    if matches!(&custom, Tag::Compound(c) if !c.is_empty()) {
        data.other.insert(i, (CUSTOM_DATA.into(), custom));
    }
    let text = |k: &str| match search.get(k) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    };
    let int = |k: &str| match search.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Byte(v)) => Some(i32::from(*v)),
        _ => None,
    };
    Some(horizon_world::loot::ExplorationMap {
        destination: text("destination")?,
        decoration: text("decoration")?,
        zoom: int("zoom").unwrap_or(2).clamp(0, 4) as u8,
        search_radius: int("search_radius").unwrap_or(50),
        skip_existing_chunks: int("skip_existing_chunks").unwrap_or(1) != 0,
    })
}

/// Whether an item still waits for its exploration map search.
pub fn has_pending_map(item: &ItemStack) -> bool {
    item.data().is_some_and(|d| {
        d.other
            .iter()
            .any(|(k, t)| k == CUSTOM_DATA && t.get(PENDING_MAP).is_some())
    })
}

/// Loot data of the resource root, loaded once per process.
pub fn loot_data(root: &Path) -> Option<Arc<LootData>> {
    static LOOT: OnceLock<Option<Arc<LootData>>> = OnceLock::new();
    LOOT.get_or_init(|| match LootData::load(root) {
        Ok(data) => Some(Arc::new(data)),
        Err(e) => {
            log::warn!("Horizon: loot tables unavailable, structure containers stay empty: {e}");
            None
        }
    })
    .clone()
}

fn text(nbt: &Compound, key: &str) -> Option<String> {
    match nbt.get(&JavaString::from(key)) {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}

/// The client block entity for one of a chunk's generated block entities,
/// before a container's loot is unpacked: `StructureTemplate.placeInWorld`
/// hands the template's block entity NBT to the level's block entity.
fn generated_block_entity(block: &str, nbt: &Compound) -> Option<BlockEntity> {
    let mut entity = BlockEntity::for_block(block)?;
    match &mut entity {
        // MonsterRoomFeature and the structures' setEntityId.
        BlockEntity::Spawner(s) => {
            **s = crate::mobs::spawner::Spawner::load(&Tag::Compound(nbt.clone()));
        }
        BlockEntity::TrialSpawner(s) => {
            **s = crate::mobs::trial_spawner::TrialSpawner::load(&Tag::Compound(nbt.clone()));
        }
        BlockEntity::Vault(v) => **v = crate::vault::Vault::load(&Tag::Compound(nbt.clone())),
        // BannerBlockEntity.loadAdditional: the End cities' black triangles.
        BlockEntity::Banner(b) => *b = crate::save::decode_banner(&Tag::Compound(nbt.clone())),
        _ => {}
    }
    Some(entity)
}

/// Client block entities for the generated block entities of a chunk.
pub fn block_entities(
    chunk: &ProtoChunk,
    loot: Option<&LootData>,
) -> Vec<((i32, i32, i32), BlockEntity)> {
    let mut out = Vec::new();
    for (pos, nbt) in &chunk.block_entities {
        let [x, y, z] = *pos;
        let block = chunk.block_name(x, y, z);
        let Some(mut entity) = generated_block_entity(block, nbt) else {
            continue;
        };
        if let (BlockEntity::Chest(slots), Some(table), Some(loot)) =
            (&mut entity, text(nbt, "LootTable"), loot)
        {
            let seed = match nbt.get(&JavaString::from("LootTableSeed")) {
                Some(Tag::Long(seed)) => *seed,
                // Seed 0 means "the level's random": derive one from the position.
                _ => horizon_core::random::position_seed(horizon_core::position::BlockPos::new(
                    x, y, z,
                )),
            };
            let biome = chunk.biome_at(x, y, z);
            for (slot, stack) in slots
                .iter_mut()
                .zip(loot.fill(&table, seed, 27, Some(biome)))
            {
                *slot = stack.map(|s| {
                    let mut item = loot_item(&s);
                    // An ominous bottle's level (the default, 0, is no component).
                    if let Some(level) = s.ominous_amplifier.filter(|l| *l > 0) {
                        item.data_mut().ominous_amplifier = Some(level.clamp(0, 4) as u8);
                    }
                    item
                });
            }
        }
        out.push(((x, y, z), entity));
    }
    out
}

/// Chest minecarts placed with a loot table (mineshafts): the table rolled
/// into `Items` as `unpackLootTable` would on first open.
pub fn unpack_entity_loot(
    chunk: &ProtoChunk,
    mut nbt: horizon_world::nbt::Compound,
    loot: Option<&LootData>,
) -> horizon_world::nbt::Compound {
    let (Some(table), Some(loot)) = (
        match nbt.get(&JavaString::from("LootTable")) {
            Some(Tag::String(s)) => Some(s.to_string_lossy()),
            _ => None,
        },
        loot,
    ) else {
        return nbt;
    };
    let seed = match nbt.get(&JavaString::from("LootTableSeed")) {
        Some(Tag::Long(seed)) => *seed,
        _ => 0,
    };
    let pos = match nbt.get(&JavaString::from("Pos")) {
        Some(Tag::List(p)) => std::array::from_fn(|i| match p.get(i) {
            Some(Tag::Double(b)) => f64::from_bits(*b).floor() as i32,
            _ => 0,
        }),
        _ => [0; 3],
    };
    let biome = chunk.biome_at(pos[0], pos[1], pos[2]);
    let slots: Vec<Option<ItemStack>> = loot
        .fill(&table, seed, 27, Some(biome))
        .into_iter()
        .map(|s| s.map(|s| loot_item(&s)))
        .collect();
    nbt.remove(&JavaString::from("LootTable"));
    nbt.remove(&JavaString::from("LootTableSeed"));
    nbt.insert("Items".into(), crate::save::encode_items(&slots));
    nbt
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::loot::{ExplorationMap, LootStack};

    #[test]
    fn exploration_maps_wait_in_custom_data_until_searched() {
        let mut stack = LootStack {
            item: "minecraft:buried_treasure_map".into(),
            count: 1,
            damage: 0,
            enchantments: Vec::new(),
            potion: None,
            map: true,
            exploration: Some(Box::new(ExplorationMap::from_json(&serde_json::json!({
                "decoration": "minecraft:red_x",
                "destination": "#minecraft:on_treasure_maps",
                "skip_existing_chunks": false,
                "zoom": 1
            })))),
            ominous_amplifier: None,
        };
        let mut item = loot_item(&stack);
        assert!(has_pending_map(&item));
        // The rest of custom_data stays.
        if let Some(Tag::Compound(c)) = item
            .data_mut()
            .other
            .iter_mut()
            .find(|(k, _)| k == CUSTOM_DATA)
            .map(|(_, t)| t)
        {
            c.insert("note".into(), Tag::Int(7));
        }
        let map = take_pending_map(&mut item).unwrap();
        assert_eq!(
            map,
            ExplorationMap {
                destination: "minecraft:on_treasure_maps".into(),
                decoration: "minecraft:red_x".into(),
                zoom: 1,
                search_radius: 50,
                skip_existing_chunks: false,
            }
        );
        assert!(!has_pending_map(&item));
        assert!(take_pending_map(&mut item).is_none());
        let custom = &item.data().unwrap().other;
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].1.get("note"), Some(&Tag::Int(7)));
        // Only the search: the component goes away with it.
        stack.item = "minecraft:abandoned_camp_map".into();
        let mut item = loot_item(&stack);
        take_pending_map(&mut item).unwrap();
        assert!(item.data().unwrap().other.is_empty());
        // A plain item has nothing pending.
        stack.exploration = None;
        assert!(!has_pending_map(&loot_item(&stack)));
    }

    /// The End cities' tower tops save their banners' patterns in the
    /// template; the placed banner must keep them (`EndCityPieces`).
    #[test]
    fn a_generated_banner_keeps_its_template_patterns() {
        let patterns = Tag::List(vec![
            Tag::compound([
                ("color", Tag::String("black".into())),
                ("pattern", Tag::String("minecraft:triangle_top".into())),
            ]),
            Tag::compound([
                ("color", Tag::String("black".into())),
                ("pattern", Tag::String("minecraft:triangle_bottom".into())),
            ]),
        ]);
        let nbt = Tag::compound([
            ("id", Tag::String("minecraft:banner".into())),
            ("patterns", patterns),
        ]);
        let Tag::Compound(nbt) = nbt else {
            unreachable!()
        };
        let Some(BlockEntity::Banner(banner)) =
            generated_block_entity("minecraft:magenta_banner", &nbt)
        else {
            panic!("a banner block makes a banner block entity");
        };
        assert_eq!(banner.layers.len(), 2);
        assert_eq!(&*banner.layers[0].pattern, "minecraft:triangle_top");
        assert_eq!(&*banner.layers[1].pattern, "minecraft:triangle_bottom");
        assert_eq!(&*banner.layers[0].color, "black");
        // A banner the template saved without patterns stays bare.
        let Some(BlockEntity::Banner(bare)) =
            generated_block_entity("minecraft:white_banner", &Compound::new())
        else {
            panic!("a banner block makes a banner block entity");
        };
        assert!(bare.layers.is_empty());
    }
}
