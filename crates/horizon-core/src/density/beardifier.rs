//! Structure terrain adaptation from ordered rigid pieces and jigsaw junctions.
//! Discovery/filtering of live structure starts is a separate world subsystem.
use super::{DensitySampler, SampleError, SamplerContext, check_buffer};
use crate::noise::DensityVolume;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StructureBounds {
    minimum: [i32; 3],
    maximum: [i32; 3],
}
impl StructureBounds {
    /// Already normalized inclusive bounds. Invalid inputs are rejected rather
    /// than invoking vanilla's development-mode logging/repair path.
    pub fn new(minimum: [i32; 3], maximum: [i32; 3]) -> Option<Self> {
        (0..3)
            .all(|axis| minimum[axis] <= maximum[axis])
            .then_some(Self { minimum, maximum })
    }
    pub fn minimum(self) -> [i32; 3] {
        self.minimum
    }
    pub fn maximum(self) -> [i32; 3] {
        self.maximum
    }
    pub fn contains(self, position: [i32; 3]) -> bool {
        (0..3).all(|i| position[i] >= self.minimum[i] && position[i] <= self.maximum[i])
    }
    fn intersects(self, minimum: [i32; 3], maximum: [i32; 3]) -> bool {
        (0..3).all(|i| self.maximum[i] >= minimum[i] && self.minimum[i] <= maximum[i])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainAdjustment {
    None,
    Bury,
    BeardThin,
    BeardBox,
    Encapsulate,
}
#[derive(Clone, Copy, Debug)]
pub struct RigidPiece {
    pub bounds: StructureBounds,
    pub adjustment: TerrainAdjustment,
    pub ground_level_delta: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct JigsawJunction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
}

/// A structure library's terrain kernel: YUNG's API's and MoogsStructureLib's
/// enhanced terrain adaptation (EnhancedTerrainAdaptation.computeDensityFactor).
/// Like vanilla's beard, but with its own size and falloff, and an action
/// above the piece's floor and one below it: bury adds terrain, carve takes
/// it away.
#[derive(Debug)]
pub struct TerrainKernel {
    size: i32,
    /// Density sign above the floor and below it: 1 buries, -1 carves, 0 none.
    top: i32,
    bottom: i32,
    weights: Vec<f32>,
}
impl TerrainKernel {
    /// A kernel `size` blocks across whose weights fall off as
    /// e^(-d²/`distance`). Size 0 is the libraries' NONE: no density.
    pub fn new(size: i32, distance: i32, top: i32, bottom: i32) -> Self {
        let size = size.max(0);
        let radius = size / 2;
        let mut weights = vec![0.; (size as usize).pow(3)];
        for x in 0..size {
            for y in 0..size {
                for z in 0..size {
                    let (kx, ky, kz) = (
                        f64::from(x - radius),
                        f64::from(y - radius) + 0.5,
                        f64::from(z - radius),
                    );
                    let squared = kx * kx + ky * ky + kz * kz;
                    weights[((z * size + x) * size + y) as usize] =
                        crate::jmath::pow(std::f64::consts::E, -squared / f64::from(distance))
                            as f32;
                }
            }
        }
        Self {
            size,
            top,
            bottom,
            weights,
        }
    }
    pub fn radius(&self) -> i32 {
        self.size / 2
    }
    /// The libraries' NONE: no kernel at all.
    pub fn is_none(&self) -> bool {
        self.size == 0
    }
    /// The density at (`dx`, `dy`, `dz`) from the nearest point of a piece
    /// (or a junction), `floor` blocks above the piece's floor.
    fn factor(&self, dx: i32, dy: i32, dz: i32, floor: i32) -> f64 {
        let radius = self.radius();
        let k = [dx, dy, dz].map(|v| v.wrapping_add(radius));
        if k.iter().any(|v| !(0..self.size).contains(v)) {
            return 0.;
        }
        let weight = self.weights[((k[2] * self.size + k[0]) * self.size + k[1]) as usize];
        let floor = f64::from(floor) + 0.5;
        let squared = f64::from(dx) * f64::from(dx) + floor * floor + f64::from(dz) * f64::from(dz);
        // Mth.invSqrt(double): 1 / sqrt, not vanilla beard's fast estimate.
        let multiplier = (floor * (1. / (squared / 2.).sqrt()) / 2.).abs();
        let sign = if floor > 0. { self.top } else { self.bottom };
        multiplier * f64::from(weight) * f64::from(sign)
    }
}
/// A rigid piece's box under a terrain kernel (already moved, padded or cut
/// to a band as the piece's adaptation says).
#[derive(Clone, Debug)]
pub struct KernelPiece {
    pub bounds: StructureBounds,
    pub kernel: Arc<TerrainKernel>,
}
/// A jigsaw junction under a terrain kernel; `source` holds the ground
/// height with the adaptation's offset.
#[derive(Clone, Debug)]
pub struct KernelJunction {
    pub source: [i32; 3],
    pub kernel: Arc<TerrainKernel>,
}

#[derive(Clone, Debug, Default)]
pub struct Beardifier {
    pieces: Vec<RigidPiece>,
    junctions: Vec<JigsawJunction>,
    kernel_pieces: Vec<KernelPiece>,
    kernel_junctions: Vec<KernelJunction>,
    affected: Option<StructureBounds>,
}
impl Beardifier {
    /// Matches the runtime constructor. A missing affected box makes the entire
    /// sampler empty, even when the supplied component lists are nonempty.
    pub fn new(
        pieces: Vec<RigidPiece>,
        junctions: Vec<JigsawJunction>,
        affected: Option<StructureBounds>,
    ) -> Self {
        Self {
            pieces,
            junctions,
            affected,
            ..Self::default()
        }
    }
    /// Adds terrain kernels' pieces and junctions, summed after vanilla's
    /// density in double precision as the libraries' Beardifier mixins do.
    /// `affected` must cover them.
    pub fn with_kernels(
        mut self,
        pieces: Vec<KernelPiece>,
        junctions: Vec<KernelJunction>,
    ) -> Self {
        self.kernel_pieces = pieces;
        self.kernel_junctions = junctions;
        self
    }
    /// True when the volume and every vanilla piece/junction stay strictly inside
    /// non-wrapping `i32` bounds (including the ±12 beard radius), so per-piece
    /// range clipping produces bit-identical `f32` sums to scanning every point.
    fn can_scatter(
        &self,
        minimum: [i32; 3],
        steps: [i32; 3],
        low: [i32; 3],
        high: [i32; 3],
    ) -> bool {
        const SAFE_MIN: i64 = i32::MIN as i64 + 16;
        const SAFE_MAX: i64 = i32::MAX as i64 - 16;
        let mut vol_min = [0i64; 3];
        let mut vol_max = [0i64; 3];
        for i in 0..3 {
            if low[i] > high[i] {
                return true;
            }
            vol_min[i] = i64::from(minimum[i]) + i64::from(low[i]) * i64::from(steps[i]);
            vol_max[i] = i64::from(minimum[i]) + i64::from(high[i]) * i64::from(steps[i]);
            if vol_min[i] < SAFE_MIN || vol_max[i] > SAFE_MAX {
                return false;
            }
        }
        for piece in &self.pieces {
            let p_lo = piece.bounds.minimum;
            let p_hi = piece.bounds.maximum;
            for i in 0..3 {
                let lo = i64::from(p_lo[i]);
                let hi = i64::from(p_hi[i]);
                if lo < SAFE_MIN || hi > SAFE_MAX {
                    return false;
                }
                if (vol_max[i] - lo).abs() > SAFE_MAX || (hi - vol_min[i]).abs() > SAFE_MAX {
                    return false;
                }
            }
            let ground = i64::from(p_lo[1]) + i64::from(piece.ground_level_delta);
            if !(SAFE_MIN..=SAFE_MAX).contains(&ground)
                || (vol_max[1] - ground).abs() > SAFE_MAX
                || (ground - vol_min[1]).abs() > SAFE_MAX
            {
                return false;
            }
        }
        for junction in &self.junctions {
            let coords = [
                junction.source_x,
                junction.source_ground_y,
                junction.source_z,
            ];
            for i in 0..3 {
                let c = i64::from(coords[i]);
                if !(SAFE_MIN..=SAFE_MAX).contains(&c)
                    || (vol_max[i] - c).abs() > SAFE_MAX
                    || (c - vol_min[i]).abs() > SAFE_MAX
                {
                    return false;
                }
            }
        }
        true
    }

    /// Accumulates each piece and junction in vanilla order, visiting only lattice
    /// cells within that component's non-zero support (`±12` for beard/encapsulate/
    /// junction, `±6`/`±12` for bury). Since skipped terms are exact `+0.0` and
    /// `buffer` is initialized to `+0.0` with no `-0.0` contributions, the final
    /// `f32` bits match [`Self::sample_unchecked`] at every cell.
    fn scatter_vanilla(
        &self,
        buffer: &mut [f32],
        sizes: [usize; 3],
        minimum: [i32; 3],
        steps: [i32; 3],
        low: [i32; 3],
        high: [i32; 3],
    ) {
        if low[0] > high[0] || low[1] > high[1] || low[2] > high[2] {
            return;
        }
        let clip_axis = |axis: usize, world_lo: i64, world_hi: i64| -> Option<(i32, i32)> {
            let min = i64::from(minimum[axis]);
            let step = i64::from(steps[axis]);
            let start = (world_lo - min + step - 1)
                .div_euclid(step)
                .max(i64::from(low[axis]));
            let end = (world_hi - min).div_euclid(step).min(i64::from(high[axis]));
            (start <= end).then_some((start as i32, end as i32))
        };
        let kernel = kernel();
        let [sx, sy, _] = sizes;
        for piece in &self.pieces {
            if piece.adjustment == TerrainAdjustment::None {
                continue;
            }
            let p_lo = piece.bounds.minimum;
            let p_hi = piece.bounds.maximum;
            let ground = p_lo[1] + piece.ground_level_delta;
            let (xz_pad, y_lo, y_hi) = match piece.adjustment {
                TerrainAdjustment::None => continue,
                TerrainAdjustment::Bury => (5i64, i64::from(ground) - 11, i64::from(ground) + 11),
                TerrainAdjustment::BeardThin => {
                    (11i64, i64::from(ground) - 12, i64::from(ground) + 11)
                }
                TerrainAdjustment::BeardBox => {
                    (11i64, i64::from(ground) - 11, i64::from(p_hi[1]) + 11)
                }
                TerrainAdjustment::Encapsulate => {
                    (11i64, i64::from(p_lo[1]) - 11, i64::from(p_hi[1]) + 11)
                }
            };
            let Some((x0, x1)) =
                clip_axis(0, i64::from(p_lo[0]) - xz_pad, i64::from(p_hi[0]) + xz_pad)
            else {
                continue;
            };
            let Some((z0, z1)) =
                clip_axis(2, i64::from(p_lo[2]) - xz_pad, i64::from(p_hi[2]) + xz_pad)
            else {
                continue;
            };
            let Some((y0, y1)) = clip_axis(1, y_lo, y_hi) else {
                continue;
            };
            for z in z0..=z1 {
                let pz = minimum[2] + z * steps[2];
                let dz = 0.max((p_lo[2] - pz).max(pz - p_hi[2]));
                let z_base = z as usize * sx;
                for x in x0..=x1 {
                    let px = minimum[0] + x * steps[0];
                    let dx = 0.max((p_lo[0] - px).max(px - p_hi[0]));
                    let col_base = (x as usize + z_base) * sy;
                    match piece.adjustment {
                        TerrainAdjustment::None => {}
                        TerrainAdjustment::Bury => {
                            let x_sq = dx as f32 * dx as f32;
                            let z_sq = dz as f32 * dz as f32;
                            if x_sq + z_sq >= 36. {
                                continue;
                            }
                            for y in y0..=y1 {
                                let py = minimum[1] + y * steps[1];
                                let fy = (py - ground) as f32 / 2.;
                                let squared = (x_sq + fy * fy) + z_sq;
                                if squared < 36. {
                                    buffer[col_base + y as usize] += 1. - squared.sqrt() / 6.;
                                }
                            }
                        }
                        TerrainAdjustment::BeardThin => {
                            let x_sq = dx as f32 * dx as f32;
                            let z_sq = dz as f32 * dz as f32;
                            let kcol = &kernel[((dz + 12) * 24 + (dx + 12)) as usize * 24..][..24];
                            for y in y0..=y1 {
                                let py = minimum[1] + y * steps[1];
                                let dy = py - ground;
                                let vertical = dy as f32 + 0.5;
                                let squared = (x_sq + vertical * vertical) + z_sq;
                                let factor = (-vertical
                                    * fast_inverse_sqrt(f64::from(squared / 2.)) as f32)
                                    / 2.;
                                buffer[col_base + y as usize] +=
                                    (factor * kcol[(dy + 12) as usize]) * 0.8;
                            }
                        }
                        TerrainAdjustment::BeardBox => {
                            let x_sq = dx as f32 * dx as f32;
                            let z_sq = dz as f32 * dz as f32;
                            let kcol = &kernel[((dz + 12) * 24 + (dx + 12)) as usize * 24..][..24];
                            for y in y0..=y1 {
                                let py = minimum[1] + y * steps[1];
                                let dy = py - ground;
                                let distance_y = 0.max((ground - py).max(py - p_hi[1]));
                                let vertical = dy as f32 + 0.5;
                                let squared = (x_sq + vertical * vertical) + z_sq;
                                let factor = (-vertical
                                    * fast_inverse_sqrt(f64::from(squared / 2.)) as f32)
                                    / 2.;
                                buffer[col_base + y as usize] +=
                                    (factor * kcol[(distance_y + 12) as usize]) * 0.8;
                            }
                        }
                        TerrainAdjustment::Encapsulate => {
                            let fx = dx as f32 / 2.;
                            let fz = dz as f32 / 2.;
                            let x_sq = fx * fx;
                            let z_sq = fz * fz;
                            if x_sq + z_sq >= 36. {
                                continue;
                            }
                            for y in y0..=y1 {
                                let py = minimum[1] + y * steps[1];
                                let distance_y = 0.max((p_lo[1] - py).max(py - p_hi[1]));
                                let fy = distance_y as f32 / 2.;
                                let squared = (x_sq + fy * fy) + z_sq;
                                if squared < 36. {
                                    buffer[col_base + y as usize] +=
                                        (1. - squared.sqrt() / 6.) * 0.8;
                                }
                            }
                        }
                    }
                }
            }
        }
        for junction in &self.junctions {
            let Some((x0, x1)) = clip_axis(
                0,
                i64::from(junction.source_x) - 12,
                i64::from(junction.source_x) + 11,
            ) else {
                continue;
            };
            let Some((z0, z1)) = clip_axis(
                2,
                i64::from(junction.source_z) - 12,
                i64::from(junction.source_z) + 11,
            ) else {
                continue;
            };
            let Some((y0, y1)) = clip_axis(
                1,
                i64::from(junction.source_ground_y) - 12,
                i64::from(junction.source_ground_y) + 11,
            ) else {
                continue;
            };
            for z in z0..=z1 {
                let dz = (minimum[2] + z * steps[2]) - junction.source_z;
                let z_sq = dz as f32 * dz as f32;
                let z_base = z as usize * sx;
                for x in x0..=x1 {
                    let dx = (minimum[0] + x * steps[0]) - junction.source_x;
                    let x_sq = dx as f32 * dx as f32;
                    let kcol = &kernel[((dz + 12) * 24 + (dx + 12)) as usize * 24..][..24];
                    let col_base = (x as usize + z_base) * sy;
                    for y in y0..=y1 {
                        let dy = (minimum[1] + y * steps[1]) - junction.source_ground_y;
                        let vertical = dy as f32 + 0.5;
                        let squared = (x_sq + vertical * vertical) + z_sq;
                        let factor =
                            (-vertical * fast_inverse_sqrt(f64::from(squared / 2.)) as f32) / 2.;
                        buffer[col_base + y as usize] += (factor * kcol[(dy + 12) as usize]) * 0.4;
                    }
                }
            }
        }
    }
    fn sample_unchecked(&self, [x, y, z]: [i32; 3]) -> f32 {
        let mut value = 0.;
        // The kernel is a process-wide constant; resolving it once keeps the
        // per-piece and per-junction beard evaluations off the OnceLock.
        let kernel = kernel();
        for piece in &self.pieces {
            let low = piece.bounds.minimum;
            let high = piece.bounds.maximum;
            let dx = 0.max(low[0].wrapping_sub(x).max(x.wrapping_sub(high[0])));
            let dz = 0.max(low[2].wrapping_sub(z).max(z.wrapping_sub(high[2])));
            let ground = low[1].wrapping_add(piece.ground_level_delta);
            let dy = y.wrapping_sub(ground);
            let distance_y = match piece.adjustment {
                TerrainAdjustment::None => 0,
                TerrainAdjustment::Bury | TerrainAdjustment::BeardThin => dy,
                TerrainAdjustment::BeardBox => {
                    0.max(ground.wrapping_sub(y).max(y.wrapping_sub(high[1])))
                }
                TerrainAdjustment::Encapsulate => {
                    0.max(low[1].wrapping_sub(y).max(y.wrapping_sub(high[1])))
                }
            };
            value += match piece.adjustment {
                TerrainAdjustment::None => 0.,
                TerrainAdjustment::Bury => bury(dx as f32, distance_y as f32 / 2., dz as f32),
                TerrainAdjustment::BeardThin | TerrainAdjustment::BeardBox => {
                    beard(kernel, dx, distance_y, dz, dy) * 0.8
                }
                TerrainAdjustment::Encapsulate => {
                    bury(dx as f32 / 2., distance_y as f32 / 2., dz as f32 / 2.) * 0.8
                }
            };
        }
        for junction in &self.junctions {
            let dx = x.wrapping_sub(junction.source_x);
            let dy = y.wrapping_sub(junction.source_ground_y);
            let dz = z.wrapping_sub(junction.source_z);
            value += beard(kernel, dx, dy, dz, dy) * 0.4;
        }
        if self.kernel_pieces.is_empty() && self.kernel_junctions.is_empty() {
            return value;
        }
        let mut density = f64::from(value);
        for piece in &self.kernel_pieces {
            let (low, high) = (piece.bounds.minimum, piece.bounds.maximum);
            let d: [i32; 3] = std::array::from_fn(|i| {
                0.max(
                    low[i]
                        .wrapping_sub([x, y, z][i])
                        .max([x, y, z][i].wrapping_sub(high[i])),
                )
            });
            density += piece
                .kernel
                .factor(d[0], d[1], d[2], y.wrapping_sub(low[1]))
                * 0.8;
        }
        for junction in &self.kernel_junctions {
            let d: [i32; 3] =
                std::array::from_fn(|i| [x, y, z][i].wrapping_sub(junction.source[i]));
            density += junction.kernel.factor(d[0], d[1], d[2], d[1]) * 0.4;
        }
        density as f32
    }
}
impl DensitySampler for Beardifier {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        Ok(
            if self
                .affected
                .is_some_and(|bounds| bounds.contains(position))
            {
                self.sample_unchecked(position)
            } else {
                0.
            },
        )
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        buffer.fill(0.);
        let Some(bounds) = self.affected else {
            return Ok(());
        };
        let minimum = volume.minimum();
        if !bounds.intersects(minimum, volume.maximum()) {
            return Ok(());
        }
        let steps = volume.steps();
        let sizes = volume.sizes();
        // Deliberately floor, not ceil: the first bulk lattice sample can lie
        // below the affected bounds and is evaluated without the scalar guard.
        let low: [i32; 3] = std::array::from_fn(|i| {
            bounds.minimum[i]
                .wrapping_sub(minimum[i])
                .max(0)
                .div_euclid(steps[i])
        });
        let high: [i32; 3] = std::array::from_fn(|i| {
            ((sizes[i] - 1) as i32).min(
                bounds.maximum[i]
                    .wrapping_sub(minimum[i])
                    .div_euclid(steps[i]),
            )
        });
        if self.kernel_pieces.is_empty()
            && self.kernel_junctions.is_empty()
            && self.can_scatter(minimum, steps, low, high)
        {
            self.scatter_vanilla(buffer, sizes, minimum, steps, low, high);
            return Ok(());
        }
        for z in low[2]..=high[2] {
            for x in low[0]..=high[0] {
                for y in low[1]..=high[1] {
                    let index = volume
                        .index([x as usize, y as usize, z as usize])
                        .expect("bounded structure density indices");
                    let local = [x, y, z];
                    let point = std::array::from_fn(|i| {
                        minimum[i].wrapping_add(local[i].wrapping_mul(steps[i]))
                    });
                    buffer[index] = self.sample_unchecked(point);
                }
            }
        }
        Ok(())
    }
}
fn bury(x: f32, y: f32, z: f32) -> f32 {
    let squared = (x * x + y * y) + z * z;
    if squared >= 36. {
        0.
    } else {
        1. - squared.sqrt() / 6.
    }
}
fn beard(kernel: &[f32; 13824], x: i32, y: i32, z: i32, ground_delta: i32) -> f32 {
    let indices = [x, y, z].map(|v| v.wrapping_add(12));
    if indices.iter().any(|v| !(0..24).contains(v)) {
        return 0.;
    }
    let vertical = ground_delta as f32 + 0.5;
    let squared = (x as f32 * x as f32 + vertical * vertical) + z as f32 * z as f32;
    let factor = (-vertical * fast_inverse_sqrt(f64::from(squared / 2.)) as f32) / 2.;
    factor * kernel[((indices[2] * 24 + indices[0]) * 24 + indices[1]) as usize]
}
fn fast_inverse_sqrt(value: f64) -> f64 {
    let half = 0.5 * value;
    let bits = 6_910_469_410_427_058_090i64.wrapping_sub((value.to_bits() as i64) >> 1);
    let estimate = f64::from_bits(bits as u64);
    estimate * (1.5 - half * estimate * estimate)
}
fn kernel() -> &'static [f32; 13824] {
    static KERNEL: OnceLock<[f32; 13824]> = OnceLock::new();
    KERNEL.get_or_init(|| {
        std::array::from_fn(|i| {
            let z = (i / 576) as f64 - 12.;
            let x = ((i / 24) % 24) as f64 - 12.;
            let y = (i % 24) as f64 - 12. + 0.5;
            let squared = (x * x + y * y) + z * z;
            crate::jmath::pow(std::f64::consts::E, -squared / 16.) as f32
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(beardifier: &Beardifier, position: [i32; 3]) -> f32 {
        beardifier
            .sample_value(&mut SamplerContext::default(), position)
            .unwrap()
    }

    /// YUNG's carved top (kernel 24, distance 24, carve above, nothing
    /// below) around a 5-block cube, values worked out from
    /// EnhancedTerrainAdaptation.computeDensityFactor.
    #[test]
    fn kernel_pieces_carve_above_the_floor_only() {
        let kernel = Arc::new(TerrainKernel::new(24, 24, -1, 0));
        let bounds = StructureBounds::new([0; 3], [4; 3]).unwrap();
        let affected = StructureBounds::new([-20; 3], [20; 3]);
        let b = Beardifier::new(Vec::new(), Vec::new(), affected)
            .with_kernels(vec![KernelPiece { bounds, kernel }], Vec::new());
        // Inside, on the floor: |0.5 / sqrt(0.125) / 2| * e^(-0.25/24) * -0.8.
        assert!((sample(&b, [2, 0, 2]) + 0.559_823_4).abs() < 1e-6);
        // Three blocks east: |0.5 / sqrt(4.625) / 2| * e^(-9.25/24) * -0.8.
        assert!((sample(&b, [7, 0, 2]) + 0.063_254_3).abs() < 1e-6);
        // Below the floor the bottom action (none) applies; past the radius
        // nothing does.
        assert_eq!(sample(&b, [2, -3, 2]), 0.);
        assert_eq!(sample(&b, [17, 2, 2]), 0.);
    }

    #[test]
    fn kernel_junctions_bury_at_four_tenths() {
        let kernel = Arc::new(TerrainKernel::new(24, 24, 1, 1));
        let b = Beardifier::new(
            Vec::new(),
            Vec::new(),
            StructureBounds::new([-20; 3], [80; 3]),
        )
        .with_kernels(
            Vec::new(),
            vec![KernelJunction {
                source: [0, 64, 0],
                kernel,
            }],
        );
        assert!((sample(&b, [0, 64, 0]) - 0.279_911_7).abs() < 1e-6);
        assert!(sample(&b, [0, 60, 0]) > 0.);
        assert!(TerrainKernel::new(0, 0, 1, 1).is_none());
    }

    /// `Beardifier.BEARD_KERNEL`: the table vanilla's static initialiser fills
    /// from `computeBeardContribution(xi - 12, yi - 12, zi - 12)`, stored at
    /// `zi * 24 * 24 + xi * 24 + yi` — `e^(-(dx² + (dy + 0.5)² + dz²) / 16)` as
    /// a `float`. 13824 values can't be read, so the ones below are pinned as
    /// the bits they have in vanilla (that initialiser's formula run under
    /// javac/java 21 and printed with `Float.floatToIntBits`), and `TABLE`
    /// hashes every entry so a change to the exponent, the offset or the index
    /// order fails here instead of in a world.
    #[test]
    fn the_beard_kernel_table_is_vanillas() {
        let k = kernel();
        // (index, float bits): the corner, the three axes' first steps, the
        // centre (dy 0.5), an off-axis point and the far corner.
        for (index, bits) in [
            (0usize, 747_230_124u32),
            (1, 763_905_896),
            (25, 781_152_350),
            (300, 956_222_450),
            (3067, 966_315_629),
            (7212, 1_065_093_109),
            (13_823, 781_758_625),
        ] {
            assert_eq!(
                k[index].to_bits(),
                bits,
                "kernel[{index}] is {} not {}",
                k[index],
                f32::from_bits(bits)
            );
        }
        // The centre entry is the largest: every step away falls off.
        assert_eq!(
            (0..k.len()).max_by(|a, b| k[*a].total_cmp(&k[*b])),
            Some(7212)
        );
        let mut hash = 1_125_899_906_842_597u64;
        for value in k.iter() {
            hash = hash
                .wrapping_mul(31)
                .wrapping_add(u64::from(value.to_bits()));
        }
        assert_eq!(hash, 0xb44d_13f8_c3d9_8c65);
    }

    #[test]
    fn sample_volume_scatter_matches_pointwise_bit_for_bit() {
        let pieces = vec![
            RigidPiece {
                bounds: StructureBounds::new([-6, 62, -4], [6, 70, 5]).unwrap(),
                adjustment: TerrainAdjustment::BeardThin,
                ground_level_delta: 2,
            },
            RigidPiece {
                bounds: StructureBounds::new([8, 60, 6], [18, 68, 16]).unwrap(),
                adjustment: TerrainAdjustment::BeardBox,
                ground_level_delta: 1,
            },
            RigidPiece {
                bounds: StructureBounds::new([2, 50, -8], [10, 58, 2]).unwrap(),
                adjustment: TerrainAdjustment::Bury,
                ground_level_delta: 0,
            },
            RigidPiece {
                bounds: StructureBounds::new([-10, 40, 8], [-2, 48, 18]).unwrap(),
                adjustment: TerrainAdjustment::Encapsulate,
                ground_level_delta: -2,
            },
            RigidPiece {
                bounds: StructureBounds::new([0, 64, 0], [4, 68, 4]).unwrap(),
                adjustment: TerrainAdjustment::None,
                ground_level_delta: 0,
            },
            // BeardBox with ground > high[1] edge case
            RigidPiece {
                bounds: StructureBounds::new([4, 65, 4], [8, 66, 8]).unwrap(),
                adjustment: TerrainAdjustment::BeardBox,
                ground_level_delta: 30,
            },
        ];
        let junctions = vec![
            JigsawJunction {
                source_x: 3,
                source_ground_y: 64,
                source_z: 4,
            },
            JigsawJunction {
                source_x: -5,
                source_ground_y: 55,
                source_z: 12,
            },
        ];
        let affected = StructureBounds::new([-24, 20, -24], [36, 100, 36]);
        let beardifier = Beardifier::new(pieces, junctions, affected);
        let volumes = [
            DensityVolume::new([5, 49, 5], [0, -64, 0], [4, 8, 4]).unwrap(),
            DensityVolume::new([16, 48, 16], [-8, 36, -8], [1, 1, 1]).unwrap(),
            DensityVolume::new([7, 25, 7], [-20, 28, -18], [3, 3, 3]).unwrap(),
        ];
        let mut ctx = SamplerContext::default();
        for volume in volumes {
            let mut got = vec![99.0_f32; volume.size()];
            beardifier
                .sample_volume(&mut ctx, &mut got, volume)
                .unwrap();
            let [sx, sy, _] = volume.sizes();
            let min = volume.minimum();
            let steps = volume.steps();
            let bounds = beardifier.affected.unwrap();
            let low: [i32; 3] = std::array::from_fn(|i| {
                bounds.minimum[i]
                    .wrapping_sub(min[i])
                    .max(0)
                    .div_euclid(steps[i])
            });
            let high: [i32; 3] = std::array::from_fn(|i| {
                ((volume.sizes()[i] - 1) as i32)
                    .min(bounds.maximum[i].wrapping_sub(min[i]).div_euclid(steps[i]))
            });
            let mut expected = vec![0.0_f32; volume.size()];
            for z in low[2]..=high[2] {
                for x in low[0]..=high[0] {
                    for y in low[1]..=high[1] {
                        let idx = (x as usize + z as usize * sx) * sy + y as usize;
                        let pt = [
                            min[0].wrapping_add(x.wrapping_mul(steps[0])),
                            min[1].wrapping_add(y.wrapping_mul(steps[1])),
                            min[2].wrapping_add(z.wrapping_mul(steps[2])),
                        ];
                        expected[idx] = beardifier.sample_unchecked(pt);
                    }
                }
            }
            for i in 0..volume.size() {
                assert_eq!(
                    got[i].to_bits(),
                    expected[i].to_bits(),
                    "mismatch at index {i} for volume {volume:?}: got {} vs expected {}",
                    got[i],
                    expected[i]
                );
            }
        }
    }
}
