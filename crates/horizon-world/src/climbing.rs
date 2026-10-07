//! Registered climbing tags and trapdoor/ladder support with entity state order.
use crate::{
    compression::invalid,
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::position::BlockPos;
use std::{collections::BTreeSet, fmt, io};
#[derive(Clone, Copy)]
struct State {
    climbable: bool,
    glide_through: bool,
    open_trapdoor: Option<u8>,
    ladder: Option<u8>,
}
pub struct ClimbingRegistry<'r> {
    registry: &'r BlockRegistry,
    states: Vec<State>,
}
impl<'r> ClimbingRegistry<'r> {
    /// Trapdoor names are imported class identities, not inferred from spelling.
    /// Rebuild this immutable snapshot when either resolved block tag reloads.
    pub fn new(
        registry: &'r BlockRegistry,
        trapdoors: &BTreeSet<String>,
        climbable: &BTreeSet<String>,
        glide_through: &BTreeSet<String>,
    ) -> io::Result<Self> {
        for names in [trapdoors, climbable, glide_through] {
            if names.iter().any(|n| registry.default_state(n).is_none()) {
                return Err(invalid("unknown climbing block member"));
            }
        }
        let mut states = Vec::with_capacity(registry.len());
        for i in 0..registry.len() {
            let s = registry.state(BlockStateId(i as u32)).unwrap();
            let facing = || -> io::Result<u8> {
                match s.properties.get("facing").map(String::as_str) {
                    Some("north") => Ok(0),
                    Some("south") => Ok(1),
                    Some("west") => Ok(2),
                    Some("east") => Ok(3),
                    _ => Err(invalid("invalid climbing facing")),
                }
            };
            let open_trapdoor = if trapdoors.contains(&s.name) {
                let f = facing()?;
                match s.properties.get("open").map(String::as_str) {
                    Some("true") => Some(f),
                    Some("false") => None,
                    _ => return Err(invalid("invalid trapdoor open state")),
                }
            } else {
                None
            };
            states.push(State {
                climbable: climbable.contains(&s.name),
                glide_through: glide_through.contains(&s.name),
                open_trapdoor,
                ladder: if s.name == "minecraft:ladder" {
                    Some(facing()?)
                } else {
                    None
                },
            });
        }
        Ok(Self { registry, states })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    fn state(&self, id: BlockStateId) -> io::Result<State> {
        self.states
            .get(id.0 as usize)
            .copied()
            .ok_or_else(|| invalid("climbing state outside registry"))
    }
    pub fn is_climbable_tag(&self, id: BlockStateId) -> io::Result<bool> {
        Ok(self.state(id)?.climbable)
    }
}
pub trait ClimbingWorld {
    type Error;
    fn is_spectator(&mut self) -> Result<bool, Self::Error>;
    fn block_position(&mut self) -> Result<BlockPos, Self::Error>;
    fn in_block_state(&mut self) -> Result<BlockStateId, Self::Error>;
    fn is_fall_flying(&mut self) -> Result<bool, Self::Error>;
    fn block_state(&mut self, position: BlockPos) -> Result<BlockStateId, Self::Error>;
    fn set_last_climbable_position(&mut self, position: BlockPos);
}
#[derive(Debug)]
pub enum ClimbingError<E> {
    Data(io::Error),
    World(E),
}
impl<E: fmt::Display> fmt::Display for ClimbingError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(e) => e.fmt(f),
            Self::World(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for ClimbingError<E> {}
/// Failed/negative queries retain the existing last-climbable position. Only a
/// positive result overwrites it, using the position captured before state reads.
pub fn on_climbable<W: ClimbingWorld>(
    world: &mut W,
    registry: &ClimbingRegistry<'_>,
) -> Result<bool, ClimbingError<W::Error>> {
    use ClimbingError::{Data, World};
    if world.is_spectator().map_err(World)? {
        return Ok(false);
    }
    let position = world.block_position().map_err(World)?;
    let state = registry
        .state(world.in_block_state().map_err(World)?)
        .map_err(Data)?;
    if world.is_fall_flying().map_err(World)? && state.glide_through {
        return Ok(false);
    }
    let climbable = if state.climbable {
        true
    } else if let Some(facing) = state.open_trapdoor {
        let below = BlockPos::new(position.x, position.y.wrapping_sub(1), position.z);
        registry
            .state(world.block_state(below).map_err(World)?)
            .map_err(Data)?
            .ladder
            == Some(facing)
    } else {
        false
    };
    if climbable {
        world.set_last_climbable_position(position);
    }
    Ok(climbable)
}
