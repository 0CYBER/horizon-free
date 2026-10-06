//! Ore placement with ordered replacement rules and air-exposure checks.
use super::{
    DecodeError, Object,
    block_feature::{BlockReplacements, FeatureBlockBindings, decode_targets},
    density::identifier,
    feature::{FeatureError, FeatureWorld},
    int_range, number_text, required,
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, random::RandomSource};

#[derive(Clone, Debug)]
pub struct ScatteredOreFeature {
    targets: BlockReplacements,
    size: i32,
    discard: f32,
    air: [BlockStateId; 3],
}
impl ScatteredOreFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        Self::decode(json, bindings, 0, &mut 65536)
    }
    pub(crate) fn decode(
        json: &str,
        bindings: &FeatureBlockBindings<'_>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let kind = identifier(&serde_json::from_str::<String>(
            required(&object, "type")?.get(),
        )?)?;
        if kind != "minecraft:scattered_ore" {
            return Err(DecodeError::Field("scattered ore type"));
        }
        let targets = decode_targets(&object, bindings, depth, remaining)?;
        let size = int_range(required(&object, "size")?, "ore size", 0, 64)?;
        let discard: f32 = number_text(required(&object, "discard_chance_on_air_exposure")?)?
            .parse()
            .map_err(|_| DecodeError::Field("ore air exposure"))?;
        if !(0.0..=1.0).contains(&discard) || discard.is_sign_negative() {
            return Err(DecodeError::Field("ore air exposure"));
        }
        Ok(Self {
            targets,
            size,
            discard,
            air: bindings.air,
        })
    }
    pub fn place<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, FeatureError<W::Error>> {
        let count = random
            .next_int_bound(self.size + 1)
            .map_err(FeatureError::Random)?;
        for attempt in 0..count {
            let spread = attempt.min(7) as f32;
            let position = origin.map(|component| {
                let offset = (random.next_float() - random.next_float()) * spread;
                // Widen before adding 0.5 so a float just below a tie cannot
                // round up during addition. Values are bounded by +/-7.
                let offset = (f64::from(offset) + 0.5).floor() as i32;
                component.wrapping_add(offset)
            });
            let current = world.block(position).map_err(FeatureError::World)?;
            for (rule, state) in &self.targets {
                if !rule.test(current, position, random) {
                    continue;
                }
                let skip_air = self.discard <= 0.0
                    || (self.discard < 1.0 && random.next_float() >= self.discard);
                if !skip_air {
                    let mut exposed = false;
                    for direction in Direction::ALL {
                        let delta = direction.normal();
                        let neighbor = std::array::from_fn(|i| position[i].wrapping_add(delta[i]));
                        if self
                            .air
                            .contains(&world.block(neighbor).map_err(FeatureError::World)?)
                        {
                            exposed = true;
                            break;
                        }
                    }
                    if exposed {
                        continue;
                    }
                }
                world
                    .set_block(position, *state, 2)
                    .map_err(FeatureError::World)?;
                break;
            }
        }
        Ok(true)
    }
}
