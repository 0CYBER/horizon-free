//! Block-to-quart biome lookup for Java Edition 26.3.
//! Selection is independent of the provider that stores or generates quart biomes.
mod seed_hash;
use crate::position::BlockPos;

/// The seed sent to clients: SHA-256 of the little-endian world seed, with the
/// first eight digest bytes interpreted as a little-endian signed long.
pub fn obfuscate_seed(world_seed: i64) -> i64 {
    seed_hash::hash(world_seed)
}

#[derive(Clone, Copy, Debug)]
pub struct BiomeZoom {
    seed: i64,
}
impl BiomeZoom {
    /// Accepts the obfuscated biome seed, not the original world seed.
    pub const fn new(obfuscated_seed: i64) -> Self {
        Self {
            seed: obfuscated_seed,
        }
    }
    /// Choose the closest of eight jittered quart corners, preserving vanilla's
    /// corner order, strict tie comparison and signed coordinate wrapping.
    pub fn quart_at_block(self, block: BlockPos) -> [i32; 3] {
        let shifted = [block.x, block.y, block.z].map(|v| v.wrapping_sub(2));
        let base = shifted.map(|v| v >> 2);
        let fractions = shifted.map(|v| f64::from(v & 3) / 4.);
        let mut selected = base;
        let mut best = f64::INFINITY;
        for corner in 0..8 {
            let offsets = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
            let quart = std::array::from_fn(|i| base[i].wrapping_add(offsets[i]));
            let delta = std::array::from_fn(|i| fractions[i] - f64::from(offsets[i]));
            let distance = fiddled_distance(self.seed, quart, delta);
            if best > distance {
                best = distance;
                selected = quart;
            }
        }
        selected
    }
}

/// Direct noise-biome lookup bypasses the eight-corner zoom.
pub fn noise_quart_at_block(block: BlockPos) -> [i32; 3] {
    [block.x, block.y, block.z].map(|v| v >> 2)
}
/// Java floors each double to a saturating int (NaN becomes zero) before shifting.
pub fn noise_quart_at_position(position: [f64; 3]) -> [i32; 3] {
    position.map(|v| (v.floor() as i32) >> 2)
}
fn next(seed: i64, salt: i64) -> i64 {
    seed.wrapping_mul(
        seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    )
    .wrapping_add(salt)
}
fn fiddle(seed: i64) -> f64 {
    (((seed >> 24) & 1023) as f64 / 1024. - 0.5) * 0.9
}
fn fiddled_distance(seed: i64, quart: [i32; 3], delta: [f64; 3]) -> f64 {
    let mut mixed = seed;
    for coordinate in quart.into_iter().cycle().take(6) {
        mixed = next(mixed, i64::from(coordinate));
    }
    let x = delta[0] + fiddle(mixed);
    mixed = next(mixed, seed);
    let y = delta[1] + fiddle(mixed);
    mixed = next(mixed, seed);
    let z = delta[2] + fiddle(mixed);
    z * z + y * y + x * x
}
