//! Mod behaviors on brain-driven mobs (arismoko/horizon#824): the parts a
//! mod attaches to an entity type register their behaviors wherever the
//! species' brain is built — its first tick and every rebuild — and the
//! engine wraps each in a [`ModBehavior`] that starts and stops it in
//! `Brain.tick`'s own order. A brain with no mod behaviors keeps an empty
//! list: nothing of this module runs, and the species' random streams stay
//! exactly as they were.
use super::{Mob, MobWorld, Species, brain::Timed, part_goals::BehaviorOp};
use horizon_core::random::LevelRandom;
use mod_api::{
    BehaviorRegistrarBackend, RegisteredBehavior,
    content::crash::{ModCall, enter_native_mod},
};

/// One mod behavior, wrapped for a brain's list: the mod and part a panic in
/// it names, its entry condition, and `Behavior`'s own bookkeeping (running,
/// and until when) in the [`Timed`] every ported behavior keeps. A clone of
/// the mob (a corpse kept for its death events) keeps the wrapper's shape
/// with the behavior left out — a corpse runs no behaviors.
pub struct ModBehavior {
    mod_id: &'static str,
    part: &'static str,
    entry: &'static [(mod_api::BrainMemory, mod_api::MemoryStatus)],
    behavior: Option<Box<dyn mod_api::Behavior>>,
    t: Timed,
}
impl Clone for ModBehavior {
    fn clone(&self) -> Self {
        Self {
            mod_id: self.mod_id,
            part: self.part,
            entry: self.entry,
            behavior: None,
            t: Timed::new(self.t.min, self.t.max),
        }
    }
}
impl std::fmt::Debug for ModBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModBehavior")
            .field("part", &self.part)
            .field("behavior", &self.behavior.as_ref().map(|b| b.name()))
            .field("running", &self.t.running)
            .finish()
    }
}
impl ModBehavior {
    /// Wrap `registration`'s behavior for a brain's list.
    pub fn new(registration: RegisteredBehavior) -> Self {
        Self {
            mod_id: registration.mod_id,
            part: registration.part,
            entry: registration.entry,
            behavior: Some(registration.behavior),
            t: Timed::new(registration.min, registration.max),
        }
    }
    /// `BehaviorControl.getStatus() == RUNNING`.
    pub fn running(&self) -> bool {
        self.t.running
    }
    /// `Behavior.hasRequiredMemories`: the entry condition over the memories
    /// the species' brain keeps, engine-side, before any mod code runs.
    fn entry_met(&self, m: &Mob) -> bool {
        self.behavior.is_some()
            && self
                .entry
                .iter()
                .all(|&(memory, want)| matches(want, memory_present(m, memory)))
    }
    /// Run `op` through the tick's goal host, which builds the behavior's
    /// context and names the mod in a panic. A panic is contained (arismoko/horizon#824's
    /// review condition): the behavior is stopped and dropped, and the tick
    /// it panicked in goes on.
    fn run(&mut self, m: &mut Mob, w: &MobWorld<'_>, op: BehaviorOp) -> Option<bool> {
        let behavior = self.behavior.as_mut()?;
        let host = w.goal_host?;
        let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.run_behavior(behavior, m, w, op, self.mod_id, self.part)
        }));
        match answer {
            Ok(answer) => answer,
            Err(_) => {
                self.behavior = None;
                self.t.running = false;
                None
            }
        }
    }
    /// `BehaviorControl.tryStart`: the entry condition and the mod's own
    /// `check_extra_start_conditions`, then the duration drawn from the level
    /// random (`Behavior.tryStart`'s draw) and `start`. A start that fails
    /// either check draws nothing.
    pub fn try_start(
        &mut self,
        m: &mut Mob,
        w: &MobWorld<'_>,
        lr: &mut LevelRandom,
        now: i64,
    ) -> bool {
        if !self.entry_met(m) {
            return false;
        }
        if !self
            .run(m, w, BehaviorOp::CheckExtraStartConditions)
            .unwrap_or(false)
        {
            return false;
        }
        self.t.begin(lr, now);
        self.run(m, w, BehaviorOp::Start);
        self.t.running
    }
    /// `BehaviorControl.tickOrStop`: while the duration holds and the mod's
    /// `can_still_use` answers yes, `tick`; else `doStop`.
    pub fn tick_or_stop(&mut self, m: &mut Mob, w: &MobWorld<'_>, now: i64) {
        let still =
            !self.t.timed_out(now) && self.run(m, w, BehaviorOp::CanStillUse).unwrap_or(false);
        if still {
            self.run(m, w, BehaviorOp::Tick);
        } else {
            self.do_stop(m, w);
        }
    }
    /// `BehaviorControl.doStop`, and what `Brain.stopAll` reaches a brain's
    /// list with: `stop` unless the behavior is already gone.
    pub fn do_stop(&mut self, m: &mut Mob, w: &MobWorld<'_>) {
        self.t.running = false;
        self.run(m, w, BehaviorOp::Stop);
    }
    /// `Brain.resetIfStop`'s shape for a brain the mob grew out of: stopped
    /// without a callback, the behavior dropped.
    pub fn reset(&mut self) {
        self.t.running = false;
        self.behavior = None;
    }
}

/// `Brain.checkMemory` over `present`: `Some(true)` a memory with a value,
/// `Some(false)` one without, `None` one the brain does not keep.
fn matches(want: mod_api::MemoryStatus, present: Option<bool>) -> bool {
    match want {
        mod_api::MemoryStatus::Present => present == Some(true),
        mod_api::MemoryStatus::Absent => present == Some(false),
        mod_api::MemoryStatus::Registered => present.is_some(),
    }
}

/// `Brain.checkMemory` for the memories the wired brains keep, by species:
/// `Some` whether the memory has a value, `None` a memory that species'
/// brain does not register (every condition on it fails, as vanilla's
/// `checkMemory` answers false for an unregistered memory).
fn memory_present(m: &Mob, memory: mod_api::BrainMemory) -> Option<bool> {
    match m.species() {
        Species::Villager => {
            let mem = super::villager_brain::mem_ref(m)?;
            let present = match memory {
                mod_api::BrainMemory::WalkTarget => mem.walk_target.present(),
                mod_api::BrainMemory::LookTarget => mem.look_target.present(),
                // The villager brain registers no attack target.
                mod_api::BrainMemory::AttackTarget => return None,
                _ => return None,
            };
            Some(present)
        }
        Species::Sniffer
        | Species::CopperGolem
        | Species::Tadpole
        | Species::HappyGhast
        | Species::Axolotl => {
            let mem = super::animal_brain::mem_ref(m)?;
            let present = match memory {
                mod_api::BrainMemory::WalkTarget => mem.walk_target.present(),
                mod_api::BrainMemory::LookTarget => mem.look_target.present(),
                // Only the axolotl's brain registers an attack target.
                mod_api::BrainMemory::AttackTarget if m.species() == Species::Axolotl => {
                    mem.attack_target.present()
                }
                _ => return None,
            };
            Some(present)
        }
        // The breeze's and the creaking's brain (its memories are the
        // villager-shaped ones).
        Species::Breeze | Species::Creaking => {
            let mem = super::monster_brain::mem_ref(m);
            Some(match memory {
                mod_api::BrainMemory::WalkTarget => mem.walk_target.present(),
                mod_api::BrainMemory::LookTarget => mem.look_target.present(),
                mod_api::BrainMemory::AttackTarget => mem.attack_target.present(),
                _ => return None,
            })
        }
        Species::Warden => {
            let mem = super::warden_brain::mem_ref(m)?;
            Some(match memory {
                mod_api::BrainMemory::WalkTarget => mem.walk_target.present(),
                mod_api::BrainMemory::LookTarget => mem.look_target.present(),
                mod_api::BrainMemory::AttackTarget => mem.attack_target.present(),
                _ => return None,
            })
        }
        _ => None,
    }
}

/// The brain's `WALK_TARGET` memory as a behavior reads it, at the position
/// the tick's view resolves its tracker to. None where the species' brain
/// keeps no walk target.
pub fn walk_target(m: &Mob, w: &MobWorld<'_>) -> Option<mod_api::BrainWalkTarget> {
    match m.species() {
        Species::Villager => {
            let walk = super::villager_brain::mem_ref(m)?.walk_target.value()?;
            let at = walk.target.position(w)?;
            Some(mod_api::BrainWalkTarget::new(
                at,
                f64::from(walk.speed),
                walk.close,
            ))
        }
        Species::Breeze | Species::Creaking => {
            let walk = super::monster_brain::mem_ref(m).walk_target.value()?;
            let at = walk.target.position(w)?;
            Some(mod_api::BrainWalkTarget::new(
                at,
                f64::from(walk.speed),
                walk.close,
            ))
        }
        Species::Warden => {
            let walk = super::warden_brain::mem_ref(m)?.walk_target.value()?;
            let at = walk.target.position(w)?;
            Some(mod_api::BrainWalkTarget::new(
                at,
                f64::from(walk.speed),
                walk.close,
            ))
        }
        Species::Sniffer
        | Species::CopperGolem
        | Species::Tadpole
        | Species::HappyGhast
        | Species::Axolotl => {
            let walk = super::animal_brain::mem_ref(m)?.walk_target.value()?;
            let at = walk.target.position(w)?;
            Some(mod_api::BrainWalkTarget::new(
                at,
                f64::from(walk.speed),
                walk.close,
            ))
        }
        _ => None,
    }
}

/// The brain's `LOOK_TARGET` memory: the position its tracker resolves to.
/// None where the species' brain keeps no look target.
pub fn look_target(m: &Mob, w: &MobWorld<'_>) -> Option<[f64; 3]> {
    match m.species() {
        Species::Villager => super::villager_brain::mem_ref(m)?
            .look_target
            .value()?
            .position(w),
        Species::Sniffer
        | Species::CopperGolem
        | Species::Tadpole
        | Species::HappyGhast
        | Species::Axolotl => super::animal_brain::mem_ref(m)?
            .look_target
            .value()?
            .position(w),
        Species::Breeze | Species::Creaking => super::monster_brain::mem_ref(m)
            .look_target
            .value()?
            .position(w),
        Species::Warden => super::warden_brain::mem_ref(m)?
            .look_target
            .value()?
            .position(w),
        _ => None,
    }
}

/// `SetWalkTarget`: writes the brain's `WALK_TARGET` memory through the
/// engine's own path, where the species' walk sink takes it from.
pub fn set_walk_target(m: &mut Mob, at: [i32; 3], speed: f64, close: i32) {
    match m.species() {
        Species::Villager => {
            super::villager_brain::mem(m).walk_target.set(
                super::villager_brain::WalkTarget::block(at, speed as f32, close),
            );
        }
        Species::Sniffer
        | Species::CopperGolem
        | Species::Tadpole
        | Species::HappyGhast
        | Species::Axolotl => {
            super::animal_brain::mem(m)
                .walk_target
                .set(super::animal_brain::WalkTarget {
                    target: super::animal_brain::Tracker::Block(at),
                    speed: speed as f32,
                    close,
                });
        }
        // The breeze's, the creaking's and the warden's memories are the
        // villager-shaped ones.
        Species::Breeze | Species::Creaking => {
            super::monster_brain::mem(m)
                .walk_target
                .set(super::villager_brain::WalkTarget::block(
                    at,
                    speed as f32,
                    close,
                ));
        }
        Species::Warden => {
            super::warden_brain::mem(m)
                .walk_target
                .set(super::villager_brain::WalkTarget::block(
                    at,
                    speed as f32,
                    close,
                ));
        }
        _ => {}
    }
}

/// Whether the species thinks with one of the brains wired for mod behaviors
/// (arismoko/horizon#824): where `register_behaviors` runs at the brain's own construction.
/// The species' brains join commit by commit: the villager, then the
/// animal five, then the monster pair and the warden.
pub fn has_brain(s: Species) -> bool {
    matches!(
        s,
        Species::Villager
            | Species::Sniffer
            | Species::CopperGolem
            | Species::Tadpole
            | Species::HappyGhast
            | Species::Axolotl
            | Species::Breeze
            | Species::Creaking
            | Species::Warden
    )
}

/// Ask the installed entity parts for the mob's behaviors (arismoko/horizon#824), each into
/// `backend` — the brain module's own, which knows its activities. The parts
/// are named in a panic the way `register_goals` names them.
pub fn attach(mob_id: &str, backend: &mut dyn BehaviorRegistrarBackend) {
    if !mod_api::__private::has_entity_parts() {
        return;
    }
    let parts = mod_api::__private::entity_parts();
    let list = parts.entity_parts(mob_id);
    if list.is_empty() {
        return;
    }
    for attached in list {
        let _guard = enter_native_mod(ModCall {
            id: attached.mod_id(),
            hook: "register_behaviors",
            part: Some(attached.type_name()),
        });
        let mut registrar = mod_api::BehaviorRegistrar::new(
            backend,
            mob_id,
            attached.mod_id(),
            attached.type_name(),
        );
        attached.part().register_behaviors(&mut registrar);
    }
}

/// `register_behaviors` for a type with no brain (arismoko/horizon#824): every add refuses
/// with the typed error, so the mod learns at construction that its behavior
/// runs nowhere on this type. Asked beside `part_goals::attach` at every
/// `Mob` construction.
pub fn attach_refused(mob_id: &str) {
    struct Refused<'a>(&'a str);
    impl BehaviorRegistrarBackend for Refused<'_> {
        fn add_behavior(
            &mut self,
            _registration: RegisteredBehavior,
        ) -> Result<(), mod_api::BehaviorRefusal> {
            Err(mod_api::BehaviorRefusal::NoBrain)
        }
        fn mob_id(&self) -> &str {
            self.0
        }
    }
    attach(mob_id, &mut Refused(mob_id));
}

/// The construction-time half of arismoko/horizon#824, beside `part_goals::attach`: a type
/// whose species has no brain gets its `register_behaviors` asked with a
/// registrar that refuses every add. A brain-driven type registers at its
/// brain's own construction instead, so nothing is asked or built twice.
pub fn attach_at_construction(m: &Mob) {
    if has_brain(m.species()) {
        return;
    }
    attach_refused(&m.kind.id);
}
