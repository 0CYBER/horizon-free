//! Seeded terracotta bands and the material system's surface-noise queries.
use crate::registry::{BlockRegistry, BlockStateId};
use horizon_core::{
    noise::Noise,
    random::{PositionalRandom, RandomSource},
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct ClayBandBlocks {
    pub terracotta: BlockStateId,
    pub orange: BlockStateId,
    pub yellow: BlockStateId,
    pub brown: BlockStateId,
    pub red: BlockStateId,
    pub white: BlockStateId,
    pub light_gray: BlockStateId,
}
impl ClayBandBlocks {
    pub fn from_registry(registry: &BlockRegistry) -> Option<Self> {
        Some(Self {
            terracotta: registry.default_state("minecraft:terracotta")?,
            orange: registry.default_state("minecraft:orange_terracotta")?,
            yellow: registry.default_state("minecraft:yellow_terracotta")?,
            brown: registry.default_state("minecraft:brown_terracotta")?,
            red: registry.default_state("minecraft:red_terracotta")?,
            white: registry.default_state("minecraft:white_terracotta")?,
            light_gray: registry.default_state("minecraft:light_gray_terracotta")?,
        })
    }
}
#[derive(Clone, Debug)]
pub struct ClayBands([BlockStateId; 192]);
impl ClayBands {
    /// The input is MaterialSystem's clay_bands child stream. Every conditional
    /// random draw and overwrite retains vanilla order.
    pub fn generate(random: &mut impl RandomSource, blocks: ClayBandBlocks) -> Self {
        let mut bands = [blocks.terracotta; 192];
        let mut i = 0;
        while i < bands.len() {
            i += random.next_int_bound(5).unwrap() as usize + 1;
            if i < bands.len() {
                bands[i] = blocks.orange;
            }
            i += 1;
        }
        Self::make_bands(random, &mut bands, 1, blocks.yellow);
        Self::make_bands(random, &mut bands, 2, blocks.brown);
        Self::make_bands(random, &mut bands, 1, blocks.red);
        let count = random.next_int_between_inclusive(9, 15).unwrap();
        let mut position = 0;
        for _ in 0..count {
            if position >= bands.len() {
                break;
            }
            bands[position] = blocks.white;
            if position > 1 && random.next_boolean() {
                bands[position - 1] = blocks.light_gray;
            }
            if position + 1 < bands.len() && random.next_boolean() {
                bands[position + 1] = blocks.light_gray;
            }
            position += random.next_int_bound(16).unwrap() as usize + 4;
        }
        Self(bands)
    }
    fn make_bands(
        random: &mut impl RandomSource,
        bands: &mut [BlockStateId; 192],
        base_width: usize,
        state: BlockStateId,
    ) {
        let count = random.next_int_between_inclusive(6, 15).unwrap();
        for _ in 0..count {
            let width = base_width + random.next_int_bound(3).unwrap() as usize;
            let start = random.next_int_bound(192).unwrap() as usize;
            let end = (start + width).min(bands.len());
            bands[start..end].fill(state);
        }
    }
    pub fn states(&self) -> &[BlockStateId; 192] {
        &self.0
    }
    /// Preserve Java's signed remainder. Extreme or unsupported negative heights
    /// can produce an invalid index, which returns None instead of an exception.
    pub fn at_noise(&self, y: i32, noise: f32) -> Option<BlockStateId> {
        let offset = round_float(noise * 4.);
        let index = y.wrapping_add(offset).wrapping_add(192) % 192;
        self.0.get(index as usize).copied()
    }
}
fn round_float(value: f32) -> i32 {
    // f64 retains every relevant half-integer boundary of a float that fits an
    // int. Rust's cast supplies Java's NaN-to-zero and saturating endpoints.
    (f64::from(value) + 0.5).floor() as i32
}
#[derive(Debug)]
pub struct MaterialSurface<P> {
    bands: ClayBands,
    random: P,
    clay_offset: Arc<dyn Noise>,
    surface: Arc<dyn Noise>,
    secondary: Arc<dyn Noise>,
}
impl<P: PositionalRandom> MaterialSurface<P> {
    /// `random` is the world's root positional factory, not a surface child.
    pub fn new(
        random: P,
        blocks: ClayBandBlocks,
        clay_offset: Arc<dyn Noise>,
        surface: Arc<dyn Noise>,
        secondary: Arc<dyn Noise>,
    ) -> Self {
        let bands = ClayBands::generate(&mut random.from_hash_of("minecraft:clay_bands"), blocks);
        Self {
            bands,
            random,
            clay_offset,
            surface,
            secondary,
        }
    }
    pub fn bands(&self) -> &ClayBands {
        &self.bands
    }
    pub fn band(&self, [x, y, z]: [i32; 3]) -> Option<BlockStateId> {
        self.bands
            .at_noise(y, self.clay_offset.get3(f64::from(x), 0., f64::from(z)))
    }
    pub fn surface_depth(&self, x: i32, z: i32) -> i32 {
        let noise = f64::from(self.surface.get3(f64::from(x), 0., f64::from(z)));
        ((noise * 2.75 + 3.)
            + self
                .random
                .at(horizon_core::position::BlockPos::new(x, 0, z))
                .next_double()
                * 0.25) as i32
    }
    pub fn surface_secondary(&self, x: i32, z: i32) -> f64 {
        f64::from(self.secondary.get3(f64::from(x), 0., f64::from(z)))
    }
}
