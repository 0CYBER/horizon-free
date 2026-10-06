use super::*;

/// Both operands evaluate in base/exponent order, including zero and one bases.
#[derive(Clone, Debug)]
pub struct PowerSampler {
    pub base: Arc<dyn DensitySampler>,
    pub exponent: Arc<dyn DensitySampler>,
}
impl DensitySampler for PowerSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let base = self.base.sample_value(context, position)?;
        let exponent = self.exponent.sample_value(context, position)?;
        Ok(super::arithmetic::java_power(base as f64, exponent as f64) as f32)
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_exact_buffer(buffer, volume)?;
        self.base.sample_volume(context, buffer, volume)?;
        context.with_buffer(volume.size(), |context, scratch| {
            self.exponent.sample_volume(context, scratch, volume)?;
            for (base, exponent) in buffer.iter_mut().zip(scratch) {
                *base = super::arithmetic::java_power(*base as f64, *exponent as f64) as f32;
            }
            Ok(())
        })
    }
}

/// Only sqrt/identity/square/cube receive the negative-exponent reciprocal.
/// Other exponents return directly from vanilla's signed fallback sampler.
pub fn compile_constant_exponent(
    base: Arc<dyn DensitySampler>,
    exponent: f32,
) -> Arc<dyn DensitySampler> {
    let positive: Arc<dyn DensitySampler> = match exponent.abs() {
        0.5 => Arc::new(UnarySampler {
            input: base,
            operation: UnaryOp::Sqrt,
        }),
        1.0 => base,
        2.0 => Arc::new(UnarySampler {
            input: base,
            operation: UnaryOp::Square,
        }),
        3.0 => Arc::new(UnarySampler {
            input: base,
            operation: UnaryOp::Cube,
        }),
        _ => {
            return Arc::new(UnarySampler {
                input: base,
                operation: UnaryOp::PowerExponent(exponent as f64),
            });
        }
    };
    if exponent < 0.0 {
        Arc::new(UnarySampler {
            input: positive,
            operation: UnaryOp::Reciprocal,
        })
    } else {
        positive
    }
}
