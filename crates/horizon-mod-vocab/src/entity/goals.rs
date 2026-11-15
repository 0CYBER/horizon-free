//! Custom mob goals: code a mod attaches to one entity
//! type at startup, asked at every `Mob` construction, spawning or loading, the
//! moment vanilla's `registerGoals` fan-out ends. A mod's goals join the two
//! selectors as vanilla's own do — same priorities, same flag locks, same
//! interruption — and tick inside the mob's own AI step.
//!
//! ```no_run
//! # use mod_api::{
//! #     EntityPart, Goal, GoalContext, GoalFlags, GoalRegistrar, Hooks, Registrar,
//! #     RegistrationContext, RandomSource,
//! # };
//! # use std::sync::atomic::{AtomicU32, Ordering};
//! #[derive(Default)]
//! struct Mod;
//! # #[allow(unused)]
//! struct ZombieGoals;
//! #[mod_api::horizon_part]
//! impl EntityPart for ZombieGoals {
//!     fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
//!         goals.add_goal(7, Stare::default());
//!     }
//! }
//!
//! /// `LookAtPlayerGoal` in miniature: watch a fixed spot for a while.
//! struct Stare { ticks: u32 }
//! impl Default for Stare {
//!     fn default() -> Self {
//!         Self { ticks: 0 }
//!     }
//! }
//! impl Goal for Stare {
//!     fn flags(&self) -> GoalFlags {
//!         GoalFlags::LOOK
//!     }
//!     fn name(&self) -> &'static str {
//!         "Stare"
//!     }
//!     fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
//!         cx.random().next_int_bound(40).unwrap_or(0) == 0
//!     }
//!     fn start(&mut self, cx: &mut GoalContext<'_>) {
//!         self.ticks = 20 + cx.random().next_int_bound(40).unwrap_or(0) as u32;
//!     }
//!     fn tick(&mut self, cx: &mut GoalContext<'_>) {
//!         self.ticks -= 1;
//!         cx.look_at(cx.mob().entity().position(cx.host()).map_or(
//!             [0.0; 3],
//!             |p| [p[0] + 8.0, p[1], p[2]],
//!         ));
//!     }
//!     fn can_continue(&mut self, _cx: &mut GoalContext<'_>) -> bool {
//!         self.ticks > 0
//!     }
//! }
//!
//! #[mod_api::horizon_mod]
//! impl Hooks for Mod {
//!     fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
//!         r.entity_part("minecraft:zombie", ZombieGoals);
//!     }
//! }
//! ```
use crate::{Host, LegacyRandom, LivingRef, MobRef, MoveToPathError, PathNode};

/// The MOVE, LOOK, JUMP and TARGET bits a [`Goal`] claims, as vanilla's
/// `Goal.Flag` set does: at most one goal holding each flag runs at a time, and
/// a goal's flags are what a higher priority interrupts.
// exhaustive: its field is private, so mods cannot build or destructure it
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub struct GoalFlags(u8);
impl GoalFlags {
    /// The empty set: a goal no other goal competes with.
    pub const NONE: Self = Self(0);
    /// `Goal.Flag.MOVE`: the goal drives the mob's navigation.
    pub const MOVE: Self = Self(1);
    /// `Goal.Flag.LOOK`: the goal drives the mob's head.
    pub const LOOK: Self = Self(2);
    /// `Goal.Flag.JUMP`: the goal drives the mob's jump control.
    pub const JUMP: Self = Self(4);
    /// `Goal.Flag.TARGET`: the goal drives the mob's attack target.
    pub const TARGET: Self = Self(8);
    /// Both sets at once.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    /// Whether `self` holds every flag of `other`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    // hidden: the engine's own selector wants the raw bits.
    #[doc(hidden)]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// A mod's own mob goal, run by the mob's goal selector beside vanilla's: the
/// same priorities and interruption, its state its own (one object per mob,
/// built in [`crate::EntityPart::register_goals`] at every `Mob` construction, so a
/// mob loaded from a chunk gets fresh goals and nothing of them is saved).
///
/// The draws a goal makes come from [`GoalContext::random`], the mob's own
/// random, so its stream keeps vanilla's shape. A goal that needs memory
/// across saves keeps it in attachments, as vanilla keeps none either.
pub trait Goal: Send + Sync + 'static {
    /// The flags the goal claims while it runs.
    fn flags(&self) -> GoalFlags;
    /// `Goal.canUse`: whether the goal may start now.
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool;
    /// `Goal.canContinueToUse`: whether a running goal keeps running. Defaults
    /// to `can_use`.
    fn can_continue(&mut self, cx: &mut GoalContext<'_>) -> bool {
        self.can_use(cx)
    }
    /// `Goal.start`: the goal began running.
    fn start(&mut self, _cx: &mut GoalContext<'_>) {}
    /// `Goal.stop`: the goal stopped, replaced or done.
    fn stop(&mut self, _cx: &mut GoalContext<'_>) {}
    /// `Goal.tick`: the goal's every-other-tick (or, with
    /// [`requires_update_every_tick`](Goal::requires_update_every_tick), every
    /// tick) step while it runs.
    fn tick(&mut self, _cx: &mut GoalContext<'_>) {}
    /// `Goal.requiresUpdateEveryTick`: tick every tick, not at vanilla's
    /// reduced cadence.
    fn requires_update_every_tick(&self) -> bool {
        false
    }
    /// `Goal.isInterruptable`: false keeps a lower-priority goal from replacing
    /// this one (the flags it holds stay locked until it ends).
    fn is_interruptable(&self) -> bool {
        true
    }
    /// The goal's name for debugging (`Goal.toString`).
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
            .rsplit("::")
            .next()
            .unwrap_or("Goal")
    }
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// What the game hands an [`crate::EntityPart`]: the two selectors of the mob being
/// built, as vanilla's `registerGoals` fills them.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct GoalRegistrar<'a> {
    inner: &'a mut dyn GoalRegistrarBackend,
}
impl<'a> GoalRegistrar<'a> {
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(inner: &'a mut dyn GoalRegistrarBackend) -> Self {
        Self { inner }
    }
    /// `GoalSelector.addGoal(priority, goal)` on the mob's goal selector.
    pub fn add_goal<P: Goal>(&mut self, priority: i32, goal: P) {
        self.inner.add_goal(priority, Box::new(goal));
    }
    /// `GoalSelector.addGoal` on the mob's target selector: the
    /// nearest-hurt-target kinds, as vanilla's second selector holds them.
    pub fn add_target_goal<P: Goal>(&mut self, priority: i32, goal: P) {
        self.inner.add_target_goal(priority, Box::new(goal));
    }
    /// The mob being built, as far as construction goes: its entity type, the
    /// datum vanilla's per-species `registerGoals` branches on.
    pub fn mob(&self) -> RegistrarMob<'_> {
        RegistrarMob {
            id: self.inner.mob_id(),
        }
    }
}

// no wire: borrowed call-scoped view; transport uses owned snapshots rather than serializing references.
/// The entity a [`GoalRegistrar`] is building goals for, at construction: its
/// type id, the datum vanilla's per-species `registerGoals` branches on.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct RegistrarMob<'a> {
    id: &'a str,
}
impl<'a> RegistrarMob<'a> {
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(id: &'a str) -> Self {
        Self { id }
    }
    /// The entity type's id (`minecraft:zombie`).
    pub fn id(&self) -> &str {
        self.id
    }
}

// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub trait GoalRegistrarBackend {
    /// `GoalRegistrar::add_goal`, the built goal boxed.
    fn add_goal(&mut self, priority: i32, goal: Box<dyn Goal>);
    /// `GoalRegistrar::add_target_goal`, the built goal boxed.
    fn add_target_goal(&mut self, priority: i32, goal: Box<dyn Goal>);
    /// The entity type id of the mob being built.
    fn mob_id(&self) -> &str;
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// What a [`Goal`] runs with: its mob, the world read view of the mob's tick,
/// the mob's own random, and the writes a goal makes to its own mob.
///
/// Reads through [`host`](GoalContext::host) answer the ticking mob itself
/// directly; every other entity answers from the tick's snapshot — a
/// `getEntities` of the moment the mob list was taken out — so a read the
/// snapshot does not carry answers `None` (the mob's gear, its effects), and
/// writes to other entities are not part of a goal at all. The level's own
/// reads work as anywhere else.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct GoalContext<'a> {
    inner: &'a mut dyn GoalCxBackend,
}
impl<'a> GoalContext<'a> {
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(inner: &'a mut dyn GoalCxBackend) -> Self {
        Self { inner }
    }
    /// The mob this goal runs on, a fresh handle each call: valid until the
    /// world reopens, as every handle is.
    pub fn mob(&self) -> MobRef {
        self.inner.mob()
    }
    /// The read view of the mob's tick: the ticking mob directly, other
    /// entities from the tick's snapshot, `None` where a read is not carried.
    pub fn host(&self) -> &dyn Host {
        self.inner.host()
    }
    /// The mob's own random (`mob.getRandom()`): the stream vanilla's goals
    /// draw from, so a mod's draws keep the mob's stream in shape.
    pub fn random(&mut self) -> &mut LegacyRandom {
        self.inner.random()
    }
    /// `getNavigation().moveTo(x, y, z, speed)`: start a path to `at`. False
    /// when the numbers are not ones the pathfinder takes or no path starts.
    /// As in vanilla, a goal of the mob's own that creates a path replaces
    /// this one next tick.
    pub fn move_to(&mut self, at: [f64; 3], speed: f64) -> bool {
        self.inner.move_to(at, speed)
    }
    /// `getNavigation().moveTo(path, speed)`: follow a path the mod computed,
    /// as [`MobRef::move_to_path`](crate::MobRef::move_to_path) does on the
    /// handle. The engine classifies each node's type on submit and trims the
    /// path; the path is followed as given rather than judged by that type.
    /// A supplied path keeps no recompute target, so a block change on it or
    /// the stuck check drops it without a search. As in vanilla, a goal of the
    /// mob's own that creates a path replaces this one next tick. The mob this
    /// goal runs on is always there, so the refusals are `NotFinite` and
    /// `Empty`, the same [`MoveToPathError`] the handle answers.
    pub fn move_to_path(&mut self, path: Vec<PathNode>, speed: f64) -> Result<(), MoveToPathError> {
        self.inner.move_to_path(path, speed)
    }
    /// `getNavigation().stop()`.
    pub fn stop_navigation(&mut self) {
        self.inner.stop_navigation()
    }
    /// `getLookControl().setLookAt(x, y, z)`.
    pub fn look_at(&mut self, at: [f64; 3]) {
        self.inner.look_at(at)
    }
    /// `setTarget`; `None` clears it. The target resolves through the tick's
    /// view: a mob, or the player the tick sees. False when the target cannot
    /// be stored — the mob's brain owns its target, as the write handle's own
    /// rules refuse — and then the mob's target is unchanged.
    pub fn set_target(&mut self, target: Option<LivingRef>) -> bool {
        self.inner.set_target(target)
    }
    /// `getJumpControl().jump()`.
    pub fn jump(&mut self) {
        self.inner.jump()
    }
}

// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub trait GoalCxBackend {
    /// `GoalContext::host`.
    fn host(&self) -> &dyn Host;
    /// `GoalContext::mob`.
    fn mob(&self) -> MobRef;
    /// `GoalContext::random`.
    fn random(&mut self) -> &mut LegacyRandom;
    /// `GoalContext::move_to`.
    fn move_to(&mut self, at: [f64; 3], speed: f64) -> bool;
    /// `GoalContext::move_to_path`.
    fn move_to_path(&mut self, path: Vec<PathNode>, speed: f64) -> Result<(), MoveToPathError>;
    /// `GoalContext::stop_navigation`.
    fn stop_navigation(&mut self);
    /// `GoalContext::look_at`.
    fn look_at(&mut self, at: [f64; 3]);
    /// `GoalContext::set_target`.
    fn set_target(&mut self, target: Option<LivingRef>) -> bool;
    /// `GoalContext::jump`.
    fn jump(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registrar over a recording backend, as the game builds it.
    struct Recording {
        goals: Vec<(i32, &'static str)>,
        targets: Vec<(i32, &'static str)>,
    }
    impl GoalRegistrarBackend for Recording {
        fn add_goal(&mut self, priority: i32, goal: Box<dyn Goal>) {
            self.goals.push((priority, goal.name()));
        }
        fn add_target_goal(&mut self, priority: i32, goal: Box<dyn Goal>) {
            self.targets.push((priority, goal.name()));
        }
        fn mob_id(&self) -> &str {
            "minecraft:zombie"
        }
    }

    #[derive(Default)]
    struct Probe;
    impl Goal for Probe {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn can_use(&mut self, _: &mut GoalContext<'_>) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "Probe"
        }
    }

    #[test]
    fn the_registrar_feeds_both_selectors_and_reads_the_type() {
        let mut backend = Recording {
            goals: Vec::new(),
            targets: Vec::new(),
        };
        let mut goals = GoalRegistrar::new(&mut backend);
        goals.add_goal(3, Probe);
        goals.add_target_goal(2, Probe);
        assert_eq!(goals.mob().id(), "minecraft:zombie");
        assert_eq!(backend.goals, [(3, "Probe")]);
        assert_eq!(backend.targets, [(2, "Probe")]);
    }

    #[test]
    fn flag_sets_union_and_contain() {
        assert!(GoalFlags::MOVE.contains(GoalFlags::MOVE));
        assert!(!GoalFlags::MOVE.contains(GoalFlags::LOOK));
        let both = GoalFlags::MOVE.union(GoalFlags::LOOK);
        assert!(both.contains(GoalFlags::MOVE));
        assert!(both.contains(GoalFlags::LOOK));
        assert!(!both.contains(GoalFlags::JUMP));
    }
}
