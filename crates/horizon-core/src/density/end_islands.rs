use super::*;
use crate::{
    noise::SimplexNoise,
    random::{LegacyRandom, RandomSource},
};

/// Java Edition 26.3's outer End island density. The center island is supplied
/// by the surrounding terrain graph, rather than by this sampler.
#[derive(Clone, Debug)]
pub struct EndIslandSampler {
    noise: SimplexNoise,
}
impl EndIslandSampler {
    pub fn new(world_seed: i64) -> Self {
        let mut random = LegacyRandom::new(world_seed);
        random.consume_count(17_292);
        Self {
            noise: SimplexNoise::with_skipped_offsets(&mut random, true),
        }
    }
    fn height(&self, x: i32, z: i32) -> f32 {
        // These divisions and remainders truncate toward zero, including for
        // negative coordinates. They are not block/chunk floor conversions.
        let center_x = x / 2;
        let center_z = z / 2;
        let local_x = x % 2;
        let local_z = z % 2;
        let mut height = -100.0;
        for offset_x in -12_i32..=12 {
            for offset_z in -12_i32..=12 {
                let island_x = i64::from(center_x.wrapping_add(offset_x));
                let island_z = i64::from(center_z.wrapping_add(offset_z));
                if island_x * island_x + island_z * island_z <= 4096
                    || self.noise.get2(island_x as f64, island_z as f64) >= -0.9
                {
                    continue;
                }
                let scale = ((island_x as f32).abs() * 3439.0 + (island_z as f32).abs() * 147.0)
                    % 13.0
                    + 9.0;
                let dx = (local_x - offset_x * 2) as f32;
                let dz = (local_z - offset_z * 2) as f32;
                let candidate = 100.0 - ((dx * dx + dz * dz) as f64).sqrt() as f32 * scale;
                let candidate = super::arithmetic::java_min(
                    super::arithmetic::java_max(candidate, -100.0),
                    80.0,
                );
                height = super::arithmetic::java_max(height, candidate);
            }
        }
        height
    }
    fn density(&self, x: i32, z: i32) -> f32 {
        (self.height(x / 8, z / 8) - 8.0) / 128.0
    }
}
impl DensitySampler for EndIslandSampler {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        Ok(self.density(position[0], position[2]))
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        let [sx, sy, sz] = volume.sizes();
        for z in 0..sz {
            let block_z = volume.block(Axis::Z, z).unwrap();
            for x in 0..sx {
                let value = self.density(volume.block(Axis::X, x).unwrap(), block_z);
                let start = (x + z * sx) * sy;
                buffer[start..start + sy].fill(value);
            }
        }
        Ok(())
    }
}
