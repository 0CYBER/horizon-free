use super::{GRADIENTS, GradientTable};
use crate::{geometry::floor_i32, random::RandomSource};

/// First 12 entries of [`GRADIENTS`] as exact doubles (every component is -1, 0 or 1).
const SIMPLEX_GRADIENTS: [[f64; 3]; 12] = {
    let mut out = [[0.0; 3]; 12];
    let mut i = 0;
    while i < 12 {
        out[i] = [
            GRADIENTS[i][0] as f64,
            GRADIENTS[i][1] as f64,
            GRADIENTS[i][2] as f64,
        ];
        i += 1;
    }
    out
};

#[derive(Clone, Debug)]
pub struct SimplexNoise {
    table: GradientTable,
    /// Doubled permutation table (`table.permutations[i & 255]`), eliminating
    /// inner `& 255` masks on `cell + delta + perm[..]` indices (all in 0..=511).
    perm: [u8; 512],
    /// `perm[i] % 12`, eliminating the integer remainder on every corner lookup.
    perm_mod12: [u8; 512],
}

impl SimplexNoise {
    pub fn new(random: &mut impl RandomSource) -> Self {
        Self::with_skipped_offsets(random, false)
    }
    pub fn with_skipped_offsets(random: &mut impl RandomSource, skip_offsets: bool) -> Self {
        let table = GradientTable::new(random, if skip_offsets { 0.0 } else { 256.0 });
        let raw = &table.permutations;
        let perm: [u8; 512] = std::array::from_fn(|i| raw[i & 255]);
        let perm_mod12: [u8; 512] = std::array::from_fn(|i| perm[i] % 12);
        Self {
            table,
            perm,
            perm_mod12,
        }
    }
    pub fn table(&self) -> &GradientTable {
        &self.table
    }
    pub fn range(&self) -> [f32; 2] {
        [-2.0, 2.0]
    }

    #[inline(always)]
    fn corner(gradient: usize, point: [f64; 3], radius: f64) -> f64 {
        let attenuation = radius - point[0] * point[0] - point[1] * point[1] - point[2] * point[2];
        // Negative attenuation contributes zero; NaN follows the arithmetic path.
        if attenuation < 0.0 {
            0.0
        } else {
            let squared = attenuation * attenuation;
            let g = SIMPLEX_GRADIENTS[gradient];
            squared * squared * (g[0] * point[0] + g[1] * point[1] + g[2] * point[2])
        }
    }

    #[inline]
    pub fn get2(&self, x: f64, y: f64) -> f32 {
        self.get2_f64(x, y) as f32
    }

    /// `get2` before its cast to `float` (1.21's `SimplexNoise.getValue(x, y)` returned the double).
    #[inline]
    pub fn get2_f64(&self, x: f64, y: f64) -> f64 {
        let x = x + self.table.offsets[0];
        let y = y + self.table.offsets[1];
        let root3 = 3.0_f64.sqrt();
        let skew = 0.5 * (root3 - 1.0);
        let unskew = (3.0 - root3) / 6.0;
        let shift = (x + y) * skew;
        let i = floor_i32(x + shift);
        let j = floor_i32(y + shift);
        let shift = f64::from(i.wrapping_add(j)) * unskew;
        let dx = x - (f64::from(i) - shift);
        let dy = y - (f64::from(j) - shift);
        let (a, b) = if dx > dy { (1_usize, 0_usize) } else { (0, 1) };
        let i = (i & 255) as usize;
        let j = (j & 255) as usize;
        let grad = |da: usize, db: usize| {
            usize::from(self.perm_mod12[i + da + usize::from(self.perm[j + db])])
        };
        let first = Self::corner(grad(0, 0), [dx, dy, 0.0], 0.5);
        let second = Self::corner(
            grad(a, b),
            [
                dx - f64::from(a as i32) + unskew,
                dy - f64::from(b as i32) + unskew,
                0.0,
            ],
            0.5,
        );
        let third = Self::corner(
            grad(1, 1),
            [dx - 1.0 + 2.0 * unskew, dy - 1.0 + 2.0 * unskew, 0.0],
            0.5,
        );
        70.0 * (first + second + third)
    }

    #[inline]
    pub fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        self.get3_f64(x, y, z) as f32
    }

    /// `get3` before its cast to `float`.
    #[inline]
    pub fn get3_f64(&self, x: f64, y: f64, z: f64) -> f64 {
        let shifted = [
            x + self.table.offsets[0],
            y + self.table.offsets[1],
            z + self.table.offsets[2],
        ];
        let skew = (shifted[0] + shifted[1] + shifted[2]) * (1.0 / 3.0);
        let cell = shifted.map(|v| floor_i32(v + skew));
        let unskew = f64::from(cell[0].wrapping_add(cell[1]).wrapping_add(cell[2])) * (1.0 / 6.0);
        let point: [f64; 3] = std::array::from_fn(|i| shifted[i] - (f64::from(cell[i]) - unskew));
        let [x, y, z] = point;
        let (a, b) = if x >= y {
            if y >= z {
                ([1_usize, 0, 0], [1_usize, 1, 0])
            } else if x >= z {
                ([1, 0, 0], [1, 0, 1])
            } else {
                ([0, 0, 1], [1, 0, 1])
            }
        } else if y < z {
            ([0, 0, 1], [0, 1, 1])
        } else if x < z {
            ([0, 1, 0], [0, 1, 1])
        } else {
            ([0, 1, 0], [1, 1, 0])
        };
        let c0 = (cell[0] & 255) as usize;
        let c1 = (cell[1] & 255) as usize;
        let c2 = (cell[2] & 255) as usize;
        let grad = |d: [usize; 3]| {
            let p2 = usize::from(self.perm[c2 + d[2]]);
            let p1 = usize::from(self.perm[c1 + d[1] + p2]);
            usize::from(self.perm_mod12[c0 + d[0] + p1])
        };
        let first = Self::corner(grad([0; 3]), point, 0.6);
        let second = Self::corner(
            grad(a),
            std::array::from_fn(|i| point[i] - f64::from(a[i] as i32) + (1.0 / 6.0)),
            0.6,
        );
        let third = Self::corner(
            grad(b),
            std::array::from_fn(|i| point[i] - f64::from(b[i] as i32) + (1.0 / 3.0)),
            0.6,
        );
        let fourth = Self::corner(grad([1; 3]), point.map(|v| v - 1.0 + 0.5), 0.6);
        32.0 * (first + second + third + fourth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::reference::{ref_simplex_get2_f64, ref_simplex_get3_f64};
    use crate::random::LegacyRandom;

    /// Compares `SimplexNoise::get2_f64` and `get3_f64` (and their `f32` wrappers)
    /// bit-for-bit against `main`'s pre-PR `permute` + `% 12` reference over several
    /// seeds and coordinate grids crossing the 256 permutation wrap up to index 511.
    #[test]
    fn simplex_fast_tables_match_reference_across_256_wrap() {
        let root3 = 3.0_f64.sqrt();
        let skew2 = 0.5 * (root3 - 1.0);
        let unskew2 = (3.0 - root3) / 6.0;
        let grid_2d = [
            -257.25_f64,
            -256.5,
            -255.75,
            -255.0,
            -2.25,
            -1.5,
            -0.5,
            0.0,
            0.5,
            1.5,
            254.25,
            255.0,
            255.5,
            256.25,
            257.0,
            510.5,
            511.5,
            512.5,
        ];
        let grid_3d = [
            -256.5_f64, -255.25, -1.25, 0.0, 254.75, 255.5, 256.25, 511.5,
        ];

        for seed in [0_i64, -1, 1234, i64::MIN] {
            for skip_offsets in [false, true] {
                let noise =
                    SimplexNoise::with_skipped_offsets(&mut LegacyRandom::new(seed), skip_offsets);
                let [ox, oy, oz] = *noise.table().offsets();

                for &x in &grid_2d {
                    for &y in &grid_2d {
                        let expected = ref_simplex_get2_f64(noise.table(), x, y);
                        assert_eq!(
                            noise.get2_f64(x, y).to_bits(),
                            expected.to_bits(),
                            "get2_f64 grid seed={seed} skip={skip_offsets} ({x:?}, {y:?})"
                        );
                        assert_eq!(
                            noise.get2(x, y).to_bits(),
                            (expected as f32).to_bits(),
                            "get2 grid seed={seed} skip={skip_offsets} ({x:?}, {y:?})"
                        );
                    }
                }

                for &x in &grid_3d {
                    for &y in &grid_3d {
                        for &z in &grid_3d {
                            let expected = ref_simplex_get3_f64(noise.table(), x, y, z);
                            assert_eq!(
                                noise.get3_f64(x, y, z).to_bits(),
                                expected.to_bits(),
                                "get3_f64 grid seed={seed} skip={skip_offsets} ({x:?}, {y:?}, {z:?})"
                            );
                            assert_eq!(
                                noise.get3(x, y, z).to_bits(),
                                (expected as f32).to_bits(),
                                "get3 grid seed={seed} skip={skip_offsets} ({x:?}, {y:?}, {z:?})"
                            );
                        }
                    }
                }

                // Drive 2D skewed cells directly onto `i & 255 == 255` across all `j in 0..256`
                // so `i + 1 + perm[j + 1]` hits every wrap index up to 511.
                for ti in [-1_i32, 255, 256] {
                    for tj in 0..256_i32 {
                        for (fu, fv) in [(0.35_f64, 0.75_f64), (0.75, 0.35)] {
                            let u = f64::from(ti) + fu;
                            let v = f64::from(tj) + fv;
                            let unskew = (u + v) * unskew2;
                            let x = (u - unskew) - ox;
                            let y = (v - unskew) - oy;
                            let sx = x + ox;
                            let sy = y + oy;
                            let shift = (sx + sy) * skew2;
                            assert_eq!(floor_i32(sx + shift), ti);
                            assert_eq!(floor_i32(sy + shift), tj);
                            let expected = ref_simplex_get2_f64(noise.table(), x, y);
                            assert_eq!(
                                noise.get2_f64(x, y).to_bits(),
                                expected.to_bits(),
                                "get2_f64 wrap seed={seed} skip={skip_offsets} cell=({ti}, {tj})"
                            );
                        }
                    }
                }

                // Drive 3D skewed cells directly onto `c0 & 255 == 255` and `c1 & 255 == 255`
                // across all `c2 in 0..256` to cover the upper half of `perm` and `perm_mod12`.
                for (c0, c1) in [(-1_i32, -1_i32), (255, 255), (255, 256)] {
                    for c2 in 0..256_i32 {
                        let u = f64::from(c0) + 0.25;
                        let v = f64::from(c1) + 0.5;
                        let w = f64::from(c2) + 0.75;
                        let unskew = (u + v + w) * (1.0 / 6.0);
                        let x = (u - unskew) - ox;
                        let y = (v - unskew) - oy;
                        let z = (w - unskew) - oz;
                        let sx = x + ox;
                        let sy = y + oy;
                        let sz = z + oz;
                        let skew = (sx + sy + sz) * (1.0 / 3.0);
                        assert_eq!(floor_i32(sx + skew), c0);
                        assert_eq!(floor_i32(sy + skew), c1);
                        assert_eq!(floor_i32(sz + skew), c2);
                        let expected = ref_simplex_get3_f64(noise.table(), x, y, z);
                        assert_eq!(
                            noise.get3_f64(x, y, z).to_bits(),
                            expected.to_bits(),
                            "get3_f64 wrap seed={seed} skip={skip_offsets} cell=({c0}, {c1}, {c2})"
                        );
                    }
                }
            }
        }
    }
}
