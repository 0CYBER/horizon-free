//! Registry-bound block movement coefficients, identities and reloadable tags.
use crate::{
    DATA_VERSION,
    compression::invalid,
    movement_support::MovementBlockWorld,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockTable},
};
use horizon_core::position::BlockPos;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, io,
};

#[derive(Clone, Copy, Debug)]
pub struct MovementCoefficients {
    pub friction: f32,
    pub speed_factor: f32,
    pub jump_factor: f32,
    pub bounce_restitution: f32,
    pub fall_distance_reduction: f32,
    pub fence_gate: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct MovementBlock {
    pub coefficients: MovementCoefficients,
    pub fence: bool,
    pub wall: bool,
    pub suppresses_bounce: bool,
    pub water: bool,
    pub bubble_column: bool,
}
#[derive(Default)]
pub struct MovementTags {
    pub fences: BTreeSet<String>,
    pub walls: BTreeSet<String>,
    pub suppresses_bounce: BTreeSet<String>,
}
/// Definitions are private version-specific data; no runtime Java is required.
pub struct MovementProperties<'r> {
    registry: &'r BlockRegistry,
    definitions: BTreeMap<String, MovementCoefficients>,
}
impl<'r> MovementProperties<'r> {
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn coefficients(&self, state: BlockStateId) -> io::Result<MovementCoefficients> {
        let state = self
            .registry
            .state(state)
            .ok_or_else(|| invalid("movement state outside registry"))?;
        Ok(self.definitions[&state.name])
    }

    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        Self::from_table(BlockTable::check(tag, registry, "movement")?)
    }
    pub fn from_table(table: BlockTable<'_, 'r>) -> io::Result<Self> {
        let (tag, registry) = (table.tag(), table.registry());
        if tag.get("DataVersion") != Some(&Tag::Int(DATA_VERSION)) {
            return Err(invalid("movement properties DataVersion mismatch"));
        }
        let Some(Tag::Compound(blocks)) = tag.get("blocks") else {
            return Err(invalid("missing movement definitions"));
        };
        let mut definitions = BTreeMap::new();
        for (name, properties) in blocks {
            let name = name.to_string().map_err(|e| invalid(e.to_string()))?;
            if registry.default_state(&name).is_none() {
                return Err(invalid("unknown movement definition"));
            }
            let float = |key: &str| -> io::Result<f32> {
                match properties.get(key) {
                    Some(Tag::Float(bits)) => Ok(f32::from_bits(*bits)),
                    _ => Err(invalid(format!("missing or invalid movement {key}"))),
                }
            };
            let fence_gate = match properties.get("fenceGate") {
                Some(Tag::Byte(0)) => false,
                Some(Tag::Byte(1)) => true,
                _ => return Err(invalid("invalid movement gate identity")),
            };
            definitions.insert(
                name,
                MovementCoefficients {
                    friction: float("friction")?,
                    speed_factor: float("speedFactor")?,
                    jump_factor: float("jumpFactor")?,
                    bounce_restitution: float("bounceRestitution")?,
                    fall_distance_reduction: float("fallDistanceReduction")?,
                    fence_gate,
                },
            );
        }
        for id in 0..registry.len() {
            if !definitions.contains_key(&registry.state(BlockStateId(id as u32)).unwrap().name) {
                return Err(invalid("missing movement definition"));
            }
        }
        Ok(Self {
            registry,
            definitions,
        })
    }
    /// Bind again when tags reload; existing bindings retain their old snapshot.
    pub fn bind(&self, tags: &MovementTags) -> io::Result<BoundMovementProperties> {
        for name in tags
            .fences
            .iter()
            .chain(&tags.walls)
            .chain(&tags.suppresses_bounce)
        {
            if self.registry.default_state(name).is_none() {
                return Err(invalid("unknown movement tag member"));
            }
        }
        let states = (0..self.registry.len())
            .map(|id| {
                let name = &self.registry.state(BlockStateId(id as u32)).unwrap().name;
                MovementBlock {
                    coefficients: self.definitions[name],
                    fence: tags.fences.contains(name),
                    wall: tags.walls.contains(name),
                    suppresses_bounce: tags.suppresses_bounce.contains(name),
                    water: name == "minecraft:water",
                    bubble_column: name == "minecraft:bubble_column",
                }
            })
            .collect();
        Ok(BoundMovementProperties { states })
    }
}
pub struct BoundMovementProperties {
    states: Vec<MovementBlock>,
}
impl BoundMovementProperties {
    pub fn get(&self, state: BlockStateId) -> io::Result<MovementBlock> {
        self.states
            .get(state.0 as usize)
            .copied()
            .ok_or_else(|| invalid("movement state outside registry"))
    }
}
pub trait MovementBlockProvider {
    type Error;
    fn movement_block_state(&mut self, position: BlockPos) -> Result<BlockStateId, Self::Error>;
}
#[derive(Debug)]
pub enum MovementBlockError<E> {
    Data(io::Error),
    World(E),
}
impl<E: fmt::Display> fmt::Display for MovementBlockError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for MovementBlockError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::Data(e) => e,
            Self::World(e) => e,
        })
    }
}
pub struct RegistryMovementWorld<'a, W> {
    pub properties: &'a BoundMovementProperties,
    pub world: &'a mut W,
}
impl<W: MovementBlockProvider> MovementBlockWorld for RegistryMovementWorld<'_, W> {
    type Error = MovementBlockError<W::Error>;
    type State = MovementBlock;
    fn block_state(&mut self, p: BlockPos) -> Result<MovementBlock, Self::Error> {
        let state = self
            .world
            .movement_block_state(p)
            .map_err(MovementBlockError::World)?;
        self.properties.get(state).map_err(MovementBlockError::Data)
    }
    fn is_fence(&mut self, s: MovementBlock) -> Result<bool, Self::Error> {
        Ok(s.fence)
    }
    fn is_wall(&mut self, s: MovementBlock) -> Result<bool, Self::Error> {
        Ok(s.wall)
    }
    fn is_fence_gate(&mut self, s: MovementBlock) -> Result<bool, Self::Error> {
        Ok(s.coefficients.fence_gate)
    }
    fn is_water(&mut self, s: MovementBlock) -> Result<bool, Self::Error> {
        Ok(s.water)
    }
    fn is_bubble_column(&mut self, s: MovementBlock) -> Result<bool, Self::Error> {
        Ok(s.bubble_column)
    }
    fn jump_factor(&mut self, s: MovementBlock) -> Result<f32, Self::Error> {
        Ok(s.coefficients.jump_factor)
    }
    fn speed_factor(&mut self, s: MovementBlock) -> Result<f32, Self::Error> {
        Ok(s.coefficients.speed_factor)
    }
}
