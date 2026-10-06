use super::*;

#[derive(Clone, Copy, Debug)]
pub enum RoundMode {
    Floor,
    Round,
    Ceil,
    Truncate,
}
impl RoundMode {
    pub(super) fn integer(self, value: f32) -> f32 {
        match self {
            Self::Floor => (value as f64).floor() as f32,
            Self::Ceil => (value as f64).ceil() as f32,
            Self::Truncate => {
                if value > 0.0 {
                    (value as f64).floor() as f32
                } else {
                    (value as f64).ceil() as f32
                }
            }
            Self::Round => {
                // Math.round(float): ties go toward positive infinity, NaNs
                // become zero, and the result saturates to Java's int domain.
                let bits = value.to_bits() as i32;
                let shift = 149 - ((bits >> 23) & 0xff);
                let rounded = if (0..32).contains(&shift) {
                    let mut significand = (bits & 0x7f_ffff) | 0x80_0000;
                    if bits < 0 {
                        significand = -significand;
                    }
                    ((significand >> shift) + 1) >> 1
                } else {
                    value as i32
                };
                rounded as f32
            }
        }
    }
    fn apply(self, input: f32, multiple: f32) -> f32 {
        if multiple == 0.0 {
            input
        } else {
            self.integer(input / multiple) * multiple
        }
    }
}

/// A missing multiple selects vanilla's literal-one specialization. A supplied
/// sampler is evaluated even when its value is zero or always one.
#[derive(Clone, Debug)]
pub struct RoundSampler {
    pub mode: RoundMode,
    pub input: Arc<dyn DensitySampler>,
    pub multiple: Option<Arc<dyn DensitySampler>>,
}
impl DensitySampler for RoundSampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let input = self.input.sample_value(context, position)?;
        Ok(if let Some(multiple) = &self.multiple {
            self.mode
                .apply(input, multiple.sample_value(context, position)?)
        } else {
            self.mode.integer(input)
        })
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        if self.multiple.is_some() {
            check_exact_buffer(buffer, volume)?;
        } else {
            check_buffer(buffer, volume)?;
        }
        self.input.sample_volume(context, buffer, volume)?;
        if let Some(multiple) = &self.multiple {
            context.with_buffer(volume.size(), |context, scratch| {
                multiple.sample_volume(context, scratch, volume)?;
                for (input, multiple) in buffer.iter_mut().zip(scratch) {
                    *input = self.mode.apply(*input, *multiple);
                }
                Ok(())
            })
        } else {
            for value in buffer {
                *value = self.mode.integer(*value);
            }
            Ok(())
        }
    }
}
