//! Climate coordinates derived from the six terrain-router climate fields.
use horizon_core::density::{DensitySampler, SampleError, SamplerContext};
use horizon_core::noise::DensityVolume;
use std::sync::Arc;
mod codec;
mod index;
pub use codec::{decode_parameter, decode_parameter_point};
pub use index::{ClimateIndex, ClimateSearch, IndexError, Parameter, ParameterPoint};

/// Vanilla multiplies in float precision before the saturating Java long cast.
pub fn quantize(value: f32) -> i64 {
    (value * 10_000.0) as i64
}
pub fn unquantize(value: i64) -> f32 {
    value as f32 / 10_000.0
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetPoint {
    pub temperature: i64,
    pub humidity: i64,
    pub continentalness: i64,
    pub erosion: i64,
    pub depth: i64,
    pub weirdness: i64,
}
impl TargetPoint {
    pub fn from_values(values: [f32; 6]) -> Self {
        let [
            temperature,
            humidity,
            continentalness,
            erosion,
            depth,
            weirdness,
        ] = values.map(quantize);
        Self {
            temperature,
            humidity,
            continentalness,
            erosion,
            depth,
            weirdness,
        }
    }
    /// The seventh parameter is the zero offset used by biome nearest searches.
    pub const fn parameters(self) -> [i64; 7] {
        [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.weirdness,
            0,
        ]
    }
}
#[derive(Clone, Debug)]
pub struct ClimateSampler {
    samplers: [Arc<dyn DensitySampler>; 6],
}
impl ClimateSampler {
    /// Temperature, humidity, continentalness, erosion, depth, weirdness.
    pub fn new(samplers: [Arc<dyn DensitySampler>; 6]) -> Self {
        Self { samplers }
    }
    /// Coordinates are biome quart positions, converted with wrapped << 2.
    /// Every child uses the same context and runs in field declaration order.
    pub fn sample(
        &self,
        context: &mut SamplerContext,
        quart: [i32; 3],
    ) -> Result<TargetPoint, SampleError> {
        let position = quart.map(|coordinate| coordinate.wrapping_shl(2));
        let mut values = [0.; 6];
        for (sampler, value) in self.samplers.iter().zip(&mut values) {
            *value = sampler.sample_value(context, position)?;
        }
        Ok(TargetPoint::from_values(values))
    }
    /// Prepare all six field buffers in declaration order on the quart lattice.
    /// Lookup quantizes the retained bulk values rather than resampling scalars.
    pub fn sample_volume(
        &self,
        context: &mut SamplerContext,
        quart_minimum: [i32; 3],
        extents: [i32; 3],
    ) -> Result<ClimateVolume, SampleError> {
        let mut storage: [Vec<f32>; 6] = Default::default();
        self.sample_volume_with_buffers(context, quart_minimum, extents, &mut storage)
    }
    /// [`Self::sample_volume`] reusing caller-owned field buffers (`ClimateVolume::recycle`).
    pub(crate) fn sample_volume_with_buffers(
        &self,
        context: &mut SamplerContext,
        quart_minimum: [i32; 3],
        extents: [i32; 3],
        storage: &mut [Vec<f32>; 6],
    ) -> Result<ClimateVolume, SampleError> {
        let volume = DensityVolume::new(extents, quart_minimum.map(|v| v.wrapping_shl(2)), [4; 3])?;
        let size = volume.size();
        let mut fields = std::mem::take(storage);
        for (sampler, field) in self.samplers.iter().zip(&mut fields) {
            field.resize(size, 0.0);
            field.fill(0.0);
            if let Err(error) = sampler.sample_volume(context, field, volume) {
                *storage = fields;
                return Err(error);
            }
        }
        Ok(ClimateVolume {
            quart_minimum,
            volume,
            fields,
        })
    }
}
#[derive(Debug)]
pub struct ClimateVolume {
    quart_minimum: [i32; 3],
    volume: DensityVolume,
    fields: [Vec<f32>; 6],
}
impl ClimateVolume {
    /// Return the six field buffers to `storage` for reuse by the next volume.
    pub(crate) fn recycle(self, storage: &mut [Vec<f32>; 6]) {
        *storage = self.fields;
    }
    /// Coordinates must be inside the prepared quart volume. The bounded Rust
    /// API rejects invalid coordinates instead of Java's unchecked buffer alias.
    #[inline]
    pub fn target(&self, quart: [i32; 3]) -> Option<TargetPoint> {
        let relative =
            std::array::from_fn(|i| quart[i].wrapping_sub(self.quart_minimum[i]) as usize);
        let index = self.volume.index(relative)?;
        Some(TargetPoint::from_values(std::array::from_fn(|i| {
            self.fields[i][index]
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::{
        density::{GradientSampler, TilingMode},
        geometry::Axis,
    };

    #[test]
    fn sample_volume_with_buffers_matches_sample_volume_and_recycles_storage() {
        let sampler = ClimateSampler::new([
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::ClampToEdge,
                -32,
                32,
                -1.0,
                1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::Y,
                TilingMode::ClampToEdge,
                -64,
                64,
                -0.5,
                0.75,
            )),
            Arc::new(GradientSampler::new(
                Axis::Z,
                TilingMode::ClampToEdge,
                -32,
                32,
                0.0,
                1.5,
            )),
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::ClampToEdge,
                0,
                64,
                1.0,
                -1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::Y,
                TilingMode::ClampToEdge,
                -16,
                48,
                -0.25,
                0.25,
            )),
            Arc::new(GradientSampler::new(
                Axis::Z,
                TilingMode::ClampToEdge,
                -16,
                16,
                0.5,
                -0.5,
            )),
        ]);
        let mut storage: [Vec<f32>; 6] = std::array::from_fn(|_| Vec::new());
        let mut ctx_pooled = SamplerContext::cached();
        let mut ctx_fresh = SamplerContext::cached();

        for (minimum, extents) in [
            ([-2, -4, 3], [2, 3, 2]),
            ([0, -2, -1], [4, 5, 4]),
            ([5, 1, -3], [1, 2, 1]),
        ] {
            let expected = sampler
                .sample_volume(&mut ctx_fresh, minimum, extents)
                .unwrap();
            let actual = sampler
                .sample_volume_with_buffers(&mut ctx_pooled, minimum, extents, &mut storage)
                .unwrap();
            for qx in minimum[0]..minimum[0] + extents[0] {
                for qy in minimum[1]..minimum[1] + extents[1] {
                    for qz in minimum[2]..minimum[2] + extents[2] {
                        let quart = [qx, qy, qz];
                        assert_eq!(actual.target(quart), expected.target(quart), "{quart:?}");
                    }
                }
            }
            actual.recycle(&mut storage);
            let needed = (extents[0] * extents[1] * extents[2]) as usize;
            assert!(storage.iter().all(|buf| buf.capacity() >= needed));
        }

        // An error during sampling must restore `storage` without discarding capacities.
        let capacities: [usize; 6] = std::array::from_fn(|i| storage[i].capacity());
        let failing = ClimateSampler::new([
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::ClampToEdge,
                0,
                16,
                0.0,
                1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::Y,
                TilingMode::Repeat,
                0,
                0,
                0.0,
                1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::Z,
                TilingMode::ClampToEdge,
                0,
                16,
                0.0,
                1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::ClampToEdge,
                0,
                16,
                0.0,
                1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::Y,
                TilingMode::ClampToEdge,
                0,
                16,
                0.0,
                1.0,
            )),
            Arc::new(GradientSampler::new(
                Axis::Z,
                TilingMode::ClampToEdge,
                0,
                16,
                0.0,
                1.0,
            )),
        ]);
        assert_eq!(
            failing
                .sample_volume_with_buffers(&mut ctx_pooled, [0, 0, 0], [2, 2, 2], &mut storage)
                .unwrap_err(),
            SampleError::ZeroGradientPeriod
        );
        for (buf, cap) in storage.iter().zip(capacities) {
            assert!(buf.capacity() >= cap);
        }
    }
}
