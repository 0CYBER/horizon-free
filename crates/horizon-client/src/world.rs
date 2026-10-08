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
    /// Edits on lit chunks (position, old state, new state), relit
    /// incrementally by streaming.
    pub light_edits: Vec<((i32, i32, i32), u16, u16)>,
    /// Chunks changed since the last save.
    pub modified: HashSet<(i32, i32)>,
    pub block_entities: HashMap<(i32, i32, i32), crate::containers::BlockEntity>,
    pub game_time: i64,
    pub day_time: i64,
    pub weather: Weather,
}
/// ServerLevel weather state (advanceWeatherCycle).
#[derive(Clone, Copy, Debug, Default)]
pub struct Weather {
    pub clear_time: i32,
    pub rain_time: i32,
    pub thunder_time: i32,
    pub raining: bool,
    pub thundering: bool,
    pub rain_level: (f32, f32),
    pub thunder_level: (f32, f32),
}
impl Weather {
    pub fn tick(&mut self, random: &mut horizon_core::random::LegacyRandom) {
        use horizon_core::random::RandomSource;
        let mut uniform = |min: i32, max: i32| min + random.next_int_bound(max - min + 1).unwrap_or(0);
        if self.clear_time > 0 {
            self.clear_time -= 1;
            self.thunder_time = if self.thundering { 0 } else { 1 };
            self.rain_time = if self.raining { 0 } else { 1 };
            self.thundering = false;
            self.raining = false;
        } else {
            if self.thunder_time > 0 {
                self.thunder_time -= 1;
                if self.thunder_time == 0 {
                    self.thundering = !self.thundering;
                }
            } else if self.thundering {
                self.thunder_time = uniform(3600, 15600);
            } else {
                self.thunder_time = uniform(12000, 180000);
            }
            if self.rain_time > 0 {
                self.rain_time -= 1;
                if self.rain_time == 0 {
                    self.raining = !self.raining;
                }
            } else if self.raining {
                self.rain_time = uniform(12000, 24000);
            } else {
                self.rain_time = uniform(12000, 180000);
            }
        }
        self.thunder_level.0 = self.thunder_level.1;
        self.thunder_level.1 = (self.thunder_level.1 + if self.thundering { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
        self.rain_level.0 = self.rain_level.1;
        self.rain_level.1 = (self.rain_level.1 + if self.raining { 0.01 } else { -0.01 }).clamp(0.0, 1.0);
    }
    /// Level.getRainLevel / getThunderLevel (thunder scales with rain).
    pub fn levels(&self, partial: f32) -> (f32, f32) {
        let rain = self.rain_level.0 + (self.rain_level.1 - self.rain_level.0) * partial;
        let thunder = self.thunder_level.0 + (self.thunder_level.1 - self.thunder_level.0) * partial;
        (rain, thunder * rain)
    }
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
            light_edits: Vec::new(),
            modified: HashSet::new(),
            block_entities: HashMap::new(),
            game_time: 0,
            day_time: 1000,
            weather: Weather::default(),
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
            if self.chunks.get(&(x >> 4, z >> 4)).is_some_and(|c| c.lit) {
                self.light_edits.push(((x, y, z), previous, state.0 as u16));
            } else {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        self.dirty_light.insert(((x >> 4) + dx, (z >> 4) + dz));
                    }
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
