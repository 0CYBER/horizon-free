use super::*;
use crate::{
    noise::{NoiseBuildError, NoiseStack, SmearedPerlinNoise},
    random::RandomSource,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftNoiseKind {
    Shift,
    ShiftA,
    ShiftB,
}

/// Compile the three offset-noise variants without changing their bulk paths.
pub fn compile_shift_noise(noise: Arc<dyn Noise>, kind: ShiftNoiseKind) -> Arc<dyn DensitySampler> {
    if kind == ShiftNoiseKind::ShiftB {
        return Arc::new(ShiftBSampler { noise });
    }
    Arc::new(UnarySampler {
        input: Arc::new(NoiseSampler {
            noise,
            xz_scale: 0.25,
            y_scale: if kind == ShiftNoiseKind::ShiftA {
                0.0
            } else {
                0.25
            },
        }),
        operation: UnaryOp::Multiply(4.0),
    })
}
#[derive(Clone, Debug)]
struct ShiftBSampler {
    noise: Arc<dyn Noise>,
}
impl DensitySampler for ShiftBSampler {
    fn sample_value(
        &self,
        _: &mut SamplerContext,
        [x, _, z]: [i32; 3],
    ) -> Result<f32, SampleError> {
        Ok(self.noise.get3(z as f64 * 0.25, x as f64 * 0.25, 0.0) * 4.0)
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        let [sx, sy, sz] = volume.sizes();
        let [mx, _, mz] = volume.minimum();
        let [dx, _, dz] = volume.steps();
        let transposed = DensityVolume::new([sz as i32, sx as i32, 1], [mz, mx, 0], [dz, dx, 1])?;
        context.with_buffer(transposed.size(), |_, plane| {
            plane.fill(0.0);
            self.noise
                .add_to_volume(plane, transposed, 0.25, 0.25, 4.0)?;
            for z in 0..sz {
                for x in 0..sx {
                    let start = (x + z * sx) * sy;
                    buffer[start..start + sy].fill(plane[x + z * sx]);
                }
            }
            Ok(())
        })
    }
}

/// Parameters for vanilla's old blended terrain density function. Serialized
/// codec limits and compile-context seed lookup are separate integration work.
#[derive(Clone, Copy, Debug)]
pub struct BlendedNoise {
    pub xz_scale: f64,
    pub y_scale: f64,
    pub xz_factor: f64,
    pub y_factor: f64,
    pub smear_scale_multiplier: f64,
}
#[derive(Clone, Debug)]
pub struct BlendedFbmSet {
    pub min_limit: NoiseStack,
    pub max_limit: NoiseStack,
    pub main: NoiseStack,
}
impl BlendedNoise {
    pub fn create_fbm_set(
        &self,
        random: &mut impl RandomSource,
    ) -> Result<BlendedFbmSet, NoiseBuildError> {
        let smear = (684.412 * self.y_scale) * self.smear_scale_multiplier;
        let main_smear = smear / self.y_factor;
        Ok(BlendedFbmSet {
            min_limit: create_blended_fbm(random, -15, smear, 0.9999847412109375)?,
            max_limit: create_blended_fbm(random, -15, smear, 0.9999847412109375)?,
            main: create_blended_fbm(random, -7, main_smear, 12.75)?,
        })
    }
    pub fn compile(&self, random: &mut impl RandomSource) -> Result<LerpSampler, NoiseBuildError> {
        let layers = self.create_fbm_set(random)?;
        let xz_scale = 684.412 * self.xz_scale;
        let y_scale = 684.412 * self.y_scale;
        let min_limit = Arc::new(NoiseSampler {
            noise: Arc::new(layers.min_limit),
            xz_scale,
            y_scale,
        });
        let max_limit = Arc::new(NoiseSampler {
            noise: Arc::new(layers.max_limit),
            xz_scale,
            y_scale,
        });
        let main = Arc::new(NoiseSampler {
            noise: Arc::new(layers.main),
            xz_scale: xz_scale / self.xz_factor,
            y_scale: y_scale / self.y_factor,
        });
        let shifted = Arc::new(UnarySampler {
            input: main,
            operation: UnaryOp::Add(0.5),
        });
        let alpha = Arc::new(UnarySampler {
            input: shifted,
            operation: UnaryOp::Clamp { min: 0.0, max: 1.0 },
        });
        Ok(LerpSampler {
            alpha,
            first: DensityValue::Sampler(min_limit),
            second: DensityValue::Sampler(max_limit),
        })
    }
    pub fn range(&self) -> Result<Interval, IntervalError> {
        let smear = (684.412 * self.y_scale) * self.smear_scale_multiplier;
        let mut range = Interval::exact(0.0)?;
        let mut frequency = 1.0;
        let mut amplitude = 0.9999847412109375 / 65535.0;
        for _ in 0..16 {
            let base = Interval::symmetric(((smear * frequency).abs() + 2.0) as f32)?;
            range = range.add_range(base.multiply_range(Interval::exact(amplitude as f32)?));
            frequency /= 2.0;
            amplitude *= 2.0;
        }
        Ok(range)
    }
}

/// Construct high-to-low frequency smeared octaves, consuming the caller's
/// stream directly. The public API caps work at 4096 layers before any draws.
pub fn create_blended_fbm(
    random: &mut impl RandomSource,
    first_octave: i32,
    smear: f64,
    amplitude: f64,
) -> Result<NoiseStack, NoiseBuildError> {
    if first_octave > 0 {
        return Err(NoiseBuildError::PositiveLegacyOctaves);
    }
    if first_octave < -4095 {
        return Err(NoiseBuildError::TooManyOctaves);
    }
    let count = 1 - first_octave;
    let power = if count > 1023 {
        f64::INFINITY
    } else {
        f64::from_bits(((count + 1023) as u64) << 52)
    };
    let mut amplitude = amplitude / (power - 1.0);
    let mut frequency = 1.0;
    let mut builder = NoiseStack::builder();
    for _ in 0..count {
        builder.add(
            Arc::new(SmearedPerlinNoise::new(random, smear * frequency)),
            frequency,
            amplitude as f32,
        );
        frequency /= 2.0;
        amplitude *= 2.0;
    }
    Ok(builder.build()?)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceMetric {
    Euclidean,
    EuclideanSquared,
    Manhattan,
    Chebyshev,
}
impl DistanceMetric {
    pub fn compute2(self, x: f32, y: f32) -> f32 {
        match self {
            Self::Euclidean => ((x * x + y * y) as f64).sqrt() as f32,
            Self::EuclideanSquared => x * x + y * y,
            Self::Manhattan => x.abs() + y.abs(),
            Self::Chebyshev => arithmetic::java_max(x.abs(), y.abs()),
        }
    }
    pub fn compute3(self, x: f32, y: f32, z: f32) -> f32 {
        match self {
            Self::Euclidean => ((x * x + y * y + z * z) as f64).sqrt() as f32,
            Self::EuclideanSquared => x * x + y * y + z * z,
            Self::Manhattan => x.abs() + y.abs() + z.abs(),
            Self::Chebyshev => {
                arithmetic::java_max(arithmetic::java_max(x.abs(), y.abs()), z.abs())
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DistanceToPointSampler {
    pub point: [i32; 3],
    pub metric: DistanceMetric,
}
impl DensitySampler for DistanceToPointSampler {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        let delta: [f32; 3] =
            std::array::from_fn(|i| self.point[i].wrapping_sub(position[i]) as f32);
        Ok(self.metric.compute3(delta[0], delta[1], delta[2]))
    }
}
