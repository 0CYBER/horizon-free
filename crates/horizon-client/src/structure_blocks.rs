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
            eprintln!("Horizon: loot tables unavailable, structure containers stay empty: {e}");
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
                *slot = stack.map(|s| ItemStack {
                    item: Arc::from(s.item.as_str()),
                    count: s.count.max(0) as u32,
                    damage: s.damage.max(0) as u32,
                    extra: None,
                });
            }
        }
        out.push(((x, y, z), entity));
    }
    out
}
