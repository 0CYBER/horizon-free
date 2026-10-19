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
    ///
    /// Each corner re-derives its distance from a fresh six-step mix driven by
    /// the quart coordinate. The first two steps depend only on the X and Y
    /// offsets, so those shared prefixes are mixed once (two X states, then four
    /// XY states) instead of eight times, and the six-step tail is evaluated for
    /// all eight corners. Since `next(m, salt + offset) == next(m, salt) +
    /// offset`, each coordinate is folded into its step with one addition.
    pub fn quart_at_block(self, block: BlockPos) -> [i32; 3] {
        let shifted = [block.x, block.y, block.z].map(|v| v.wrapping_sub(2));
        let base = shifted.map(|v| v >> 2);
        let fractions = shifted.map(|v| f64::from(v & 3) / 4.);
        let coordinate = base.map(i64::from);
        let seed = self.seed;
        const A: i64 = 6364136223846793005;
        const B: i64 = 1442695040888963407;
        let g = |m: i64| m.wrapping_mul(m.wrapping_mul(A).wrapping_add(B));
        let root = g(seed);
        let m0: [i64; 2] =
            std::array::from_fn(|x: usize| root.wrapping_add(coordinate[0].wrapping_add(x as i64)));
        let m1: [[i64; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                g(m0[x]).wrapping_add(coordinate[1].wrapping_add(y as i64))
            })
        });
        let m2: [[[i64; 2]; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                std::array::from_fn(|z: usize| {
                    g(m1[x][y]).wrapping_add(coordinate[2].wrapping_add(z as i64))
                })
            })
        });
        let m3: [[[i64; 2]; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                std::array::from_fn(|z: usize| {
                    g(m2[x][y][z]).wrapping_add(coordinate[0].wrapping_add(x as i64))
                })
            })
        });
        let m4: [[[i64; 2]; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                std::array::from_fn(|z: usize| {
                    g(m3[x][y][z]).wrapping_add(coordinate[1].wrapping_add(y as i64))
                })
            })
        });
        let m5: [[[i64; 2]; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                std::array::from_fn(|z: usize| {
                    g(m4[x][y][z]).wrapping_add(coordinate[2].wrapping_add(z as i64))
                })
            })
        });
        let m6: [[[i64; 2]; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                std::array::from_fn(|z: usize| g(m5[x][y][z]).wrapping_add(seed))
            })
        });
        let m7: [[[i64; 2]; 2]; 2] = std::array::from_fn(|x: usize| {
            std::array::from_fn(|y: usize| {
                std::array::from_fn(|z: usize| g(m6[x][y][z]).wrapping_add(seed))
            })
        });
        let mut selected = base;
        let mut best = f64::INFINITY;
        for x in 0..2 {
            for y in 0..2 {
                for z in 0..2 {
                    let dx = fractions[0] - x as f64 + fiddle(m5[x][y][z]);
                    let dy = fractions[1] - y as f64 + fiddle(m6[x][y][z]);
                    let dz = fractions[2] - z as f64 + fiddle(m7[x][y][z]);
                    let distance = dz * dz + dy * dy + dx * dx;
                    if best > distance {
                        best = distance;
                        selected = [
                            base[0].wrapping_add(x as i32),
                            base[1].wrapping_add(y as i32),
                            base[2].wrapping_add(z as i32),
                        ];
                    }
                }
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
fn fiddle(seed: i64) -> f64 {
    (((seed >> 24) & 1023) as f64 / 1024. - 0.5) * 0.9
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The straightforward per-corner formulation, kept as the parity oracle
    /// for the prefix-shared implementation.
    fn reference(seed: i64, block: BlockPos) -> [i32; 3] {
        let shifted = [block.x, block.y, block.z].map(|v| v.wrapping_sub(2));
        let base = shifted.map(|v| v >> 2);
        let fractions = shifted.map(|v| f64::from(v & 3) / 4.);
        let mut selected = base;
        let mut best = f64::INFINITY;
        for corner in 0..8 {
            let offsets = [(corner >> 2) & 1, (corner >> 1) & 1, corner & 1];
            let quart: [i32; 3] = std::array::from_fn(|i| base[i].wrapping_add(offsets[i]));
            let delta: [f64; 3] = std::array::from_fn(|i| fractions[i] - f64::from(offsets[i]));
            let mut mixed = seed;
            for coordinate in quart.into_iter().cycle().take(6) {
                mixed = mixed
                    .wrapping_mul(
                        mixed
                            .wrapping_mul(6364136223846793005)
                            .wrapping_add(1442695040888963407),
                    )
                    .wrapping_add(i64::from(coordinate));
            }
            let x = delta[0] + fiddle(mixed);
            mixed = mixed
                .wrapping_mul(
                    mixed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407),
                )
                .wrapping_add(seed);
            let y = delta[1] + fiddle(mixed);
            mixed = mixed
                .wrapping_mul(
                    mixed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407),
                )
                .wrapping_add(seed);
            let z = delta[2] + fiddle(mixed);
            let distance = z * z + y * y + x * x;
            if best > distance {
                best = distance;
                selected = quart;
            }
        }
        selected
    }

    #[test]
    fn quart_at_block_matches_reference() {
        let seeds = [
            0i64,
            1,
            -1,
            i64::MIN,
            i64::MAX,
            0x0123_4567_89ab_cdef,
            -0x0123_4567_89ab_cdef,
        ];
        let extremes = [
            i32::MIN,
            i32::MIN + 1,
            -1,
            0,
            1,
            2,
            3,
            4,
            -2,
            i32::MAX - 1,
            i32::MAX,
        ];
        for &seed in &seeds {
            let zoom = BiomeZoom::new(seed);
            for x in -34..34 {
                for y in -34..34 {
                    for z in -34..34 {
                        let block = BlockPos::new(x, y, z);
                        assert_eq!(
                            zoom.quart_at_block(block),
                            reference(seed, block),
                            "seed {seed} at {x} {y} {z}"
                        );
                    }
                }
            }
            for &x in &extremes {
                for &y in &extremes {
                    for &z in &extremes {
                        let block = BlockPos::new(x, y, z);
                        assert_eq!(
                            zoom.quart_at_block(block),
                            reference(seed, block),
                            "seed {seed} at {x} {y} {z}"
                        );
                    }
                }
            }
        }
    }
}
