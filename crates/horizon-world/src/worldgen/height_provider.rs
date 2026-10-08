//! Anchored scalar height distributions used by cave and canyon generation.
//! Codec encoding remains separate work.
use super::{
    DecodeError, Object,
    anchor::{VerticalAnchor, WorldGenerationContext},
    density::identifier,
    int_range, integer, optional, required,
    weighted::WeightedList,
};
use horizon_core::random::{InvalidBound, RandomSource};

#[derive(Clone, Debug)]
pub enum HeightProvider {
    Weighted(WeightedList<Self>),
    Constant(VerticalAnchor),
    Uniform {
        min: VerticalAnchor,
        max: VerticalAnchor,
    },
    BiasedToBottom {
        min: VerticalAnchor,
        max: VerticalAnchor,
        inner: i32,
    },
    VeryBiasedToBottom {
        min: VerticalAnchor,
        max: VerticalAnchor,
        inner: i32,
    },
    Trapezoid {
        min: VerticalAnchor,
        max: VerticalAnchor,
        plateau: i32,
    },
}
impl HeightProvider {
    pub fn from_json(json: &str) -> Result<Self, DecodeError> {
        Self::decode(json, 0, &mut 65536)
    }
    fn decode(json: &str, depth: usize, remaining: &mut usize) -> Result<Self, DecodeError> {
        if depth > 256 || *remaining == 0 {
            return Err(DecodeError::Field("height provider depth or node count"));
        }
        *remaining -= 1;
        if let Ok(anchor) = VerticalAnchor::from_json(json) {
            return Ok(Self::Constant(anchor));
        }
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        let anchor = |name| VerticalAnchor::from_json(required(&object, name)?.get());
        if kind == "minecraft:constant" {
            return Ok(Self::Constant(anchor("value")?));
        }
        if kind == "minecraft:weighted_list" {
            return Ok(Self::Weighted(WeightedList::decode(
                required(&object, "distribution")?,
                |raw| Self::decode(raw.get(), depth + 1, remaining),
            )?));
        }
        let min = anchor("min_inclusive")?;
        let max = anchor("max_inclusive")?;
        match kind.as_str() {
            "minecraft:uniform" => Ok(Self::Uniform { min, max }),
            "minecraft:biased_to_bottom" | "minecraft:very_biased_to_bottom" => {
                let inner = optional(&object, "inner")
                    .map(|raw| int_range(raw, "inner", 1, i32::MAX))
                    .transpose()?
                    .unwrap_or(1);
                Ok(if kind == "minecraft:biased_to_bottom" {
                    Self::BiasedToBottom { min, max, inner }
                } else {
                    Self::VeryBiasedToBottom { min, max, inner }
                })
            }
            "minecraft:trapezoid" => Ok(Self::Trapezoid {
                min,
                max,
                plateau: optional(&object, "plateau")
                    .map(integer)
                    .transpose()?
                    .unwrap_or(0),
            }),
            _ => Err(DecodeError::Field("height provider type")),
        }
    }
    pub fn sample(
        &self,
        random: &mut impl RandomSource,
        context: WorldGenerationContext,
    ) -> Result<i32, InvalidBound> {
        let (min, max) = match self {
            Self::Weighted(list) => return list.sample(random)?.sample(random, context),
            Self::Constant(anchor) => return Ok(anchor.resolve(context)),
            Self::Uniform { min, max }
            | Self::BiasedToBottom { min, max, .. }
            | Self::VeryBiasedToBottom { min, max, .. }
            | Self::Trapezoid { min, max, .. } => (min.resolve(context), max.resolve(context)),
        };
        match self {
            Self::Constant(_) | Self::Weighted(_) => unreachable!(),
            Self::Uniform { .. } => {
                if min > max {
                    Ok(min)
                } else {
                    random.next_int_between_inclusive(min, max)
                }
            }
            Self::BiasedToBottom { inner, .. } | Self::VeryBiasedToBottom { inner, .. } => {
                let range = max.wrapping_sub(min).wrapping_sub(*inner).wrapping_add(1);
                if range <= 0 {
                    return Ok(min);
                }
                if matches!(self, Self::BiasedToBottom { .. }) {
                    let first = random.next_int_bound(range)?;
                    Ok(random
                        .next_int_bound(first.wrapping_add(*inner))?
                        .wrapping_add(min))
                } else {
                    let first = next_int(random, min.wrapping_add(*inner), max)?;
                    let second = next_int(random, min, first.wrapping_sub(1))?;
                    next_int(random, min, second.wrapping_sub(1).wrapping_add(*inner))
                }
            }
            Self::Trapezoid { plateau, .. } => {
                if min > max {
                    return Ok(min);
                }
                let range = max.wrapping_sub(min);
                if *plateau >= range {
                    return random.next_int_between_inclusive(min, max);
                }
                let slope = range.wrapping_sub(*plateau) / 2;
                let middle = range.wrapping_sub(slope);
                let left = random.next_int_between_inclusive(0, middle)?;
                let right = random.next_int_between_inclusive(0, slope)?;
                Ok(min.wrapping_add(left).wrapping_add(right))
            }
        }
    }
}
fn next_int(random: &mut impl RandomSource, min: i32, max: i32) -> Result<i32, InvalidBound> {
    if min >= max {
        Ok(min)
    } else {
        random.next_int_between_inclusive(min, max)
    }
}
