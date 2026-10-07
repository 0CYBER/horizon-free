//! Block placement (`BlockItem.place`, `BlockPlaceContext`, `getStateForPlacement`).
use crate::registry::BlockStateId;
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};

/// `BlockPlaceContext` inputs from the player's use-item-on-block action.
#[derive(Clone, Debug)]
pub struct PlaceContext {
    pub clicked_pos: BlockPos,
    pub face: Direction,
    pub hit: Vec3,
    pub inside: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub sneaking: bool,
    pub item: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceOutcome {
    Placed { pos: BlockPos, state: BlockStateId },
    Failed,
}
