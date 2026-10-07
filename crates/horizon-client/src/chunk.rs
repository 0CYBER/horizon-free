//! Resident client chunk storage. Sections are reference-counted so worker
//! threads can mesh and light immutable snapshots while the game thread edits
//! (copy-on-write through `Arc::make_mut`).
use horizon_world::registry::BlockStateId;
use std::sync::Arc;

pub const MIN_Y: i32 = -64;
pub const HEIGHT: i32 = 384;
pub const SECTIONS: usize = (HEIGHT / 16) as usize;

#[derive(Clone)]
pub enum Blocks {
    Uniform(u16),
    Full(Box<[u16; 4096]>),
}
/// Packed light per block: sky light in the high nibble, block light in the low one.
#[derive(Clone)]
pub enum Light {
    Uniform(u8),
    Full(Box<[u8; 4096]>),
}
#[inline]
pub fn index(x: i32, y: i32, z: i32) -> usize {
    ((y & 15) << 8 | (z & 15) << 4 | (x & 15)) as usize
}
#[derive(Clone)]
pub struct Section {
    pub blocks: Blocks,
    pub light: Light,
}
impl Default for Section {
    fn default() -> Self {
        Self {
            blocks: Blocks::Uniform(0),
            light: Light::Uniform(0xF0),
        }
    }
}
impl Section {
    #[inline]
    pub fn get(&self, i: usize) -> u16 {
        match &self.blocks {
            Blocks::Uniform(v) => *v,
            Blocks::Full(a) => a[i],
        }
    }
    pub fn set(&mut self, i: usize, v: u16) -> u16 {
        match &mut self.blocks {
            Blocks::Uniform(u) if *u == v => v,
            Blocks::Uniform(u) => {
                let old = *u;
                let mut a = Box::new([old; 4096]);
                a[i] = v;
                self.blocks = Blocks::Full(a);
                old
            }
            Blocks::Full(a) => std::mem::replace(&mut a[i], v),
        }
    }
    #[inline]
    pub fn light(&self, i: usize) -> u8 {
        match &self.light {
            Light::Uniform(v) => *v,
            Light::Full(a) => a[i],
        }
    }
    pub fn is_uniform(&self, v: u16) -> bool {
        match &self.blocks {
            Blocks::Uniform(u) => *u == v,
            Blocks::Full(a) => a.iter().all(|&b| b == v),
        }
    }
    /// Collapse storage that became uniform.
    pub fn compact(&mut self) {
        if let Blocks::Full(a) = &self.blocks
            && a.iter().all(|&b| b == a[0])
        {
            self.blocks = Blocks::Uniform(a[0]);
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
    /// Biome index per 4x4x4 cell (x,z in 0..4, y in 0..96) into `Biomes`.
    pub biomes: Arc<Vec<u16>>,
    /// Highest non-air, light-blocking-or-fluid Y per column (MOTION_BLOCKING-like), for rain/sky.
    pub heights: Arc<[i16; 256]>,
    pub lit: bool,
    /// Newly generated (not loaded from a save): eligible for generation-time spawns.
    pub fresh: bool,
}
impl Chunk {
    pub fn empty(x: i32, z: i32) -> Self {
        Self {
            x,
            z,
            sections: (0..SECTIONS).map(|_| Arc::new(Section::default())).collect(),
            biomes: Arc::new(vec![0; 16 * (HEIGHT as usize / 4)]),
            heights: Arc::new([MIN_Y as i16; 256]),
            lit: false,
            fresh: false,
        }
    }
    #[inline]
    pub fn get(&self, x: i32, y: i32, z: i32) -> u16 {
        let s = (y - MIN_Y) >> 4;
        if !(0..SECTIONS as i32).contains(&s) {
            return 0;
        }
        self.sections[s as usize].get(index(x, y, z))
    }
    #[inline]
    pub fn block(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        BlockStateId(u32::from(self.get(x, y, z)))
    }
    pub fn set(&mut self, x: i32, y: i32, z: i32, v: u16) -> u16 {
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
    #[inline]
    pub fn biome(&self, x: i32, y: i32, z: i32) -> u16 {
        let qy = ((y - MIN_Y) >> 2).clamp(0, HEIGHT / 4 - 1);
        self.biomes[(qy * 16 + ((z & 15) >> 2) * 4 + ((x & 15) >> 2)) as usize]
    }
}
