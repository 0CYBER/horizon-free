//! Bounded recursive chorus generation with native connection-state refreshes.
use super::{DecodeError, block_feature::FeatureBlockBindings, feature::FeatureWorld};
use crate::registry::BlockStateId;
use horizon_core::random::{InvalidBound, RandomSource};
use std::{collections::BTreeSet, sync::Arc};
const HORIZONTAL: [[i32; 3]; 4] = [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]];
const NEIGHBORS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [1, 0, 0],
    [0, 0, 1],
    [-1, 0, 0],
];
fn offset(p: [i32; 3], d: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|i| p[i].wrapping_add(d[i]))
}
#[derive(Clone, Debug)]
pub struct ChorusPlantFeature {
    air: [BlockStateId; 3],
    support: Arc<BTreeSet<BlockStateId>>,
    connectors: BTreeSet<BlockStateId>,
    plant: [BlockStateId; 64],
    flower: BlockStateId,
}
#[derive(Debug)]
pub enum ChorusPlantError<E> {
    World(E),
    Random(InvalidBound),
}
impl<E: std::fmt::Display> std::fmt::Display for ChorusPlantError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ChorusPlantError<E> {}
impl ChorusPlantFeature {
    pub fn new(bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let registry = bindings.registry();
        let get = |name| {
            registry
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown chorus block {name}")))
        };
        let default = get("minecraft:chorus_plant")?;
        let mut plant = [default; 64];
        for (mask, id) in plant.iter_mut().enumerate() {
            let mut state = registry.state(default).unwrap().clone();
            for (i, name) in ["down", "up", "north", "east", "south", "west"]
                .into_iter()
                .enumerate()
            {
                state
                    .properties
                    .insert(name.into(), (mask & (1 << i) != 0).to_string());
            }
            *id = registry
                .id(&state)
                .ok_or(DecodeError::Field("chorus connections"))?;
        }
        let mut flower = registry
            .state(get("minecraft:chorus_flower")?)
            .unwrap()
            .clone();
        flower.properties.insert("age".into(), "5".into());
        let flower = registry
            .id(&flower)
            .ok_or(DecodeError::Field("chorus flower age"))?;
        let mut connectors = BTreeSet::new();
        for name in ["minecraft:chorus_plant", "minecraft:chorus_flower"] {
            connectors.extend(
                bindings
                    .blocks
                    .get(name)
                    .ok_or(DecodeError::Field("chorus identity"))?
                    .iter()
                    .copied(),
            );
        }
        Ok(Self {
            air: bindings.air,
            support: bindings
                .tags
                .get("minecraft:supports_chorus_plant")
                .cloned()
                .unwrap_or_default(),
            connectors,
            plant,
            flower,
        })
    }
    pub fn place<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, ChorusPlantError<W::Error>> {
        if !self.empty(world, origin)?
            || !self.support.contains(
                &world
                    .block(offset(origin, [0, -1, 0]))
                    .map_err(ChorusPlantError::World)?,
            )
        {
            return Ok(false);
        }
        self.connect(world, origin)?;
        self.grow(world, random, origin, origin, 0)?;
        Ok(true)
    }
    fn empty<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<bool, ChorusPlantError<W::Error>> {
        Ok(self
            .air
            .contains(&world.block(p).map_err(ChorusPlantError::World)?))
    }
    fn neighbors_empty<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
        except: Option<usize>,
    ) -> Result<bool, ChorusPlantError<W::Error>> {
        for (i, d) in HORIZONTAL.into_iter().enumerate() {
            if except != Some(i) && !self.empty(world, offset(p, d))? {
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn connect<W: FeatureWorld>(
        &self,
        world: &mut W,
        p: [i32; 3],
    ) -> Result<(), ChorusPlantError<W::Error>> {
        let mut mask = 0;
        for (i, d) in NEIGHBORS.into_iter().enumerate() {
            let state = world.block(offset(p, d)).map_err(ChorusPlantError::World)?;
            if self.connectors.contains(&state) || i == 0 && self.support.contains(&state) {
                mask |= 1 << i;
            }
        }
        world
            .set_block(p, self.plant[mask], 2)
            .map_err(ChorusPlantError::World)?;
        Ok(())
    }
    fn grow<W: FeatureWorld>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        p: [i32; 3],
        root: [i32; 3],
        depth: u8,
    ) -> Result<(), ChorusPlantError<W::Error>> {
        let height = random.next_int_bound(4).map_err(ChorusPlantError::Random)?
            + 1
            + if depth == 0 { 1 } else { 0 };
        for step in 1..=height {
            let above = offset(p, [0, step, 0]);
            if !self.neighbors_empty(world, above, None)? {
                return Ok(());
            }
            self.connect(world, above)?;
            self.connect(world, offset(above, [0, -1, 0]))?;
        }
        let mut branched = false;
        if depth < 4 {
            let attempts = random.next_int_bound(4).map_err(ChorusPlantError::Random)?
                + if depth == 0 { 1 } else { 0 };
            for _ in 0..attempts {
                let direction =
                    random.next_int_bound(4).map_err(ChorusPlantError::Random)? as usize;
                let branch = offset(offset(p, [0, height, 0]), HORIZONTAL[direction]);
                if branch[0].wrapping_sub(root[0]).wrapping_abs() >= 8
                    || branch[2].wrapping_sub(root[2]).wrapping_abs() >= 8
                {
                    continue;
                }
                let opposite = (direction + 2) % 4;
                if self.empty(world, branch)?
                    && self.empty(world, offset(branch, [0, -1, 0]))?
                    && self.neighbors_empty(world, branch, Some(opposite))?
                {
                    branched = true;
                    self.connect(world, branch)?;
                    self.connect(world, offset(branch, HORIZONTAL[opposite]))?;
                    self.grow(world, random, branch, root, depth + 1)?;
                }
            }
        }
        if !branched {
            world
                .set_block(offset(p, [0, height, 0]), self.flower, 2)
                .map_err(ChorusPlantError::World)?;
        }
        Ok(())
    }
}
