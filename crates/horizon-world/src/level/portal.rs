//! `PortalShape`: the obsidian frame search used by nether portal blocks.
use super::{LevelAccess, Sim, update::rel};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos};

const MIN_WIDTH: i32 = 2;
const MAX_WIDTH: i32 = 21;
const MIN_HEIGHT: i32 = 3;
const MAX_HEIGHT: i32 = 21;

/// The result of `PortalShape.findAnyShape`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalShape {
    pub x_axis: bool,
    pub right: Direction,
    pub bottom_left: BlockPos,
    pub width: i32,
    pub height: i32,
    pub portal_blocks: i32,
}

impl PortalShape {
    pub const fn is_valid(&self) -> bool {
        self.width >= MIN_WIDTH && self.width <= MAX_WIDTH && self.height >= MIN_HEIGHT && self.height <= MAX_HEIGHT
    }
    pub const fn is_complete(&self) -> bool {
        self.is_valid() && self.portal_blocks == self.width * self.height
    }
}

fn step(pos: BlockPos, dir: Direction, n: i32) -> BlockPos {
    let [dx, dy, dz] = dir.normal();
    pos.offset(dx * n, dy * n, dz * n)
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn portal_frame(&self, state: BlockStateId) -> bool {
        let b = self.b;
        b.tag("minecraft:nether_portal_frame").is_some_and(|tag| b.in_set(state, tag))
    }
    /// `PortalShape.isEmpty`: air, fire or a portal block.
    fn portal_empty(&self, state: BlockStateId) -> bool {
        let b = self.b;
        b.is_air(state)
            || b.tag("minecraft:fire").is_some_and(|tag| b.in_set(state, tag))
            || b.is_block(state, b.named.nether_portal)
    }
    fn portal_distance_until_edge_above_frame(&self, pos: BlockPos, dir: Direction) -> i32 {
        for width in 0..=MAX_WIDTH {
            let at = step(pos, dir, width);
            let state = self.state(at);
            if !self.portal_empty(state) {
                return if self.portal_frame(state) { width } else { 0 };
            }
            if !self.portal_frame(self.state(rel(at, Direction::Down))) {
                break;
            }
        }
        0
    }
    /// `PortalShape.findAnyShape`.
    pub fn find_portal_shape(&self, pos: BlockPos, x_axis: bool) -> PortalShape {
        let right = if x_axis { Direction::West } else { Direction::South };
        let mut shape = PortalShape { x_axis, right, bottom_left: pos, width: 0, height: 0, portal_blocks: 0 };
        // calculateBottomLeft
        let min_y = self.level.min_y().max(pos.y - MAX_HEIGHT);
        let mut start = pos;
        while start.y > min_y && self.portal_empty(self.state(rel(start, Direction::Down))) {
            start = rel(start, Direction::Down);
        }
        let left = right.opposite();
        let edge = self.portal_distance_until_edge_above_frame(start, left) - 1;
        if edge < 0 {
            return shape;
        }
        shape.bottom_left = step(start, left, edge);
        // calculateWidth
        let width = self.portal_distance_until_edge_above_frame(shape.bottom_left, right);
        if !(MIN_WIDTH..=MAX_WIDTH).contains(&width) {
            return shape;
        }
        shape.width = width;
        // calculateHeight: getDistanceUntilTop counts portal blocks as it goes.
        let mut height = MAX_HEIGHT;
        'rows: for h in 0..MAX_HEIGHT {
            let row = step(shape.bottom_left, Direction::Up, h);
            if !self.portal_frame(self.state(step(row, right, -1)))
                || !self.portal_frame(self.state(step(row, right, width)))
            {
                height = h;
                break;
            }
            for i in 0..width {
                let state = self.state(step(row, right, i));
                if !self.portal_empty(state) {
                    height = h;
                    break 'rows;
                }
                if self.b.is_block(state, self.b.named.nether_portal) {
                    shape.portal_blocks += 1;
                }
            }
        }
        let top = step(shape.bottom_left, Direction::Up, height);
        let has_top = (0..width).all(|i| self.portal_frame(self.state(step(top, right, i))));
        shape.height = if (MIN_HEIGHT..=MAX_HEIGHT).contains(&height) && has_top { height } else { 0 };
        shape
    }
}
