use super::{StateProviderBindings, StateProviderError};
use crate::registry::BlockStateId;
use crate::worldgen::{
    DecodeError, Object, decode_noise, int_range, integer, number_text, required,
};
use horizon_core::{
    noise::{Noise, NoiseStack},
    random::{LegacyRandom, RandomSource, WorldgenRandom},
};
use serde_json::value::RawValue;
#[derive(Clone, Debug)]
pub(super) struct NoiseProvider {
    noise: NoiseStack,
    scale: f32,
    choice: Choice,
}
#[derive(Clone, Debug)]
enum Choice {
    States(Vec<BlockStateId>),
    Threshold {
        threshold: f32,
        chance: f32,
        default: BlockStateId,
        low: Vec<BlockStateId>,
        high: Vec<BlockStateId>,
    },
    Dual {
        states: Vec<BlockStateId>,
        variety: [i32; 2],
        slow: NoiseStack,
        scale: f32,
    },
}
impl NoiseProvider {
    pub fn decode(
        object: &Object,
        bindings: &StateProviderBindings<'_>,
        kind: &str,
    ) -> Result<Self, DecodeError> {
        let seed = long(required(object, "seed")?)?;
        let noise = decode_noise(required(object, "noise")?.get())?
            .create(&mut WorldgenRandom::new(LegacyRandom::new(seed)))?;
        let scale = positive(required(object, "scale")?)?;
        let choice = match kind {
            "minecraft:noise" => Choice::States(states(required(object, "states")?, bindings)?),
            "minecraft:noise_threshold" => Choice::Threshold {
                threshold: range(required(object, "threshold")?, -1., 1.)?,
                chance: range(required(object, "high_chance")?, 0., 1.)?,
                default: bindings.state(required(object, "default_state")?.get(), false)?,
                low: states(required(object, "low_states")?, bindings)?,
                high: states(required(object, "high_states")?, bindings)?,
            },
            "minecraft:dual_noise" => Choice::Dual {
                states: states(required(object, "states")?, bindings)?,
                variety: variety(required(object, "variety")?)?,
                slow: decode_noise(required(object, "slow_noise")?.get())?
                    .create(&mut WorldgenRandom::new(LegacyRandom::new(seed)))?,
                scale: positive(required(object, "slow_scale")?)?,
            },
            _ => return Err(DecodeError::Field("noise state provider type")),
        };
        Ok(Self {
            noise,
            scale,
            choice,
        })
    }
    pub fn sample<E>(
        &self,
        random: &mut impl RandomSource,
        p: [i32; 3],
    ) -> Result<BlockStateId, StateProviderError<E>> {
        let fast = || {
            self.noise.get3(
                f64::from(p[0]) * f64::from(self.scale),
                f64::from(p[1]) * f64::from(self.scale),
                f64::from(p[2]) * f64::from(self.scale),
            )
        };
        Ok(match &self.choice {
            Choice::States(states) => states[index(fast(), states.len())],
            Choice::Threshold {
                threshold,
                chance,
                default,
                low,
                high,
            } => {
                if fast() < *threshold {
                    low[random.next_int_bound(low.len() as i32)? as usize]
                } else if random.next_float() < *chance {
                    high[random.next_int_bound(high.len() as i32)? as usize]
                } else {
                    *default
                }
            }
            Choice::Dual {
                states,
                variety,
                slow,
                scale,
            } => {
                // The slow path intentionally multiplies at float precision.
                let sample = |p: [i32; 3]| {
                    slow.get3(
                        f64::from(p[0] as f32 * *scale),
                        f64::from(p[1] as f32 * *scale),
                        f64::from(p[2] as f32 * *scale),
                    )
                };
                let fraction = (f64::from(sample(p)) - -1.) / 2.;
                let low = f64::from(variety[0]);
                let high = f64::from(variety[1] + 1);
                let count = (if fraction < 0. {
                    low
                } else if fraction > 1. {
                    high
                } else {
                    low + fraction * (high - low)
                }) as i32;
                if count <= 0 {
                    return Err(StateProviderError::EmptyNoiseSelection);
                }
                // Every candidate is a pure noise query. Compute just the one
                // the final fast-noise lookup selects, avoiding a temporary list.
                let selected = index(fast(), count as usize) as i32;
                let offset = [
                    p[0].wrapping_add(selected.wrapping_mul(54545)),
                    p[1],
                    p[2].wrapping_add(selected.wrapping_mul(34234)),
                ];
                states[index(sample(offset), states.len())]
            }
        })
    }
}
fn index(noise: f32, count: usize) -> usize {
    (((1.0f32 + noise) / 2.0f32).clamp(0., 0.9999) * count as f32) as usize
}
fn states(
    raw: &RawValue,
    bindings: &StateProviderBindings<'_>,
) -> Result<Vec<BlockStateId>, DecodeError> {
    let values: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
    if values.is_empty() || values.len() > 8192 {
        return Err(DecodeError::Field("noise provider states"));
    }
    values
        .iter()
        .map(|v| bindings.state(v.get(), false))
        .collect()
}
fn float(raw: &RawValue) -> Result<f32, DecodeError> {
    number_text(raw)?
        .parse()
        .map_err(|_| DecodeError::Field("provider float"))
}
fn positive(raw: &RawValue) -> Result<f32, DecodeError> {
    let value = float(raw)?;
    if value > 0. && value <= f32::MAX {
        Ok(value)
    } else {
        Err(DecodeError::Field("positive provider scale"))
    }
}
fn range(raw: &RawValue, low: f32, high: f32) -> Result<f32, DecodeError> {
    let value = float(raw)?;
    if value.total_cmp(&low).is_ge() && value.total_cmp(&high).is_le() {
        Ok(value)
    } else {
        Err(DecodeError::Field("provider float range"))
    }
}
fn variety(raw: &RawValue) -> Result<[i32; 2], DecodeError> {
    let range = if let Ok(value) = integer(raw) {
        [value, value]
    } else if let Ok(values) = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) {
        if values.len() != 2 {
            return Err(DecodeError::Field("provider variety"));
        }
        [integer(&values[0])?, integer(&values[1])?]
    } else {
        let object: Object = serde_json::from_str(raw.get())?;
        [
            int_range(required(&object, "min_inclusive")?, "variety", 1, 64)?,
            int_range(required(&object, "max_inclusive")?, "variety", 1, 64)?,
        ]
    };
    if range[0] < 1 || range[1] > 64 || range[0] > range[1] {
        Err(DecodeError::Field("provider variety"))
    } else {
        Ok(range)
    }
}
// Gson's LONG codec truncates decimal tokens and retains the low 64 bits.
fn long(raw: &RawValue) -> Result<i64, DecodeError> {
    let text = number_text(raw)?;
    let negative = text.starts_with('-');
    let text = text.strip_prefix('-').unwrap_or(text);
    let (mantissa, exponent) = match text.split_once(['e', 'E']) {
        Some((m, e)) => (
            m,
            e.parse::<i32>()
                .map_err(|_| DecodeError::Field("long exponent"))? as i64,
        ),
        None => (text, 0),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let length = whole.len() as i64 + exponent;
    if length <= 0 {
        return Ok(0);
    }
    let zeroes = (length - (whole.len() + fraction.len()) as i64).max(0);
    if zeroes >= 64 {
        return Ok(0);
    }
    let mut result = 0u64;
    for digit in whole.bytes().chain(fraction.bytes()).take(length as usize) {
        result = result
            .wrapping_mul(10)
            .wrapping_add(u64::from(digit - b'0'));
    }
    for _ in 0..zeroes {
        result = result.wrapping_mul(10);
    }
    Ok(if negative {
        result.wrapping_neg()
    } else {
        result
    } as i64)
}
