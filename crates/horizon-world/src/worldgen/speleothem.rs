//! Small pointed formations, base patches and contained-water placement.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    block_predicate::PredicateBlockResolver,
    feature::FeatureWorld,
    number_text, optional,
    placement::{PlacementContext, PlacementQueryError},
    required,
};
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::Direction,
    random::{InvalidBound, RandomSource},
};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct SpeleothemFeature {
    base: BlockStateId,
    base_states: Arc<BTreeSet<BlockStateId>>,
    replace: Arc<BTreeSet<BlockStateId>>,
    empty_or_water: BTreeSet<BlockStateId>,
    // Direction (up/down), thickness (tip/frustum), waterlogged. Unsupported
    // properties remain deferred placement failures, as in the native codec.
    pointed: [[Option<[Option<BlockStateId>; 2]>; 2]; 2],
    chances: [f32; 4],
}
#[derive(Debug)]
pub enum SpeleothemError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
    InvalidPointedState,
}
impl<E: std::fmt::Display> std::fmt::Display for SpeleothemError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::InvalidPointedState => f.write_str("pointed block lacks required properties"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for SpeleothemError<E> {}
impl SpeleothemFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let registry = bindings.registry();
        let block = |field| -> Result<BlockStateId, DecodeError> {
            let state = bindings
                .codec
                .decode(required(&object, field)?.get(), false)
                .map_err(|e| DecodeError::Registry(e.to_string()))?;
            registry
                .default_state(&registry.state(state).unwrap().name)
                .ok_or(DecodeError::Field("speleothem block identity"))
        };
        let base = block("base_block")?;
        let point = registry.state(block("pointed_block")?).unwrap();
        let mut pointed = [[None; 2]; 2];
        for (direction, row) in ["up", "down"].into_iter().zip(&mut pointed) {
            for (thickness, entry) in ["tip", "frustum"].into_iter().zip(row) {
                let mut state = point.clone();
                state
                    .properties
                    .insert("vertical_direction".into(), direction.into());
                state
                    .properties
                    .insert("thickness".into(), thickness.into());
                if registry.id(&state).is_some() {
                    *entry = Some([false, true].map(|water| {
                        let mut wet = state.clone();
                        wet.properties
                            .insert("waterlogged".into(), water.to_string());
                        registry.id(&wet)
                    }));
                }
            }
        }
        let mut chances = [0.2, 0.7, 0.5, 0.5];
        for (name, chance) in [
            "chance_of_taller_generation",
            "chance_of_directional_spread",
            "chance_of_spread_radius2",
            "chance_of_spread_radius3",
        ]
        .into_iter()
        .zip(&mut chances)
        {
            if let Some(raw) = optional(&object, name) {
                *chance = number_text(raw)?
                    .parse::<f64>()
                    .map_err(|_| DecodeError::Field("speleothem chance"))?
                    as f32;
                if !(0.0..=1.0).contains(chance) || chance.is_sign_negative() {
                    return Err(DecodeError::Field("speleothem chance"));
                }
            }
        }
        let mut empty_or_water = BTreeSet::from(bindings.air);
        empty_or_water.extend(
            bindings
                .blocks
                .get("minecraft:water")
                .ok_or(DecodeError::Field("water bindings"))?
                .iter()
                .copied(),
        );
        Ok(Self {
            base,
            base_states: bindings.blocks[&registry.state(base).unwrap().name].clone(),
            replace: bindings
                .predicate_block_set(required(&object, "replaceable_blocks")?.get())?,
            empty_or_water,
            pointed,
            chances,
        })
    }
    fn is_base(&self, state: BlockStateId) -> bool {
        self.base_states.contains(&state) || self.replace.contains(&state)
    }
    fn base_at<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<(), SpeleothemError<W::Error>> {
        if self
            .replace
            .contains(&world.block(p).map_err(SpeleothemError::World)?)
        {
            world
                .set_block(p, self.base, 2)
                .map_err(SpeleothemError::World)?;
        }
        Ok(())
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, SpeleothemError<W::Error>> {
        let ceiling = self.is_base(
            world
                .block(offset(origin, [0, 1, 0]))
                .map_err(SpeleothemError::World)?,
        );
        let floor = self.is_base(
            world
                .block(offset(origin, [0, -1, 0]))
                .map_err(SpeleothemError::World)?,
        );
        let direction = match (ceiling, floor) {
            (true, true) => {
                if random.next_boolean() {
                    Direction::Down
                } else {
                    Direction::Up
                }
            }
            (true, false) => Direction::Down,
            (false, true) => Direction::Up,
            (false, false) => return Ok(false),
        };
        let step = direction.normal();
        let base = offset(origin, step.map(|v| -v));
        self.base_at(world, base)?;
        for d in [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]] {
            if random.next_float() > self.chances[1] {
                continue;
            }
            let mut p = offset(base, d);
            self.base_at(world, p)?;
            for chance in &self.chances[2..] {
                if random.next_float() > *chance {
                    break;
                }
                let d = Direction::ALL
                    [random.next_int_bound(6).map_err(SpeleothemError::Random)? as usize];
                p = offset(p, d.normal());
                self.base_at(world, p)?;
            }
        }
        let tall = random.next_float() < self.chances[0]
            && self.empty_or_water.contains(
                &world
                    .block(offset(origin, step))
                    .map_err(SpeleothemError::World)?,
            );
        if self.is_base(world.block(base).map_err(SpeleothemError::World)?) {
            let row = &self.pointed[usize::from(direction == Direction::Down)];
            let mut p = origin;
            for thickness in if tall { &[1, 0][..] } else { &[0][..] } {
                let variants = row[*thickness].ok_or(SpeleothemError::InvalidPointedState)?;
                let water = world.water_at(p).map_err(SpeleothemError::Query)?;
                let state =
                    variants[usize::from(water)].ok_or(SpeleothemError::InvalidPointedState)?;
                world
                    .set_block(p, state, 2)
                    .map_err(SpeleothemError::World)?;
                p = offset(p, step);
            }
        }
        Ok(true)
    }
}
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
