//! Base noise samplers for the pinned 26.3 target.
//!
//! Coordinates use double precision; Perlin interpolation and its derivatives
//! use single precision. Keep operation order: algebraic rearrangement changes
//! world generation. Complete density graphs and terrain generation remain absent.
mod normal;
mod perlin;
mod simplex;
mod stack;
mod volume;

pub use normal::{
    NoiseBuildError, NormalNoise, NormalNoiseBuilder, Normalization, OctaveInfo, create_legacy_fbm,
};
pub use perlin::{PerlinNoise, SmearedPerlinNoise};
pub use simplex::SimplexNoise;
pub use stack::{Noise, NoiseLayer, NoiseStack, NoiseStackBuilder};
pub use volume::{DensityError, DensityVolume};

use crate::random::RandomSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NanRange;
impl std::fmt::Display for NanRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("noise interval bounds cannot include NaN")
    }
}
impl std::error::Error for NanRange {}

/// The immutable offsets and permutation table shared by gradient samplers.
#[derive(Clone, Debug)]
pub struct GradientTable {
    offsets: [f64; 3],
    permutations: [u8; 256],
}

impl GradientTable {
    pub fn new(random: &mut impl RandomSource, offset_scale: f64) -> Self {
        // Zero scale still consumes all three draws.
        let offsets = std::array::from_fn(|_| random.next_double() * offset_scale);
        let mut permutations = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            let other = i + random.next_int_bound((256 - i) as i32).unwrap() as usize;
            permutations.swap(i, other);
        }
        Self {
            offsets,
            permutations,
        }
    }

    pub fn offsets(&self) -> &[f64; 3] {
        &self.offsets
    }
    pub fn permutations(&self) -> &[u8; 256] {
        &self.permutations
    }
}

// Perlin's 16-direction table includes four repeated directions. Simplex uses
// the first twelve directions, selecting them with remainder instead of masking.
const GRADIENTS: [[i32; 3]; 16] = [
    [1, 1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [-1, -1, 0],
    [1, 0, 1],
    [-1, 0, 1],
    [1, 0, -1],
    [-1, 0, -1],
    [0, 1, 1],
    [0, -1, 1],
    [0, 1, -1],
    [0, -1, -1],
    [1, 1, 0],
    [0, -1, 1],
    [-1, 1, 0],
    [0, -1, -1],
];

/// [`GRADIENTS`] as floats, so a dot product needs no conversions: every
/// component is -1, 0 or 1, which a float holds exactly.
const PERLIN_GRADIENTS: [[f32; 3]; 16] = {
    let mut out = [[0.0; 3]; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = [
            GRADIENTS[i][0] as f32,
            GRADIENTS[i][1] as f32,
            GRADIENTS[i][2] as f32,
        ];
        i += 1;
    }
    out
};

#[inline(always)]
fn dot(gradient: [f32; 3], point: [f32; 3]) -> f32 {
    gradient[0] * point[0] + gradient[1] * point[1] + gradient[2] * point[2]
}

/// Vanilla's periodic coordinate reduction with the normal Far Lands flag off.
/// The fast-path boundary is the double immediately below 2^24.
#[inline]
pub fn wrap(value: f64) -> f64 {
    let half = f64::from_bits(16_777_216.0_f64.to_bits() - 1);
    if value >= -half && value < half {
        value
    } else {
        value - (value / 33_554_432.0 + 0.5).floor() * 33_554_432.0
    }
}

#[inline(always)]
fn smoothstep(value: f32) -> f32 {
    value * value * value * (value * (value * 6.0 - 15.0) + 10.0)
}
#[inline(always)]
fn smoothstep_derivative(value: f32) -> f32 {
    30.0 * value * value * (value - 1.0) * (value - 1.0)
}
#[inline(always)]
fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}
#[inline(always)]
fn lerp2(a: f32, b: f32, values: [f32; 4]) -> f32 {
    lerp(
        b,
        lerp(a, values[0], values[1]),
        lerp(a, values[2], values[3]),
    )
}
#[inline(always)]
fn lerp3(fade: [f32; 3], values: [f32; 8]) -> f32 {
    lerp(
        fade[2],
        lerp2(
            fade[0],
            fade[1],
            [values[0], values[1], values[2], values[3]],
        ),
        lerp2(
            fade[0],
            fade[1],
            [values[4], values[5], values[6], values[7]],
        ),
    )
}

#[cfg(test)]
mod reference {
    //! Verbatim reference implementations copied from `main` before the fast-path
    //! optimizations, used solely to prove bit-exact equivalence in tests.
    use super::{
        DensityError, DensityVolume, GRADIENTS, GradientTable, PERLIN_GRADIENTS, dot, smoothstep,
        wrap,
    };
    use crate::geometry::floor_i32;

    fn ref_permute(table: &GradientTable, index: i32) -> i32 {
        i32::from(table.permutations()[(index & 255) as usize])
    }

    fn ref_lerp(delta: f32, start: f32, end: f32) -> f32 {
        start + delta * (end - start)
    }

    fn ref_lerp2(a: f32, b: f32, values: [f32; 4]) -> f32 {
        ref_lerp(
            b,
            ref_lerp(a, values[0], values[1]),
            ref_lerp(a, values[2], values[3]),
        )
    }

    fn ref_lerp3(fade: [f32; 3], values: [f32; 8]) -> f32 {
        ref_lerp(
            fade[2],
            ref_lerp2(fade[0], fade[1], values[..4].try_into().unwrap()),
            ref_lerp2(fade[0], fade[1], values[4..].try_into().unwrap()),
        )
    }

    fn ref_gradient_indices_at(
        table: &GradientTable,
        p: i32,
        q: i32,
        cell_y: i32,
        cell_z: i32,
    ) -> [usize; 8] {
        let lower = ref_permute(table, p.wrapping_add(cell_y));
        let upper = ref_permute(table, p.wrapping_add(cell_y).wrapping_add(1));
        let lower_x = ref_permute(table, q.wrapping_add(cell_y));
        let upper_x = ref_permute(table, q.wrapping_add(cell_y).wrapping_add(1));
        let rows = [
            lower, lower_x, upper, upper_x, lower, lower_x, upper, upper_x,
        ];
        std::array::from_fn(|i| {
            (ref_permute(
                table,
                rows[i].wrapping_add(cell_z).wrapping_add((i / 4) as i32),
            ) & 15) as usize
        })
    }

    fn ref_gradients_at(
        table: &GradientTable,
        p: i32,
        q: i32,
        cell_y: i32,
        cell_z: i32,
    ) -> [[f32; 3]; 8] {
        let mut gradients = [[0.0; 3]; 8];
        for (gradient, index) in gradients
            .iter_mut()
            .zip(ref_gradient_indices_at(table, p, q, cell_y, cell_z))
        {
            *gradient = PERLIN_GRADIENTS[index];
        }
        gradients
    }

    fn ref_gradients(table: &GradientTable, cell: [i32; 3]) -> [[f32; 3]; 8] {
        ref_gradients_at(
            table,
            ref_permute(table, cell[0]),
            ref_permute(table, cell[0].wrapping_add(1)),
            cell[1],
            cell[2],
        )
    }

    fn ref_corner_values(gradients: [[f32; 3]; 8], point: [f32; 3]) -> [f32; 8] {
        std::array::from_fn(|i| {
            dot(
                gradients[i],
                [
                    point[0] - (i & 1) as f32,
                    point[1] - ((i >> 1) & 1) as f32,
                    point[2] - ((i >> 2) & 1) as f32,
                ],
            )
        })
    }

    fn ref_perlin_coordinates(table: &GradientTable, point: [f64; 3]) -> ([i32; 3], [f64; 3]) {
        let shifted: [f64; 3] = std::array::from_fn(|i| wrap(point[i]) + table.offsets()[i]);
        let cell = shifted.map(floor_i32);
        (
            cell,
            std::array::from_fn(|i| shifted[i] - f64::from(cell[i])),
        )
    }

    fn ref_perlin_sample(
        table: &GradientTable,
        cell: [i32; 3],
        point: [f32; 3],
        interpolation_y: f32,
    ) -> f32 {
        let values = ref_corner_values(ref_gradients(table, cell), point);
        ref_lerp3(
            [
                smoothstep(point[0]),
                smoothstep(interpolation_y),
                smoothstep(point[2]),
            ],
            values,
        )
    }

    pub(super) fn ref_perlin_get3(table: &GradientTable, x: f64, y: f64, z: f64) -> f32 {
        let (cell, relative) = ref_perlin_coordinates(table, [x, y, z]);
        let point = relative.map(|v| v as f32);
        ref_perlin_sample(table, cell, point, point[1])
    }

    pub(super) fn ref_perlin_get2(table: &GradientTable, x: f64, z: f64) -> f32 {
        ref_perlin_get3(table, wrap(x), 0.0, wrap(z))
    }

    fn ref_compute_fudge_y(input_y: f64, relative_y: f64, scale: f64) -> f64 {
        let limited_y = if input_y >= 0.0 && input_y < relative_y {
            input_y
        } else {
            relative_y
        };
        f64::from(floor_i32(limited_y / scale + f64::from(1e-7_f32))) * scale
    }

    pub(super) fn ref_smeared_get3(
        table: &GradientTable,
        fudge_y_scale: f64,
        x: f64,
        y: f64,
        z: f64,
    ) -> f32 {
        let (cell, relative) = ref_perlin_coordinates(table, [x, y, z]);
        let fudge = ref_compute_fudge_y(y, relative[1], fudge_y_scale);
        ref_perlin_sample(
            table,
            cell,
            [
                relative[0] as f32,
                (relative[1] - fudge) as f32,
                relative[2] as f32,
            ],
            relative[1] as f32,
        )
    }

    pub(super) fn ref_smeared_get2(
        table: &GradientTable,
        fudge_y_scale: f64,
        x: f64,
        z: f64,
    ) -> f32 {
        ref_smeared_get3(table, fudge_y_scale, wrap(x), 0.0, wrap(z))
    }

    fn ref_simplex_corner(gradient: usize, point: [f64; 3], radius: f64) -> f64 {
        let attenuation = radius - point[0] * point[0] - point[1] * point[1] - point[2] * point[2];
        if attenuation < 0.0 {
            0.0
        } else {
            let squared = attenuation * attenuation;
            let g = GRADIENTS[gradient];
            squared
                * squared
                * (f64::from(g[0]) * point[0]
                    + f64::from(g[1]) * point[1]
                    + f64::from(g[2]) * point[2])
        }
    }

    pub(super) fn ref_simplex_get2_f64(table: &GradientTable, x: f64, y: f64) -> f64 {
        let x = x + table.offsets()[0];
        let y = y + table.offsets()[1];
        let root3 = 3.0_f64.sqrt();
        let skew = 0.5 * (root3 - 1.0);
        let unskew = (3.0 - root3) / 6.0;
        let shift = (x + y) * skew;
        let i = floor_i32(x + shift);
        let j = floor_i32(y + shift);
        let shift = f64::from(i.wrapping_add(j)) * unskew;
        let dx = x - (f64::from(i) - shift);
        let dy = y - (f64::from(j) - shift);
        let (a, b) = if dx > dy { (1, 0) } else { (0, 1) };
        let i = i & 255;
        let j = j & 255;
        let grad = |a, b| (ref_permute(table, i + a + ref_permute(table, j + b)) % 12) as usize;
        let first = ref_simplex_corner(grad(0, 0), [dx, dy, 0.0], 0.5);
        let second = ref_simplex_corner(
            grad(a, b),
            [dx - f64::from(a) + unskew, dy - f64::from(b) + unskew, 0.0],
            0.5,
        );
        let third = ref_simplex_corner(
            grad(1, 1),
            [dx - 1.0 + 2.0 * unskew, dy - 1.0 + 2.0 * unskew, 0.0],
            0.5,
        );
        70.0 * (first + second + third)
    }

    pub(super) fn ref_simplex_get2(table: &GradientTable, x: f64, y: f64) -> f32 {
        ref_simplex_get2_f64(table, x, y) as f32
    }

    pub(super) fn ref_simplex_get3_f64(table: &GradientTable, x: f64, y: f64, z: f64) -> f64 {
        let shifted = [
            x + table.offsets()[0],
            y + table.offsets()[1],
            z + table.offsets()[2],
        ];
        let skew = (shifted[0] + shifted[1] + shifted[2]) * (1.0 / 3.0);
        let cell = shifted.map(|v| floor_i32(v + skew));
        let unskew = f64::from(cell[0].wrapping_add(cell[1]).wrapping_add(cell[2])) * (1.0 / 6.0);
        let point: [f64; 3] = std::array::from_fn(|i| shifted[i] - (f64::from(cell[i]) - unskew));
        let [x, y, z] = point;
        let (a, b) = if x >= y {
            if y >= z {
                ([1, 0, 0], [1, 1, 0])
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
        let cell = cell.map(|v| v & 255);
        let grad = |delta: [i32; 3]| {
            (ref_permute(
                table,
                cell[0]
                    + delta[0]
                    + ref_permute(
                        table,
                        cell[1] + delta[1] + ref_permute(table, cell[2] + delta[2]),
                    ),
            ) % 12) as usize
        };
        let first = ref_simplex_corner(grad([0; 3]), point, 0.6);
        let second = ref_simplex_corner(
            grad(a),
            std::array::from_fn(|i| point[i] - f64::from(a[i]) + (1.0 / 6.0)),
            0.6,
        );
        let third = ref_simplex_corner(
            grad(b),
            std::array::from_fn(|i| point[i] - f64::from(b[i]) + (1.0 / 3.0)),
            0.6,
        );
        let fourth = ref_simplex_corner(grad([1; 3]), point.map(|v| v - 1.0 + 0.5), 0.6);
        32.0 * (first + second + third + fourth)
    }

    pub(super) fn ref_simplex_get3(table: &GradientTable, x: f64, y: f64, z: f64) -> f32 {
        ref_simplex_get3_f64(table, x, y, z) as f32
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ref_add_to_volume(
        table: &GradientTable,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        fudge_y_scale: Option<f64>,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [size_x, size_y, size_z] = volume.sizes();
        let mut cells = vec![0_i32; size_y];
        let mut fades = vec![0.0_f32; size_y];
        let mut offsets = vec![0.0_f32; size_y];
        for y in 0..size_y {
            let input_y = f64::from(volume.coordinate(1, y)) * y_scale;
            let shifted_y = wrap(input_y) + table.offsets()[1];
            let cell = floor_i32(shifted_y);
            let dy = shifted_y - f64::from(cell);
            cells[y] = cell;
            fades[y] = smoothstep(dy as f32);
            offsets[y] = match fudge_y_scale {
                Some(scale) => (dy - ref_compute_fudge_y(input_y, dy, scale)) as f32,
                None => dy as f32,
            };
        }
        let mut index = 0;
        for z in 0..size_z {
            let shifted_z =
                wrap(f64::from(volume.coordinate(2, z)) * xz_scale) + table.offsets()[2];
            let cell_z = floor_i32(shifted_z);
            let dz = (shifted_z - f64::from(cell_z)) as f32;
            let fade_z = smoothstep(dz);
            for x in 0..size_x {
                let shifted_x =
                    wrap(f64::from(volume.coordinate(0, x)) * xz_scale) + table.offsets()[0];
                let cell_x = floor_i32(shifted_x);
                let dx = (shifted_x - f64::from(cell_x)) as f32;
                let fade_x = smoothstep(dx);
                let p = ref_permute(table, cell_x);
                let q = ref_permute(table, cell_x.wrapping_add(1));
                let mut previous_y = i32::MIN;
                let mut xz_dots = [0.0_f32; 8];
                let mut y_gradients = [0.0_f32; 8];
                let column = &mut buffer[index..index + size_y];
                for y in 0..size_y {
                    let cell = cells[y];
                    if previous_y != cell {
                        let gradients = ref_gradients_at(table, p, q, cell, cell_z);
                        for i in 0..8 {
                            xz_dots[i] = gradients[i][0] * (dx - (i & 1) as f32)
                                + gradients[i][2] * (dz - ((i >> 2) & 1) as f32);
                            y_gradients[i] = gradients[i][1];
                        }
                        previous_y = cell;
                    }
                    let dy = offsets[y];
                    let values = std::array::from_fn(|i| {
                        xz_dots[i] + y_gradients[i] * (dy - ((i >> 1) & 1) as f32)
                    });
                    column[y] += amplitude * ref_lerp3([fade_x, fades[y], fade_z], values);
                }
                index += size_y;
            }
        }
        Ok(())
    }

    pub(super) fn ref_simplex_add_to_volume(
        table: &GradientTable,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [sx, sy, sz] = volume.sizes();
        let mut index = 0;
        for z in 0..sz {
            let z = f64::from(volume.coordinate(2, z)) * xz_scale;
            for x in 0..sx {
                let x = f64::from(volume.coordinate(0, x)) * xz_scale;
                for y in 0..sy {
                    let y = f64::from(volume.coordinate(1, y)) * y_scale;
                    buffer[index] += amplitude * ref_simplex_get3(table, x, y, z);
                    index += 1;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        geometry::Axis,
        random::{LegacyRandom, XoroshiroRandom},
    };

    #[test]
    fn vanilla_seed_zero_samples_and_derivatives() {
        // Values captured by the original adapter from the pinned server JAR.
        let mut random = LegacyRandom::new(0);
        let noise = PerlinNoise::new(&mut random);
        assert_eq!(random.next_long(), -6_819_959_017_335_327_829);
        let mut derivative = [0.0; 3];
        assert_eq!(
            noise
                .noise_with_derivative(0.0, 0.0, 0.0, &mut derivative)
                .to_bits(),
            3_183_733_582
        );
        assert_eq!(
            derivative.map(f32::to_bits),
            [3_189_117_465, 3_214_556_526, 1_017_326_887]
        );
        let mut random = LegacyRandom::new(0);
        let simplex = SimplexNoise::with_skipped_offsets(&mut random, true);
        assert_eq!(simplex.table().offsets(), &[0.0; 3]);
        assert_eq!(simplex.table().permutations(), noise.table().permutations());
        assert_eq!(random.next_long(), -6_819_959_017_335_327_829);
        assert_eq!(simplex.get2(0.25, 0.75).to_bits(), 3_146_180_247);
        assert_eq!(simplex.get3(0.25, 0.5, 0.75).to_bits(), 1_062_011_824);
        assert!(simplex.get3(f64::NAN, 0.0, 0.0).is_nan());
        assert_eq!(
            SmearedPerlinNoise::new(&mut random, f64::NAN).range(),
            Err(NanRange)
        );
    }

    #[test]
    fn bulk_accumulation_preserves_tail_and_rejects_short_buffers_before_writes() {
        let noise = PerlinNoise::new(&mut LegacyRandom::new(0));
        let volume = DensityVolume::new([1; 3], [0; 3], [1; 3]).unwrap();
        let mut random = LegacyRandom::new(0);
        let mut values =
            std::array::from_fn::<_, 5, _>(|_| f32::from_bits(random.next_int() as u32));
        let tail = values[1..].iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        noise
            .add_to_volume(&mut values, volume, 1.0, 1.0, 1.0)
            .unwrap();
        assert_eq!(values[0].to_bits(), 3_184_062_705);
        assert_eq!(
            values[1..].iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            tail
        );
        let before = values.map(f32::to_bits);
        let too_large = DensityVolume::new([2; 3], [0; 3], [1; 3]).unwrap();
        assert_eq!(
            noise.add_to_volume(&mut values, too_large, 1.0, 1.0, 1.0),
            Err(DensityError::BufferTooShort)
        );
        assert_eq!(values.map(f32::to_bits), before);
    }

    #[test]
    fn density_dimensions_are_bounded_and_coordinates_wrap() {
        assert_eq!(
            DensityVolume::new([0, 1, 1], [0; 3], [1; 3]),
            Err(DensityError::NonpositiveDimensions)
        );
        assert_eq!(
            DensityVolume::new([1; 3], [0; 3], [1, 0, 1]),
            Err(DensityError::NonpositiveSteps)
        );
        assert_eq!(
            DensityVolume::new([i32::MAX; 3], [0; 3], [1; 3]),
            Err(DensityError::VolumeTooLarge)
        );
        let volume = DensityVolume::new([2, 3, 4], [i32::MAX, 0, -1], [2, 4, 8]).unwrap();
        assert_eq!(volume.block(Axis::X, 1), Some(i32::MIN + 1));
        assert_eq!(volume.block(Axis::X, 2), None);
        assert_eq!(volume.index([1, 2, 3]), Some(23));
        assert_eq!(volume.index([1, 3, 3]), None);
    }

    #[test]
    fn normal_noise_reference_values_and_legacy_failure_draws() {
        let mut builder = NormalNoise::builder();
        builder.set_base_octave(-3);
        assert_eq!(
            builder.set_amplitude_modifier(-1, 0.0).unwrap_err(),
            NoiseBuildError::ModifierIndex
        );
        builder.set_octave_count(3).unwrap();
        builder.set_amplitude_modifier(1, 0.0).unwrap();
        assert_eq!(
            builder.set_octave_count(4).unwrap_err(),
            NoiseBuildError::CountAfterModifiers
        );
        let definition = builder.build().unwrap();
        assert_eq!(
            definition.normalization_factor().to_bits(),
            4_607_442_513_526_237_848
        );
        assert_eq!(
            definition
                .octaves()
                .iter()
                .map(|o| o.octave_index)
                .collect::<Vec<_>>(),
            [-3, -1]
        );
        let mut random = LegacyRandom::new(0);
        let modern = definition.create(&mut random).unwrap();
        assert_eq!(modern.get3(0.0, 0.0, 0.0).to_bits(), 1_042_650_710);
        assert_eq!(random.next_long(), -6_688_467_811_848_818_630);
        let mut random = LegacyRandom::new(0);
        let legacy = definition
            .create_for_legacy_nether_biome(&mut random)
            .unwrap();
        assert_eq!(legacy.get3(0.0, 0.0, 0.0).to_bits(), 3_199_632_936);
        assert_eq!(random.next_long(), 4_966_550_093_728_345_520);
        let mut empty = NormalNoise::builder();
        empty.set_base_octave(2).set_octave_count(0).unwrap();
        let mut random = LegacyRandom::new(0);
        assert_eq!(
            empty
                .build()
                .unwrap()
                .create_for_legacy_nether_biome(&mut random)
                .unwrap_err(),
            NoiseBuildError::PositiveLegacyOctaves
        );
        assert_eq!(random.next_long(), -6_819_959_017_335_327_829);
        let mut random = LegacyRandom::new(0);
        assert_eq!(
            create_legacy_fbm(&mut random, i32::MIN, &[1.0]).unwrap_err(),
            NoiseBuildError::LegacyWorkLimit
        );
        assert_eq!(random.next_long(), LegacyRandom::new(0).next_long());
    }

    #[test]
    fn stack_snapshots_share_layers_and_preserve_zero_amplitude_semantics() {
        use std::sync::Arc;
        let sampler: Arc<dyn Noise> = Arc::new(PerlinNoise::new(&mut LegacyRandom::new(0)));
        let mut builder = NoiseStack::builder();
        builder.add(Arc::clone(&sampler), 1.0, 0.0);
        let zero = builder.build().unwrap();
        builder.add(Arc::clone(&sampler), 2.0, 1.0);
        assert_eq!(zero.layers().len(), 1);
        assert!(Arc::ptr_eq(zero.get_layer(0).unwrap(), &sampler));
        assert!(zero.get_layer(1).is_none());
        let mut flattened = NoiseStack::builder();
        flattened.add_stack(&zero, 1.0, f32::INFINITY);
        assert_eq!(flattened.build().unwrap_err(), NanRange);
        let nested = NoiseStack::builder()
            .add(Arc::new(zero), 1.0, f32::INFINITY)
            .build()
            .unwrap();
        assert_eq!(nested.range().unwrap(), [0.0; 2]);
        assert!(nested.get3(1.0, 2.0, 3.0).is_nan());
    }

    struct FixedOffsetRandom {
        double_draw: f64,
        inner: LegacyRandom,
    }

    struct FixedOffsetPositional;

    impl crate::random::PositionalRandom for FixedOffsetPositional {
        type Source = FixedOffsetRandom;
        fn at(&self, _position: crate::position::BlockPos) -> Self::Source {
            unreachable!()
        }
        fn from_seed(&self, _seed: i64) -> Self::Source {
            unreachable!()
        }
        fn from_hash_of(&self, _name: &str) -> Self::Source {
            unreachable!()
        }
        fn from_hash_of_utf16(&self, _name: &[u16]) -> Self::Source {
            unreachable!()
        }
    }

    impl RandomSource for FixedOffsetRandom {
        type Positional = FixedOffsetPositional;
        fn set_seed(&mut self, seed: i64) {
            self.inner.set_seed(seed);
        }
        fn next_int(&mut self) -> i32 {
            self.inner.next_int()
        }
        fn next_int_bound(&mut self, bound: i32) -> Result<i32, crate::random::InvalidBound> {
            self.inner.next_int_bound(bound)
        }
        fn next_long(&mut self) -> i64 {
            self.inner.next_long()
        }
        fn next_boolean(&mut self) -> bool {
            self.inner.next_boolean()
        }
        fn next_float(&mut self) -> f32 {
            self.inner.next_float()
        }
        fn next_double(&mut self) -> f64 {
            self.double_draw
        }
        fn next_gaussian(&mut self) -> f64 {
            self.inner.next_gaussian()
        }
        fn fork_positional(&mut self) -> Self::Positional {
            FixedOffsetPositional
        }
        fn fork(&mut self) -> Self {
            Self {
                double_draw: self.double_draw,
                inner: self.inner.fork(),
            }
        }
    }

    /// Proves every scalar and stack fast path is bit-for-bit identical to `main`'s
    /// verbatim reference implementations over 10^5 random inputs plus edge cases
    /// (`±0`, `NaN`, `±inf`, `±2^24`, and cells near `i32::MAX`/`i32::MIN`).
    #[test]
    fn bit_exact_against_main_reference_across_seeds_random_and_edges() {
        fn assert_f32_exact(got: f32, expected: f32, ctx: &str) {
            assert!(
                (got.is_nan() && expected.is_nan()) || got.to_bits() == expected.to_bits(),
                "{ctx}: got {got:?} ({:#010x}) expected {expected:?} ({:#010x})",
                got.to_bits(),
                expected.to_bits()
            );
        }
        fn assert_f64_exact(got: f64, expected: f64, ctx: &str) {
            assert!(
                (got.is_nan() && expected.is_nan()) || got.to_bits() == expected.to_bits(),
                "{ctx}: got {got:?} ({:#018x}) expected {expected:?} ({:#018x})",
                got.to_bits(),
                expected.to_bits()
            );
        }
        use std::sync::Arc;

        const SEEDS: [i64; 4] = [0, -1, 1234, i64::MIN];
        const RANDOM_PER_SEED: usize = 25_000;

        let half = f64::from_bits(16_777_216.0_f64.to_bits() - 1);
        let edges = [
            0.0_f64,
            -0.0,
            f64::from_bits(1),
            f64::from_bits((1_u64 << 63) | 1),
            -1.0,
            1.0,
            -256.0,
            256.0,
            -half,
            half,
            -16_777_216.0,
            16_777_216.0,
            f64::from_bits(16_777_216.0_f64.to_bits() + 1),
            -33_554_432.0,
            33_554_432.0,
            f64::from(i32::MAX) - 1.5,
            f64::from(i32::MAX),
            f64::from(i32::MAX) + 1.5,
            f64::from(i32::MIN) - 1.5,
            f64::from(i32::MIN),
            f64::from(i32::MIN) + 1.5,
            1e20,
            -1e20,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];
        let y_edges = [
            0.0_f64,
            -0.0,
            -0.5,
            0.5,
            -16_777_216.0,
            16_777_216.0,
            f64::from(i32::MAX),
            f64::from(i32::MIN),
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
        ];

        for seed in SEEDS {
            let perlin = PerlinNoise::new(&mut LegacyRandom::new(seed));
            let perlin2 = PerlinNoise::new(&mut XoroshiroRandom::new(seed));
            let smeared_scale = 0.125_f64;
            let smeared = SmearedPerlinNoise::new(&mut LegacyRandom::new(seed), smeared_scale);
            let smeared2_scale = 0.5_f64;
            let smeared2 = SmearedPerlinNoise::new(&mut XoroshiroRandom::new(seed), smeared2_scale);
            let simplex = SimplexNoise::new(&mut LegacyRandom::new(seed));
            let simplex_skip =
                SimplexNoise::with_skipped_offsets(&mut XoroshiroRandom::new(seed), true);

            let mut pb = NoiseStack::builder();
            pb.add(Arc::new(perlin.clone()), 1.0, 0.5)
                .add(Arc::new(perlin2.clone()), -0.25, -0.75);
            let perlin_stack = pb.build().unwrap();

            let mut sb = NoiseStack::builder();
            sb.add(Arc::new(smeared.clone()), 0.5, 1.0)
                .add(Arc::new(smeared2.clone()), -1.5, 0.25);
            let smeared_stack = sb.build().unwrap();

            let mut xb = NoiseStack::builder();
            xb.add(Arc::new(simplex.clone()), 0.75, 0.5).add(
                Arc::new(simplex_skip.clone()),
                -2.0,
                -0.25,
            );
            let simplex_stack = xb.build().unwrap();

            let mut mb = NoiseStack::builder();
            mb.add(Arc::new(perlin.clone()), 1.0, 0.5)
                .add(Arc::new(smeared.clone()), 0.5, -0.25)
                .add(Arc::new(simplex.clone()), -0.75, 0.75);
            let mixed_stack = mb.build().unwrap();

            let check_point = |x: f64, y: f64, z: f64, tag: &str| {
                assert_f32_exact(
                    perlin.get2(x, z),
                    reference::ref_perlin_get2(perlin.table(), x, z),
                    &format!("perlin.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                assert_f32_exact(
                    perlin.get3(x, y, z),
                    reference::ref_perlin_get3(perlin.table(), x, y, z),
                    &format!("perlin.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );
                assert_f32_exact(
                    smeared.get2(x, z),
                    reference::ref_smeared_get2(smeared.table(), smeared_scale, x, z),
                    &format!("smeared.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                assert_f32_exact(
                    smeared.get3(x, y, z),
                    reference::ref_smeared_get3(smeared.table(), smeared_scale, x, y, z),
                    &format!("smeared.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );
                assert_f64_exact(
                    simplex.get2_f64(x, z),
                    reference::ref_simplex_get2_f64(simplex.table(), x, z),
                    &format!("simplex.get2_f64 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                assert_f32_exact(
                    simplex.get2(x, z),
                    reference::ref_simplex_get2(simplex.table(), x, z),
                    &format!("simplex.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                assert_f64_exact(
                    simplex.get3_f64(x, y, z),
                    reference::ref_simplex_get3_f64(simplex.table(), x, y, z),
                    &format!("simplex.get3_f64 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );
                assert_f32_exact(
                    simplex.get3(x, y, z),
                    reference::ref_simplex_get3(simplex.table(), x, y, z),
                    &format!("simplex.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );
                assert_f32_exact(
                    simplex_skip.get2(x, z),
                    reference::ref_simplex_get2(simplex_skip.table(), x, z),
                    &format!("simplex_skip.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                assert_f32_exact(
                    simplex_skip.get3(x, y, z),
                    reference::ref_simplex_get3(simplex_skip.table(), x, y, z),
                    &format!("simplex_skip.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );

                let expected_p2 = (0.0_f32
                    + 0.5_f32 * reference::ref_perlin_get2(perlin.table(), x * 1.0, z * 1.0))
                    + (-0.75_f32)
                        * reference::ref_perlin_get2(perlin2.table(), x * -0.25, z * -0.25);
                assert_f32_exact(
                    perlin_stack.get2(x, z),
                    expected_p2,
                    &format!("perlin_stack.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                let expected_p3 = (0.0_f32
                    + 0.5_f32
                        * reference::ref_perlin_get3(perlin.table(), x * 1.0, y * 1.0, z * 1.0))
                    + (-0.75_f32)
                        * reference::ref_perlin_get3(
                            perlin2.table(),
                            x * -0.25,
                            y * -0.25,
                            z * -0.25,
                        );
                assert_f32_exact(
                    perlin_stack.get3(x, y, z),
                    expected_p3,
                    &format!("perlin_stack.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );

                let expected_s2 = (0.0_f32
                    + 1.0_f32
                        * reference::ref_smeared_get2(
                            smeared.table(),
                            smeared_scale,
                            x * 0.5,
                            z * 0.5,
                        ))
                    + 0.25_f32
                        * reference::ref_smeared_get2(
                            smeared2.table(),
                            smeared2_scale,
                            x * -1.5,
                            z * -1.5,
                        );
                assert_f32_exact(
                    smeared_stack.get2(x, z),
                    expected_s2,
                    &format!("smeared_stack.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                let expected_s3 = (0.0_f32
                    + 1.0_f32
                        * reference::ref_smeared_get3(
                            smeared.table(),
                            smeared_scale,
                            x * 0.5,
                            y * 0.5,
                            z * 0.5,
                        ))
                    + 0.25_f32
                        * reference::ref_smeared_get3(
                            smeared2.table(),
                            smeared2_scale,
                            x * -1.5,
                            y * -1.5,
                            z * -1.5,
                        );
                assert_f32_exact(
                    smeared_stack.get3(x, y, z),
                    expected_s3,
                    &format!("smeared_stack.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );

                let expected_x2 = (0.0_f32
                    + 0.5_f32 * reference::ref_simplex_get2(simplex.table(), x * 0.75, z * 0.75))
                    + (-0.25_f32)
                        * reference::ref_simplex_get2(simplex_skip.table(), x * -2.0, z * -2.0);
                assert_f32_exact(
                    simplex_stack.get2(x, z),
                    expected_x2,
                    &format!("simplex_stack.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                let expected_x3 = (0.0_f32
                    + 0.5_f32
                        * reference::ref_simplex_get3(
                            simplex.table(),
                            x * 0.75,
                            y * 0.75,
                            z * 0.75,
                        ))
                    + (-0.25_f32)
                        * reference::ref_simplex_get3(
                            simplex_skip.table(),
                            x * -2.0,
                            y * -2.0,
                            z * -2.0,
                        );
                assert_f32_exact(
                    simplex_stack.get3(x, y, z),
                    expected_x3,
                    &format!("simplex_stack.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );

                let expected_m2 = ((0.0_f32
                    + 0.5_f32 * reference::ref_perlin_get2(perlin.table(), x * 1.0, z * 1.0))
                    + (-0.25_f32)
                        * reference::ref_smeared_get2(
                            smeared.table(),
                            smeared_scale,
                            x * 0.5,
                            z * 0.5,
                        ))
                    + 0.75_f32 * reference::ref_simplex_get2(simplex.table(), x * -0.75, z * -0.75);
                assert_f32_exact(
                    mixed_stack.get2(x, z),
                    expected_m2,
                    &format!("mixed_stack.get2 seed={seed} {tag} ({x:?}, {z:?})"),
                );
                let expected_m3 = ((0.0_f32
                    + 0.5_f32
                        * reference::ref_perlin_get3(perlin.table(), x * 1.0, y * 1.0, z * 1.0))
                    + (-0.25_f32)
                        * reference::ref_smeared_get3(
                            smeared.table(),
                            smeared_scale,
                            x * 0.5,
                            y * 0.5,
                            z * 0.5,
                        ))
                    + 0.75_f32
                        * reference::ref_simplex_get3(
                            simplex.table(),
                            x * -0.75,
                            y * -0.75,
                            z * -0.75,
                        );
                assert_f32_exact(
                    mixed_stack.get3(x, y, z),
                    expected_m3,
                    &format!("mixed_stack.get3 seed={seed} {tag} ({x:?}, {y:?}, {z:?})"),
                );
            };

            let mut rng = XoroshiroRandom::new(seed ^ 0x517c_c1b7_2722_0a95_i64);
            for i in 0..RANDOM_PER_SEED {
                let (x, y, z) = match i % 5 {
                    0 => (
                        (rng.next_double() - 0.5) * 20_000.0,
                        (rng.next_double() - 0.5) * 20_000.0,
                        (rng.next_double() - 0.5) * 20_000.0,
                    ),
                    1 => (
                        (rng.next_double() - 0.5) * 8.0,
                        (rng.next_double() - 0.5) * 8.0,
                        (rng.next_double() - 0.5) * 8.0,
                    ),
                    2 => {
                        let base = if rng.next_boolean() {
                            16_777_216.0
                        } else {
                            33_554_432.0
                        };
                        let sx = if rng.next_boolean() { base } else { -base };
                        let sy = if rng.next_boolean() { base } else { -base };
                        let sz = if rng.next_boolean() { base } else { -base };
                        (
                            sx + (rng.next_double() - 0.5) * 8.0,
                            sy + (rng.next_double() - 0.5) * 8.0,
                            sz + (rng.next_double() - 0.5) * 8.0,
                        )
                    }
                    3 => {
                        let base = |pos: bool| {
                            if pos {
                                f64::from(i32::MAX)
                            } else {
                                f64::from(i32::MIN)
                            }
                        };
                        (
                            base(rng.next_boolean()) + (rng.next_double() - 0.5) * 16.0,
                            base(rng.next_boolean()) + (rng.next_double() - 0.5) * 16.0,
                            base(rng.next_boolean()) + (rng.next_double() - 0.5) * 16.0,
                        )
                    }
                    _ => (
                        (rng.next_double() - 0.5) * 200_000.0,
                        if i.is_multiple_of(2) { 0.0 } else { -0.0 },
                        (rng.next_double() - 0.5) * 200_000.0,
                    ),
                };
                check_point(x, y, z, "random");
            }

            // Exhaustive edge-value grid (including ±0, NaN, ±inf, ±2^24, ±2^25, i32::MAX/MIN).
            for (xi, &x) in edges.iter().enumerate() {
                for (zi, &z) in edges.iter().enumerate() {
                    let y = y_edges[(xi + zi) % y_edges.len()];
                    check_point(x, y, z, "edge-grid");
                }
            }
            for &y in &y_edges {
                for &x in &[
                    0.0,
                    -0.0,
                    1.0,
                    -half,
                    16_777_216.0,
                    f64::from(i32::MAX),
                    f64::from(i32::MIN),
                    f64::INFINITY,
                    f64::NAN,
                ] {
                    check_point(x, y, -x, "y-edge");
                }
            }

            // Non-finite frequencies on a homogeneous Perlin stack at y = ±0 and non-zero y.
            for freq in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
                let mut nf = NoiseStack::builder();
                nf.add(Arc::new(perlin.clone()), freq, 1.0);
                let stack = nf.build().unwrap();
                for y in [0.0_f64, -0.0, 1.5, f64::NAN] {
                    let expected = 0.0_f32
                        + 1.0_f32
                            * reference::ref_perlin_get3(
                                perlin.table(),
                                1.0 * freq,
                                y * freq,
                                -freq,
                            );
                    assert_f32_exact(
                        stack.get3(1.0, y, -1.0),
                        expected,
                        &format!("nonfinite freq={freq:?} y={y:?} seed={seed}"),
                    );
                }
            }

            // NoiseStack add_to_volume / add_to_needed (including wide size_x > STACK_X heap path).
            let volumes = [
                (
                    DensityVolume::new([3, 1, 2], [-7, 13, 5], [2, 1, 3]).unwrap(),
                    0.25,
                    0.25,
                ),
                (
                    DensityVolume::new([3, 7, 2], [-7, -3, 5], [2, 1, 3]).unwrap(),
                    0.25,
                    0.0,
                ),
                (
                    DensityVolume::new([40, 17, 2], [-19, -64, 41], [4, 1, 8]).unwrap(),
                    0.125,
                    0.5,
                ),
                (
                    DensityVolume::new([4, 65, 2], [16_777_214, -32, -16_777_218], [1, 1, 1])
                        .unwrap(),
                    1.0,
                    0.25,
                ),
            ];
            for (volume, xz_scale, y_scale) in volumes {
                let [sx, sy, sz] = volume.sizes();
                let columns = sx * sz;
                let mut needed = vec![0_u64; columns];
                for col in 0..columns {
                    needed[col] = match col % 4 {
                        0 => 0,
                        1 => u64::MAX,
                        2 => 1_u64 << (col % sy.min(64)),
                        _ => rng.next_long() as u64,
                    };
                }
                let initial: Vec<f32> = (0..volume.size() + 3)
                    .map(|k| [0.0, -0.0, 1.5, f32::from_bits(1)][k % 4])
                    .collect();
                let amp = 0.75_f32;

                // Perlin stack
                let mut expected_p = initial.clone();
                reference::ref_add_to_volume(
                    perlin.table(),
                    &mut expected_p,
                    volume,
                    xz_scale * 1.0,
                    y_scale * 1.0,
                    amp * 0.5,
                    None,
                )
                .unwrap();
                reference::ref_add_to_volume(
                    perlin2.table(),
                    &mut expected_p,
                    volume,
                    xz_scale * -0.25,
                    y_scale * -0.25,
                    amp * -0.75,
                    None,
                )
                .unwrap();
                let mut actual_p = initial.clone();
                let mut actual_p_needed = initial.clone();
                perlin_stack
                    .add_to_volume(&mut actual_p, volume, xz_scale, y_scale, amp)
                    .unwrap();
                perlin_stack
                    .add_to_needed(
                        &mut actual_p_needed,
                        volume,
                        xz_scale,
                        y_scale,
                        amp,
                        &needed,
                    )
                    .unwrap();
                assert_eq!(
                    actual_p.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected_p.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "perlin_stack.add_to_volume seed={seed} vol={volume:?}",
                );
                for (idx, ((&exp, &got), &init)) in expected_p
                    .iter()
                    .zip(&actual_p_needed)
                    .zip(&initial)
                    .enumerate()
                {
                    if idx >= volume.size() {
                        assert_eq!(
                            got.to_bits(),
                            init.to_bits(),
                            "perlin_stack.add_to_needed tail seed={seed} vol={volume:?} idx={idx}"
                        );
                    } else if sy > 64 || (needed[idx / sy] >> (idx % sy)) & 1 == 1 {
                        assert_f32_exact(
                            got,
                            exp,
                            &format!(
                                "perlin_stack.add_to_needed seed={seed} vol={volume:?} idx={idx}"
                            ),
                        );
                    }
                }

                // Smeared stack
                let mut expected_s = initial.clone();
                reference::ref_add_to_volume(
                    smeared.table(),
                    &mut expected_s,
                    volume,
                    xz_scale * 0.5,
                    y_scale * 0.5,
                    amp * 1.0,
                    Some(smeared_scale),
                )
                .unwrap();
                reference::ref_add_to_volume(
                    smeared2.table(),
                    &mut expected_s,
                    volume,
                    xz_scale * -1.5,
                    y_scale * -1.5,
                    amp * 0.25,
                    Some(smeared2_scale),
                )
                .unwrap();
                let mut actual_s = initial.clone();
                let mut actual_s_needed = initial.clone();
                smeared_stack
                    .add_to_volume(&mut actual_s, volume, xz_scale, y_scale, amp)
                    .unwrap();
                smeared_stack
                    .add_to_needed(
                        &mut actual_s_needed,
                        volume,
                        xz_scale,
                        y_scale,
                        amp,
                        &needed,
                    )
                    .unwrap();
                assert_eq!(
                    actual_s.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected_s.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "smeared_stack.add_to_volume seed={seed} vol={volume:?}",
                );
                for (idx, ((&exp, &got), &init)) in expected_s
                    .iter()
                    .zip(&actual_s_needed)
                    .zip(&initial)
                    .enumerate()
                {
                    if idx >= volume.size() {
                        assert_eq!(
                            got.to_bits(),
                            init.to_bits(),
                            "smeared_stack.add_to_needed tail seed={seed} vol={volume:?} idx={idx}"
                        );
                    } else if sy > 64 || (needed[idx / sy] >> (idx % sy)) & 1 == 1 {
                        assert_f32_exact(
                            got,
                            exp,
                            &format!(
                                "smeared_stack.add_to_needed seed={seed} vol={volume:?} idx={idx}"
                            ),
                        );
                    }
                }

                // Simplex stack
                let mut expected_x = initial.clone();
                reference::ref_simplex_add_to_volume(
                    simplex.table(),
                    &mut expected_x,
                    volume,
                    xz_scale * 0.75,
                    y_scale * 0.75,
                    amp * 0.5,
                )
                .unwrap();
                reference::ref_simplex_add_to_volume(
                    simplex_skip.table(),
                    &mut expected_x,
                    volume,
                    xz_scale * -2.0,
                    y_scale * -2.0,
                    amp * -0.25,
                )
                .unwrap();
                let mut actual_x = initial.clone();
                let mut actual_x_needed = initial.clone();
                simplex_stack
                    .add_to_volume(&mut actual_x, volume, xz_scale, y_scale, amp)
                    .unwrap();
                simplex_stack
                    .add_to_needed(
                        &mut actual_x_needed,
                        volume,
                        xz_scale,
                        y_scale,
                        amp,
                        &needed,
                    )
                    .unwrap();
                assert_eq!(
                    actual_x.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected_x.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "simplex_stack.add_to_volume seed={seed} vol={volume:?}",
                );
                assert_eq!(
                    actual_x_needed
                        .iter()
                        .map(|v| v.to_bits())
                        .collect::<Vec<_>>(),
                    expected_x.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "simplex_stack.add_to_needed seed={seed} vol={volume:?}"
                );

                // Mixed (Dynamic) stack
                let mut expected_m = initial.clone();
                reference::ref_add_to_volume(
                    perlin.table(),
                    &mut expected_m,
                    volume,
                    xz_scale * 1.0,
                    y_scale * 1.0,
                    amp * 0.5,
                    None,
                )
                .unwrap();
                reference::ref_add_to_volume(
                    smeared.table(),
                    &mut expected_m,
                    volume,
                    xz_scale * 0.5,
                    y_scale * 0.5,
                    amp * -0.25,
                    Some(smeared_scale),
                )
                .unwrap();
                reference::ref_simplex_add_to_volume(
                    simplex.table(),
                    &mut expected_m,
                    volume,
                    xz_scale * -0.75,
                    y_scale * -0.75,
                    amp * 0.75,
                )
                .unwrap();
                let mut actual_m = initial.clone();
                mixed_stack
                    .add_to_volume(&mut actual_m, volume, xz_scale, y_scale, amp)
                    .unwrap();
                assert_eq!(
                    actual_m.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected_m.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "mixed_stack.add_to_volume seed={seed} vol={volume:?}"
                );
            }
        }

        // Explicitly drive PerlinNoise and SmearedPerlinNoise cells onto i32::MAX / i32::MIN
        // (including i32::MAX -> i32::MIN wraparound across perm[cx + 1], perm_y0_hi, cell_grads).
        for target_offset in [
            f64::from(i32::MAX) - 1.0,
            f64::from(i32::MAX),
            f64::from(i32::MIN),
            f64::from(i32::MIN) + 1.0,
        ] {
            let mut rng = FixedOffsetRandom {
                double_draw: target_offset / 256.0,
                inner: LegacyRandom::new(1234),
            };
            let perlin = PerlinNoise::new(&mut rng);
            let mut rng2 = FixedOffsetRandom {
                double_draw: target_offset / 256.0,
                inner: LegacyRandom::new(1234),
            };
            let smeared = SmearedPerlinNoise::new(&mut rng2, 0.25);
            let mut pb = NoiseStack::builder();
            pb.add(Arc::new(perlin.clone()), 1.0, 1.0);
            let stack = pb.build().unwrap();

            let coords = [-2.5_f64, -1.5, -0.5, -0.0, 0.0, 0.5, 1.5, 2.5];
            for &x in &coords {
                for &z in &coords {
                    assert_f32_exact(
                        perlin.get2(x, z),
                        reference::ref_perlin_get2(perlin.table(), x, z),
                        &format!("extreme offset={target_offset} perlin.get2({x}, {z})"),
                    );
                    for &y in &coords {
                        assert_f32_exact(
                            perlin.get3(x, y, z),
                            reference::ref_perlin_get3(perlin.table(), x, y, z),
                            &format!("extreme offset={target_offset} perlin.get3({x}, {y}, {z})"),
                        );
                        assert_f32_exact(
                            smeared.get3(x, y, z),
                            reference::ref_smeared_get3(smeared.table(), 0.25, x, y, z),
                            &format!("extreme offset={target_offset} smeared.get3({x}, {y}, {z})"),
                        );
                        assert_f32_exact(
                            stack.get3(x, y, z),
                            0.0_f32 + 1.0_f32 * reference::ref_perlin_get3(perlin.table(), x, y, z),
                            &format!("extreme offset={target_offset} stack.get3({x}, {y}, {z})"),
                        );
                    }
                }
            }
        }
    }
}
