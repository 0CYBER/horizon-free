//! Custom brain behaviors: code a mod attaches to one
//! entity type at startup, asked wherever the species' brain is built — its
//! first tick, and every rebuild (a villager's profession change, growing up).
//! A mod's behaviors join the brain's list as vanilla's own do: same
//! activities, same priorities, started and stopped by `Brain.tick`'s order,
//! and a brain with none of them runs exactly as before.
//!
//! ```no_run
//! # use mod_api::{
//! #     Activity, Behavior, BehaviorCx, BehaviorRegistrar, EntityPart, Hooks,
//! #     Registrar, RegistrationContext,
//! # };
//! #[derive(Default)]
//! struct Mod;
//! # #[allow(unused)]
//! struct VillagerBehaviors;
//! #[mod_api::horizon_part]
//! impl EntityPart for VillagerBehaviors {
//!     fn register_behaviors(&self, behaviors: &mut BehaviorRegistrar<'_>) {
//!         behaviors.add(Activity::Idle, 5, 40, 80, &[], Stroll::default());
//!     }
//! }
//!
//! /// `StrollRandomly` in miniature: walk to a fixed spot for a while.
//! struct Stroll { ticks: u32 }
//! impl Default for Stroll {
//!     fn default() -> Self {
//!         Self { ticks: 0 }
//!     }
//! }
//! impl Behavior for Stroll {
//!     fn name(&self) -> &'static str {
//!         "Stroll"
//!     }
//!     fn start(&mut self, cx: &mut BehaviorCx<'_>) {
//!         let at = cx.mob().entity().position(cx.host()).map_or(
//!             [0.0; 3],
//!             |p| [p[0] + 4.0, p[1], p[2]],
//!         );
//!         cx.set_walk_target(
//!             [at[0] as i32, at[1] as i32, at[2] as i32],
//!             0.5,
//!             0,
//!         );
//!     }
//!     fn can_still_use(&mut self, _cx: &mut BehaviorCx<'_>) -> bool {
//!         self.ticks += 1;
//!         self.ticks < 30
//!     }
//! }
//!
//! #[mod_api::horizon_mod]
//! impl Hooks for Mod {
//!     fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
//!         r.entity_part("minecraft:villager", VillagerBehaviors);
//!     }
//! }
//! ```
use crate::{Host, LivingRef, MobRef, entity::goals::RegistrarMob};

/// `Activity`: the named lanes a brain's behaviors start in. Only the ones
/// this port's brains keep; a species' own brain has its own subset, and an
/// activity its brain has no slot for refuses at registration.
// exhaustive: a closed set the game matches without a wildcard
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub enum Activity {
    /// `core`: always active.
    Core,
    /// `idle`.
    Idle,
    /// `work`.
    Work,
    /// `play`.
    Play,
    /// `rest`.
    Rest,
    /// `meet`.
    Meet,
    /// `panic`.
    Panic,
    /// `raid`.
    Raid,
    /// `pre_raid`.
    PreRaid,
    /// `hide`.
    Hide,
    /// `fight`.
    Fight,
    /// `play_dead`.
    PlayDead,
    /// `sniff`.
    Sniff,
    /// `dig`.
    Dig,
    /// `emerge`.
    Emerge,
    /// `roar`.
    Roar,
    /// `investigate`.
    Investigate,
}

/// A memory the ported brains keep, as `BrainMemoryType` names vanilla's.
/// What a species' brain does not keep answers absent.
// exhaustive: a closed set the game matches without a wildcard
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub enum BrainMemory {
    /// `WALK_TARGET`: where the mob walks, how fast, how close is there.
    WalkTarget,
    /// `LOOK_TARGET`: what the mob looks at.
    LookTarget,
    /// `ATTACK_TARGET`: what the mob attacks.
    AttackTarget,
}

/// `MemoryStatus`: what an entry condition asks of a memory.
// exhaustive: a closed set the game matches without a wildcard
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, horizon_mod_api_macros::Wire)]
pub enum MemoryStatus {
    /// `VALUE_PRESENT`.
    Present,
    /// `VALUE_ABSENT`.
    Absent,
    /// `REGISTERED`: the brain keeps the memory at all.
    Registered,
}

/// Why `BehaviorRegistrar::add` refused.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum BehaviorRefusal {
    /// The entity type has no brain (`Behavior` runs nowhere on it).
    NoBrain,
    /// The species' brain has no slot for the activity.
    UnknownActivity(Activity),
    /// The duration window is not one vanilla's `nextInt(max + 1 - min)`
    /// draws: it needs `min >= 0` and `1 <= max + 1 - min <= i32::MAX`.
    BadDuration {
        /// The window's lower end, as registered.
        min: i32,
        /// The window's upper end, as registered.
        max: i32,
    },
}
impl std::fmt::Display for BehaviorRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBrain => write!(f, "the entity type has no brain"),
            Self::UnknownActivity(a) => write!(f, "the brain has no activity {a:?}"),
            Self::BadDuration { min, max } => write!(
                f,
                "the duration ({min}, {max}) is not one vanilla draws: min >= 0 and \
                 1 <= max + 1 - min <= i32::MAX"
            ),
        }
    }
}
impl std::error::Error for BehaviorRefusal {}

/// A mod's own brain behavior, run by the species' brain beside vanilla's:
/// started by `Brain.tick`'s start pass when its activity is active and its
/// entry condition holds, ticking until its duration runs out or
/// [`can_still_use`](Behavior::can_still_use) fails, stopped with vanilla's
/// own `stopAll`. One object per mob, built in
/// [`crate::EntityPart::register_behaviors`] wherever the brain is built, so
/// nothing of it is saved and a rebuilt brain starts fresh.
pub trait Behavior: Send + Sync + 'static {
    /// `Behavior.checkExtraStartConditions`: asked after the entry condition
    /// holds, before the duration is drawn.
    fn check_extra_start_conditions(&mut self, _cx: &mut BehaviorCx<'_>) -> bool {
        true
    }
    /// `Behavior.start`: the behavior began running.
    fn start(&mut self, _cx: &mut BehaviorCx<'_>) {}
    /// `Behavior.canStillUse`: whether a running behavior keeps running.
    /// Defaults to `false`, as vanilla's does — a behavior that does not
    /// override it lives out the brain tick that started it.
    fn can_still_use(&mut self, _cx: &mut BehaviorCx<'_>) -> bool {
        false
    }
    /// `Behavior.tick`: every brain tick while the behavior runs.
    fn tick(&mut self, _cx: &mut BehaviorCx<'_>) {}
    /// `Behavior.stop`: the behavior stopped — its end, `can_still_use`
    /// failing, or the brain's own `stopAll`.
    fn stop(&mut self, _cx: &mut BehaviorCx<'_>) {}
    /// The behavior's name for debugging (`Behavior.debugString`).
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
            .rsplit("::")
            .next()
            .unwrap_or("Behavior")
    }
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// What the game hands an [`crate::EntityPart`] where the species' brain is
/// built: the behaviors that join it, as `Brain.addActivity` fills vanilla's.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BehaviorRegistrar<'a> {
    inner: &'a mut dyn BehaviorRegistrarBackend,
    mob_id: &'a str,
    mod_id: &'static str,
    part: &'static str,
}
impl<'a> BehaviorRegistrar<'a> {
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(
        inner: &'a mut dyn BehaviorRegistrarBackend,
        mob_id: &'a str,
        mod_id: &'static str,
        part: &'static str,
    ) -> Self {
        Self {
            inner,
            mob_id,
            mod_id,
            part,
        }
    }
    /// Add `behavior` to the brain: it starts in `activity` lanes at
    /// `priority`, runs a duration drawn from the level random in
    /// `min_duration..=max_duration` (`Behavior.tryStart`'s draw), and starts
    /// only while every `entry` condition holds (`Behavior`'s entry
    /// condition map). It joins after the brain's own of the same activity
    /// and priority.
    pub fn add<P: Behavior>(
        &mut self,
        activity: Activity,
        priority: i32,
        min_duration: i32,
        max_duration: i32,
        entry: &'static [(BrainMemory, MemoryStatus)],
        behavior: P,
    ) -> Result<(), BehaviorRefusal> {
        let span = i64::from(max_duration) + 1 - i64::from(min_duration);
        if min_duration < 0 || !(1..=i64::from(i32::MAX)).contains(&span) {
            return Err(BehaviorRefusal::BadDuration {
                min: min_duration,
                max: max_duration,
            });
        }
        self.inner.add_behavior(RegisteredBehavior {
            mod_id: self.mod_id,
            part: self.part,
            activity,
            priority,
            min: min_duration,
            max: max_duration,
            entry,
            behavior: Box::new(behavior),
        })
    }
    /// The mob whose brain is being built, as far as construction goes: its
    /// entity type, the datum vanilla's brain packages branch on.
    pub fn mob(&self) -> RegistrarMob<'_> {
        RegistrarMob::new(self.mob_id)
    }
}

// no wire: registered callback and static mod attribution stay in the process-local behavior table.
/// One `BehaviorRegistrar::add` crossing into the game.
// hidden: the game's own transport, not part of the mod surface
// exhaustive: the game destructures it; mods never see it
#[doc(hidden)]
pub struct RegisteredBehavior {
    /// The registering mod's id.
    pub mod_id: &'static str,
    /// The registering part's type name.
    pub part: &'static str,
    /// `BehaviorRegistrar::add`'s activity.
    pub activity: Activity,
    /// `BehaviorRegistrar::add`'s priority.
    pub priority: i32,
    /// `BehaviorRegistrar::add`'s duration window.
    pub min: i32,
    /// `BehaviorRegistrar::add`'s duration window.
    pub max: i32,
    /// `BehaviorRegistrar::add`'s entry condition.
    pub entry: &'static [(BrainMemory, MemoryStatus)],
    /// The behavior itself, boxed.
    pub behavior: Box<dyn Behavior>,
}

// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub trait BehaviorRegistrarBackend {
    /// `BehaviorRegistrar::add`, the built behavior boxed.
    fn add_behavior(&mut self, registration: RegisteredBehavior) -> Result<(), BehaviorRefusal>;
    /// The entity type id of the mob being built.
    fn mob_id(&self) -> &str;
}

/// A brain's `WALK_TARGET` memory as a behavior reads it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, horizon_mod_api_macros::Wire)]
pub struct BrainWalkTarget {
    /// Where the mob walks.
    pub at: [f64; 3],
    /// How fast.
    pub speed: f64,
    /// How close counts as there.
    pub close: i32,
}
impl BrainWalkTarget {
    // hidden: the game's own construction, not part of the mod surface
    #[doc(hidden)]
    pub fn new(at: [f64; 3], speed: f64, close: i32) -> Self {
        Self { at, speed, close }
    }
}

// no wire: borrowed host or registrar backend; access stays scoped to the owning call.
/// What a [`Behavior`] runs with: its mob and the tick's read view — the
/// same mob-tick view [`crate::GoalContext`] is (the ticking mob itself, the
/// player the tick sees, the snapshot of the others), with the brain's own
/// memories typed on top. Writes go through the engine's own setters.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BehaviorCx<'a> {
    inner: &'a mut dyn BehaviorCxBackend,
}
impl<'a> BehaviorCx<'a> {
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn new(inner: &'a mut dyn BehaviorCxBackend) -> Self {
        Self { inner }
    }
    /// The mob this behavior runs on, a fresh handle each call.
    pub fn mob(&self) -> MobRef {
        self.inner.mob()
    }
    /// The read view of the mob's tick, as [`crate::GoalContext::host`].
    pub fn host(&self) -> &dyn Host {
        self.inner.host()
    }
    /// `WALK_TARGET`, where the brain walks to.
    pub fn walk_target(&self) -> Option<BrainWalkTarget> {
        self.inner.walk_target()
    }
    /// `LOOK_TARGET`, the position the brain looks at.
    pub fn look_target(&self) -> Option<[f64; 3]> {
        self.inner.look_target()
    }
    /// `ATTACK_TARGET`, the living entity the brain attacks, as the tick's
    /// view resolves it.
    pub fn attack_target(&self) -> Option<LivingRef> {
        self.inner.attack_target()
    }
    /// `set_brain_target` on the mob's own handle: writes the brain's
    /// `ATTACK_TARGET` memory through the engine's setter, with its refusals
    /// (a species whose brain keeps no attack target refuses, as the write
    /// handle does). `None` clears it.
    pub fn set_brain_target(&mut self, target: Option<LivingRef>) -> bool {
        self.inner.set_brain_target(target)
    }
    /// `SetWalkTarget`: writes the brain's `WALK_TARGET` memory through the
    /// engine's own path, where the brain's walk sink takes it from.
    pub fn set_walk_target(&mut self, at: [i32; 3], speed: f64, close: i32) {
        self.inner.set_walk_target(at, speed, close)
    }
}

// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub trait BehaviorCxBackend {
    /// `BehaviorCx::host`.
    fn host(&self) -> &dyn Host;
    /// `BehaviorCx::mob`.
    fn mob(&self) -> MobRef;
    /// `BehaviorCx::walk_target`.
    fn walk_target(&self) -> Option<BrainWalkTarget>;
    /// `BehaviorCx::look_target`.
    fn look_target(&self) -> Option<[f64; 3]>;
    /// `BehaviorCx::attack_target`.
    fn attack_target(&self) -> Option<LivingRef>;
    /// `BehaviorCx::set_brain_target`.
    fn set_brain_target(&mut self, target: Option<LivingRef>) -> bool;
    /// `BehaviorCx::set_walk_target`.
    fn set_walk_target(&mut self, at: [i32; 3], speed: f64, close: i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registrar over a recording backend, as the game builds it.
    struct Recording {
        added: Vec<(&'static str, Activity, i32, i32, i32, usize)>,
        refused: Vec<BehaviorRefusal>,
    }
    impl BehaviorRegistrarBackend for Recording {
        fn add_behavior(&mut self, r: RegisteredBehavior) -> Result<(), BehaviorRefusal> {
            self.added.push((
                r.behavior.name(),
                r.activity,
                r.priority,
                r.min,
                r.max,
                r.entry.len(),
            ));
            Ok(())
        }
        fn mob_id(&self) -> &str {
            "minecraft:villager"
        }
    }

    #[derive(Default)]
    struct Probe;
    impl Behavior for Probe {
        fn name(&self) -> &'static str {
            "Probe"
        }
    }

    #[test]
    fn the_registrar_passes_the_registration_through() {
        let mut backend = Recording {
            added: Vec::new(),
            refused: Vec::new(),
        };
        let mut behaviors =
            BehaviorRegistrar::new(&mut backend, "minecraft:villager", "test", "Part");
        behaviors
            .add(
                Activity::Idle,
                5,
                40,
                80,
                &[(BrainMemory::WalkTarget, MemoryStatus::Present)],
                Probe,
            )
            .unwrap();
        assert_eq!(behaviors.mob().id(), "minecraft:villager");
        assert_eq!(backend.added, &[("Probe", Activity::Idle, 5, 40, 80, 1)]);
        assert!(backend.refused.is_empty());
    }

    #[test]
    fn durations_outside_vanillas_draw_refuse_at_add() {
        let mut backend = Recording {
            added: Vec::new(),
            refused: Vec::new(),
        };
        let mut behaviors =
            BehaviorRegistrar::new(&mut backend, "minecraft:villager", "test", "Part");
        // min > max: the span is negative.
        assert_eq!(
            behaviors.add(Activity::Idle, 0, 80, 40, &[], Probe),
            Err(BehaviorRefusal::BadDuration { min: 80, max: 40 })
        );
        // Negative: no vanilla behavior runs one.
        assert_eq!(
            behaviors.add(Activity::Idle, 0, -1, 40, &[], Probe),
            Err(BehaviorRefusal::BadDuration { min: -1, max: 40 })
        );
        // The full int span overflows the draw's bound.
        assert_eq!(
            behaviors.add(Activity::Idle, 0, 0, i32::MAX, &[], Probe),
            Err(BehaviorRefusal::BadDuration {
                min: 0,
                max: i32::MAX
            })
        );
        // The vanilla corners pass: a fixed window, i32::MAX itself, and the
        // widest span that still draws.
        behaviors
            .add(Activity::Idle, 0, 60, 60, &[], Probe)
            .unwrap();
        behaviors
            .add(Activity::Idle, 0, i32::MAX, i32::MAX, &[], Probe)
            .unwrap();
        behaviors
            .add(Activity::Idle, 0, 0, i32::MAX - 1, &[], Probe)
            .unwrap();
        assert_eq!(backend.added.len(), 3);
    }

    #[test]
    fn refusals_explain_themselves() {
        assert_eq!(
            BehaviorRefusal::NoBrain.to_string(),
            "the entity type has no brain"
        );
        assert_eq!(
            BehaviorRefusal::UnknownActivity(Activity::Roar).to_string(),
            "the brain has no activity Roar"
        );
        assert_eq!(
            BehaviorRefusal::BadDuration { min: 80, max: 40 }.to_string(),
            "the duration (80, 40) is not one vanilla draws: min >= 0 and 1 <= max + 1 - min <= i32::MAX"
        );
    }
}
