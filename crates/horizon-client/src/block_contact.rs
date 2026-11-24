//! Synchronous mod-block contacts from the server's simulation code.
use crate::health::DamageLevel;
use horizon_core::position::BlockPos;
use horizon_world::{
    level::{InsideWrite, LandingWrite},
    registry::BlockStateId,
};
use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
};

/// Whether `state` is a block of a code mod that is installed and switched off now, so its
/// template's own behaviour must wait (`LevelAccess::template_suppressed`). The registry's
/// `code_block_owners` table names each state's owning mod (`u16::MAX`: none); a vanilla block
/// costs one table read. Ask it only on the authoritative side: a client replays its server.
#[inline]
pub fn template_suppressed(
    registry: &horizon_world::registry::BlockRegistry,
    state: BlockStateId,
) -> bool {
    match registry
        .code_block_owners
        .get()
        .and_then(|owners| owners.get(state.0 as usize))
    {
        Some(&owner) if owner != u16::MAX => !mod_api::__private::mod_index_on(usize::from(owner)),
        _ => false,
    }
}

/// `getEntityInsideCollisionShape` as the shared walk sees it: the whole block, boxes
/// to sweep the move against, or nothing. This is the one point arismoko/horizon#1011 adds the Part's
/// own shape at.
#[derive(Clone, Debug, PartialEq)]
pub enum InsideShape {
    /// `Shapes.block()`: every move through the block is inside it.
    Full,
    /// The shape's boxes, block-local, swept along the move.
    Boxes(Vec<horizon_core::geometry::Aabb>),
    /// An empty shape: nothing is inside it.
    Empty,
}

/// One entity kind's view of `Entity.checkInsideBlocks`: what the walk needs from the
/// kind, and the kind's own `entityInside` moments, engine blocks and fluid, which run
/// mid-walk where vanilla runs them and queue into the shared collector.
pub trait InsideWalk {
    type Error;
    /// `makeBoundingBox(at)`: the entity's box at a position.
    fn bounds_at(&self, at: horizon_core::geometry::Vec3) -> horizon_core::geometry::Aabb;
    /// `isAlive`: the walk stops the moment the entity dies.
    fn is_alive(&self) -> bool;
    /// `level.getBlockState(pos)`.
    fn block(&self, at: BlockPos) -> BlockStateId;
    /// `state.isAir()`, read as the sweep visits.
    fn is_air(&self, state: BlockStateId) -> bool;
    /// `state.getEntityInsideCollisionShape(level, pos, this)` for the walk.
    fn inside_shape(&mut self, state: BlockStateId, at: BlockPos) -> InsideShape;
    /// `collidedWithFluid(state.getFluidState(), pos, from, to)`.
    fn collided_with_fluid(
        &self,
        _at: BlockPos,
        _from: horizon_core::geometry::Vec3,
        _to: horizon_core::geometry::Vec3,
    ) -> bool {
        false
    }
    /// `advanceStep` → `state.entityInside`, then `onInsideBlock`: the kind's own
    /// per-block effects and the Part's contact, queueing into `effects`.
    fn entity_inside(
        &mut self,
        at: BlockPos,
        state: BlockStateId,
        step: i32,
        precise: bool,
        effects: &mut mod_api::InsideEffects,
    ) -> Result<(), Self::Error>;
    /// `advanceStep` → `fluidState.entityInside`.
    fn fluid_inside(
        &mut self,
        _at: BlockPos,
        _step: i32,
        _effects: &mut mod_api::InsideEffects,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// `Entity.checkInsideBlocks(from, to, …)`: the blocks the box at `to` sweeps through
/// from `from`, in the walk's order, each visited once, stopping when the entity dies
/// or the iteration budget runs out. Returns the iterations used plus one. Air is
/// passed over; a block is inside when its shape is the whole block, the move sweeps
/// its boxes, or its fluid collides, and the step advances before the block's
/// `entityInside` and again before the fluid's, as vanilla does.
pub fn sweep_inside_blocks<W: InsideWalk>(
    walk: &mut W,
    from: horizon_core::geometry::Vec3,
    to: horizon_core::geometry::Vec3,
    visited: &mut crate::fast_hash::FxHashSet<i64>,
    effects: &mut mod_api::InsideEffects,
    max_iterations: i32,
) -> Result<i32, W::Error> {
    let deflated = walk
        .bounds_at(to)
        .deflate(horizon_core::geometry::Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
    // Mth.square(0.9999900000002526): a long move counts every block it crosses as
    // precise.
    let far = 0.999_990_000_000_252_6_f64;
    let moved_far = from.distance_squared(to) > far * far;
    let mut iterations = 0;
    let mut hits: Vec<(i32, BlockPos, BlockStateId)> = Vec::new();
    let _ = horizon_core::geometry::sweep_intersected_blocks(
        from,
        to,
        deflated,
        1_000_000,
        |p, iteration| {
            if !walk.is_alive() || iteration >= max_iterations {
                return false;
            }
            iterations = iteration;
            let state = walk.block(p);
            if !walk.is_air(state) {
                hits.push((iteration, p, state));
            }
            true
        },
    );
    for (step, p, state) in hits {
        let shape = walk.inside_shape(state, p);
        let inside_block = match shape {
            InsideShape::Full => true,
            InsideShape::Empty => false,
            InsideShape::Boxes(boxes) => walk
                .bounds_at(from)
                .collided_along_vector(to.subtract(from), &boxes),
        };
        let inside_fluid = walk.collided_with_fluid(p, from, to);
        if !(inside_block || inside_fluid) || !visited.insert(p.pack()) {
            continue;
        }
        if inside_block {
            // `isPrecise` and the step advance, then the block's own moment.
            let precise = moved_far || deflated.intersects(horizon_core::geometry::Aabb::block(p));
            effects.advance_step(step);
            walk.entity_inside(p, state, step, precise, effects)?;
        }
        if inside_fluid {
            effects.advance_step(step);
            walk.fluid_inside(p, step, effects)?;
        }
    }
    Ok(iterations + 1)
}

/// `Entity.checkInsideBlocks(movements, collector)`: every movement of a tick, each
/// axis of a recorded original in `axisStepOrder` within its own 16-iteration budget,
/// then the target alone when the budget ran out, over one visited set, ending with
/// `applyAndClear`'s flush left to the kind's own tick.
pub fn sweep_movements<W: InsideWalk>(
    walk: &mut W,
    movements: &[horizon_world::entity_move::MovementRecord],
    visited: &mut crate::fast_hash::FxHashSet<i64>,
    effects: &mut mod_api::InsideEffects,
) -> Result<(), W::Error> {
    for movement in movements {
        sweep_movement_segments(*movement, |from, to, budget| {
            sweep_inside_blocks(walk, from, to, visited, effects, budget)
        })?;
    }
    Ok(())
}

/// Split a recorded move in vanilla's axis order, sharing the step budget across segments.
/// The visitor reports how many steps it used, or stops the walk with an error.
pub fn sweep_movement_segments<E>(
    movement: horizon_world::entity_move::MovementRecord,
    mut sweep: impl FnMut(
        horizon_core::geometry::Vec3,
        horizon_core::geometry::Vec3,
        i32,
    ) -> Result<i32, E>,
) -> Result<(), E> {
    use horizon_core::geometry::{Axis, Direction};
    let mut from = movement.from;
    let delta = movement.to.subtract(from);
    let mut budget = 16;
    match movement.original {
        Some(original) if delta.length_squared() > 0.0 => {
            for axis in Axis::step_order(original) {
                let step = delta.component(axis);
                if step != 0.0 {
                    let to = from.relative(
                        match axis {
                            Axis::X => Direction::East,
                            Axis::Y => Direction::Up,
                            Axis::Z => Direction::South,
                        },
                        step,
                    );
                    budget -= sweep(from, to, budget)?;
                    from = to;
                }
            }
        }
        _ => {
            budget -= sweep(movement.from, movement.to, budget)?;
        }
    }
    if budget <= 0 {
        sweep(movement.to, movement.to, 1)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub struct Contact<'a> {
    pub entity: i32,
    pub kind: &'a str,
    pub state: BlockStateId,
    pub position: BlockPos,
}

/// Where the contacts are answered: the game binds these to the hooks of the block
/// parts, and says which block states have a part for each.
pub struct Hooks {
    /// `Block.stepOn` for a mod block, in the dimension.
    pub step_on: fn(&str, Contact<'_>),
    pub any_inside: fn() -> bool,
    pub inside_listens: fn(BlockStateId) -> bool,
    pub pass_listens: fn(BlockStateId) -> bool,
    pub fall_listens: fn(BlockStateId) -> bool,
    pub bounce_listens: fn(BlockStateId) -> bool,
    /// `Block.fallOn`: change `damage`, the fall damage `causeFallDamage` calculated, and
    /// push the writes the part asks for.
    pub fall_on: fn(&str, &mut DamageLevel<'_>, Contact<'_>, f64, &mut f32, &mut Vec<LandingWrite>),
    /// `Block.bounceOn`: change `movement`, the movement the bounce leaves the entity.
    pub bounce_on:
        fn(&str, &mut DamageLevel<'_>, Contact<'_>, f64, &mut horizon_core::geometry::Vec3),
    /// `Block.entityInside` told mid-walk: the part's own contact on the level
    /// host, its whole callback staged into the fresh collector and write list,
    /// so only an answering callback's writes reach the walk.
    pub inside_fire: fn(
        &mut DamageLevel<'_>,
        &'static str,
        Contact<'_>,
        bool,
        &mod_api::SweptEntity,
        &mut mod_api::InsideEffects,
        &mut Vec<InsideWrite>,
    ),
}

pub static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// Where `IBlockExtension.getBounceRestitution` is answered, on both sides: the game says
/// which block states have a part for it and binds the part's answer.
pub struct RestitutionHooks {
    pub listens: fn(BlockStateId) -> bool,
    /// Change `factor`, the restitution of `Contact`'s block, for the entity landing on it.
    pub restitution: fn(&mut DamageLevel<'_>, Contact<'_>, &mut f32),
}

pub static RESTITUTION_HOOKS: OnceLock<RestitutionHooks> = OnceLock::new();

/// What a contact asked to keep until the entity's batch ends, delivered in
/// the order it happened: the parts' `entity_inside` contacts themselves fire
/// mid-walk now, on the shared inside sweep, so what stays here is the level
/// writes they and the landings asked for, and the minecart passes.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Landed(Landed),
    MinecartPass(MinecartPass),
    /// Level writes a mid-walk `entity_inside` part asked for, at the block it
    /// touched: applied at the batch end, guarded on the touched state.
    InsideWritten(InsideWritten),
}

/// The level writes an `entity_inside` part asked for at one block.
#[derive(Clone, Debug, PartialEq)]
pub struct InsideWritten {
    pub state: BlockStateId,
    pub position: BlockPos,
    pub writes: Vec<InsideWrite>,
}

/// An entity that landed on a block whose Part asked for writes (`Block.fallOn`).
#[derive(Clone, Debug, PartialEq)]
pub struct Landed {
    pub state: BlockStateId,
    pub position: BlockPos,
    pub writes: Vec<LandingWrite>,
}

/// A minecart on a rail whose Part listens to `minecart_pass`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinecartPass {
    pub entity: i32,
    pub state: BlockStateId,
    pub position: BlockPos,
}

thread_local! {
    static LEVEL: Cell<Option<&'static str>> = const { Cell::new(None) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
}

/// Scope authority to this thread and restore it even on an error or panic.
/// Nested guest actions may temporarily enter a different dimension.
pub fn in_level<R>(dimension: Option<&'static str>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static str>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LEVEL.set(self.0);
        }
    }
    let _restore = Restore(LEVEL.replace(dimension));
    f()
}

/// `Block.stepOn` for a mod block: the entity stands on `contact`'s block.
pub fn step_on(block: &str, contact: Contact<'_>) {
    if block.starts_with("minecraft:") {
        return;
    }
    if let Some(dimension) = LEVEL.get()
        && let Some(hooks) = HOOKS.get()
    {
        (hooks.step_on)(dimension, contact);
    }
}

/// The dimension the current thread's contacts answer in, as `inside` and the
/// landing routes read it: the server's tick set it around the entity passes.
pub fn current_level() -> Option<&'static str> {
    LEVEL.get()
}

/// Whether a part of the block `state` answers `fall_on`.
pub fn fall_listens(state: BlockStateId) -> bool {
    HOOKS.get().is_some_and(|hooks| (hooks.fall_listens)(state))
}

/// `Block.fallOn`: the fall damage `damage` calculated for the entity that landed on
/// `contact`'s block, after any part of that block changed it. A non-finite answer is
/// ignored and a negative one is zero. The writes the part asked for are kept until the
/// entity's batch ends. Nothing runs off the server or without a part.
pub fn fall(level: &mut DamageLevel<'_>, contact: Contact<'_>, distance: f64, damage: f32) -> f32 {
    let (Some(dimension), Some(hooks)) = (LEVEL.get(), HOOKS.get()) else {
        return damage;
    };
    if !(hooks.fall_listens)(contact.state) {
        return damage;
    }
    let mut answer = damage;
    let mut writes = Vec::new();
    (hooks.fall_on)(
        dimension,
        level,
        contact,
        distance,
        &mut answer,
        &mut writes,
    );
    landed(contact.state, contact.position, writes);
    accepted_damage(answer, damage)
}

/// A part's fall damage answer: negative is zero, and a non-finite one is ignored.
fn accepted_damage(answer: f32, kept: f32) -> f32 {
    if answer.is_finite() {
        answer.max(0.0)
    } else {
        kept
    }
}

/// A part's bounce answer, ignored unless every component is finite.
fn accepted_movement(
    answer: horizon_core::geometry::Vec3,
    kept: horizon_core::geometry::Vec3,
) -> horizon_core::geometry::Vec3 {
    if answer.x.is_finite() && answer.y.is_finite() && answer.z.is_finite() {
        answer
    } else {
        kept
    }
}

/// `Block.bounceOn`: the movement a bounce off `contact`'s block leaves, after any part of
/// that block changed it. A non-finite answer is ignored.
pub fn bounce(
    level: &mut DamageLevel<'_>,
    contact: Contact<'_>,
    fall_distance: f64,
    movement: horizon_core::geometry::Vec3,
) -> horizon_core::geometry::Vec3 {
    let (Some(dimension), Some(hooks)) = (LEVEL.get(), HOOKS.get()) else {
        return movement;
    };
    if !(hooks.bounce_listens)(contact.state) {
        return movement;
    }
    let mut answer = movement;
    (hooks.bounce_on)(dimension, level, contact, fall_distance, &mut answer);
    accepted_movement(answer, movement)
}

/// Whether a part of the block `state` answers `bounce_restitution`.
pub fn restitution_listens(state: BlockStateId) -> bool {
    RESTITUTION_HOOKS
        .get()
        .is_some_and(|hooks| (hooks.listens)(state))
}

/// `Entity.getBlockBounciness`' `Block.getBounceRestitution` for the entity landing on
/// `contact`'s block, whose own restitution is `own`, after any part of that block changed it
/// (`IBlockExtension.getBounceRestitution`) in `level`, on either side. A non-finite answer is
/// ignored: it would make the entity's velocity not a number.
pub fn bounce_restitution(level: &mut DamageLevel<'_>, contact: Contact<'_>, own: f32) -> f32 {
    let Some(hooks) = RESTITUTION_HOOKS.get() else {
        return own;
    };
    if !(hooks.listens)(contact.state) {
        return own;
    }
    let mut answer = own;
    (hooks.restitution)(level, contact, &mut answer);
    if answer.is_finite() { answer } else { own }
}

/// Keep the writes a landing on the block `state` at `position` asked for until the
/// entity's batch ends; nothing is kept when none were asked.
pub fn landed(state: BlockStateId, position: BlockPos, writes: Vec<LandingWrite>) {
    if !writes.is_empty() {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::Landed(Landed {
                state,
                position,
                writes,
            }));
        });
    }
}

/// Keep the level writes a mid-walk `entity_inside` contact asked for until the
/// entity's batch ends; nothing is kept when none were asked.
pub fn inside_written(state: BlockStateId, position: BlockPos, writes: Vec<InsideWrite>) {
    if !writes.is_empty() {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::InsideWritten(InsideWritten {
                state,
                position,
                writes,
            }));
        });
    }
}

/// Note a minecart on the rail `state` at `position`, if a part there listens; a world
/// without such a part pays one test per rail step.
pub fn minecart_pass(entity: i32, state: BlockStateId, position: BlockPos) {
    if LEVEL.get().is_some() && HOOKS.get().is_some_and(|hooks| (hooks.pass_listens)(state)) {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::MinecartPass(MinecartPass {
                entity,
                state,
                position,
            }));
        });
    }
}

/// Whether any contact is noted and not taken yet.
pub fn has_pending() -> bool {
    PENDING.with_borrow(|pending| !pending.is_empty())
}

/// The contacts noted since the last call, in the order they happened.
pub fn take_pending() -> Vec<Pending> {
    PENDING.with_borrow_mut(std::mem::take)
}

/// `Entity.checkInsideBlocks`' `isPrecise` for the block at `position`: the
/// move from `from` to `to` was long, or the box at `to` touches the block.
pub fn is_precise(
    from: horizon_core::geometry::Vec3,
    to: horizon_core::geometry::Vec3,
    at_target: horizon_core::geometry::Aabb,
    position: BlockPos,
) -> bool {
    let far = 0.999_990_000_000_252_6_f64;
    from.distance_squared(to) > far * far
        || at_target.intersects(horizon_core::geometry::Aabb::block(position))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `checkInsideBlocks`' `isPrecise`: a long move, or the box at the target
    /// touching the block.
    #[test]
    fn precision_follows_the_move_length_and_the_box() {
        use horizon_core::geometry::{Aabb, Vec3};
        let at = Aabb::new(Vec3::new(2.25, 2.0, 3.25), Vec3::new(2.75, 2.5, 3.75));
        let block = BlockPos::new(2, 2, 3);
        let from = Vec3::new(2.5, 2.0, 3.5);
        assert!(is_precise(from, from, at, block));
        assert!(!is_precise(from, from, at, BlockPos::new(1, 2, 3)));
        assert!(is_precise(
            Vec3::new(0.5, 2.0, 3.5),
            from,
            at,
            BlockPos::new(1, 2, 3)
        ));
    }

    /// A fall answer is clamped at zero and a non-finite one ignored; a bounce answer is
    /// used unless one component is non-finite.
    #[test]
    fn landing_answers_are_checked() {
        use horizon_core::geometry::Vec3;
        assert_eq!(accepted_damage(-2.0, 3.0), 0.0);
        assert_eq!(accepted_damage(5.0, 3.0), 5.0);
        assert_eq!(accepted_damage(f32::NAN, 3.0), 3.0);
        let kept = Vec3::new(0.0, 0.8, 0.0);
        assert_eq!(
            accepted_movement(Vec3::new(0.0, 0.4, 0.0), kept),
            Vec3::new(0.0, 0.4, 0.0)
        );
        assert_eq!(
            accepted_movement(Vec3::new(0.0, f64::INFINITY, 0.0), kept),
            kept
        );
    }

    /// A walk over a named grid: what it visited, in the walk's order, block moments
    /// and fluid moments apart.
    #[derive(Default)]
    struct Grid {
        blocks: std::collections::HashMap<[i32; 3], (BlockStateId, InsideShape, bool)>,
        fluid: std::collections::HashSet<[i32; 3]>,
        alive: bool,
        visits: Vec<(i32, [i32; 3], &'static str)>,
    }
    const STONE: BlockStateId = BlockStateId(1);
    impl Grid {
        fn cell(&mut self, at: [i32; 3], shape: InsideShape, fluid: bool) -> &mut Self {
            self.blocks.insert(at, (STONE, shape, fluid));
            if fluid {
                self.fluid.insert(at);
            }
            self
        }
    }
    impl super::InsideWalk for Grid {
        type Error = ();
        fn bounds_at(&self, at: horizon_core::geometry::Vec3) -> horizon_core::geometry::Aabb {
            horizon_core::geometry::Aabb::new(
                at.subtract(horizon_core::geometry::Vec3::new(0.3, 0.0, 0.3)),
                at.add_vector(horizon_core::geometry::Vec3::new(0.3, 1.8, 0.3)),
            )
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
        fn block(&self, at: BlockPos) -> BlockStateId {
            self.blocks
                .get(&[at.x, at.y, at.z])
                .map_or(BlockStateId(0), |(state, _, _)| *state)
        }
        fn is_air(&self, state: BlockStateId) -> bool {
            state == BlockStateId(0)
        }
        fn inside_shape(&mut self, state: BlockStateId, at: BlockPos) -> InsideShape {
            match self.blocks.get(&[at.x, at.y, at.z]) {
                Some((block, shape, _)) if *block == state => shape.clone(),
                _ => InsideShape::Full,
            }
        }
        fn collided_with_fluid(
            &self,
            at: BlockPos,
            _: horizon_core::geometry::Vec3,
            _: horizon_core::geometry::Vec3,
        ) -> bool {
            self.fluid.contains(&[at.x, at.y, at.z])
        }
        fn entity_inside(
            &mut self,
            at: BlockPos,
            _: BlockStateId,
            step: i32,
            precise: bool,
            _: &mut mod_api::InsideEffects,
        ) -> Result<(), ()> {
            self.visits.push((
                step,
                [at.x, at.y, at.z],
                if precise { "precise" } else { "rough" },
            ));
            Ok(())
        }
        fn fluid_inside(
            &mut self,
            at: BlockPos,
            step: i32,
            _: &mut mod_api::InsideEffects,
        ) -> Result<(), ()> {
            self.visits.push((step, [at.x, at.y, at.z], "fluid"));
            Ok(())
        }
    }

    fn records(
        from_to: [([f64; 3], [f64; 3]); 2],
    ) -> [horizon_world::entity_move::MovementRecord; 2] {
        from_to.map(|(from, to)| horizon_world::entity_move::MovementRecord {
            from: horizon_core::geometry::Vec3::new(from[0], from[1], from[2]),
            to: horizon_core::geometry::Vec3::new(to[0], to[1], to[2]),
            original: None,
        })
    }

    fn walk(grid: &mut Grid, movements: &[horizon_world::entity_move::MovementRecord]) {
        let mut visited = crate::fast_hash::FxHashSet::default();
        super::sweep_movements(grid, movements, &mut visited, &mut Default::default()).unwrap();
    }

    /// The walk visits a straight move's blocks in move order, one step each, `precise`
    /// when the box at the target touches the block.
    #[test]
    fn a_straight_move_visits_in_order() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        for x in 1..=3 {
            grid.cell([x, 64, 1], InsideShape::Full, false);
        }
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [3.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [3.5, 64.0, 1.5]),
            ]),
        );
        // The move crosses three blocks, so it is `movedFar`: every block is precise.
        assert_eq!(
            grid.visits,
            vec![
                (1, [1, 64, 1], "precise"),
                (2, [2, 64, 1], "precise"),
                (3, [3, 64, 1], "precise"),
            ]
        );
    }

    /// A recorded original walks its axes in `axisStepOrder`: y first, then the longer
    /// horizontal axis, with a fresh step per axis.
    #[test]
    fn an_axis_original_walks_y_first_then_the_longer_axis() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Full, false);
        grid.cell([1, 65, 1], InsideShape::Full, false);
        grid.cell([1, 66, 2], InsideShape::Full, false);
        let record = horizon_world::entity_move::MovementRecord {
            from: horizon_core::geometry::Vec3::new(1.5, 64.5, 1.5),
            to: horizon_core::geometry::Vec3::new(1.5, 66.5, 2.5),
            original: Some(horizon_core::geometry::Vec3::new(0.0, 2.0, 1.0)),
        };
        let mut visited = crate::fast_hash::FxHashSet::default();
        super::sweep_movements(&mut grid, &[record], &mut visited, &mut Default::default())
            .unwrap();
        assert_eq!(
            grid.visits,
            vec![
                (0, [1, 64, 1], "precise"),
                (0, [1, 65, 1], "precise"),
                (1, [1, 66, 2], "precise")
            ]
        );
    }

    /// Air is passed over, an empty shape is never inside, boxes sweep the move, and
    /// one visited set covers every movement of the tick.
    #[test]
    fn shapes_decide_inside_and_visited_is_shared() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Empty, false);
        // A one-high top slab shape the standing box only reaches at the end.
        grid.cell(
            [2, 64, 1],
            InsideShape::Boxes(vec![horizon_core::geometry::Aabb::new(
                horizon_core::geometry::Vec3::new(2.0, 64.9, 1.0),
                horizon_core::geometry::Vec3::new(3.0, 65.0, 2.0),
            )]),
            false,
        );
        grid.cell([3, 64, 1], InsideShape::Full, false);
        // The way back crosses both again over the same visited set: nothing new.
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [3.5, 64.0, 1.5]),
                ([3.5, 64.0, 1.5], [0.5, 64.0, 1.5]),
            ]),
        );
        assert_eq!(
            grid.visits,
            vec![(2, [2, 64, 1], "precise"), (3, [3, 64, 1], "precise")]
        );
    }

    /// The block's moment and the fluid's each get their own step advance, block first,
    /// and a dead entity stops the walk.
    #[test]
    fn fluid_advances_after_the_block_and_death_stops_the_walk() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Full, true);
        grid.cell([2, 64, 1], InsideShape::Full, false);
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
            ]),
        );
        assert_eq!(
            grid.visits,
            vec![
                (1, [1, 64, 1], "precise"),
                (1, [1, 64, 1], "fluid"),
                (2, [2, 64, 1], "precise"),
            ]
        );
        grid.alive = false;
        grid.visits.clear();
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
            ]),
        );
        assert!(grid.visits.is_empty(), "a dead entity is not walked");
    }

    /// A move over the 16-iteration budget ends with the target checked alone.
    #[test]
    fn an_exhausted_budget_checks_the_target_alone() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        for x in 1..=20 {
            grid.cell([x, 64, 1], InsideShape::Full, false);
        }
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [20.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [20.5, 64.0, 1.5]),
            ]),
        );
        // The budget ran out mid-move: fifteen cells on the way, then the target
        // alone, and nothing between them.
        assert_eq!(grid.visits.len(), 16);
        assert_eq!(grid.visits[0], (1, [1, 64, 1], "precise"));
        assert_eq!(grid.visits[14], (15, [15, 64, 1], "precise"));
        assert_eq!(grid.visits[15], (0, [20, 64, 1], "precise"));
        assert!(
            !grid
                .visits
                .iter()
                .any(|&(_, p, _)| (16..20).contains(&p[0])),
            "nothing between the budget and the target is visited"
        );
    }
}
