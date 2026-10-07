//! Scalar distributions used by cave/canyon definitions. Gaussian providers
//! remain separate work; this module does not yet cover every provider.
use super::weighted::WeightedList;
use super::{DecodeError, Object, density::identifier, integer, number_text, required};
use horizon_core::random::{InvalidBound, RandomSource};
use serde_json::value::RawValue;

#[derive(Clone, Debug)]
pub enum FloatProvider {
    Constant(f32),
    Uniform {
        min: f32,
        max: f32,
    },
    Trapezoid {
        min: f32,
        max: f32,
        plateau: f32,
    },
    ClampedNormal {
        mean: f32,
        deviation: f32,
        min: f32,
        max: f32,
    },
}
impl FloatProvider {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let raw: Box<RawValue> = serde_json::from_str(json)?;
        if let Ok(value) = float(&raw) {
            return Ok(Self::Constant(if value == 0. { 0. } else { value }));
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let field = |name| required(&object, name).and_then(float);
        match kind(&object)?.as_str() {
            "minecraft:constant" => {
                let value = field("value")?;
                Ok(Self::Constant(if value == 0. { 0. } else { value }))
            }
            "minecraft:uniform" => {
                let (min, max) = (field("min_inclusive")?, field("max_exclusive")?);
                if max <= min {
                    return Err(DecodeError::Field("uniform float bounds"));
                }
                Ok(Self::Uniform { min, max })
            }
            "minecraft:trapezoid" => {
                let (min, max, plateau) = (field("min")?, field("max")?, field("plateau")?);
                if max < min || plateau > max - min {
                    return Err(DecodeError::Field("trapezoid float bounds"));
                }
                Ok(Self::Trapezoid { min, max, plateau })
            }
            "minecraft:clamped_normal" => {
                let (min, max) = (field("min")?, field("max")?);
                if max < min {
                    return Err(DecodeError::Field("clamped normal float bounds"));
                }
                Ok(Self::ClampedNormal {
                    mean: field("mean")?,
                    deviation: field("deviation")?,
                    min,
                    max,
                })
            }
            _ => Err(DecodeError::Field("float provider type")),
        }
    }
    pub fn bounds(&self) -> [f32; 2] {
        match *self {
            Self::Constant(value) => [value; 2],
            Self::Uniform { min, max }
            | Self::Trapezoid { min, max, .. }
            | Self::ClampedNormal { min, max, .. } => [min, max],
        }
    }
    pub fn sample(&self, random: &mut impl RandomSource) -> f32 {
        match *self {
            Self::Constant(value) => value,
            Self::Uniform { min, max } => random.next_float() * (max - min) + min,
            Self::Trapezoid { min, max, plateau } => {
                let range = max - min;
                let slope = (range - plateau) / 2.;
                let middle = range - slope;
                (min + random.next_float() * middle) + random.next_float() * slope
            }
            Self::ClampedNormal {
                mean,
                deviation,
                min,
                max,
            } => {
                let value = mean + random.next_gaussian() as f32 * deviation;
                if value < min { min } else { value.min(max) }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum IntProvider {
    Weighted(WeightedList<Self>),
    Constant(i32),
    Uniform {
        min: i32,
        max: i32,
    },
    BiasedToBottom {
        min: i32,
        max: i32,
    },
    VeryBiasedToBottom {
        min: i32,
        max: i32,
    },
    Trapezoid {
        min: i32,
        max: i32,
        plateau: i32,
    },
    Clamped {
        source: Box<Self>,
        min: i32,
        max: i32,
    },
    ClampedNormal {
        mean: f32,
        deviation: f32,
        min: i32,
        max: i32,
    },
}
impl IntProvider {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        let raw: Box<RawValue> = serde_json::from_str(json)?;
        Self::decode(&raw, 0, &mut 65536)
    }
    pub(crate) fn decode(
        raw: &RawValue,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        if depth > 256 || *remaining == 0 {
            return Err(DecodeError::Field("integer provider depth"));
        }
        *remaining -= 1;
        if let Ok(value) = integer(raw) {
            return Ok(Self::Constant(value));
        }
        let object: Object = serde_json::from_str(raw.get())?;
        let kind = kind(&object)?;
        let field = |name| required(&object, name).and_then(integer);
        if kind == "minecraft:constant" {
            return Ok(Self::Constant(field("value")?));
        }
        if kind == "minecraft:weighted_list" {
            return Ok(Self::Weighted(WeightedList::decode(
                required(&object, "distribution")?,
                |raw| Self::decode(raw, depth + 1, remaining),
            )?));
        }
        let trapezoid = kind == "minecraft:trapezoid";
        let min = field(if trapezoid { "min" } else { "min_inclusive" })?;
        let max = field(if trapezoid { "max" } else { "max_inclusive" })?;
        if max < min {
            return Err(DecodeError::Field("integer provider bounds"));
        }
        match kind.as_str() {
            "minecraft:uniform" => Ok(Self::Uniform { min, max }),
            "minecraft:biased_to_bottom" => Ok(Self::BiasedToBottom { min, max }),
            "minecraft:very_biased_to_bottom" => Ok(Self::VeryBiasedToBottom { min, max }),
            "minecraft:trapezoid" => {
                let plateau = field("plateau")?;
                if plateau > max.wrapping_sub(min) {
                    return Err(DecodeError::Field("integer provider plateau"));
                }
                Ok(Self::Trapezoid { min, max, plateau })
            }
            "minecraft:clamped_normal" => Ok(Self::ClampedNormal {
                mean: required(&object, "mean").and_then(float)?,
                deviation: required(&object, "deviation").and_then(float)?,
                min,
                max,
            }),
            "minecraft:clamped" => Ok(Self::Clamped {
                source: Box::new(Self::decode(
                    required(&object, "source")?,
                    depth + 1,
                    remaining,
                )?),
                min,
                max,
            }),
            _ => Err(DecodeError::Field("integer provider type")),
        }
    }
    pub fn bounds(&self) -> [i32; 2] {
        match self {
            Self::Weighted(list) => {
                list.entries()
                    .iter()
                    .fold([i32::MAX, i32::MIN], |bounds, (provider, _)| {
                        let next = provider.bounds();
                        [bounds[0].min(next[0]), bounds[1].max(next[1])]
                    })
            }
            Self::Constant(value) => [*value; 2],
            Self::Uniform { min, max }
            | Self::BiasedToBottom { min, max }
            | Self::VeryBiasedToBottom { min, max }
            | Self::Trapezoid { min, max, .. }
            | Self::ClampedNormal { min, max, .. } => [*min, *max],
            Self::Clamped { source, min, max } => {
                let bounds = source.bounds();
                [(*min).max(bounds[0]), (*max).min(bounds[1])]
            }
        }
    }
    pub fn sample(&self, random: &mut impl RandomSource) -> Result<i32, InvalidBound> {
        match self {
            Self::Weighted(list) => list.sample(random)?.sample(random),
            Self::Constant(value) => Ok(*value),
            Self::Uniform { min, max } => random.next_int_between_inclusive(*min, *max),
            Self::BiasedToBottom { min, max } | Self::VeryBiasedToBottom { min, max } => {
                let mut value = random.next_int_bound(max.wrapping_sub(*min).wrapping_add(1))?;
                value = random.next_int_bound(value.wrapping_add(1))?;
                if matches!(self, Self::VeryBiasedToBottom { .. }) {
                    value = random.next_int_bound(value.wrapping_add(1))?;
                }
                Ok(min.wrapping_add(value))
            }
            Self::Trapezoid { min, max, plateau } => {
                if *plateau == 0 && *max == min.wrapping_neg() {
                    let left = random.next_int_bound(max.wrapping_add(1))?;
                    let right = random.next_int_bound(max.wrapping_add(1))?;
                    return Ok(left.wrapping_sub(right));
                }
                let range = max.wrapping_sub(*min);
                if *plateau == range {
                    return random.next_int_between_inclusive(*min, *max);
                }
                let slope = range.wrapping_sub(*plateau) / 2;
                let middle = range.wrapping_sub(slope);
                let left = random.next_int_between_inclusive(0, middle)?;
                let right = random.next_int_between_inclusive(0, slope)?;
                Ok(min.wrapping_add(left).wrapping_add(right))
            }
            Self::Clamped { source, min, max } => {
                let value = source.sample(random)?;
                Ok(if value < *min { *min } else { value.min(*max) })
            }
            // Mth.normal, then float Mth.clamp and a saturating int cast.
            Self::ClampedNormal {
                mean,
                deviation,
                min,
                max,
            } => {
                let value = mean + random.next_gaussian() as f32 * deviation;
                let (min, max) = (*min as f32, *max as f32);
                Ok((if value < min { min } else { value.min(max) }) as i32)
            }
        }
    }
}
fn float(raw: &RawValue) -> Result<f32, DecodeError> {
    number_text(raw)?
        .parse()
        .map_err(|_| DecodeError::Field("float provider number"))
}
fn kind(object: &Object) -> Result<String, DecodeError> {
    identifier(&serde_json::from_str::<String>(
        required(object, "type")?.get(),
    )?)
}
