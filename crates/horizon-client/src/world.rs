//! Resident client world: streamed chunk columns plus the bookkeeping the game
//! loop needs to relight and remesh after edits.
use crate::chunk::{Chunk, MIN_Y, SECTIONS};
use horizon_core::random::LegacyRandom;
use horizon_world::registry::BlockStateId;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub struct World {
    pub chunks: HashMap<(i32, i32), Arc<Chunk>>,
    pub random: LegacyRandom,
    pub revision: u64,
    pub health_rules: crate::health::HealthRules,
    pub air: BlockStateId,
    /// Sections whose geometry changed and need remeshing.
    pub dirty_sections: HashSet<(i32, i32, i32)>,
    /// Chunks whose light must be recomputed.
    pub dirty_light: HashSet<(i32, i32)>,
    /// Chunks changed since the last save.
    pub modified: HashSet<(i32, i32)>,
    pub block_entities: HashMap<(i32, i32, i32), crate::containers::BlockEntity>,
    pub game_time: i64,
    pub day_time: i64,
}
impl World {
    pub fn new(air: BlockStateId, seed: i64) -> Self {
        Self {
            chunks: HashMap::new(),
            random: LegacyRandom::new(seed),
            revision: 0,
            health_rules: crate::health::HealthRules::default(),
            air,
            dirty_sections: HashSet::new(),
            dirty_light: HashSet::new(),
            modified: HashSet::new(),
            block_entities: HashMap::new(),
            game_time: 0,
            day_time: 1000,
        }
    }
    /// Resident storage write; marks the touched sections (and neighbours on
    /// section borders) for remeshing and the surrounding chunks for relighting.
    /// Neighbour updates, drops and block entities belong to the gameplay layer.
    pub fn set(&mut self, x: i32, y: i32, z: i32, state: BlockStateId) -> crate::Result<()> {
        let Some(chunk) = self.chunks.get_mut(&(x >> 4, z >> 4)) else {
            return Err("write outside resident chunks".into());
        };
        if y < MIN_Y || y >= MIN_Y + (SECTIONS as i32) * 16 {
            return Err("write outside world height".into());
        }
        let previous = Arc::make_mut(chunk).set(x & 15, y, z & 15, state.0 as u16);
        if u32::from(previous) != state.0 {
            self.revision = self.revision.wrapping_add(1);
            self.modified.insert((x >> 4, z >> 4));
            let sy = (y - MIN_Y) >> 4;
            for dz in -1..=1 {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (bx, by, bz) = (x + dx, y + dy, z + dz);
                        let s = (by - MIN_Y) >> 4;
                        if !(0..SECTIONS as i32).contains(&s) {
                            continue;
                        }
                        self.dirty_sections.insert((bx >> 4, s, bz >> 4));
                    }
                }
            }
            let _ = sy;
            for dz in -1..=1 {
                for dx in -1..=1 {
                    self.dirty_light.insert(((x >> 4) + dx, (z >> 4) + dz));
                }
            }
        }
        Ok(())
    }
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.chunks
            .get(&(x >> 4, z >> 4))
            .map_or(self.air, |chunk| chunk.block(x & 15, y, z & 15))
    }
    pub fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        self.chunks
            .get(&(x >> 4, z >> 4))
            .map_or(0xF0, |chunk| chunk.light(x & 15, y, z & 15))
    }
    pub fn is_air(&self, state: BlockStateId) -> bool {
        state == self.air
    }
    pub fn loaded(&self, x: i32, z: i32) -> bool {
        self.chunks.contains_key(&(x >> 4, z >> 4))
    }
}
