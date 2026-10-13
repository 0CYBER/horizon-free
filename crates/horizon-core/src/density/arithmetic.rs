use super::*;

pub(super) fn java_power(base: f64, exponent: f64) -> f64 {
    // Native libm can propagate a signaling NaN base even for exponent zero;
    // Java returns one for every base in this case.
    if exponent == 0.0 {
        1.0
    } else if exponent.is_nan() || (base.abs() == 1.0 && exponent.is_infinite()) {
        f64::NAN
    } else {
        crate::jmath::pow(base, exponent)
    }
}

pub(super) fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else if a <= b {
        a
    } else {
        b
    }
}
pub(super) fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else if a >= b {
        a
    } else {
        b
    }
}
fn clamp(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else {
        java_min(value, max)
    }
}

/// Selected compiled unary samplers, including constant-operand specializations.
/// Graph compilation is implemented separately.
#[derive(Clone, Copy, Debug)]
pub enum UnaryOp {
    Abs,
    Negate,
    Square,
    Cube,
    Sqrt,
    Log,
    PowerBase(f64),
    PowerExponent(f64),
    Reciprocal,
    Sign,
    Squeeze,
    LeakyRelu(f32),
    Add(f32),
    SubtractFrom(f32),
    Multiply(f32),
    DivideInto(f32),
    Minimum(f32),
    Maximum(f32),
    ConstantRangeChoice {
        min_inclusive: f32,
        max_exclusive: f32,
        inside: f32,
        outside: f32,
    },
    Clamp {
        min: f32,
        max: f32,
    },
}
impl UnaryOp {
    pub(super) fn apply(self, value: f32, bulk: bool) -> f32 {
        match self {
            Self::Abs => value.abs(),
            Self::Negate => -value,
            Self::Square => value * value,
            Self::Cube => value * value * value,
            Self::Sqrt => (value as f64).sqrt() as f32,
            Self::Log => crate::jmath::log(value as f64) as f32,
            Self::PowerBase(base) => java_power(base, value as f64) as f32,
            Self::PowerExponent(exponent) => java_power(value as f64, exponent) as f32,
            Self::Reciprocal => 1.0 / value,
            Self::Sign => {
                if value == 0.0 || value.is_nan() {
                    value
                } else {
                    value.signum()
                }
            }
            Self::Squeeze => {
                let v = clamp(value, -1.0, 1.0);
                v / 2.0 - v * v * v / 24.0
            }
            Self::LeakyRelu(factor) => {
                if value > 0.0 {
                    value
                } else {
                    value * factor
                }
            }
            Self::Add(right) => value + right,
            Self::SubtractFrom(left) => left - value,
            Self::Multiply(right) => value * right,
            Self::DivideInto(left) => left / value,
            Self::Minimum(right) => {
                if bulk {
                    if right < value { right } else { value }
                } else {
                    java_min(value, right)
                }
            }
            Self::Maximum(right) => {
                if bulk {
                    if right > value { right } else { value }
                } else {
                    java_max(value, right)
                }
            }
            Self::Clamp { min, max } => clamp(value, min, max),
            Self::ConstantRangeChoice {
                min_inclusive,
                max_exclusive,
                inside,
                outside,
            } => {
                if value >= min_inclusive && value < max_exclusive {
                    inside
                } else {
                    outside
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct UnarySampler {
    pub input: Arc<dyn DensitySampler>,
    pub operation: UnaryOp,
}
impl DensitySampler for UnarySampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        Ok(self
            .operation
            .apply(self.input.sample_value(context, position)?, false))
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_buffer(buffer, volume)?;
        self.input.sample_volume(context, buffer, volume)?;
        for value in buffer {
            *value = self.operation.apply(*value, true);
        }
        Ok(())
    }
}

/// Range bounds are the values baked into vanilla's compiled min/max samplers.
/// Computing those bounds from a density graph is a separate compiler step.
#[derive(Clone, Copy, Debug)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Minimum { right_min: f32 },
    Maximum { right_max: f32 },
}
impl BinaryOp {
    fn apply(self, left: f32, right: f32, bulk: bool) -> f32 {
        match self {
            Self::Add => left + right,
            Self::Subtract => left - right,
            Self::Multiply => left * right,
            Self::Divide => left / right,
            Self::Minimum { .. } => {
                if bulk {
                    if right < left { right } else { left }
                } else {
                    java_min(left, right)
                }
            }
            Self::Maximum { .. } => {
                if bulk {
                    if right > left { right } else { left }
                } else {
                    java_max(left, right)
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct BinarySampler {
    pub left: Arc<dyn DensitySampler>,
    pub right: Arc<dyn DensitySampler>,
    pub operation: BinaryOp,
}
impl DensitySampler for BinarySampler {
    fn sample_value(
        &self,
        context: &mut SamplerContext,
        position: [i32; 3],
    ) -> Result<f32, SampleError> {
        let left = self.left.sample_value(context, position)?;
        match self.operation {
            BinaryOp::Multiply | BinaryOp::Divide if left == 0.0 => return Ok(0.0),
            BinaryOp::Minimum { right_min } if left <= right_min => return Ok(left),
            BinaryOp::Maximum { right_max } if left >= right_max => return Ok(left),
            _ => {}
        }
        Ok(self
            .operation
            .apply(left, self.right.sample_value(context, position)?, false))
    }
    fn sample_volume(
        &self,
        context: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        check_exact_buffer(buffer, volume)?;
        // Vanilla reads a volume-sized temporary through every output index.
        // Restrict this API to the valid equal-size case before invoking children.
        self.left.sample_volume(context, buffer, volume)?;
        context.with_buffer(volume.size(), |context, right| {
            self.right.sample_volume(context, right, volume)?;
            for (left, right) in buffer.iter_mut().zip(right) {
                *left = self.operation.apply(*left, *right, true);
            }
            Ok(())
        })
    }
}
