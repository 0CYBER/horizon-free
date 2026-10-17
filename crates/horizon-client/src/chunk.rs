//! Resident client chunk storage. Sections are reference-counted so worker
//! threads can mesh and light immutable snapshots while the game thread edits
//! (copy-on-write through `Arc::make_mut`).
use horizon_core::paletted::{PalettedContainer, Strategy};
use horizon_world::registry::BlockStateId;
use std::sync::Arc;

pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;
pub const SECTIONS: usize = (HEIGHT / 16) as usize;

/// Biomes are 4x4x4 containers over the biome table (67 entries: a 7-bit
/// global palette).
pub const BIOME_STRATEGY: Strategy = Strategy::biomes(7);

/// Bits of the block states' global palette: 16 for vanilla's 35,723 states
/// (what vanilla servers and clients send), more when content mods add states
/// past 65,536 (docs/modpacks.md). Set once, before the first chunk.
static BLOCK_BITS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(16);

/// Block states are `PalettedContainer`s over the registry: vanilla's 16-bit
/// global palette, wider with enough content mods.
pub fn block_strategy() -> Strategy {
    Strategy::blocks(BLOCK_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Sizes the block states' global palette for a registry of `states`.
pub fn set_block_states(states: usize) {
    let bits = horizon_core::paletted::ceil_log2(states).max(16);
    BLOCK_BITS.store(bits, std::sync::atomic::Ordering::Relaxed);
}
/// Packed light per block: sky light in the high nibble, block light in the low one.
#[derive(Clone)]
pub enum Light {
    Uniform(u8),
    Full(Box<[u8; 4096]>),
}
impl Light {
    #[inline]
    pub fn get(&self, i: usize) -> u8 {
        match self {
            Self::Uniform(v) => *v,
            Self::Full(a) => a[i],
        }
    }
}
#[inline]
pub fn index(x: i32, y: i32, z: i32) -> usize {
    ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize
}
#[derive(Clone)]
pub struct Section {
    pub blocks: PalettedContainer,
    /// Biome table index per 4x4x4 cell, `(y << 2 | z) << 2 | x`.
    pub biomes: PalettedContainer,
    pub light: Light,
}
impl Default for Section {
    fn default() -> Self {
        Self {
            blocks: PalettedContainer::new(block_strategy(), 0),
            biomes: PalettedContainer::new(BIOME_STRATEGY, 0),
            light: Light::Uniform(0xF0),
        }
    }
}
impl Section {
    #[inline]
    pub fn get(&self, i: usize) -> u32 {
        self.blocks.get(i)
    }
    pub fn set(&mut self, i: usize, v: u32) -> u32 {
        self.blocks.set(i, v)
    }
    /// All 4,096 block states in index order.
    pub fn unpack(&self, out: &mut [u32; 4096]) {
        self.blocks.unpack(out);
    }
    /// The block states of a whole section at once.
    pub fn from_blocks(values: &[u32; 4096]) -> PalettedContainer {
        PalettedContainer::from_values(block_strategy(), values)
    }
    #[inline]
    pub fn light(&self, i: usize) -> u8 {
        self.light.get(i)
    }
    pub fn is_uniform(&self, v: u32) -> bool {
        self.blocks.is_uniform(v)
    }
    /// Heap bytes held by the block, biome and light storage (benchmark accounting).
    pub fn heap_bytes(&self) -> usize {
        self.blocks.heap_bytes()
            + self.biomes.heap_bytes()
            + match &self.light {
                Light::Uniform(_) => 0,
                Light::Full(_) => 4096,
            }
    }
    /// Collapse storage that became uniform, and drop palette entries no
    /// block uses any more.
    pub fn compact(&mut self) {
        for container in [&mut self.blocks, &mut self.biomes] {
            if container.bits() != 0
                && let Some(v) = container.uniform()
            {
                *container = PalettedContainer::new(container.strategy(), v);
            }
        }
        if let Light::Full(a) = &self.light
            && a.iter().all(|&b| b == a[0])
        {
            self.light = Light::Uniform(a[0]);
        }
    }
}
#[derive(Clone)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub sections: Vec<Arc<Section>>,
    /// Highest non-air, light-blocking-or-fluid Y per column (MOTION_BLOCKING-like), for rain/sky.
    pub heights: Arc<[i16; 256]>,
    pub lit: bool,
    /// Light read from a save with `isLightOn`: no light job is needed.
    pub light_loaded: bool,
    /// Newly generated (not loaded from a save): eligible for generation-time spawns.
    pub fresh: bool,
    /// Block entities read from a save, moved into the world on arrival.
    pub block_entities: Vec<((i32, i32, i32), crate::containers::BlockEntity)>,
    /// `CustomName` of the container block entities (shown as the screen title).
    pub block_names: Vec<((i32, i32, i32), String)>,
    /// Generation postprocessing marks (ProtoChunk.postProcessing) as world
    /// positions, applied once by LevelChunk.postProcessGeneration.
    pub postprocess: Vec<[i32; 3]>,
    /// Saved data Horizon keeps but does not simulate (save.rs).
    pub saved: Option<Arc<crate::save::ChunkExtras>>,
    /// Entities read from a save (vanilla entity NBT), spawned on arrival.
    pub entities: Vec<horizon_world::nbt::Tag>,
    /// Structure starts reaching into this chunk (`structures.References`):
    /// structure id and the start's chunk.
    pub structure_refs: Vec<(Arc<str>, (i32, i32))>,
}
impl Chunk {
    pub fn empty(x: i32, z: i32) -> Self {
        Self {
            x,
            z,
            sections: (0..SECTIONS)
                .map(|_| Arc::new(Section::default()))
                .collect(),
            heights: Arc::new([MIN_Y as i16; 256]),
            lit: false,
            light_loaded: false,
            fresh: false,
            block_entities: Vec::new(),
            block_names: Vec::new(),
            structure_refs: Vec::new(),
            postprocess: Vec::new(),
            saved: None,
            entities: Vec::new(),
        }
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u32 {
        let s = (y - MIN_Y) >> 4;
        if !(0..SECTIONS as i32).contains(&s) {
            return 0;
        }
        self.sections[s as usize].get(index(x, y, z))
    }
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        BlockStateId(self.get(x, y, z))
    }
    /// The highest y where `matches`, scanning down. Sections holding only
    /// `skip` are passed over whole: `matches` must be false for it.
    pub fn highest(&self, skip: u32, mut matches: impl FnMut(i32) -> bool) -> Option<i32> {
        self.sections
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, section)| !section.is_uniform(skip))
            .find_map(|(s, _)| {
                let base = MIN_Y + s as i32 * 16;
                (base..base + 16).rev().find(|&y| matches(y))
            })
    }
    pub fn set(&mut self, x: i32, y: i32, z: i32, v: u32) -> u32 {
        let s = (y - MIN_Y) >> 4;
        if !(0..SECTIONS as i32).contains(&s) {
            return v;
        }
        Arc::make_mut(&mut self.sections[s as usize]).set(index(x, y, z), v)
    }
    #[inline]
    pub fn light(&self, x: i32, y: i32, z: i32) -> u8 {
        let s = (y - MIN_Y) >> 4;
        if s < 0 {
            return 0;
        }
        if s >= SECTIONS as i32 {
            return 0xF0;
        }
        self.sections[s as usize].light(index(x, y, z))
    }
    /// Heap bytes held by the sections (benchmark accounting).
    pub fn heap_bytes(&self) -> usize {
        self.sections.iter().map(|s| s.heap_bytes()).sum::<usize>()
    }
    /// Biome table index at a block position (`ChunkAccess.getNoiseBiome` with
    /// the quart y clamped to the column).
    #[inline]
    pub fn biome(&self, x: i32, y: i32, z: i32) -> u16 {
        let qy = ((y - MIN_Y) >> 2).clamp(0, HEIGHT / 4 - 1);
        self.sections[(qy >> 2) as usize]
            .biomes
            .get(((qy & 3) << 4 | ((z & 15) >> 2) << 2 | (x & 15) >> 2) as usize) as u16
    }
    /// Sets the biome of one quart cell (`qy` counts from the column's bottom).
    pub fn set_biome_cell(&mut self, qx: i32, qy: i32, qz: i32, biome: u16) {
        let section = Arc::make_mut(&mut self.sections[(qy >> 2) as usize]);
        section.biomes.set(
            ((qy & 3) << 4 | (qz & 3) << 2 | (qx & 3)) as usize,
            u32::from(biome),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn highest_skips_only_sections_that_cannot_match() {
        let (air, cave_air, stone) = (0, 7, 1);
        let mut chunk = Chunk::empty(0, 0);
        // Stone low down, cave air (uniform in its section) above it, and a
        // section of air with one stone block higher up.
        for y in MIN_Y..MIN_Y + 40 {
            chunk.set(3, y, 4, stone);
        }
        for i in 0..4096 {
            Arc::make_mut(&mut chunk.sections[5]).set(i, cave_air);
        }
        chunk.set(3, 150, 4, stone);
        let full = |c: &Chunk, matches: &dyn Fn(u32) -> bool| {
            (MIN_Y..MIN_Y + HEIGHT)
                .rev()
                .find(|&y| matches(c.get(3, y, 4)))
        };
        for matches in [
            &(|v: u32| v != air) as &dyn Fn(u32) -> bool,
            &|v: u32| v == stone,
            &|v: u32| v == cave_air,
        ] {
            let fast = chunk.highest(air, |y| matches(chunk.get(3, y, 4)));
            assert_eq!(fast, full(&chunk, matches));
        }
        assert_eq!(
            chunk.highest(air, |y| chunk.get(3, y, 4) == stone),
            Some(150)
        );
        assert_eq!(Chunk::empty(0, 0).highest(air, |_| unreachable!()), None);
    }
}
