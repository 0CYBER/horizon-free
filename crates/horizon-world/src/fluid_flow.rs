//! Neighbor-derived water/lava currents and falling-fluid wall deflection.
use crate::fluid::{FluidKind, FluidState};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};

pub trait FluidFlowWorld {
    type Error;
    type Block: Copy;
    fn fluid(&mut self, position: BlockPos) -> Result<FluidState, Self::Error>;
    fn block(&mut self, position: BlockPos) -> Result<Self::Block, Self::Error>;
    /// Resolved BLOCKS_FLUID_FLOW tag membership for the captured state.
    fn blocks_fluid_flow(&self, state: Self::Block) -> bool;
    /// Actual IceBlock class membership, including its subclasses.
    fn is_ice_class(&self, state: Self::Block) -> bool;
    fn is_face_sturdy(
        &mut self,
        state: Self::Block,
        position: BlockPos,
        direction: Direction,
    ) -> Result<bool, Self::Error>;
}
/// Native wall test reads both block and fluid before any early return. The
/// supplied face is the scan direction, rather than its opposite.
pub fn solid_face<W: FluidFlowWorld>(
    world: &mut W,
    kind: FluidKind,
    position: BlockPos,
    direction: Direction,
) -> Result<bool, W::Error> {
    let block = world.block(position)?;
    let fluid = world.fluid(position)?;
    if fluid.kind().same_family(kind) {
        return Ok(false);
    }
    if direction == Direction::Up {
        return Ok(true);
    }
    if world.is_ice_class(block) {
        return Ok(false);
    }
    world.is_face_sturdy(block, position, direction)
}
fn affects_flow(kind: FluidKind, neighbor: FluidState) -> bool {
    neighbor.kind() == FluidKind::Empty || neighbor.kind().same_family(kind)
}
const HORIZONTAL: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];
/// The height differences and directional products round as f32 before being
/// accumulated in f64. Falling currents normalize before and after the Y push.
pub fn flow_at<W: FluidFlowWorld>(
    world: &mut W,
    state: FluidState,
    position: BlockPos,
) -> Result<Vec3, W::Error> {
    if state.kind() == FluidKind::Empty {
        return Ok(Vec3::ZERO);
    }
    let mut x = 0.0;
    let mut z = 0.0;
    for direction in HORIZONTAL {
        let [dx, dy, dz] = direction.normal();
        let p = position.offset(dx, dy, dz);
        let neighbor = world.fluid(p)?;
        if !affects_flow(state.kind(), neighbor) {
            continue;
        }
        let mut height = neighbor.own_height();
        let mut difference = 0.0f32;
        if height == 0.0 {
            let block = world.block(p)?;
            if !world.blocks_fluid_flow(block) {
                let below = world.fluid(p.offset(0, -1, 0))?;
                if affects_flow(state.kind(), below) {
                    height = below.own_height();
                    if height > 0.0 {
                        difference = state.own_height() - (height - 0.8888889f32);
                    }
                }
            }
        } else if height > 0.0 {
            difference = state.own_height() - height;
        }
        if difference != 0.0 {
            x += f64::from(dx as f32 * difference);
            z += f64::from(dz as f32 * difference);
        }
    }
    let mut result = Vec3::new(x, 0.0, z);
    if state.falling() {
        for direction in HORIZONTAL {
            let [dx, dy, dz] = direction.normal();
            let p = position.offset(dx, dy, dz);
            if solid_face(world, state.kind(), p, direction)?
                || solid_face(world, state.kind(), p.offset(0, 1, 0), direction)?
            {
                result = result.normalize().add_vector(Vec3::new(0.0, -6.0, 0.0));
                break;
            }
        }
    }
    Ok(result.normalize())
}
