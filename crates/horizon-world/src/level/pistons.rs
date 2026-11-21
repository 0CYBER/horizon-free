//! Pistons: `PistonBaseBlock` (signal checks, block events, `moveBlocks`),
//! `PistonStructureResolver`, `PistonHeadBlock`, and the
//! `PistonMovingBlockEntity` progress ticking, plus the level's block-event
//! queue (`ServerLevel.blockEvent` / `runBlockEvents`).
//!
//! Entity pushing by moving blocks is the caller's (see `piston_movement`).
use super::{
    Behaviours, LevelAccess, LevelEvent, Signals, Sim, Ticks,
    data::PushReaction,
    states::BlockId,
    update::{above, rel},
};
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};
use std::cmp::Ordering;
use {
    crate::registry::BlockStateId,
    horizon_core::java_hash_map::{JavaHashMap, JavaKey},
};

/// `PistonMovingBlockEntity` state, owned by [`Ticks::moving_pistons`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovingPiston {
    pub moved_state: BlockStateId,
    pub direction: Direction,
    pub extending: bool,
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    pub last_ticked: i64,
    /// `PistonMovingBlockEntity.deathTicks`: on a client the entity waits
    /// five ticks past progress 1.0 before placing the block itself.
    pub death_ticks: i32,
    /// Identity of this block entity (a replaced entity is a new ticker).
    pub id: u64,
}
impl MovingPiston {
    /// What collision and rendering need from this block entity.
    pub fn shape_state(&self) -> crate::piston_shape::MovingPistonState {
        crate::piston_shape::MovingPistonState {
            moved_state: self.moved_state,
            direction: self.direction,
            extending: self.extending,
            source: self.source,
            progress: self.progress,
            previous_progress: self.progress_o,
        }
    }
    pub fn movement_direction(&self) -> Direction {
        if self.extending {
            self.direction
        } else {
            self.direction.opposite()
        }
    }
}

/// `MovingPistonBlock.getDrops`: a broken moving piston drops as the block its
/// block entity is moving, and drops nothing when that block entity is gone
/// (vanilla returns `Collections.emptyList()`). Every other block drops as
/// itself.
pub(crate) fn dropped_state(
    b: &Behaviours,
    moving: &[(BlockPos, MovingPiston)],
    state: BlockStateId,
    pos: BlockPos,
) -> Option<BlockStateId> {
    if !b.is_block(state, b.named.moving_piston) {
        return Some(state);
    }
    moving
        .iter()
        .find(|(p, _)| *p == pos)
        .map(|(_, piston)| piston.moved_state)
}

/// `BlockEventData`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BlockEventData {
    pub pos: BlockPos,
    pub block: BlockId,
    pub param_a: i32,
    pub param_b: i32,
    /// Hook-selected instrument and note, never decoded from raw event parameters.
    pub note_override: Option<(u8, i32)>,
}

#[derive(PartialEq, Eq)]
struct PosKey(BlockPos);
impl JavaKey for PosKey {
    fn java_hash(&self) -> i32 {
        (self.0.y.wrapping_add(self.0.z.wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(self.0.x)
    }
    fn comparable_compare(&self, other: &Self) -> Option<Ordering> {
        // Vec3i implements Comparable: y, then z, then x.
        Some(
            self.0
                .y
                .cmp(&other.0.y)
                .then(self.0.z.cmp(&other.0.z))
                .then(self.0.x.cmp(&other.0.x)),
        )
    }
    fn tie_break(&self, _other: &Self) -> Ordering {
        Ordering::Less
    }
}

struct Resolver {
    vanilla_parts: bool,
    part_changed: std::cell::Cell<bool>,
    piston_pos: BlockPos,
    extending: bool,
    start: BlockPos,
    push_direction: Direction,
    piston_direction: Direction,
    to_push: Vec<BlockPos>,
    to_destroy: Vec<BlockPos>,
}

fn rel_n(pos: BlockPos, direction: Direction, n: i32) -> BlockPos {
    let [x, y, z] = direction.normal();
    pos.offset(x * n, y * n, z * n)
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    // ---- block events ----
    pub(crate) fn block_event(
        &mut self,
        pos: BlockPos,
        block: BlockId,
        param_a: i32,
        param_b: i32,
    ) {
        let event = BlockEventData {
            pos,
            block,
            param_a,
            param_b,
            note_override: None,
        };
        self.ticks.queue_block_event(event);
    }
    /// `BlockState.triggerEvent`.
    fn trigger_event(&mut self, state: BlockStateId, pos: BlockPos, a: i32, b: i32) -> bool {
        // A piston template would extend or retract; one already moving still lands.
        if self.level.template_suppressed(state) {
            return false;
        }
        let data = self.b.data(state);
        if data.is(super::classes::Class::PistonBase) {
            self.piston_trigger_event(pos, state, a, b)
        } else if data.is(super::classes::Class::Note) {
            self.note_trigger_event(pos, state);
            true
        } else if data.is(super::classes::Class::Bell) {
            // BellBlockEntity.triggerEvent handles event 1; the caller
            // reports handled events (`runBlockEvents` broadcasts them).
            a == 1
        } else if data.is(super::classes::Class::PotentSulfur) {
            // PotentSulfurBlock.triggerEvent: the block entity notes the
            // eruption's game time (the caller's), and the event is handled.
            true
        } else {
            false
        }
    }

    // ---- moving piston block entities ----
    fn set_moving_piston(&mut self, pos: BlockPos, piston: MovingPiston) {
        self.ticks.add_moving_piston(pos, piston);
    }
    pub(crate) fn remove_moving_piston(&mut self, pos: BlockPos) -> Option<MovingPiston> {
        self.ticks.take_moving_piston(pos)
    }
    /// The block entity the moving piston at `pos` carried, once it is done
    /// (arismoko/horizon#3184): installed at `pos`, where `moved` lands, or, when the block
    /// does not land (`lands` false), installed and removed again with its
    /// removal side effects as for a broken block (`side_effects`), so a
    /// container drops what it held instead of losing it.
    pub(crate) fn land_carried(
        &mut self,
        pos: BlockPos,
        moved: BlockStateId,
        lands: bool,
        side_effects: bool,
    ) {
        if self.level.land_block_entity(pos) && !lands {
            self.level.remove_block_entity(pos, moved, side_effects);
        }
    }
    /// `PistonMovingBlockEntity.finalTick`.
    fn moving_piston_final_tick(&mut self, pos: BlockPos) {
        let Some(piston) = self.moving_piston(pos) else {
            return;
        };
        // finalTick: `progressO < 1.0 || level.isClientSide` — a client
        // finishes its copy even at full progress.
        if piston.progress_o >= 1.0 && !self.level.is_client_side() {
            return;
        }
        self.remove_moving_piston(pos);
        let b = self.b;
        if b.is_block(self.state(pos), b.named.moving_piston) {
            let new = if piston.source {
                b.air()
            } else {
                self.update_from_neighbour_shapes(piston.moved_state, pos)
            };
            self.land_carried(pos, piston.moved_state, !b.is_air(new), true);
            self.set_block_and_update(pos, new);
            let block = b.block(new);
            let orientation = self.initial_orientation(Some(piston.movement_direction()), None);
            self.neighbor_changed_at_oriented(pos, block, orientation);
        }
    }
    /// `PistonMovingBlockEntity.tick`. A piston that is still moving reports
    /// the push `moveCollidedEntities` would do at its new progress; the caller
    /// moves the entities, because `Ticks` has none (`piston_pushes`).
    fn moving_piston_tick(&mut self, pos: BlockPos) {
        let Some(mut piston) = self.moving_piston(pos) else {
            return;
        };
        let b = self.b;
        piston.last_ticked = self.level.game_time();
        piston.progress_o = piston.progress;
        if piston.progress_o >= 1.0 {
            if self.level.is_client_side() && piston.death_ticks < 5 {
                // The client's copy waits for the server's state before
                // placing the block itself (deathTicks < 5).
                piston.death_ticks += 1;
                if let Some(slot) = self.ticks.moving_piston_slot(pos) {
                    self.ticks.moving_pistons[slot].1 = piston;
                }
                return;
            }
            self.remove_moving_piston(pos);
            if b.is_block(self.state(pos), b.named.moving_piston) {
                let mut new = self.update_from_neighbour_shapes(piston.moved_state, pos);
                // Before the block: one that `update_or_destroy` breaks takes
                // its block entity with it, side effects and all.
                self.land_carried(pos, piston.moved_state, true, true);
                if b.is_air(new) {
                    self.set_block(pos, piston.moved_state, 340);
                    self.update_or_destroy(
                        piston.moved_state,
                        new,
                        pos,
                        3,
                        super::flags::UPDATE_LIMIT,
                    );
                } else {
                    if b.is_waterlogged(new) {
                        new = b.table.set_bool(new, b.props.waterlogged, false);
                    }
                    self.set_block(pos, new, 67);
                    let block = b.block(new);
                    let orientation =
                        self.initial_orientation(Some(piston.movement_direction()), None);
                    self.neighbor_changed_at_oriented(pos, block, orientation);
                }
            }
        } else {
            // moveCollidedEntities and moveStuckEntities run with the new
            // progress but read the old one off the entity (their delta),
            // so the push carries the piston as it was before the advance.
            let old = piston;
            let new_progress = (old.progress + 0.5).min(1.0);
            if let Some(slot) = self.ticks.moving_piston_slot(pos) {
                piston.progress = new_progress;
                self.ticks.moving_pistons[slot].1 = piston;
                self.ticks.piston_pushes.push((pos, old, new_progress));
            }
        }
    }

    // ---- PistonBaseBlock ----
    fn is_sticky_piston(&self, state: BlockStateId) -> bool {
        self.b.is_block(state, self.b.named.sticky_piston)
    }
    fn piston_neighbor_signal(&self, pos: BlockPos, push: Direction) -> bool {
        for direction in Direction::ALL {
            if direction != push && self.has_signal(rel(pos, direction), direction) {
                return true;
            }
        }
        if self.has_signal(pos, Direction::Down) {
            return true;
        }
        let up = above(pos);
        Direction::ALL.into_iter().any(|direction| {
            direction != Direction::Down && self.has_signal(rel(up, direction), direction)
        })
    }
    /// `PistonBaseBlock.checkIfExtend`, which `neighborChanged`, `onPlace`
    /// and `setPlacedBy` call only server-side: a client moves its pistons on
    /// the server's block events alone.
    pub(crate) fn piston_check_if_extend(&mut self, pos: BlockPos, state: BlockStateId) {
        if self.level.is_client_side() {
            return;
        }
        let b = self.b;
        let direction = b.facing(state);
        let extend = self.piston_neighbor_signal(pos, direction);
        let extended = b.table.bool(state, b.props.extended);
        let block = b.block(state);
        if extend && !extended {
            if self.piston_resolve(pos, direction, true).is_some() {
                self.block_event(pos, block, 0, direction as i32);
            }
        } else if !extend && extended {
            let pushed_pos = rel_n(pos, direction, 2);
            let pushed = self.state(pushed_pos);
            let mut event = 1;
            if b.is_block(pushed, b.named.moving_piston)
                && b.facing(pushed) == direction
                && let Some(piston) = self.moving_piston(pushed_pos)
                && piston.extending
                && (piston.progress_o < 0.5
                    || self.level.game_time() == piston.last_ticked
                    || self.ticks.is_handling_tick())
            {
                event = 2;
            }
            self.block_event(pos, block, event, direction as i32);
        }
    }
    /// `PistonBaseBlock.isPushable`.
    fn is_pushable(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        direction: Direction,
        allow_destroyable: bool,
        connection: Direction,
    ) -> bool {
        let b = self.b;
        let (min_y, max_y) = (self.level.min_y(), self.level.max_y() - 1);
        if pos.y < min_y || pos.y > max_y || !self.level.within_world_border(pos) {
            return false;
        }
        if b.is_air(state) {
            return true;
        }
        if direction == Direction::Down && pos.y == min_y {
            return false;
        }
        if direction == Direction::Up && pos.y == max_y {
            return false;
        }
        let is_piston =
            b.is_block(state, b.named.piston) || b.is_block(state, b.named.sticky_piston);
        if !is_piston {
            if b.facts(state).hardness == -1.0 {
                return false;
            }
            match b.piston_reaction(state) {
                PushReaction::Immoveable => return false,
                PushReaction::Popped => return allow_destroyable,
                PushReaction::Push if direction != connection => return false,
                _ => {}
            }
        } else if b.table.bool(state, b.props.extended) {
            return false;
        }
        // One order for every moving reaction: reaction first, then the block-entity rule.
        // Diverges from vanilla: push only never discards a block entity.
        let has_entity = b.has_flag(state, super::data::state_flags::BLOCK_ENTITY)
            || self.level.has_mod_block_entity(pos)
            || self.level.has_native_block_entity(state);
        self.level
            .adjust_piston_carries_block_entity(pos, state)
            .unwrap_or(!has_entity)
    }
    /// `IBlockExtension.isSlimeBlock`: vanilla's slime block, or what a part says.
    fn is_slime(&self, r: &Resolver, state: BlockStateId) -> bool {
        let vanilla = self.b.is_block(state, self.b.named.slime_block);
        if r.vanilla_parts {
            return vanilla;
        }
        let adjusted = self.level.adjust_is_slime_block(state, vanilla);
        if adjusted != vanilla {
            r.part_changed.set(true);
        }
        adjusted
    }
    /// `PistonStructureResolver.isSticky` (`IBlockExtension.isStickyBlock`): vanilla's slime
    /// and honey blocks, or what a part says.
    fn is_sticky(&self, r: &Resolver, state: BlockStateId) -> bool {
        let vanilla = self.b.is_block(state, self.b.named.slime_block)
            || self.b.is_block(state, self.b.named.honey_block);
        if r.vanilla_parts {
            return vanilla;
        }
        let adjusted = self.level.adjust_is_sticky_block(state, vanilla);
        if adjusted != vanilla {
            r.part_changed.set(true);
        }
        adjusted
    }
    /// `IBlockExtension.canStickTo`: slime and honey never hold on to each other, and
    /// otherwise a block sticks to its neighbour when either of them is sticky.
    fn sticks_to(&self, r: &Resolver, state: BlockStateId, other: BlockStateId) -> bool {
        let honey = self.b.named.honey_block;
        let slime_beside_honey = (self.b.is_block(state, honey) && self.is_slime(r, other))
            || (self.is_slime(r, state) && self.b.is_block(other, honey));
        let vanilla = !slime_beside_honey && (self.is_sticky(r, state) || self.is_sticky(r, other));
        if r.vanilla_parts {
            return vanilla;
        }
        let adjusted = self.level.adjust_can_stick_to(state, other, vanilla);
        if adjusted != vanilla {
            r.part_changed.set(true);
        }
        adjusted
    }
    /// `PistonStructureResolver.canStickToEachOther`: each side agrees to stick.
    fn can_stick(&self, r: &Resolver, first: BlockStateId, second: BlockStateId) -> bool {
        self.sticks_to(r, first, second) && self.sticks_to(r, second, first)
    }
    /// `PistonStructureResolver.resolve`; None when the structure cannot move.
    fn piston_resolve(
        &self,
        piston_pos: BlockPos,
        direction: Direction,
        extending: bool,
    ) -> Option<(Vec<BlockPos>, Vec<BlockPos>, Direction)> {
        self.piston_resolve_stick(piston_pos, direction, extending, false)
            .ok()
            .map(|r| (r.to_push, r.to_destroy, r.push_direction))
    }
    /// Mark both ends of replayed moves for the next tick's ordinary block updates.
    fn correct_piston_replay(
        &mut self,
        cells: impl Iterator<Item = BlockPos>,
        direction: Direction,
    ) {
        for pos in cells {
            self.level.send_block_updated(pos);
            self.level.send_block_updated(rel(pos, direction));
        }
    }
    /// Failure retains whether a part changed an answer, even when its structure cannot move.
    fn piston_resolve_stick(
        &self,
        piston_pos: BlockPos,
        direction: Direction,
        extending: bool,
        vanilla_parts: bool,
    ) -> Result<Resolver, bool> {
        let mut r = Resolver {
            vanilla_parts,
            part_changed: std::cell::Cell::new(false),
            piston_pos,
            extending,
            start: if extending {
                rel(piston_pos, direction)
            } else {
                rel_n(piston_pos, direction, 2)
            },
            push_direction: if extending {
                direction
            } else {
                direction.opposite()
            },
            piston_direction: direction,
            to_push: Vec::new(),
            to_destroy: Vec::new(),
        };
        let next = self.state(r.start);
        if !self.is_pushable(next, r.start, r.push_direction, false, r.piston_direction) {
            // PistonStructureResolver.resolve(): a POPPED block is destroyed
            // even at the build limit, where isPushable fails on the Y bound.
            if r.extending && self.b.piston_reaction(next) == PushReaction::Popped {
                r.to_destroy.push(r.start);
                return Ok(r);
            }
            return Err(r.part_changed.get());
        }
        let start = r.start;
        let push = r.push_direction;
        if !self.add_block_line(&mut r, start, push) {
            return Err(r.part_changed.get());
        }
        let mut i = 0;
        while i < r.to_push.len() {
            let pos = r.to_push[i];
            if self.is_sticky(&r, self.state(pos)) && !self.add_branching_blocks(&mut r, pos) {
                return Err(r.part_changed.get());
            }
            i += 1;
        }
        Ok(r)
    }
    fn add_block_line(&self, r: &mut Resolver, start: BlockPos, direction: Direction) -> bool {
        let mut next = self.state(start);
        if self.b.is_air(next) {
            return true;
        }
        if !self.is_pushable(next, start, r.push_direction, false, direction)
            || start == r.piston_pos
            || r.to_push.contains(&start)
        {
            return true;
        }
        let mut count = 1;
        if count + r.to_push.len() > 12 {
            return false;
        }
        while self.is_sticky(r, next) {
            let pos = rel_n(start, r.push_direction.opposite(), count as i32);
            let previous = next;
            next = self.state(pos);
            if self.b.is_air(next)
                || !self.can_stick(r, previous, next)
                || !self.is_pushable(
                    next,
                    pos,
                    r.push_direction,
                    false,
                    r.push_direction.opposite(),
                )
                || pos == r.piston_pos
            {
                break;
            }
            count += 1;
            if count + r.to_push.len() > 12 {
                return false;
            }
        }
        let mut added = 0;
        for i in (0..count).rev() {
            r.to_push
                .push(rel_n(start, r.push_direction.opposite(), i as i32));
            added += 1;
        }
        let mut i = 1;
        loop {
            let pos = rel_n(start, r.push_direction, i);
            if let Some(collision) = r.to_push.iter().position(|p| *p == pos) {
                self.reorder_at_collision(r, added, collision);
                for j in 0..=collision + added {
                    let block_pos = r.to_push[j];
                    if self.is_sticky(r, self.state(block_pos))
                        && !self.add_branching_blocks(r, block_pos)
                    {
                        return false;
                    }
                }
                return true;
            }
            next = self.state(pos);
            if self.b.is_air(next) {
                return true;
            }
            if !self.is_pushable(next, pos, r.push_direction, true, r.push_direction)
                || pos == r.piston_pos
            {
                return false;
            }
            if self.b.piston_reaction(next) == PushReaction::Popped {
                r.to_destroy.push(pos);
                return true;
            }
            if r.to_push.len() >= 12 {
                return false;
            }
            r.to_push.push(pos);
            added += 1;
            i += 1;
        }
    }
    fn reorder_at_collision(&self, r: &mut Resolver, added: usize, collision: usize) {
        let len = r.to_push.len();
        let head: Vec<BlockPos> = r.to_push[..collision].to_vec();
        let last_line: Vec<BlockPos> = r.to_push[len - added..].to_vec();
        let middle: Vec<BlockPos> = r.to_push[collision..len - added].to_vec();
        r.to_push.clear();
        r.to_push.extend(head);
        r.to_push.extend(last_line);
        r.to_push.extend(middle);
    }
    fn add_branching_blocks(&self, r: &mut Resolver, from: BlockPos) -> bool {
        let from_state = self.state(from);
        for direction in Direction::ALL {
            if super::query::axis_of(direction) != super::query::axis_of(r.push_direction) {
                let neighbour = rel(from, direction);
                let neighbour_state = self.state(neighbour);
                if self.can_stick(r, neighbour_state, from_state)
                    && !self.add_block_line(r, neighbour, direction)
                {
                    return false;
                }
            }
        }
        true
    }
    fn moving_piston_state(&self, direction: Direction, sticky: bool) -> BlockStateId {
        let b = self.b;
        let state = b.table.set_dir(
            b.default_of(b.named.moving_piston),
            b.props.facing,
            direction,
        );
        b.table.set_text(
            state,
            b.props.kind,
            if sticky { "sticky" } else { "normal" },
        )
    }
    /// `PistonBaseBlock.moveBlocks`: whether the blocks moved, or None when
    /// the level refused the move (`LevelAccess::allow_piston_move`).
    fn piston_move_blocks(
        &mut self,
        piston_pos: BlockPos,
        direction: Direction,
        extending: bool,
        sticky: bool,
    ) -> Option<bool> {
        let b = self.b;
        let arm = rel(piston_pos, direction);
        if !extending && b.is_block(self.state(arm), b.named.piston_head) {
            self.set_block(arm, b.air(), 276);
        }
        let resolved = match self.piston_resolve_stick(piston_pos, direction, extending, false) {
            Ok(resolved) => resolved,
            Err(part_changed) => {
                if part_changed
                    && !self.level.is_client_side()
                    && let Ok(replay) =
                        self.piston_resolve_stick(piston_pos, direction, extending, true)
                {
                    self.correct_piston_replay(
                        replay.to_push.iter().chain(&replay.to_destroy).copied(),
                        replay.push_direction,
                    );
                }
                return Some(false);
            }
        };
        // Only a changed sticky, slime, or adhesion answer pays for a second resolution. Include entire
        // structures: a refused neighbour may itself pull a line or sticky branch.
        // These marks join the event's block log, delivered on the next tick.
        let correct_replay = resolved.part_changed.get() && !self.level.is_client_side();
        let replay = if correct_replay {
            self.piston_resolve_stick(piston_pos, direction, extending, true)
                .ok()
        } else {
            None
        };
        let Resolver {
            to_push,
            to_destroy,
            push_direction,
            ..
        } = resolved;
        if !self.level.allow_piston_move(
            piston_pos,
            push_direction,
            extending,
            &to_push,
            &to_destroy,
        ) {
            return None;
        }
        if correct_replay {
            self.correct_piston_replay(
                to_push
                    .iter()
                    .chain(&to_destroy)
                    .chain(
                        replay
                            .iter()
                            .flat_map(|r| r.to_push.iter().chain(&r.to_destroy)),
                    )
                    .copied(),
                push_direction,
            );
        }
        let move_direction = if extending {
            direction
        } else {
            direction.opposite()
        };
        let mut delete_after: JavaHashMap<PosKey, BlockStateId> = JavaHashMap::default();
        let mut shapes = Vec::with_capacity(to_push.len());
        for pos in &to_push {
            let state = self.state(*pos);
            shapes.push(state);
            delete_after.put(PosKey(*pos), state);
            // An allowed block entity travels through the existing carry machinery,
            // taken out before any source is overwritten (arismoko/horizon#3184).
            if b.has_flag(state, super::data::state_flags::BLOCK_ENTITY)
                || self.level.has_mod_block_entity(*pos)
                || self.level.has_native_block_entity(state)
            {
                self.level
                    .carry_block_entity(*pos, rel(*pos, move_direction));
            }
        }
        let mut to_update = Vec::with_capacity(to_push.len() + to_destroy.len());
        for pos in to_destroy.iter().rev() {
            let state = self.state(*pos);
            self.drop_resources(state, *pos);
            // The client's copy plays the destroy effect itself (the
            // server sends no 2001 for piston moves).
            let fire = b.is_block(state, b.named.fire) || b.is_block(state, b.named.soul_fire);
            if !fire && self.level.is_client_side() {
                self.level_event(2001, *pos, state.0 as i32);
            }
            // NeoForge's onDestroyedByPushReaction: the level hears it before the removal.
            self.level.on_destroyed_by_push_reaction(
                self.ticks,
                *pos,
                state,
                push_direction,
                b.fluid(state),
            );
            self.set_block(*pos, b.air(), 18);
            self.game_event(*pos, "minecraft:block_destroy");
            to_update.push(state);
        }
        for (i, pos) in to_push.iter().enumerate().rev() {
            let state = self.state(*pos);
            let target = rel(*pos, move_direction);
            delete_after.remove(&PosKey(target));
            let moving = b.table.set_dir(
                b.default_of(b.named.moving_piston),
                b.props.facing,
                direction,
            );
            self.set_block(target, moving, 324);
            self.set_moving_piston(
                target,
                MovingPiston {
                    moved_state: shapes[i],
                    direction,
                    extending,
                    source: false,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                },
            );
            to_update.push(state);
        }
        if extending {
            let head = b.table.set_text(
                b.table
                    .set_dir(b.default_of(b.named.piston_head), b.props.facing, direction),
                b.props.kind,
                if sticky { "sticky" } else { "normal" },
            );
            let moving = self.moving_piston_state(direction, sticky);
            delete_after.remove(&PosKey(arm));
            self.set_block(arm, moving, 324);
            self.set_moving_piston(
                arm,
                MovingPiston {
                    moved_state: head,
                    direction,
                    extending: true,
                    source: true,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                },
            );
        }
        let air = b.air();
        let entries: Vec<(BlockPos, BlockStateId)> =
            delete_after.entries().map(|(k, v)| (k.0, *v)).collect();
        for (pos, _) in &entries {
            self.set_block(*pos, air, 82);
        }
        for (pos, old) in &entries {
            self.update_indirect_neighbour_shapes(*old, *pos, 2, super::flags::UPDATE_LIMIT);
            self.update_neighbour_shapes(air, *pos, 2, super::flags::UPDATE_LIMIT);
            self.update_indirect_neighbour_shapes(air, *pos, 2, super::flags::UPDATE_LIMIT);
        }
        let orientation = self.initial_orientation(Some(push_direction), None);
        let mut index = 0;
        for pos in to_destroy.iter().rev() {
            let state = to_update[index];
            index += 1;
            self.affect_neighbors_after_removal(state, *pos, false);
            self.update_indirect_neighbour_shapes(state, *pos, 2, super::flags::UPDATE_LIMIT);
            let block = b.block(state);
            self.update_neighbors_at_oriented(*pos, block, orientation);
        }
        for pos in to_push.iter().rev() {
            let block = b.block(to_update[index]);
            index += 1;
            self.update_neighbors_at_oriented(*pos, block, orientation);
        }
        if extending {
            self.update_neighbors_at_oriented(arm, b.named.piston_head, orientation);
        }
        self.level.observe_piston_moved(
            piston_pos,
            push_direction,
            extending,
            &to_push,
            &to_destroy,
        );
        Some(true)
    }
    /// `PistonBaseBlock.triggerEvent`.
    fn piston_trigger_event(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        a: i32,
        param_b: i32,
    ) -> bool {
        let b = self.b;
        let p = b.props;
        let direction = b.facing(state);
        let extended_state = b.table.set_bool(state, p.extended, true);
        let sticky = self.is_sticky_piston(state);
        // A pull the level refused isn't sent: clients, who aren't asked,
        // would replay it, so they get the blocks' changes instead.
        let mut refused = false;
        // The signal re-check is server-only (`!level.isClientSide`): a
        // client trusts the event as sent.
        if !self.level.is_client_side() {
            let extend = self.piston_neighbor_signal(pos, direction);
            if extend && (a == 1 || a == 2) {
                self.set_block(pos, extended_state, 2);
                return false;
            }
            if !extend && a == 0 {
                return false;
            }
        }
        if a == 0 {
            if self.piston_move_blocks(pos, direction, true, sticky) != Some(true) {
                return false;
            }
            self.set_block(pos, extended_state, 67);
            let pitch = self.random().next_float() * 0.25 + 0.6;
            self.emit(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.piston.extend".into(),
                volume: 0.5,
                pitch,
            });
            self.game_event(pos, "minecraft:block_activate");
        } else if a == 1 || a == 2 {
            self.moving_piston_final_tick(rel(pos, direction));
            let moving = self.moving_piston_state(direction, sticky);
            self.set_block(pos, moving, 276);
            let piston_default = b.table.set_dir(
                b.default_of(b.block(state)),
                p.facing,
                Direction::ALL[(param_b & 7).min(5) as usize],
            );
            self.set_moving_piston(
                pos,
                MovingPiston {
                    moved_state: piston_default,
                    direction,
                    extending: false,
                    source: true,
                    progress: 0.0,
                    progress_o: 0.0,
                    last_ticked: 0,
                    death_ticks: 0,
                    id: 0,
                },
            );
            self.update_neighbors_at(pos, b.named.moving_piston);
            self.update_neighbour_shapes(moving, pos, 2, super::flags::UPDATE_LIMIT);
            if sticky {
                let two = rel_n(pos, direction, 2);
                let moving_state = self.state(two);
                let mut piston_piece = false;
                if b.is_block(moving_state, b.named.moving_piston)
                    && let Some(entity) = self.moving_piston(two)
                    && entity.direction == direction
                    && entity.extending
                {
                    self.moving_piston_final_tick(two);
                    piston_piece = true;
                }
                if !piston_piece {
                    let is_piston = b.is_block(moving_state, b.named.piston)
                        || b.is_block(moving_state, b.named.sticky_piston);
                    if a != 1
                        || b.is_air(moving_state)
                        || !self.is_pushable(
                            moving_state,
                            two,
                            direction.opposite(),
                            false,
                            direction,
                        )
                        || (b.piston_reaction(moving_state) != PushReaction::PushPull && !is_piston)
                    {
                        self.remove_block(rel(pos, direction), false);
                    } else {
                        refused = self
                            .piston_move_blocks(pos, direction, false, sticky)
                            .is_none();
                    }
                }
            } else {
                self.remove_block(rel(pos, direction), false);
            }
            let pitch = self.random().next_float() * 0.15 + 0.6;
            self.emit(LevelEvent::Sound {
                pos,
                sound: "minecraft:block.piston.contract".into(),
                volume: 0.5,
                pitch,
            });
            self.game_event(pos, "minecraft:block_deactivate");
        }
        !refused
    }

    // ---- PistonHeadBlock ----
    pub(crate) fn piston_head_fitting_base(&self, head: BlockStateId, base: BlockStateId) -> bool {
        let b = self.b;
        let expected = if b.table.text(head, b.props.kind) == "sticky" {
            b.named.sticky_piston
        } else {
            b.named.piston
        };
        b.is_block(base, expected)
            && b.table.bool(base, b.props.extended)
            && b.facing(base) == b.facing(head)
    }
    pub(crate) fn piston_head_can_survive(&self, head: BlockStateId, pos: BlockPos) -> bool {
        let b = self.b;
        let base = self.rel_state(pos, b.facing(head).opposite());
        self.piston_head_fitting_base(head, base)
            || (b.is_block(base, b.named.moving_piston) && b.facing(base) == b.facing(head))
    }
}

impl Behaviours {
    /// `ServerLevel.tick`'s block phases for a caller with nothing to do
    /// between them: the scheduled block and fluid ticks, then the block
    /// events, inside the tick's `handlingTick`
    /// ([`Ticks::begin_handling_tick`] … [`Ticks::end_handling_tick`],
    /// ServerLevel.java:360 sets it, :394-396 runs the ticks, :409 the
    /// events, :412 clears it). The game splits the same span to apply each
    /// scheduled tick's side effects (level events, spilled items) before the
    /// next runs, as vanilla runs them inline.
    pub fn tick_level_blocks<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        ticks.begin_handling_tick();
        self.tick_scheduled(level, ticks);
        self.run_block_events(level, ticks);
        ticks.end_handling_tick();
    }
    /// `ServerLevel.runBlockEvents`: run queued block events (piston moves).
    /// Call once per tick after scheduled and random ticks, inside the level
    /// tick's `handlingTick` ([`Ticks::begin_handling_tick`]): vanilla runs
    /// these events at ServerLevel.java:409, between :360 and :412.
    pub fn run_block_events<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        let mut reschedule = Vec::new();
        while let Some(event) = ticks.pop_block_event() {
            if !level.should_tick_blocks_at(event.pos.chunk()) {
                reschedule.push(event);
                continue;
            }
            let mut sim = self.sim(level, ticks);
            let state = sim.state(event.pos);
            if self.block(state) == event.block
                && sim.trigger_event(
                    event.note_override.map_or(state, |(instrument, note)| {
                        sim.note_played_state(state, instrument, note)
                    }),
                    event.pos,
                    event.param_a,
                    event.param_b,
                )
            {
                // ServerLevel.runBlockEvents broadcasts a handled event to
                // the clients (`ClientboundBlockEventPacket`); their copy of
                // the block re-runs `triggerEvent` (a piston's `moveBlocks`).
                sim.emit(LevelEvent::BlockEntityEvent {
                    pos: event.pos,
                    state,
                    a: event.param_a,
                    b: event.param_b,
                });
            }
        }
        for event in reschedule {
            ticks.queue_block_event(event);
        }
    }
    /// `Level.tickBlockEntities` for moving pistons (the only block entities
    /// whose ticking changes blocks). Entries added during the pass wait for
    /// the next tick, as vanilla's pending tickers do. Outside the level
    /// tick's `handlingTick`: vanilla ticks block entities at
    /// ServerLevel.java:455, below :412, so a neighbour change a piston's
    /// `finalTick` makes here answers `checkIfExtend` with event 1.
    pub fn tick_block_entities<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        let tickers: Vec<(BlockPos, u64)> = ticks
            .moving_pistons
            .iter()
            .map(|(p, m)| (*p, m.id))
            .collect();
        for (pos, id) in tickers {
            if !level.is_ticking_chunk(pos.chunk()) || !ticks.moving_piston_has(pos, id) {
                continue;
            }
            self.sim(level, ticks).moving_piston_tick(pos);
        }
    }
    /// `ClientLevel.blockEvent`: the client re-runs the block's
    /// `triggerEvent` on its own level (a piston's `moveBlocks` builds the
    /// moving block entities it then animates) instead of waiting for the
    /// server's states. No queue and no ticking-chunk gate: a client runs
    /// the event where the packet lands.
    pub fn client_block_event<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
        pos: BlockPos,
        a: i32,
        b: i32,
    ) -> bool {
        let triggered = {
            let mut sim = self.sim(level, ticks);
            let state = sim.state(pos);
            let triggered = sim.trigger_event(state, pos, a, b);
            if triggered {
                // The client's copy of a handled event reaches the client
                // copy of the block entity (a bell's ring).
                sim.emit(LevelEvent::BlockEntityEvent { pos, state, a, b });
            }
            triggered
        };
        self.client_drain_block_events(level, ticks);
        triggered
    }
    /// `ClientLevel.tickBlockEntities` for moving pistons: the client copy of
    /// [`Behaviours::tick_block_entities`], without the server's
    /// ticking-chunk gate (a client ticks every loaded block entity). One
    /// whose block was replaced is dropped (`TickingBlockEntity.isRemoved`).
    pub fn client_tick_block_entities<L: LevelAccess + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Ticks,
    ) {
        let tickers: Vec<(BlockPos, u64)> = ticks
            .moving_pistons
            .iter()
            .map(|(p, m)| (*p, m.id))
            .collect();
        for (pos, id) in tickers {
            if !ticks.moving_piston_has(pos, id) {
                continue;
            }
            {
                let mut sim = self.sim(level, ticks);
                let state = sim.state(pos);
                if sim.b.is_block(state, sim.b.named.moving_piston) {
                    sim.moving_piston_tick(pos);
                } else {
                    sim.remove_moving_piston(pos);
                    sim.level.discard_carried_block_entity(pos);
                }
            }
            self.client_drain_block_events(level, ticks);
        }
    }
    /// Block events a client-side `triggerEvent` queued
    /// (`PistonBaseBlock.checkIfExtend` in a neighbour update): a client
    /// runs them as they come (`ClientLevel.blockEvent`), not in a tick pass.
    fn client_drain_block_events<L: LevelAccess + ?Sized>(&self, level: &mut L, ticks: &mut Ticks) {
        while let Some(event) = ticks.pop_block_event() {
            let mut sim = self.sim(level, ticks);
            let state = sim.state(event.pos);
            if self.block(state) == event.block
                && sim.trigger_event(
                    event.note_override.map_or(state, |(instrument, note)| {
                        sim.note_played_state(state, instrument, note)
                    }),
                    event.pos,
                    event.param_a,
                    event.param_b,
                )
            {
                sim.emit(LevelEvent::BlockEntityEvent {
                    pos: event.pos,
                    state,
                    a: event.param_a,
                    b: event.param_b,
                });
            }
        }
    }
}

impl<L: super::signal_access::SignalAccess + ?Sized> Signals<'_, L> {
    pub(crate) fn moving_piston(&self, pos: BlockPos) -> Option<MovingPiston> {
        let slot = self.ticks.moving_piston_slot(pos)?;
        Some(self.ticks.moving_pistons[slot].1)
    }
}

/// `Sim` reads these through its [`Signals`], so there is one implementation of each.
impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub(crate) fn moving_piston(&self, pos: BlockPos) -> Option<MovingPiston> {
        self.signals().moving_piston(pos)
    }
}
