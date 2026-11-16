//! Turning hanging entities for mods: a painting's or item frame's facing
//! (`entity_set_hanging_direction`) and an item frame's item (`entity_set_item_frame_rotation`).
use super::{Game, commands::Target};
use horizon_core::geometry::Direction;
use mod_api::EntityRef;

impl Game<'_> {
    /// `HangingEntity.setDirection`: the facing, from which the yaw, pitch and bounds follow.
    pub(in crate::game) fn entity_set_hanging_direction(
        &mut self,
        entity: EntityRef,
        direction: Direction,
    ) -> bool {
        let pos = match self.native_entity_target(entity) {
            Some(Target::Frame(id)) => {
                let Some(frame) = self.item_frames.iter_mut().find(|f| f.id == id) else {
                    return false;
                };
                frame.facing = data_3d(direction);
                frame.pos
            }
            Some(Target::Painting(id)) => {
                let Some(facing) = data_2d(direction) else {
                    return false;
                };
                let Some(painting) = self.paintings.iter_mut().find(|p| p.id == id) else {
                    return false;
                };
                painting.facing = facing;
                painting.pos
            }
            _ => return false,
        };
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        true
    }

    /// `ItemFrame.setRotation`: `rotation % 8`, then the comparators reading the frame.
    pub(in crate::game) fn entity_set_item_frame_rotation(
        &mut self,
        entity: EntityRef,
        rotation: u8,
    ) -> bool {
        let Some(Target::Frame(id)) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(frame) = self.item_frames.iter_mut().find(|f| f.id == id) else {
            return false;
        };
        frame.rotation = (rotation % 8) as i8;
        let pos = frame.pos;
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.frame_output_changed(pos);
        true
    }
}

/// `Direction.get3DDataValue`: down 0, up 1, north 2, south 3, west 4, east 5.
fn data_3d(direction: Direction) -> u8 {
    match direction {
        Direction::Down => 0,
        Direction::Up => 1,
        Direction::North => 2,
        Direction::South => 3,
        Direction::West => 4,
        Direction::East => 5,
    }
}

/// `Direction.get2DDataValue` (south 0, west 1, north 2, east 3), the value a painting keeps;
/// None for up and down.
fn data_2d(direction: Direction) -> Option<u8> {
    match direction {
        Direction::South => Some(0),
        Direction::West => Some(1),
        Direction::North => Some(2),
        Direction::East => Some(3),
        Direction::Down | Direction::Up => None,
    }
}
