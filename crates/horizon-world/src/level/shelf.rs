//! `ShelfBlock` power changes and its `SideChainPartBlock` linking: up to
//! three powered wooden shelves facing the same way join into one chain.
use super::{
    LevelAccess, Sim,
    query::{clockwise, counter_clockwise},
    update::rel,
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos};
use std::collections::HashMap;

/// `SideChainPart`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Part {
    Unconnected,
    Right,
    Center,
    Left,
}

impl Part {
    fn parse(text: &str) -> Self {
        match text {
            "right" => Self::Right,
            "center" => Self::Center,
            "left" => Self::Left,
            _ => Self::Unconnected,
        }
    }
    const fn name(self) -> &'static str {
        match self {
            Self::Unconnected => "unconnected",
            Self::Right => "right",
            Self::Center => "center",
            Self::Left => "left",
        }
    }
    const fn is_connection_towards(self, end: Self) -> bool {
        matches!(self, Self::Center) || self as u8 == end as u8
    }
    const fn is_chain_end(self) -> bool {
        !matches!(self, Self::Center)
    }
    const fn connected_to_the_right(self) -> Self {
        match self {
            Self::Unconnected | Self::Left => Self::Left,
            Self::Right | Self::Center => Self::Center,
        }
    }
    const fn connected_to_the_left(self) -> Self {
        match self {
            Self::Unconnected | Self::Right => Self::Right,
            Self::Center | Self::Left => Self::Center,
        }
    }
    const fn disconnected_from_the_right(self) -> Self {
        match self {
            Self::Unconnected | Self::Left => Self::Unconnected,
            Self::Right | Self::Center => Self::Right,
        }
    }
    const fn disconnected_from_the_left(self) -> Self {
        match self {
            Self::Unconnected | Self::Right => Self::Unconnected,
            Self::Center | Self::Left => Self::Left,
        }
    }
}

/// `SideChainPartBlock.Neighbors`: neighbours along the chain are read once
/// (the part is captured when first seen, as vanilla's cache does).
struct Neighbors {
    facing: Direction,
    center: BlockPos,
    cache: HashMap<BlockPos, Option<Part>>,
}

const MAX_CHAIN_LENGTH: usize = 3;

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn shelf_part(&self, state: BlockStateId) -> Part {
        Part::parse(self.b.table.text(state, self.b.props.side_chain))
    }
    /// `ShelfBlock.isConnectable`.
    fn shelf_connectable(&self, state: BlockStateId) -> bool {
        let b = self.b;
        b.tag("minecraft:wooden_shelves")
            .is_some_and(|tag| b.in_set(state, tag))
            && b.table.has(state, b.props.powered)
            && b.table.bool(state, b.props.powered)
    }
    fn chain_neighbor(
        &mut self,
        n: &mut Neighbors,
        left: bool,
        steps: i32,
    ) -> (BlockPos, Option<Part>) {
        let dir = if left {
            clockwise(n.facing)
        } else {
            counter_clockwise(n.facing)
        };
        let mut pos = n.center;
        for _ in 0..steps {
            pos = rel(pos, dir);
        }
        let part = *n.cache.entry(pos).or_insert_with(|| {
            let state = self.state(pos);
            (self.shelf_connectable(state) && self.b.facing(state) == n.facing)
                .then(|| self.shelf_part(state))
        });
        (pos, part)
    }
    /// `SideChainPartBlock.setPart`.
    fn shelf_set_part(&mut self, pos: BlockPos, part: Part) {
        let state = self.state(pos);
        if self.shelf_part(state) != part {
            let next = self
                .b
                .table
                .set_text(state, self.b.props.side_chain, part.name());
            self.set_block_and_update(pos, next);
        }
    }
    /// `SideChainPartBlock.getAllBlocksConnectedTo(...).size()`.
    fn shelf_chain_size(&mut self, pos: BlockPos) -> usize {
        let state = self.state(pos);
        if !self.shelf_connectable(state) {
            return 0;
        }
        let mut n = Neighbors {
            facing: self.b.facing(state),
            center: pos,
            cache: HashMap::new(),
        };
        let mut size = 1;
        for (left, end) in [(true, Part::Left), (false, Part::Right)] {
            for steps in 1..MAX_CHAIN_LENGTH as i32 {
                let (_, part) = self.chain_neighbor(&mut n, left, steps);
                if part.is_some_and(|p| p.is_connection_towards(end)) {
                    size += 1;
                }
                if part.is_none_or(Part::is_chain_end) {
                    break;
                }
            }
        }
        size
    }
    /// `SideChainPartBlock.updateNeighborsAfterPoweringDown`.
    pub(crate) fn shelf_after_powering_down(&mut self, pos: BlockPos, state: BlockStateId) {
        let mut n = Neighbors {
            facing: self.b.facing(state),
            center: pos,
            cache: HashMap::new(),
        };
        if let (at, Some(part)) = self.chain_neighbor(&mut n, true, 1) {
            self.shelf_set_part(at, part.disconnected_from_the_right());
        }
        if let (at, Some(part)) = self.chain_neighbor(&mut n, false, 1) {
            self.shelf_set_part(at, part.disconnected_from_the_left());
        }
    }
    /// `SideChainPartBlock.updateSelfAndNeighborsOnPoweringUp`.
    fn shelf_on_powering_up(&mut self, pos: BlockPos, state: BlockStateId, old: BlockStateId) {
        if !self.shelf_connectable(state) {
            return;
        }
        let updated_by_neighbor = self.shelf_part(state) != Part::Unconnected
            || self.shelf_connectable(old) && self.shelf_part(old) != Part::Unconnected;
        if updated_by_neighbor {
            return;
        }
        let mut n = Neighbors {
            facing: self.b.facing(state),
            center: pos,
            cache: HashMap::new(),
        };
        let (left_pos, left) = self.chain_neighbor(&mut n, true, 1);
        let (right_pos, right) = self.chain_neighbor(&mut n, false, 1);
        let on_left = if left.is_some() {
            self.shelf_chain_size(left_pos)
        } else {
            0
        };
        let on_right = if right.is_some() {
            self.shelf_chain_size(right_pos)
        } else {
            0
        };
        let mut own = Part::Unconnected;
        let mut length = 1;
        if on_left > 0 && length + on_left <= MAX_CHAIN_LENGTH {
            own = own.connected_to_the_left();
            if let Some(part) = left {
                self.shelf_set_part(left_pos, part.connected_to_the_right());
            }
            length += on_left;
        }
        if on_right > 0 && length + on_right <= MAX_CHAIN_LENGTH {
            own = own.connected_to_the_right();
            if let Some(part) = right {
                self.shelf_set_part(right_pos, part.connected_to_the_left());
            }
        }
        self.shelf_set_part(pos, own);
    }
    /// `ShelfBlock.onPlace`.
    pub(crate) fn shelf_on_place(&mut self, pos: BlockPos, state: BlockStateId, old: BlockStateId) {
        if self.b.table.bool(state, self.b.props.powered) {
            self.shelf_on_powering_up(pos, state, old);
        } else {
            self.shelf_after_powering_down(pos, state);
        }
    }
    /// `ShelfBlock.neighborChanged`.
    pub(crate) fn shelf_neighbor_changed(&mut self, pos: BlockPos, state: BlockStateId) {
        let b = self.b;
        let signal = self.has_neighbor_signal(pos);
        if b.table.bool(state, b.props.powered) == signal {
            return;
        }
        let mut next = b.table.set_bool(state, b.props.powered, signal);
        if !signal {
            next = b
                .table
                .set_text(next, b.props.side_chain, Part::Unconnected.name());
        }
        self.set_block_and_update(pos, next);
        let (sound, event) = if signal {
            ("minecraft:block.shelf.activate", "minecraft:block_activate")
        } else {
            (
                "minecraft:block.shelf.deactivate",
                "minecraft:block_deactivate",
            )
        };
        self.sound(pos, &sound.into(), 1.0, 1.0);
        self.game_event_state(pos, event, next);
    }
}
