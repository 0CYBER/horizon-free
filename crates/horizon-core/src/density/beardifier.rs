//! Structure terrain adaptation from ordered rigid pieces and jigsaw junctions.
//! Discovery/filtering of live structure starts is a separate world subsystem.
use super::{DensitySampler, SampleError, SamplerContext, check_buffer};
use crate::noise::DensityVolume;
use std::sync::OnceLock;

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

#[derive(Clone, Debug, Default)]
pub struct Beardifier {
    pieces: Vec<RigidPiece>,
    junctions: Vec<JigsawJunction>,
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
        }
    }
    pub fn affected_bounds(&self) -> Option<StructureBounds> {
        self.affected
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
        value
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
