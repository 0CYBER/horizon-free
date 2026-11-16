//! Tests for the mod behavior runner (arismoko/horizon#824): the vanilla
//! `Behavior` contract over a real game — the start checks, the level-random
//! duration draw, the timeout, `can_still_use` and its one-tick default,
//! panic containment, and the typed `NoBrain` refusal — driven through the
//! same goal-host view the goal tests use.
use super::super::creatures;
use super::super::native_entity_refs::tests::with_game;
use super::tests::zombie;
use horizon_client::health::DamageLevel;
use horizon_client::join_seq::RestoreOrder;
use horizon_client::mobs::{self, Mob, MobWorld};
use mod_api::{
    Activity, Behavior, BehaviorCx, BehaviorRegistrar, BrainMemory, EntityPart, Hooks,
    MemoryStatus, ModEntry, RegisteredBehavior, Registrar, RegistrationContext,
};
use std::sync::Mutex;

// The journals and registration plan below are shared across these tests.
static TEST: Mutex<()> = Mutex::new(());

/// The mods of these tests, installed with their parts, as a game's open does.
fn install_game_mods() {
    mod_platform::__private::install(vec![ModEntry::new("test", Box::new(Mod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("test", Box::new(Mod))]).unwrap();
}

/// What one test behavior did, in order, tagged by behavior.
static CALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The refusals a `register_behaviors` saw.
static REFUSALS: Mutex<Vec<mod_api::BehaviorRefusal>> = Mutex::new(Vec::new());

fn record(call: &str) {
    CALLS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(call.to_owned());
}

/// The behavior every runner test drives: journals its calls and answers
/// `can_still_use` for `still_for` ticks.
struct Probe {
    tag: &'static str,
    still_for: i32,
    ticks: i32,
    panic_in_start: bool,
}
impl Probe {
    fn still_for(still_for: i32) -> Self {
        Self {
            tag: "probe",
            still_for,
            ticks: 0,
            panic_in_start: false,
        }
    }
    fn named(tag: &'static str, still_for: i32) -> Self {
        Self {
            tag,
            ..Self::still_for(still_for)
        }
    }
    fn fire(&self, call: &str) {
        record(&format!("{}/{}", self.tag, call));
    }
}
impl Behavior for Probe {
    fn name(&self) -> &'static str {
        "Probe"
    }
    fn check_extra_start_conditions(&mut self, _cx: &mut BehaviorCx<'_>) -> bool {
        self.fire("check");
        true
    }
    fn start(&mut self, cx: &mut BehaviorCx<'_>) {
        self.fire("start");
        // The mob's own handle answers through the tick's view.
        assert_eq!(cx.mob().entity().id(), 8);
        if self.panic_in_start {
            panic!("a behavior's bug");
        }
    }
    fn can_still_use(&mut self, _cx: &mut BehaviorCx<'_>) -> bool {
        self.ticks += 1;
        self.ticks <= self.still_for
    }
    fn tick(&mut self, _cx: &mut BehaviorCx<'_>) {
        self.fire("tick");
    }
    fn stop(&mut self, _cx: &mut BehaviorCx<'_>) {
        self.fire("stop");
    }
}

/// One wrapped behavior, as a brain's list holds it.
fn wrapped(min: i32, max: i32, behavior: Box<dyn Behavior>) -> mobs::mod_behaviors::ModBehavior {
    mobs::mod_behaviors::ModBehavior::new(RegisteredBehavior {
        mod_id: "test",
        part: "ZombieBehaviors",
        activity: Activity::Idle,
        priority: 5,
        min,
        max,
        entry: &[],
        behavior,
    })
}

/// The mob's brain-tick shape, as the goal tests lend it (arismoko/horizon#1077): the mob
/// lifted out of the game, the world with the goal host beside it, and a
/// level random the test owns, all handed to `drive` together.
fn with_mob_world(
    game: &mut super::super::Game<'_>,
    drive: impl FnOnce(&mut Mob, &MobWorld<'_>, &mut DamageLevel<'_>),
) {
    game.options.dedicated = Some(Vec::new());
    let mut mobs = game.mobs.lift();
    let snapshot: Vec<_> = mobs
        .iter()
        .map(|m| creatures::seen(m, game.level.player_uuid, game.items()))
        .collect();
    let mut w = game.mob_world_base(&snapshot);
    w.goal_host = Some(&*game);
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let mut level = test_level.level();
    drive(&mut mobs[0], &w, &mut level);
    game.mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
}

/// The level random's state, for the draw assertions.
fn random_of(level: &mut DamageLevel<'_>) -> (u64, Option<f64>) {
    (&**level.random()).into()
}

/// The mods of the `NoBrain` test: a part on a zombie whose one behavior add
/// records its refusal, and a part on the villager whose adds the test
/// switches through `ADDS` (empty: a part that adds none).
struct ZombieBehaviors;
#[mod_api::horizon_part]
impl EntityPart for ZombieBehaviors {
    fn register_behaviors(&self, behaviors: &mut BehaviorRegistrar<'_>) {
        if let Err(e) = behaviors.add(Activity::Idle, 5, 40, 80, &[], Probe::still_for(1)) {
            REFUSALS.lock().unwrap().push(e);
        }
    }
}
/// What the parts add, one `(entity type, add)` per entry; the test sets it
/// before the species' brain is built.
type Add = for<'a> fn(&mut BehaviorRegistrar<'a>);
static ADDS: Mutex<Vec<(&'static str, Add)>> = Mutex::new(Vec::new());
/// One `(entity type, add)` entry, typed so the closures coerce.
fn plan(id: &'static str, add: Add) -> (&'static str, Add) {
    (id, add)
}
/// One part over the shared plan, for one entity type.
macro_rules! plan_part {
    ($name:ident, $id:literal) => {
        struct $name;
        #[mod_api::horizon_part]
        impl EntityPart for $name {
            fn register_behaviors(&self, behaviors: &mut BehaviorRegistrar<'_>) {
                for (id, add) in ADDS.lock().unwrap().iter() {
                    if *id == $id {
                        add(behaviors);
                    }
                }
            }
        }
    };
}
plan_part!(VillagerBehaviors, "minecraft:villager");
plan_part!(SnifferBehaviors, "minecraft:sniffer");
plan_part!(AxolotlBehaviors, "minecraft:axolotl");
plan_part!(GhastlingBehaviors, "minecraft:happy_ghast");
plan_part!(BreezeBehaviors, "minecraft:breeze");
plan_part!(WardenBehaviors, "minecraft:warden");
struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        r.entity_part("minecraft:zombie", ZombieBehaviors);
        r.entity_part("minecraft:villager", VillagerBehaviors);
        r.entity_part("minecraft:sniffer", SnifferBehaviors);
        r.entity_part("minecraft:axolotl", AxolotlBehaviors);
        r.entity_part("minecraft:happy_ghast", GhastlingBehaviors);
        r.entity_part("minecraft:breeze", BreezeBehaviors);
        r.entity_part("minecraft:warden", WardenBehaviors);
    }
}

#[test]
#[ignore = "daily"]
fn a_start_that_passes_its_checks_draws_once_and_runs() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    with_game(|game| {
        zombie(game, 8, [3.5, 65.0, 3.5]);
        let mut probe = wrapped(3, 3, Box::new(Probe::still_for(10)));
        with_mob_world(game, |mob, w, level| {
            let before = random_of(level);
            assert!(probe.try_start(mob, w, level.random(), 0));
            assert_ne!(random_of(level), before, "the duration is one draw");
        });
        assert!(probe.running());
        assert_eq!(*CALLS.lock().unwrap(), ["probe/check", "probe/start"]);
    });
}

#[test]
#[ignore = "daily"]
fn a_failed_entry_condition_draws_nothing_and_calls_nothing() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    with_game(|game| {
        zombie(game, 8, [3.5, 65.0, 3.5]);
        // No species' brain keeps a wired memory yet, so `Present` fails.
        let mut probe = mobs::mod_behaviors::ModBehavior::new(RegisteredBehavior {
            mod_id: "test",
            part: "ZombieBehaviors",
            activity: Activity::Idle,
            priority: 5,
            min: 3,
            max: 3,
            entry: &[(BrainMemory::WalkTarget, MemoryStatus::Present)],
            behavior: Box::new(Probe::still_for(10)),
        });
        with_mob_world(game, |mob, w, level| {
            let before = random_of(level);
            assert!(!probe.try_start(mob, w, level.random(), 0));
            assert_eq!(random_of(level), before, "a failed start draws nothing");
        });
        assert!(!probe.running());
        assert!(
            CALLS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_duration_ends_it_and_can_still_use_keeps_it() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    with_game(|game| {
        zombie(game, 8, [3.5, 65.0, 3.5]);
        // A fixed window of 3: `end = 0 + 3`, stopped at the first tick
        // past it.
        let mut probe = wrapped(3, 3, Box::new(Probe::still_for(10)));
        with_mob_world(game, |mob, w, level| {
            assert!(probe.try_start(mob, w, level.random(), 0));
            probe.tick_or_stop(mob, w, 1);
            probe.tick_or_stop(mob, w, 2);
            probe.tick_or_stop(mob, w, 3);
            assert!(probe.running(), "timestamp > end is the stop");
            probe.tick_or_stop(mob, w, 4);
        });
        assert!(!probe.running());
        assert_eq!(
            *CALLS.lock().unwrap(),
            [
                "probe/check",
                "probe/start",
                "probe/tick",
                "probe/tick",
                "probe/tick",
                "probe/stop"
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn without_can_still_use_it_lives_one_brain_tick() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    with_game(|game| {
        zombie(game, 8, [3.5, 65.0, 3.5]);
        // `still_for(0)`: `can_still_use` answers false the first time, as
        // a behavior that does not override it does.
        let mut probe = wrapped(60, 60, Box::new(Probe::still_for(0)));
        with_mob_world(game, |mob, w, level| {
            assert!(probe.try_start(mob, w, level.random(), 0));
            probe.tick_or_stop(mob, w, 0);
        });
        assert!(!probe.running());
        assert_eq!(
            *CALLS.lock().unwrap(),
            ["probe/check", "probe/start", "probe/stop"]
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_panic_is_contained_and_the_behavior_is_done() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    with_game(|game| {
        zombie(game, 8, [3.5, 65.0, 3.5]);
        let mut panicking = Probe::still_for(10);
        panicking.panic_in_start = true;
        let mut probe = wrapped(3, 3, Box::new(panicking));
        with_mob_world(game, |mob, w, level| {
            // The start itself panics; the runner contains it, so the call
            // returns (as a failed start) instead of unwinding.
            assert!(!probe.try_start(mob, w, level.random(), 0));
            assert!(!probe.running());
            // The behavior is gone: no further call reaches it.
            assert!(!probe.try_start(mob, w, level.random(), 1));
        });
        assert_eq!(*CALLS.lock().unwrap(), ["probe/check", "probe/start"]);
    });
}

#[test]
#[ignore = "daily"]
fn a_type_with_no_brain_is_refused_at_construction() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    REFUSALS.lock().unwrap().clear();
    install_game_mods();
    with_game(|game| {
        zombie(game, 8, [3.5, 65.0, 3.5]);
        assert_eq!(
            *REFUSALS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            [mod_api::BehaviorRefusal::NoBrain],
            "the part learned at construction that a zombie has no brain"
        );
    });
}

// ---- The villager brain (arismoko/horizon#824's first wiring) ---------------------------

/// Values the villager behaviors read back, for the journal.
static READS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Writes a walk target through the engine's setter and journals what the
/// context then answers: the walk target read back, the absent look target,
/// and the attack-target write a villager refuses (its brain keeps none).
struct WalkWriter;
impl Behavior for WalkWriter {
    fn name(&self) -> &'static str {
        "WalkWriter"
    }
    fn start(&mut self, cx: &mut BehaviorCx<'_>) {
        record("writer/start");
        cx.set_walk_target([7, 64, 9], 0.5, 1);
        let read = |what: String| READS.lock().unwrap().push(what);
        read(format!("{:?}", cx.walk_target()));
        read(format!("{:?}", cx.look_target()));
        let zombie = cx.host().entity(9).map(mod_api::LivingRef::new);
        read(format!("{}", cx.set_brain_target(zombie)));
    }
}

/// `with_game` over a phase that answers its random states.
fn with_game_states(
    phase: fn(&mut super::super::Game<'_>) -> ((u64, Option<f64>), (u64, Option<f64>)),
) -> ((u64, Option<f64>), (u64, Option<f64>)) {
    let mut states = None;
    with_game(|game| states = Some(phase(game)));
    states.unwrap()
}

/// A villager in the game's list, as `Mob::new` builds it.
fn villager(game: &mut super::super::Game<'_>, id: u32, at: [f64; 3]) {
    use horizon_core::geometry::Vec3;
    let kind = game
        .res
        .mob_types()
        .get("minecraft:villager")
        .unwrap()
        .clone();
    let mut m = Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
    m.on_ground = true;
    game.mobs.push(m);
}

/// One pass over the villager's own brain tick as `server_ai_step` reaches
/// it: the world with the goal host beside it, the events buffer and the
/// level random, all handed to `f` together. `level` is the caller's, so a
/// run of ticks draws one stream.
fn with_villager_tick(
    game: &mut super::super::Game<'_>,
    id: u32,
    game_time: i64,
    day_time: i64,
    level: &mut DamageLevel<'_>,
    f: impl FnOnce(&mut Mob, &MobWorld<'_>, &mut DamageLevel<'_>, &mut Vec<mobs::MobEvent>),
) {
    game.options.dedicated = Some(Vec::new());
    game.world.game_time = game_time;
    game.world.day_time = day_time;
    let mut mobs = game.mobs.lift();
    let snapshot: Vec<_> = mobs
        .iter()
        .map(|m| creatures::seen(m, game.level.player_uuid, game.items()))
        .collect();
    let mut w = game.mob_world_base(&snapshot);
    w.goal_host = Some(&*game);
    let mut events = Vec::new();
    let index = mobs.iter().position(|m| m.id == id).unwrap();
    f(&mut mobs[index], &w, level, &mut events);
    game.mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
}

/// `villager_brain::tick` once at this time, on the caller's level random.
fn villager_tick(
    game: &mut super::super::Game<'_>,
    id: u32,
    game_time: i64,
    day_time: i64,
    level: &mut DamageLevel<'_>,
) {
    with_villager_tick(
        game,
        id,
        game_time,
        day_time,
        level,
        |m, w, level, events| {
            mobs::villager_brain::tick(m, w, level, events);
        },
    );
}

/// The mob's own random and the level random after a run of ticks: what the
/// zero-cost test compares.
fn random_states(
    game: &mut super::super::Game<'_>,
    id: u32,
    level: &mut DamageLevel<'_>,
) -> ((u64, Option<f64>), (u64, Option<f64>)) {
    let mobs = game.mobs.lift();
    let mob = mobs.iter().find(|m| m.id == id).unwrap();
    let mob_state = (&*mob.random).into();
    game.mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
    (mob_state, (&**level.random()).into())
}

#[test]
#[ignore = "daily"]
fn behaviors_join_after_vanilla_of_their_priority_and_activity() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Core, 0, 60, 60, &[], Probe::still_for(1))
            .unwrap();
    }));
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Idle, 5, 40, 80, &[], Probe::still_for(1))
            .unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        villager(game, 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        villager_tick(game, 8, 100, 3000, &mut level);
        let keys = mobs::villager_brain::entry_keys(game.mobs.iter().find(|m| m.id == 8).unwrap())
            .unwrap();
        let mods: Vec<_> = keys.iter().enumerate().filter(|(_, k)| k.2).collect();
        assert_eq!(mods.len(), 2, "both behaviors joined");
        for (i, key) in &mods {
            let before = keys
                .iter()
                .take(*i)
                .filter(|k| !k.2 && k.0 == key.0 && k.1 == key.1)
                .count();
            assert!(before > 0, "vanilla ({:?}, {:?}) comes first", key.0, key.1);
            let after_vanilla = keys
                .iter()
                .skip(i + 1)
                .any(|k| !k.2 && k.0 == key.0 && k.1 == key.1);
            assert!(!after_vanilla, "no vanilla of the pair follows the mod's");
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_behavior_writes_the_walk_target_and_the_refusals_hold() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    READS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Idle, 99, 60, 60, &[], WalkWriter).unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        villager(game, 8, [3.5, 65.0, 3.5]);
        zombie(game, 9, [5.5, 65.0, 5.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        // Idle at day 3000 (no job site): the behavior starts.
        villager_tick(game, 8, 100, 3000, &mut level);
        let reads = READS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(
            reads[0], "Some(BrainWalkTarget { at: [7.5, 64.5, 9.5], speed: 0.5, close: 1 })",
            "the walk target reads back at the block's centre"
        );
        assert_eq!(reads[1], "None", "no look target is set");
        assert_eq!(
            reads[2], "false",
            "the villager brain keeps no attack target"
        );
    });
}

#[test]
#[ignore = "daily"]
fn only_an_active_activity_starts_but_core_always_does() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    // Hide runs only in a raid; Idle is the day's fallback activity (no job
    // site), and Core is always active.
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Hide, 50, 60, 60, &[], Probe::named("hide", 1))
            .unwrap();
    }));
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Idle, 50, 60, 60, &[], Probe::named("idle", 1))
            .unwrap();
    }));
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Core, 50, 60, 60, &[], Probe::named("core", 1))
            .unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        villager(game, 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        for i in 0..10 {
            villager_tick(game, 8, 100 + i, 3000, &mut level);
        }
        let calls = CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(
            calls.iter().all(|c| !c.starts_with("hide/")),
            "a hidden activity never starts: {calls:?}"
        );
        assert!(
            calls.iter().any(|c| c.starts_with("idle/")),
            "the active activity starts: {calls:?}"
        );
        assert!(
            calls.iter().any(|c| c.starts_with("core/")),
            "core always starts: {calls:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_running_behavior_keeps_ticking_across_the_activity_change() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Idle, 99, 1000, 1000, &[], Probe::named("one", 60))
            .unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        villager(game, 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        // Day: Idle starts the behavior. Night (day 13000): Rest takes the
        // activity, and the running behavior keeps ticking anyway, as
        // vanilla's `tickEachRunningBehavior` ignores activities.
        for i in 0..5 {
            villager_tick(game, 8, 100 + i, 3000, &mut level);
        }
        let starts = CALLS
            .lock()
            .unwrap()
            .iter()
            .filter(|c| *c == "one/start")
            .count();
        assert_eq!(starts, 1, "it started once");
        for i in 0..5 {
            villager_tick(game, 8, 110 + i, 13000, &mut level);
        }
        let calls = CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let ticks_after = calls.iter().filter(|c| *c == "one/tick").count();
        assert!(ticks_after >= 5, "still ticking at night: {calls:?}");
        assert_eq!(
            calls.iter().filter(|c| *c == "one/start").count(),
            1,
            "no restart: {calls:?}"
        );
    });
}

#[test]
#[ignore = "daily"]
fn refresh_stops_a_running_behavior_and_the_rebuild_is_fresh() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Idle, 99, 1000, 1000, &[], Probe::named("one", 60))
            .unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        villager(game, 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        villager_tick(game, 8, 100, 3000, &mut level);
        assert_eq!(
            CALLS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            ["one/check", "one/start", "one/tick"],
            "it runs, ticked in the same brain tick that started it"
        );
        // `Villager.refreshBrain`: stopAll, then the brain rebuilt from its
        // memories — the behaviors re-registered fresh.
        with_villager_tick(game, 8, 101, 3000, &mut level, |m, w, level, events| {
            mobs::villager_brain::age_boundary_reached(m, w, level, events);
        });
        assert_eq!(
            CALLS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            ["one/check", "one/start", "one/tick", "one/stop"],
            "refreshBrain stopped it"
        );
        villager_tick(game, 8, 102, 3000, &mut level);
        assert_eq!(
            CALLS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            [
                "one/check",
                "one/start",
                "one/tick",
                "one/stop",
                "one/check",
                "one/start",
                "one/tick"
            ],
            "the rebuilt brain runs a fresh one"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_villager_without_mod_behaviors_draws_what_a_parts_free_one_does() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // The 50-tick run of one phase: the same villager spot, the same game
    // times, one level random of the test's own.
    fn fifty_ticks(game: &mut super::super::Game<'_>) -> ((u64, Option<f64>), (u64, Option<f64>)) {
        villager(game, 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        for i in 0..50 {
            villager_tick(game, 8, 100 + i, 3000, &mut level);
        }
        let states = random_states(game, 8, &mut level);
        game.mobs.clear();
        states
    }
    // Phase A: no parts at all (each game resolves whatever is installed as
    // it opens, so the phases install between games, as the goal tests do).
    mod_platform::__private::install(Vec::new().into());
    let bare = with_game_states(fifty_ticks);
    // Phase B: the mods installed, a part on the villager that adds no
    // behavior (and one on a zombie besides).
    ADDS.lock().unwrap().clear();
    install_game_mods();
    let unadded = with_game_states(fifty_ticks);
    assert_eq!(bare, unadded, "a part with no behaviors costs nothing");
    // Phase C: one behavior joined — the streams part ways, so the
    // comparison above can see a change.
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(Activity::Core, 0, 60, 60, &[], Probe::still_for(60))
            .unwrap();
    }));
    let joined = with_game_states(fifty_ticks);
    assert_ne!(bare, joined, "a joined behavior shifts the level stream");
}

// ---- The animal brain (the second wiring) --------------------------------

/// A mob of `kind` in the game's list, as `Mob::new` builds it.
fn mob_of(game: &mut super::super::Game<'_>, kind: &str, id: u32, at: [f64; 3]) {
    use horizon_core::geometry::Vec3;
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut m = Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
    m.on_ground = true;
    game.mobs.push(m);
}

/// `animal_brain::tick` once at this time, on the caller's level random.
fn animal_tick(
    game: &mut super::super::Game<'_>,
    id: u32,
    game_time: i64,
    day_time: i64,
    level: &mut DamageLevel<'_>,
) {
    with_villager_tick(
        game,
        id,
        game_time,
        day_time,
        level,
        |m, w, level, events| {
            mobs::animal_brain::tick(m, w, level, events);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_sniffer_behavior_sets_a_walk_target() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    READS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:sniffer", |b| {
        b.add(Activity::Idle, 99, 60, 60, &[], WalkWriter).unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        mob_of(game, "minecraft:sniffer", 8, [3.5, 65.0, 3.5]);
        zombie(game, 9, [5.5, 65.0, 5.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        animal_tick(game, 8, 100, 3000, &mut level);
        let reads = READS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(
            reads[0], "Some(BrainWalkTarget { at: [7.5, 64.5, 9.5], speed: 0.5, close: 1 })",
            "the sniffer's walk target reads back at the block's centre"
        );
        assert_eq!(
            reads[2], "false",
            "the sniffer brain keeps no attack target, so the write refuses"
        );
    });
}

#[test]
#[ignore = "daily"]
fn animal_behaviors_join_after_vanilla_of_their_priority() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:sniffer", |b| {
        b.add(Activity::Core, 0, 60, 60, &[], Probe::still_for(1))
            .unwrap();
    }));
    ADDS.lock().unwrap().push(plan("minecraft:sniffer", |b| {
        b.add(Activity::Idle, 3, 60, 60, &[], Probe::still_for(1))
            .unwrap();
    }));
    // The vanilla order of a parts-free sniffer, to prove the sort keeps it
    // once the behaviors join (the mods install between the games, as the
    // goal tests install).
    let bare = {
        let mut bare = None;
        with_game(|game| {
            mob_of(game, "minecraft:sniffer", 8, [3.5, 65.0, 3.5]);
            let mut test_level = horizon_client::health::TestLevel::new(0);
            let mut level = test_level.level();
            animal_tick(game, 8, 100, 3000, &mut level);
            bare = Some(
                mobs::animal_brain::entry_keys(game.mobs.iter().find(|m| m.id == 8).unwrap())
                    .unwrap(),
            );
        });
        bare.unwrap()
    };
    install_game_mods();
    with_game(|game| {
        mob_of(game, "minecraft:sniffer", 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        animal_tick(game, 8, 100, 3000, &mut level);
        let joined =
            mobs::animal_brain::entry_keys(game.mobs.iter().find(|m| m.id == 8).unwrap()).unwrap();
        let vanilla: Vec<_> = joined.iter().filter(|k| !k.2).cloned().collect();
        assert_eq!(vanilla, bare, "vanilla's order stands");
        let core_mod = joined
            .iter()
            .position(|k| k.2 && k.0 == 0 && k.1 == mobs::animal_brain::Act::Core)
            .unwrap();
        let idle_vanilla = joined
            .iter()
            .position(|k| !k.2 && k.0 == 0 && k.1 == mobs::animal_brain::Act::Idle)
            .unwrap();
        assert!(core_mod < idle_vanilla, "Core's whole bucket precedes Idle");
        for (i, key) in joined.iter().enumerate().filter(|(_, k)| k.2) {
            let before = joined
                .iter()
                .take(i)
                .filter(|k| !k.2 && k.0 == key.0 && k.1 == key.1)
                .count();
            assert!(before > 0, "vanilla ({key:?}) comes first");
            let after_vanilla = joined
                .iter()
                .skip(i + 1)
                .any(|k| !k.2 && k.0 == key.0 && k.1 == key.1);
            assert!(!after_vanilla, "no vanilla of the pair follows the mod's");
        }
    });
}

#[test]
#[ignore = "daily"]
fn grown_up_stops_a_running_ghastling_behavior() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock()
        .unwrap()
        .push(plan("minecraft:happy_ghast", |b| {
            b.add(Activity::Idle, 99, 1000, 1000, &[], Probe::named("one", 60))
                .unwrap();
        }));
    install_game_mods();
    with_game(|game| {
        mob_of(game, "minecraft:happy_ghast", 8, [3.5, 70.0, 3.5]);
        {
            let mut mobs = game.mobs.lift();
            mobs::finalize::set_baby(&mut mobs[0], true);
            game.mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
        }
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        animal_tick(game, 8, 100, 3000, &mut level);
        assert_eq!(
            CALLS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone(),
            ["one/check", "one/start", "one/tick"],
            "the ghastling's behavior runs"
        );
        // `HappyGhast.adultGhastSetup`: `Brain.stopAll` reaches the running
        // behavior's stop, then nothing more reaches it.
        with_villager_tick(game, 8, 101, 3000, &mut level, |m, w, _level, _events| {
            mobs::animal_brain::grown_up(m, w);
        });
        let calls = CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(
            calls,
            ["one/check", "one/start", "one/tick", "one/stop"],
            "the stop callback ran once"
        );
        for i in 0..5 {
            animal_tick(game, 8, 102 + i, 3000, &mut level);
        }
        let calls = CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(calls.len(), 4, "not one call after the stop: {calls:?}");
    });
}

// ---- The monster and warden brains (the last wiring) ---------------------

/// `monster_brain::tick` once at this time, on the caller's level random.
fn monster_tick(
    game: &mut super::super::Game<'_>,
    id: u32,
    game_time: i64,
    day_time: i64,
    level: &mut DamageLevel<'_>,
) {
    with_villager_tick(
        game,
        id,
        game_time,
        day_time,
        level,
        |m, w, level, events| {
            mobs::monster_brain::tick(m, w, level, events);
        },
    );
}

/// `warden_brain::tick` once at this time, on the caller's level random.
fn warden_tick(
    game: &mut super::super::Game<'_>,
    id: u32,
    game_time: i64,
    day_time: i64,
    level: &mut DamageLevel<'_>,
) {
    with_villager_tick(
        game,
        id,
        game_time,
        day_time,
        level,
        |m, w, level, events| {
            mobs::warden_brain::tick(m, w, level, events);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_breeze_behavior_starts_in_fight_and_writes_its_target() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    READS.lock().unwrap().clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:breeze", |b| {
        b.add(Activity::Fight, 99, 60, 60, &[], WalkWriter).unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        mob_of(game, "minecraft:breeze", 8, [3.5, 65.0, 3.5]);
        zombie(game, 9, [5.5, 65.0, 5.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        // A breeze's default activity is FIGHT.
        monster_tick(game, 8, 100, 3000, &mut level);
        let reads = READS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(
            reads[0], "Some(BrainWalkTarget { at: [7.5, 64.5, 9.5], speed: 0.5, close: 1 })",
            "the breeze's walk target reads back"
        );
        assert_eq!(
            reads[2], "true",
            "the breeze brain keeps an attack target, so the write goes through"
        );
    });
}

#[test]
#[ignore = "daily"]
fn warden_behaviors_join_after_vanilla_of_their_priority() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:warden", |b| {
        b.add(Activity::Idle, 10, 60, 60, &[], Probe::still_for(1))
            .unwrap();
    }));
    // The vanilla order of a parts-free warden.
    let bare = {
        let mut bare = None;
        with_game(|game| {
            mob_of(game, "minecraft:warden", 8, [3.5, 65.0, 3.5]);
            {
                let mut mobs = game.mobs.lift();
                mobs::warden::register(&mut mobs[0]);
                game.mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
            }
            let mut test_level = horizon_client::health::TestLevel::new(0);
            let mut level = test_level.level();
            warden_tick(game, 8, 100, 3000, &mut level);
            bare = Some(
                mobs::warden_brain::entry_keys(game.mobs.iter().find(|m| m.id == 8).unwrap())
                    .unwrap(),
            );
        });
        bare.unwrap()
    };
    // The joined one (the mods install between the games).
    install_game_mods();
    with_game(|game| {
        mob_of(game, "minecraft:warden", 8, [3.5, 65.0, 3.5]);
        {
            let mut mobs = game.mobs.lift();
            mobs::warden::register(&mut mobs[0]);
            game.mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
        }
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        warden_tick(game, 8, 100, 3000, &mut level);
        let joined =
            mobs::warden_brain::entry_keys(game.mobs.iter().find(|m| m.id == 8).unwrap()).unwrap();
        let vanilla: Vec<_> = joined.iter().filter(|k| !k.2).cloned().collect();
        assert_eq!(vanilla, bare, "vanilla's order stands");
        let (i, key) = joined
            .iter()
            .enumerate()
            .find(|(_, k)| k.2)
            .expect("the behavior joined");
        let before = joined
            .iter()
            .take(i)
            .filter(|k| !k.2 && k.0 == key.0 && k.1 == key.1)
            .count();
        assert!(before > 0, "vanilla ({key:?}) comes first");
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_behavior_leaves_the_rest_of_the_tick_working() {
    let _test = TEST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CALLS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    ADDS.lock().unwrap().clear();
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        let mut panicking = Probe::named("boom", 60);
        panicking.panic_in_start = true;
        b.add(Activity::Core, 50, 1000, 1000, &[], panicking)
            .unwrap();
    }));
    ADDS.lock().unwrap().push(plan("minecraft:villager", |b| {
        b.add(
            Activity::Core,
            51,
            1000,
            1000,
            &[],
            Probe::named("calm", 60),
        )
        .unwrap();
    }));
    install_game_mods();
    with_game(|game| {
        villager(game, 8, [3.5, 65.0, 3.5]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        // One behavior panics in its start; the other starts and ticks in
        // the same brain tick, and the mob's own goal selector answers
        // after it (no journal of the engine's stayed poisoned).
        villager_tick(game, 8, 100, 3000, &mut level);
        let calls = CALLS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert!(
            calls.contains(&"boom/check".to_owned()) && calls.contains(&"boom/start".to_owned()),
            "the panicking one was asked: {calls:?}"
        );
        assert!(
            calls.contains(&"calm/check".to_owned())
                && calls.contains(&"calm/start".to_owned())
                && calls.contains(&"calm/tick".to_owned()),
            "the other one runs the same tick: {calls:?}"
        );
        // The villager's own goal selector still ticks (the goal tests'
        // Counter part is not installed here; the selector itself answers).
        with_villager_tick(game, 8, 101, 3000, &mut level, |m, w, level, events| {
            let mut cx = mobs::goals::Cx { w, level, events };
            let mut selector = std::mem::take(&mut m.ai.goals);
            selector.tick(m, &mut cx);
            m.ai.goals = selector;
        });
    });
}
