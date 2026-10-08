//! Block placement operations used by the simple-block world-generation feature.
use crate::{
    block_properties::{BlockPropertiesRegistry, BlockProperty, BoundBlockProperty},
    compression::invalid,
    fluid::FluidStateTable,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
    },
};
use horizon_core::geometry::Direction;
use std::{collections::BTreeSet, io, sync::Arc};

const HORIZONTAL: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];
const SIDE_NAMES: [&str; 4] = ["north", "east", "south", "west"];

#[derive(Clone, Copy, Debug)]
enum Placement {
    Single,
    Double(usize),
    Moss,
}
#[derive(Clone, Copy, Debug)]
struct DoublePlant {
    halves: [[BlockStateId; 2]; 2],
    waterlogged: bool,
}
#[derive(Clone, Copy, Debug)]
struct Moss {
    bottom: bool,
    sides: [u8; 4],
}
impl Moss {
    fn code(self) -> usize {
        self.sides
            .into_iter()
            .fold(usize::from(self.bottom), |code, side| {
                code * 3 + usize::from(side)
            })
    }
    fn has_faces(self) -> bool {
        self.bottom || self.sides.into_iter().any(|side| side != 0)
    }
}
#[derive(Debug)]
pub enum BlockPlacementError<E> {
    World(E),
    Registry(io::Error),
    Query(PlacementQueryError),
}
impl<E: std::fmt::Display> std::fmt::Display for BlockPlacementError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(error) => error.fmt(f),
            Self::Registry(error) => error.fmt(f),
            Self::Query(error) => error.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BlockPlacementError<E> {}

pub struct BlockPlacementBindings<'r> {
    registry: &'r BlockRegistry,
    operations: Arc<BlockPlacement>,
}
/// Precompiled transitions shared by decoded features in one registry snapshot.
#[derive(Debug)]
pub struct BlockPlacement {
    kinds: Vec<Placement>,
    doubles: Vec<DoublePlant>,
    replaceable: BoundBlockProperty,
    fluids: FluidStateTable,
    air: [BlockStateId; 3],
    defaults: Vec<BlockStateId>,
    moss: Vec<Option<Moss>>,
    moss_ids: [BlockStateId; 162],
    moss_default: BlockStateId,
}
impl<'r> BlockPlacementBindings<'r> {
    pub fn new(
        registry: &'r BlockRegistry,
        properties: &BlockPropertiesRegistry<'r>,
        double_plants: &BTreeSet<String>,
        mossy_carpets: &BTreeSet<String>,
    ) -> io::Result<Self> {
        if !std::ptr::eq(registry, properties.registry()) {
            return Err(invalid("placement properties use another registry"));
        }
        for name in double_plants.iter().chain(mossy_carpets) {
            if registry.default_state(name).is_none() {
                return Err(invalid("unknown placement definition"));
            }
        }
        let default = |name: &str| {
            registry
                .default_state(name)
                .ok_or_else(|| invalid("missing placement block"))
        };
        let mut kinds = Vec::with_capacity(registry.len());
        let mut doubles = Vec::new();
        let mut defaults = Vec::new();
        let mut moss = Vec::new();
        let mut moss_ids = [None; 162];
        for index in 0..registry.len() {
            let id = BlockStateId(index as u32);
            let state = registry.state(id).unwrap();
            defaults.push(default(&state.name)?);
            let kind = if double_plants.contains(&state.name) {
                let waterlogged = state.properties.contains_key("waterlogged");
                let mut halves = [[id; 2]; 2];
                for (half, values) in halves.iter_mut().enumerate() {
                    for (water, target) in values.iter_mut().enumerate() {
                        let mut descriptor = state.clone();
                        descriptor.properties.insert(
                            "half".into(),
                            if half == 0 { "lower" } else { "upper" }.into(),
                        );
                        if waterlogged {
                            descriptor.properties.insert(
                                "waterlogged".into(),
                                if water == 0 { "false" } else { "true" }.into(),
                            );
                        }
                        *target = registry
                            .id(&descriptor)
                            .ok_or_else(|| invalid("missing double-plant transition"))?;
                    }
                }
                let selected = doubles.len();
                doubles.push(DoublePlant {
                    halves,
                    waterlogged,
                });
                Placement::Double(selected)
            } else if mossy_carpets.contains(&state.name) {
                Placement::Moss
            } else {
                Placement::Single
            };
            kinds.push(kind);
            let value = if state.name == "minecraft:pale_moss_carpet" {
                let bottom = match state.properties.get("bottom").map(String::as_str) {
                    Some("true") => true,
                    Some("false") => false,
                    _ => return Err(invalid("invalid moss bottom")),
                };
                let mut sides = [0; 4];
                for (slot, name) in sides.iter_mut().zip(SIDE_NAMES) {
                    *slot = match state.properties.get(name).map(String::as_str) {
                        Some("none") => 0,
                        Some("low") => 1,
                        Some("tall") => 2,
                        _ => return Err(invalid("invalid moss side")),
                    };
                }
                let value = Moss { bottom, sides };
                if moss_ids[value.code()].replace(id).is_some() {
                    return Err(invalid("ambiguous moss state"));
                }
                Some(value)
            } else {
                None
            };
            moss.push(value);
        }
        let moss_ids = moss_ids
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| invalid("incomplete moss states"))?
            .try_into()
            .map_err(|_| invalid("invalid moss state count"))?;
        Ok(Self {
            registry,
            operations: Arc::new(BlockPlacement {
                kinds,
                doubles,
                replaceable: properties.bind(BlockProperty::Replaceable),
                fluids: FluidStateTable::new(registry)?,
                air: [
                    default("minecraft:air")?,
                    default("minecraft:cave_air")?,
                    default("minecraft:void_air")?,
                ],
                defaults,
                moss,
                moss_ids,
                moss_default: default("minecraft:pale_moss_carpet")?,
            }),
        })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn operations(&self) -> Arc<BlockPlacement> {
        self.operations.clone()
    }
}
impl BlockPlacement {
    fn read<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        position: [i32; 3],
    ) -> Result<BlockStateId, BlockPlacementError<W::Error>> {
        let state = world.block(position).map_err(BlockPlacementError::World)?;
        if state.0 as usize >= self.kinds.len() {
            return Err(BlockPlacementError::Registry(invalid(
                "unregistered placement world state",
            )));
        }
        Ok(state)
    }
    fn moss_state(&self, state: BlockStateId) -> Option<Moss> {
        self.moss[state.0 as usize]
    }
    fn updated_moss<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        state: Moss,
        position: [i32; 3],
        force: bool,
    ) -> Result<Moss, BlockPlacementError<W::Error>> {
        let force = force | state.bottom;
        let mut result = state;
        let mut above = None;
        let mut below = None;
        for (index, direction) in HORIZONTAL.into_iter().enumerate() {
            let mut side = if world
                .attachment_at(position, direction)
                .map_err(BlockPlacementError::Query)?
            {
                if force { 1 } else { state.sides[index] }
            } else {
                0
            };
            if side == 1 {
                let upper = match above {
                    Some(id) => id,
                    None => {
                        let id = self.read(world, relative(position, Direction::Up))?;
                        above = Some(id);
                        id
                    }
                };
                if self
                    .moss_state(upper)
                    .is_some_and(|value| !value.bottom && value.sides[index] != 0)
                {
                    side = 2;
                }
                if !state.bottom {
                    let lower = match below {
                        Some(id) => id,
                        None => {
                            let id = self.read(world, relative(position, Direction::Down))?;
                            below = Some(id);
                            id
                        }
                    };
                    if self
                        .moss_state(lower)
                        .is_some_and(|value| value.sides[index] == 0)
                    {
                        side = 0;
                    }
                }
            }
            result.sides[index] = side;
        }
        Ok(result)
    }
    fn topper<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        position: [i32; 3],
    ) -> Result<Option<BlockStateId>, BlockPlacementError<W::Error>> {
        let above = relative(position, Direction::Up);
        let current = self.read(world, above)?;
        if self.moss_state(current).is_some_and(|value| value.bottom)
            || (self.moss_state(current).is_none() && !self.replaceable.contains(current))
        {
            return Ok(None);
        }
        let mut state = self.moss_state(self.moss_default).unwrap();
        state.bottom = false;
        state = self.updated_moss(world, state, above, true)?;
        for side in &mut state.sides {
            if *side != 0
                && !world
                    .next_world_boolean()
                    .map_err(BlockPlacementError::Query)?
            {
                *side = 0;
            }
        }
        let id = self.moss_ids[state.code()];
        Ok((state.has_faces() && id != current).then_some(id))
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        state: BlockStateId,
        position: [i32; 3],
        flags: i32,
    ) -> Result<bool, BlockPlacementError<W::Error>> {
        let kind = *self.kinds.get(state.0 as usize).ok_or_else(|| {
            BlockPlacementError::Registry(invalid("unregistered placement candidate"))
        })?;
        match kind {
            Placement::Single => {
                world
                    .set_block(position, state, flags)
                    .map_err(BlockPlacementError::World)?;
            }
            Placement::Double(index) => {
                let above = relative(position, Direction::Up);
                let current = self.read(world, above)?;
                if !self.air.contains(&current)
                    && (self.fluids.get(state) != self.fluids.get(current)
                        || !self.replaceable.contains(current))
                {
                    return Ok(false);
                }
                let plant = self.doubles[index];
                for (half, at) in [position, above].into_iter().enumerate() {
                    let water = plant.waterlogged
                        && world.water_at(at).map_err(BlockPlacementError::Query)?;
                    world
                        .set_block(at, plant.halves[half][usize::from(water)], flags)
                        .map_err(BlockPlacementError::World)?;
                }
            }
            Placement::Moss => {
                let lower = self.updated_moss(
                    world,
                    self.moss_state(self.moss_default).unwrap(),
                    position,
                    true,
                )?;
                world
                    .set_block(position, self.moss_ids[lower.code()], flags)
                    .map_err(BlockPlacementError::World)?;
                if let Some(top) = self.topper(world, position)? {
                    world
                        .set_block(relative(position, Direction::Up), top, flags)
                        .map_err(BlockPlacementError::World)?;
                    let lower = self.updated_moss(world, lower, position, true)?;
                    world
                        .set_block(position, self.moss_ids[lower.code()], flags)
                        .map_err(BlockPlacementError::World)?;
                }
            }
        }
        Ok(true)
    }
    pub fn schedule_tick<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        position: [i32; 3],
        delay: i32,
    ) -> Result<(), BlockPlacementError<W::Error>> {
        let state = self.read(world, position)?;
        world
            .schedule_block_tick(position, self.defaults[state.0 as usize], delay)
            .map_err(BlockPlacementError::Query)
    }
}
fn relative(position: [i32; 3], direction: Direction) -> [i32; 3] {
    let step = direction.normal();
    std::array::from_fn(|i| position[i].wrapping_add(step[i]))
}
