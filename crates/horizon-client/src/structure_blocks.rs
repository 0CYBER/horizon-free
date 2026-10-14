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

/// Client block entities for the generated block entities of a chunk.
pub fn block_entities(
    chunk: &ProtoChunk,
    loot: Option<&LootData>,
) -> Vec<((i32, i32, i32), BlockEntity)> {
    let mut out = Vec::new();
    for (pos, nbt) in &chunk.block_entities {
        let [x, y, z] = *pos;
        let block = chunk.block_name(x, y, z);
        let Some(mut entity) = BlockEntity::for_block(block) else {
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
                    let mut item = ItemStack {
                        item: Arc::from(s.item.as_str()),
                        count: s.count.max(0) as u32,
                        damage: s.damage.max(0) as u32,
                        extra: None,
                    };
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
        .map(|s| {
            s.map(|s| ItemStack {
                item: Arc::from(s.item.as_str()),
                count: s.count.max(0) as u32,
                damage: s.damage.max(0) as u32,
                extra: None,
            })
        })
        .collect();
    nbt.remove(&JavaString::from("LootTable"));
    nbt.remove(&JavaString::from("LootTableSeed"));
    nbt.insert("Items".into(), crate::save::encode_items(&slots));
    nbt
}
