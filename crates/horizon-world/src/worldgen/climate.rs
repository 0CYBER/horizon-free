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
        let volume = DensityVolume::new(extents, quart_minimum.map(|v| v.wrapping_shl(2)), [4; 3])?;
        let mut fields = std::array::from_fn(|_| vec![0.; volume.size()]);
        for (sampler, field) in self.samplers.iter().zip(&mut fields) {
            sampler.sample_volume(context, field, volume)?;
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
    /// Coordinates must be inside the prepared quart volume. The bounded Rust
    /// API rejects invalid coordinates instead of Java's unchecked buffer alias.
    pub fn target(&self, quart: [i32; 3]) -> Option<TargetPoint> {
        let relative =
            std::array::from_fn(|i| quart[i].wrapping_sub(self.quart_minimum[i]) as usize);
        let index = self.volume.index(relative)?;
        Some(TargetPoint::from_values(std::array::from_fn(|i| {
            self.fields[i][index]
        })))
    }
}
