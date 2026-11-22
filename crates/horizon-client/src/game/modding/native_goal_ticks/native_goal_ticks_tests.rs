//! Tests for `native_goal_ticks` (arismoko/horizon#1077): registration order,
//! priorities and interruption, per-mob state and random streams, the read
//! view, `set_target`, and the no-mod-goals fast path.
//!
//! The `zombie` fixture is shared with the behavior tests (arismoko/horizon#824).
use super::super::creatures;
use super::super::native_entity_refs::tests::with_game;
use horizon_client::mobs::{self, Mob, goals::Cx};
use horizon_core::{geometry::Vec3, random::RandomSource};
use mod_api::{
    EntityPart, EntityRef, Goal, GoalContext, GoalFlags, GoalRegistrar, Hooks, LivingRef, ModEntry,
    Registrar, RegistrationContext,
};
use std::sync::Mutex;

/// The mods of these tests, installed with their parts, as a game's open does:
/// `with_mods` for the mods, `install_parts` for what they register,
/// `resolve_parts` for the game's own table.
fn install_game_mods() {
    mod_platform::__private::install(vec![ModEntry::new("test", Box::new(Mod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("test", Box::new(Mod))]).unwrap();
}

/// What one test goal did, by mob id.
static EVENTS: Mutex<Vec<(u32, &'static str)>> = Mutex::new(Vec::new());
/// The other mob a tick reads through the view, as a handle.
static OTHER: Mutex<Option<EntityRef>> = Mutex::new(None);
/// What the last tick read of `OTHER`: its health, position and a refused
/// read (a potion check, which the snapshot does not carry).
static READS: Mutex<Vec<(Option<f32>, Option<[f64; 3]>, bool)>> = Mutex::new(Vec::new());

fn record(id: u32, call: &'static str) {
    EVENTS.lock().unwrap().push((id, call));
}

fn set_other(handle: EntityRef) {
    *OTHER.lock().unwrap() = Some(handle);
}

/// The mod goal of every test: counts its calls in its own state and in the
/// shared journal, draws from the mob's own random at start, and reads the
/// other mob through the tick's view on tick.
#[derive(Default)]
struct Counter {
    ticks: u32,
    draw: i32,
}
impl Goal for Counter {
    fn flags(&self) -> GoalFlags {
        GoalFlags::MOVE
    }
    fn name(&self) -> &'static str {
        "Counter"
    }
    fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
        record(cx.mob().entity().id(), "can_use");
        true
    }
    fn start(&mut self, cx: &mut GoalContext<'_>) {
        record(cx.mob().entity().id(), "start");
        self.draw = cx.random().next_int_bound(1000).unwrap_or(0);
    }
    fn tick(&mut self, cx: &mut GoalContext<'_>) {
        self.ticks += 1;
        record(cx.mob().entity().id(), "tick");
        if let Some(other) = OTHER.lock().unwrap().as_ref().copied() {
            let health = LivingRef::new(other).health(cx.host());
            let position = other.position(cx.host());
            // A read the view refuses: the snapshot carries no rotation of
            // another mob (only the ticking mob's own answers).
            let refused = other.rotation(cx.host()).is_some();
            READS.lock().unwrap().push((health, position, refused));
        }
    }
    fn stop(&mut self, cx: &mut GoalContext<'_>) {
        record(cx.mob().entity().id(), "stop");
    }
}

struct ZombieGoals;
#[mod_api::horizon_part]
impl EntityPart for ZombieGoals {
    fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
        assert_eq!(goals.mob().id(), "minecraft:zombie");
        goals.add_goal(7, Counter::default());
    }
}

/// How often the husk's part was asked for its goals (`a_screens_entity_registers_no_mod_goals`).
static ASKED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// A part on the husk that only counts its `register_goals` calls.
struct Asked;
#[mod_api::horizon_part]
impl EntityPart for Asked {
    fn register_goals(&self, _: &mut GoalRegistrar<'_>) {
        ASKED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        r.entity_part("minecraft:zombie", ZombieGoals);
        r.entity_part("minecraft:husk", Asked);
    }
}

/// A vanilla-standin goal for the interruption test: a MOVE goal whose
/// `can_use` says yes.
#[derive(Debug, Clone)]
struct Vanilla;
impl mobs::goals::Goal for Vanilla {
    fn flags(&self) -> u8 {
        mobs::goals::MOVE
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut Cx) -> bool {
        true
    }
    fn box_clone(&self) -> Box<dyn mobs::goals::Goal> {
        Box::new(self.clone())
    }
    fn name(&self) -> &'static str {
        "Vanilla"
    }
}

/// A zombie in the game's list, as `Mob::new` builds it.
pub(in crate::game) fn zombie(game: &mut super::super::Game<'_>, id: u32, at: [f64; 3]) {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    let mut m = Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
    m.on_ground = true;
    game.mobs.push(m);
}

/// The zombie's goal selector ticks once, as `server_ai_step` ticks it: the
/// world out of the game, the goal host beside it.
fn tick_goals(game: &mut super::super::Game<'_>, index: usize) {
    tick_goals_as(game, index, None, None);
}

/// The same tick with a player view lent, as a server's mob tick lends a
/// guest's, and the identity captured beside it — what a goal resolves
/// (arismoko/horizon#1077): the real guest's uuid and entity id, never a scan per read. A
/// lent view with no identity is a fake player's: nothing resolves.
fn tick_goals_as(
    game: &mut super::super::Game<'_>,
    index: usize,
    player: Option<horizon_client::mobs::PlayerView>,
    identity: Option<([i32; 4], u32)>,
) {
    // The native-authoritative side, whose mob tick builds the goal host.
    game.options.dedicated = Some(Vec::new());
    let mut mobs = game.mobs.lift();
    let snapshot: Vec<_> = mobs
        .iter()
        .map(|m| creatures::seen(m, game.level.player_uuid, game.items()))
        .collect();
    let mut index_by_id = horizon_client::fast_hash::FxHashMap::<u32, u32>::default();
    for (i, s) in snapshot.iter().enumerate() {
        index_by_id.entry(s.id).or_insert(i as u32);
    }
    let mut test_level = horizon_client::health::TestLevel::new(0);
    let mut level = test_level.level();
    let mut events = Vec::new();
    {
        let mut w = game.mob_world_base(&snapshot);
        w.others_index = Some(&index_by_id);
        w.player = player;
        w.player_identity = identity;
        let mut selector = std::mem::take(&mut mobs[index].ai.goals);
        let mut cx = Cx {
            w: &w,
            level: &mut level,
            events: &mut events,
        };
        selector.tick(&mut mobs[index], &mut cx);
        mobs[index].ai.goals = selector;
    }
    game.mobs
        .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
}

/// The events of one mob's goal, in order.
fn calls(id: u32) -> Vec<&'static str> {
    EVENTS
        .lock()
        .unwrap()
        .iter()
        .filter(|(mob, _)| *mob == id)
        .map(|(_, call)| *call)
        .collect()
}

#[test]
#[ignore = "daily"]
fn goals_register_after_the_species_own_and_only_on_their_type() {
    EVENTS.lock().unwrap().clear();
    let mut bare = Vec::new();
    with_game(|game| {
        // A zombie without the parts: the species list alone.
        bare = {
            zombie(game, 90, [3.5, 65.0, 3.5]);
            let names: Vec<_> = game.mobs.last().unwrap().ai.goals.entries().collect();
            game.mobs.clear();
            names
        };
    });
    install_game_mods();
    with_game(|game| {
        zombie(game, 91, [3.5, 65.0, 3.5]);
        let entries: Vec<_> = game.mobs.last().unwrap().ai.goals.entries().collect();
        assert_eq!(entries[0], bare[0], "the species goals lead");
        assert_eq!(
            entries.last().unwrap(),
            &(7, "Counter", false),
            "the mod goal joins last, at its priority"
        );
        assert_eq!(entries.len(), bare.len() + 1, "exactly one goal was added");
        // A cow of the same game: nothing of it.
        let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
        let cow = Mob::new(92, kind, Vec3::new(3.5, 65.0, 5.5), 0.0);
        assert!(
            !cow.ai.goals.entries().any(|(_, name, _)| name == "Counter"),
            "a type with no part pays nothing"
        );
    });
}

#[test]
#[ignore = "daily"]
fn two_mobs_keep_independent_state_and_random_streams() {
    EVENTS.lock().unwrap().clear();
    install_game_mods();
    with_game(|game| {
        zombie(game, 10, [3.5, 65.0, 3.5]);
        zombie(game, 11, [23.5, 65.0, 23.5]);
        // Only the mod goal, so the species' own draws stay out of the count.
        game.mobs[0].ai.goals.clear();
        game.mobs[1].ai.goals.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        mobs::part_goals::attach(&mut game.mobs[1]);
        // Construction already consumed the living entity's yaw draw.
        // Snapshot at the goal boundary, not before the entity constructor.
        let mut after_first = game.mobs[0].random.clone();
        let mut after_second = game.mobs[1].random.clone();
        tick_goals(game, 0);
        tick_goals(game, 1);
        assert_eq!(calls(10), ["can_use", "start", "tick"]);
        assert_eq!(calls(11), ["can_use", "start", "tick"]);
        // Each start drew once from its own mob's generator: the next draws
        // match the pre-tick copies advanced by one, and the two drew differently.
        let (first_draw, second_draw) = (
            after_first.next_int_bound(1000).unwrap(),
            after_second.next_int_bound(1000).unwrap(),
        );
        assert_ne!(first_draw, second_draw);
        assert_eq!(
            game.mobs[0].random.next_int_bound(1000).unwrap(),
            after_first.next_int_bound(1000).unwrap()
        );
        assert_eq!(
            game.mobs[1].random.next_int_bound(1000).unwrap(),
            after_second.next_int_bound(1000).unwrap()
        );
    });
}

#[test]
#[ignore = "daily"]
fn reads_answer_the_tick_view_and_refuse_what_it_lacks() {
    EVENTS.lock().unwrap().clear();
    READS.lock().unwrap().clear();
    install_game_mods();
    with_game(|game| {
        zombie(game, 20, [3.5, 65.0, 3.5]);
        zombie(game, 21, [13.5, 65.0, 13.5]);
        game.mobs[1].health = 12.5;
        let other = game.entity(21).unwrap();
        set_other(other);
        tick_goals(game, 0);
        let reads = READS.lock().unwrap();
        let (health, position, refused) = reads.last().copied().unwrap();
        assert_eq!(health, Some(12.5), "the other mob's snapshot health");
        assert_eq!(
            position,
            Some([13.5, 65.0, 13.5]),
            "the other mob's snapshot position"
        );
        assert!(
            !refused,
            "a read the view refuses: not carried by the snapshot"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_higher_priority_goal_interrupts_a_running_mod_goal() {
    EVENTS.lock().unwrap().clear();
    install_game_mods();
    with_game(|game| {
        zombie(game, 30, [3.5, 65.0, 3.5]);
        // Just the two competing goals: the mod goal (registered at 7 by the
        // part) and a vanilla standin above it at 3.
        game.mobs[0].ai.goals.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        assert!(
            game.mobs[0]
                .ai
                .goals
                .entries()
                .any(|(_, name, _)| name == "Counter"),
            "the re-attach registered the mod goal"
        );
        game.mobs[0].ai.goals.add(3, Vanilla);
        tick_goals(game, 0);
        // The mod goal started (can_use, start, its reduced-cadence tick),
        // then the higher-priority standin replaced it: stop ran, and the
        // standin holds MOVE.
        // The standin replaced the mod goal within the same tick, so the
        // mod goal started and stopped without a tick of its own (across
        // ticks it ticks first; the live check shows that shape).
        assert_eq!(
            calls(30),
            ["can_use", "start", "stop"],
            "the mod goal ran and was interrupted"
        );
        assert!(
            game.mobs[0].ai.goals.is_running("Vanilla"),
            "the higher-priority goal holds MOVE"
        );
        assert!(
            !game.mobs[0].ai.goals.is_running("Counter"),
            "the interrupted goal stopped"
        );
    });
}

#[test]
#[ignore = "daily"]
fn set_target_follows_the_write_rules() {
    use horizon_client::mobs::Who;
    struct SetTarget;
    impl Goal for SetTarget {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "SetTarget"
        }
        fn can_use(&mut self, _cx: &mut GoalContext<'_>) -> bool {
            true
        }
        fn start(&mut self, cx: &mut GoalContext<'_>) {
            let other = OTHER.lock().unwrap().as_ref().copied();
            cx.set_target(other.map(LivingRef::new));
        }
    }
    struct Targets;
    #[mod_api::horizon_part]
    impl EntityPart for Targets {
        fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
            goals.add_goal(5, SetTarget);
        }
    }
    struct TargetMod;
    #[mod_api::__subscriptions]
    impl Hooks for TargetMod {
        fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
            r.entity_part("minecraft:zombie", Targets);
        }
    }
    EVENTS.lock().unwrap().clear();
    mod_platform::__private::install(vec![ModEntry::new("targets", Box::new(TargetMod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("targets", Box::new(TargetMod))])
        .unwrap();
    with_game(|game| {
        zombie(game, 40, [3.5, 65.0, 3.5]);
        zombie(game, 41, [13.5, 65.0, 13.5]);
        let other = game.entity(41).unwrap();
        set_other(other);
        // Only the writing goal, so no species goal races it.
        game.mobs[0].ai.goals.clear();
        game.mobs[0].ai.targets.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        tick_goals(game, 0);
        assert_eq!(
            game.mobs[0].ai.target,
            Some(Who::Mob(41)),
            "a snapshot mob target stores"
        );
    });
}

#[test]
#[ignore = "daily"]
fn mobs_without_mod_goals_tick_as_before() {
    EVENTS.lock().unwrap().clear();
    with_game(|game| {
        // No parts at all: no goal host is built, and the species goals
        // answer for themselves.
        zombie(game, 50, [3.5, 65.0, 3.5]);
        game.mobs[0].ai.goals.clear();
        game.mobs[0].ai.goals.add(3, Vanilla);
        tick_goals(game, 0);
        assert!(
            game.mobs[0].ai.goals.is_running("Vanilla"),
            "a plain goal ticks with no goal host in place"
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_host_answers_without_panicking() {
    /// Reads a block, a tag, the neighbour as a living and the day time
    /// through `cx.host()`, reporting what came back.
    static READS: Mutex<Vec<(&'static str, String)>> = Mutex::new(Vec::new());
    struct Reader;
    impl Goal for Reader {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "Reader"
        }
        fn can_use(&mut self, _cx: &mut GoalContext<'_>) -> bool {
            true
        }
        fn start(&mut self, cx: &mut GoalContext<'_>) {
            let host = cx.host();
            // The level's own reads forward to the game host behind the tick.
            let stone = host.block_state_id("minecraft:stone");
            let tagged = host.entity_type_in_tag("minecraft:zombie", "minecraft:undead");
            // The time of day through the level handle, the canonical route
            // since the Host rebuild (arismoko/horizon#1158): the handle's own backend read
            // forwards through the view to the real backend.
            let day_time = host
                .level("minecraft:overworld")
                .and_then(|level| level.day_time(host));
            // A loaded chunk answers `is_loaded` through the same handle.
            let loaded = host
                .level("minecraft:overworld")
                .is_some_and(|level| level.is_loaded(host, [3, 65, 3]));
            // The entity lookups answer the tick's own rules: the neighbour
            // comes back as a handle, and it reads from the snapshot.
            let other = OTHER.lock().unwrap().as_ref().copied();
            let (found, health) = match other {
                Some(other) => match host.living(other) {
                    Some(living) => (true, living.health(host)),
                    None => (false, None),
                },
                None => (false, None),
            };
            let line = format!(
                "stone={:?} tagged={} day_time={:?} loaded={} neighbour={:?} health={:?}",
                stone.is_some(),
                tagged,
                day_time,
                loaded,
                found,
                health
            );
            READS.lock().unwrap().push(("host", line.clone()));
            // The entity lookup builds a handle the same way: from an id.
            let by_id = other.and_then(|other| host.entity(other.id()));
            READS
                .lock()
                .unwrap()
                .push(("host", format!("by_id={:?}", by_id.is_some())));
        }
    }
    struct Readers;
    #[mod_api::horizon_part]
    impl EntityPart for Readers {
        fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
            goals.add_goal(4, Reader);
        }
    }
    struct ReaderMod;
    #[mod_api::__subscriptions]
    impl Hooks for ReaderMod {
        fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
            r.entity_part("minecraft:zombie", Readers);
        }
    }
    EVENTS.lock().unwrap().clear();
    mod_platform::__private::install(vec![ModEntry::new("reader", Box::new(ReaderMod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("reader", Box::new(ReaderMod))])
        .unwrap();
    with_game(|game| {
        zombie(game, 70, [3.5, 65.0, 3.5]);
        zombie(game, 71, [13.5, 65.0, 13.5]);
        game.mobs[1].health = 12.5;
        // A loaded chunk the level handle can vouch for (`is_loaded`).
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        let other = game.entity(71).unwrap();
        set_other(other);
        // Only the reader goal, so nothing else races it.
        game.mobs[0].ai.goals.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        game.options.dedicated = Some(Vec::new());
        assert!(
            game.mob_world_base(&[]).goal_host.is_some(),
            "the goal host is in place"
        );
        tick_goals(game, 0);
        let reads = READS.lock().unwrap();
        let line = reads
            .iter()
            .find(|(case, _)| *case == "host")
            .map(|(_, value)| value.clone())
            .expect("the reader ran");
        assert!(line.contains("stone=true"), "{line}");
        assert!(line.contains("tagged=true"), "{line}");
        assert!(
            line.contains("day_time=Some("),
            "the level's own time answers through the handle: {line}"
        );
        assert!(
            line.contains("loaded=true"),
            "a loaded chunk answers through the handle: {line}"
        );
        assert!(line.contains("neighbour=true"), "{line}");
        assert!(line.contains("health=Some(12.5)"), "{line}");
        assert!(
            reads
                .iter()
                .any(|(case, value)| *case == "host" && value == "by_id=true"),
            "the id lookup answers too"
        );
    });
}

/// A mob of `kind` in the game's list, as `Mob::new` builds it.
fn mob_of(game: &mut super::super::Game<'_>, id: u32, kind: &str, at: [f64; 3]) {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut m = Mob::new(id, kind, Vec3::new(at[0], at[1], at[2]), 0.0);
    m.on_ground = true;
    game.mobs.push(m);
}

/// A survival player standing at `at`, as mobs see one (`world::test_player`'s
/// shape, local to these tests for the crate-split source tree).
fn view_player(at: [f64; 3]) -> horizon_client::mobs::PlayerView {
    use horizon_client::mobs::PlayerView;
    PlayerView {
        position: Vec3::new(at[0], at[1], at[2]),
        bounds: horizon_core::geometry::Aabb::new(
            Vec3::new(at[0] - 0.3, at[1], at[2] - 0.3),
            Vec3::new(at[0] + 0.3, at[1] + 1.8, at[2] + 0.3),
        ),
        eye: [at[0], at[1] + 1.62, at[2]],
        alive: true,
        invulnerable: false,
        creative: false,
        sneaking: false,
        swimming: false,
        look: [0.0; 3],
        main_hand: None,
        off_hand: None,
        head: None,
        chest: None,
        legs: None,
        feet: None,
        armor_cover: 0.0,
        invisible: false,
        spectator: false,
        on_climbable: false,
        gold_armor: false,
        hurt_by: None,
        last_hurt: None,
        velocity: Vec3::ZERO,
        yaw: 0.0,
        sprinting: false,
        riding: false,
        hero: false,
        effects: Vec::new(),
        uuid: [1, 2, 3, 4],
        allies: Default::default(),
        remote: false,
        sleeping: false,
        sleep_timer: 0,
        passenger: false,
        riding_vehicle: None,
        vehicle: None,
    }
}

#[test]
#[ignore = "daily"]
fn a_client_copy_registers_no_mod_goals() {
    install_game_mods();
    with_game(|game| {
        // The authoritative construction registers the part's goal…
        zombie(game, 80, [3.5, 65.0, 3.5]);
        assert!(
            game.mobs[0]
                .ai
                .goals
                .entries()
                .any(|(_, name, _)| name == "Counter"),
            "the authoritative construction runs register_goals"
        );
        // …the client's copy of a spawn packet does not: its `register_goals`
        // moment stays on the authoritative side.
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let copy = Mob::new_remote(81, kind, Vec3::new(3.5, 65.0, 5.5), 0.0);
        assert!(
            !copy
                .ai
                .goals
                .entries()
                .any(|(_, name, _)| name == "Counter"),
            "a client copy runs no register_goals"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_screens_entity_registers_no_mod_goals() {
    install_game_mods();
    let asked = || ASKED.load(std::sync::atomic::Ordering::Relaxed);
    super::super::native_entity_refs::tests::with_client_game(|game| {
        let kind = game.res.mob_types().get("minecraft:husk").unwrap().clone();
        // The authoritative construction asks the part…
        let _ = Mob::new(90, kind, Vec3::new(0.5, 65.0, 0.5), 0.0);
        let before = asked();
        assert!(before >= 1, "the part is installed");
        // …a screen drawing the type, by type and from saved data, does not.
        let look = mod_api::GuiEntityLook::At([50.0, 40.0]);
        let view = mod_api::GuiEntityView::new([26.0, 8.0, 75.0, 78.0], 30.0, 0.0625, look);
        let mut data = horizon_core::nbt::Compound::new();
        data.insert("IsBaby".into(), horizon_core::nbt::Tag::Byte(1));
        for entity in [
            mod_api::GuiEntity::kind("minecraft:husk"),
            mod_api::GuiEntity::with_data("minecraft:husk", data),
        ] {
            let mut batch = horizon_client::hud::HudBatch::scratch([320, 240]);
            assert_eq!(game.native_draw_entity(&mut batch, &entity, view), Ok(()));
            assert!(!batch.vertices.is_empty());
        }
        assert_eq!(asked(), before, "a GUI draw runs no register_goals");
    });
}

#[test]
#[ignore = "daily"]
fn a_goal_jump_goes_through_the_jump_control_input() {
    struct Jumper;
    impl Goal for Jumper {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "Jumper"
        }
        fn can_use(&mut self, _cx: &mut GoalContext<'_>) -> bool {
            true
        }
        fn start(&mut self, cx: &mut GoalContext<'_>) {
            cx.jump();
        }
    }
    struct Jumpers;
    #[mod_api::horizon_part]
    impl EntityPart for Jumpers {
        fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
            goals.add_goal(6, Jumper);
        }
    }
    struct JumpMod;
    #[mod_api::__subscriptions]
    impl Hooks for JumpMod {
        fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
            r.entity_part("minecraft:zombie", Jumpers);
        }
    }
    mod_platform::__private::install(vec![ModEntry::new("jumps", Box::new(JumpMod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("jumps", Box::new(JumpMod))]).unwrap();
    with_game(|game| {
        zombie(game, 82, [3.5, 65.0, 3.5]);
        game.mobs[0].ai.goals.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        tick_goals(game, 0);
        // The goal asked the jump control, not its output: the production
        // `jump_control_tick` assigns the output from this wish.
        assert!(game.mobs[0].ai.jump, "the goal set the control's input");
        assert!(!game.mobs[0].ai.jumping, "not its output");
        game.mobs[0].jump_control_tick();
        assert!(
            game.mobs[0].ai.jumping,
            "the control jumps on the mob's next own tick"
        );
        assert!(!game.mobs[0].ai.jump, "the wish is consumed");
    });
}

#[test]
#[ignore = "daily"]
fn set_target_refuses_what_the_write_handle_refuses() {
    use horizon_client::mobs::Who;
    /// What the `SetTarget` goal's call returned.
    static ANSWERS: Mutex<Vec<bool>> = Mutex::new(Vec::new());
    static TARGET: Mutex<Option<EntityRef>> = Mutex::new(None);
    struct SetTarget;
    impl Goal for SetTarget {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "SetTarget"
        }
        // Never starts: every tick asks afresh, so each target is written once.
        fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
            let target = TARGET.lock().unwrap().as_ref().copied();
            ANSWERS
                .lock()
                .unwrap()
                .push(cx.set_target(target.map(LivingRef::new)));
            false
        }
    }
    struct Targets;
    #[mod_api::horizon_part]
    impl EntityPart for Targets {
        fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
            goals.add_goal(5, SetTarget);
        }
    }
    struct TargetMod;
    #[mod_api::__subscriptions]
    impl Hooks for TargetMod {
        fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
            r.entity_part("minecraft:iron_golem", Targets);
        }
    }
    ANSWERS.lock().unwrap().clear();
    mod_platform::__private::install(vec![ModEntry::new("golems", Box::new(TargetMod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("golems", Box::new(TargetMod))])
        .unwrap();
    with_game(|game| {
        mob_of(game, 90, "minecraft:iron_golem", [3.5, 65.0, 3.5]);
        mob_of(game, 91, "minecraft:zombie", [13.5, 65.0, 13.5]);
        mob_of(game, 92, "minecraft:creeper", [23.5, 65.0, 23.5]);
        mob_of(game, 93, "minecraft:ghast", [33.5, 70.0, 33.5]);
        // Only the writing goal, so no species goal races it.
        game.mobs[0].ai.goals.clear();
        game.mobs[0].ai.targets.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        for id in [91, 92, 93] {
            *TARGET.lock().unwrap() = Some(game.entity(id).unwrap());
            tick_goals(game, 0);
            // The write handle's valid-target rules, over the tick's facts:
            // a zombie stores, a creeper (`IronGolem.canAttack`) and a ghast
            // (`asValidTarget`) refuse.
            assert_eq!(
                ANSWERS.lock().unwrap().last().copied(),
                Some(id == 91),
                "target {id}"
            );
            if id == 91 {
                assert_eq!(game.mobs[0].ai.target, Some(Who::Mob(91)));
            } else {
                assert_ne!(
                    game.mobs[0].ai.target,
                    Some(Who::Mob(id)),
                    "a refused target does not store"
                );
            }
        }
        // The player the tick sees, through the same rule set: a fighting
        // difficulty stores her, Peaceful refuses, and a golem a player
        // built never targets a player.
        let epoch = game.entity(91).unwrap().epoch();
        let player = EntityRef::new(epoch, 5000);
        *TARGET.lock().unwrap() = Some(player);
        let view = view_player([9.5, 65.0, 9.5]);
        tick_goals_as(game, 0, Some(view.clone()), Some(([1, 2, 3, 4], 5000)));
        assert_eq!(
            ANSWERS.lock().unwrap().last().copied(),
            Some(true),
            "a survival player on a fighting difficulty stores"
        );
        assert_eq!(game.mobs[0].ai.target, Some(Who::Player));
        game.world.health_rules.difficulty = horizon_world::food::Difficulty::Peaceful;
        tick_goals_as(game, 0, Some(view.clone()), Some(([1, 2, 3, 4], 5000)));
        assert_eq!(
            ANSWERS.lock().unwrap().last().copied(),
            Some(false),
            "Peaceful refuses a player target"
        );
        game.world.health_rules.difficulty = horizon_world::food::Difficulty::Normal;
        game.mobs[0].extra.player_created = true;
        tick_goals_as(game, 0, Some(view), Some(([1, 2, 3, 4], 5000)));
        assert_eq!(
            ANSWERS.lock().unwrap().last().copied(),
            Some(false),
            "a player-built golem never targets a player"
        );
        assert_eq!(
            game.mobs[0].ai.target,
            Some(Who::Player),
            "a refused player target does not replace the stored one"
        );
    });
}

#[test]
#[ignore = "daily"]
fn move_and_look_inputs_pass_the_navigation_boundary() {
    /// The `move_to` answers, and what the look control ended up wanting.
    static RESULTS: Mutex<Vec<(bool, bool, Option<[f64; 3]>)>> = Mutex::new(Vec::new());
    struct Mover;
    impl Goal for Mover {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "Mover"
        }
        fn can_use(&mut self, _cx: &mut GoalContext<'_>) -> bool {
            true
        }
        fn start(&mut self, cx: &mut GoalContext<'_>) {
            // The navigation handle's own boundary: NaN and infinities refuse,
            // the speed must be finite.
            let nan_at = cx.move_to([f64::NAN, 65.0, 3.5], 1.0);
            let inf_speed = cx.move_to([3.5, 65.0, 3.5], f64::INFINITY);
            cx.look_at([10.5, 66.0, 0.0]);
            RESULTS.lock().unwrap().push((nan_at, inf_speed, None));
        }
        fn tick(&mut self, cx: &mut GoalContext<'_>) {
            // A look with a NaN refuses: the control keeps its last wish.
            cx.look_at([f64::NAN, 66.0, 0.0]);
        }
    }
    struct Movers;
    #[mod_api::horizon_part]
    impl EntityPart for Movers {
        fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
            goals.add_goal(6, Mover);
        }
    }
    struct MoveMod;
    #[mod_api::__subscriptions]
    impl Hooks for MoveMod {
        fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
            r.entity_part("minecraft:zombie", Movers);
        }
    }
    RESULTS.lock().unwrap().clear();
    mod_platform::__private::install(vec![ModEntry::new("moves", Box::new(MoveMod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("moves", Box::new(MoveMod))]).unwrap();
    with_game(|game| {
        zombie(game, 83, [3.5, 65.0, 3.5]);
        game.mobs[0].ai.goals.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        tick_goals(game, 0);
        let (nan_at, inf_speed, _) = RESULTS.lock().unwrap().last().copied().unwrap();
        assert!(!nan_at, "a NaN coordinate refuses");
        assert!(!inf_speed, "an infinite speed refuses");
        assert_eq!(game.mobs[0].ai.look.wanted, [10.5, 66.0, 0.0]);
        // The tick's NaN look kept it.
        tick_goals(game, 0);
        assert_eq!(
            game.mobs[0].ai.look.wanted,
            [10.5, 66.0, 0.0],
            "a NaN look refuses, the control keeps its wish"
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_player_the_tick_sees_resolves_by_its_captured_identity() {
    /// What each read of the tick's player found: by uuid, by id (and where
    /// it stands), and whether it narrows to a mob handle.
    static READS: Mutex<Vec<(bool, Option<[f64; 3]>, bool)>> = Mutex::new(Vec::new());
    struct PlayerReader;
    impl Goal for PlayerReader {
        fn flags(&self) -> GoalFlags {
            GoalFlags::MOVE
        }
        fn name(&self) -> &'static str {
            "PlayerReader"
        }
        // Never starts: every tick reads the player afresh as a candidate.
        fn can_use(&mut self, cx: &mut GoalContext<'_>) -> bool {
            let host = cx.host();
            let uuid = horizon_client::lan::playerdata::ints_uuid([1, 2, 3, 4]);
            let by_uuid = host.player(uuid).is_some();
            let entity = host.entity(5000);
            let position = entity.and_then(|entity| entity.position(host));
            let as_mob = entity.is_some_and(|entity| host.mob(entity).is_some());
            READS.lock().unwrap().push((by_uuid, position, as_mob));
            false
        }
    }
    struct Readers;
    #[mod_api::horizon_part]
    impl EntityPart for Readers {
        fn register_goals(&self, goals: &mut GoalRegistrar<'_>) {
            goals.add_goal(6, PlayerReader);
        }
    }
    struct PlayerMod;
    #[mod_api::__subscriptions]
    impl Hooks for PlayerMod {
        fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
            r.entity_part("minecraft:zombie", Readers);
        }
    }
    READS.lock().unwrap().clear();
    mod_platform::__private::install(vec![ModEntry::new("players", Box::new(PlayerMod))].into());
    crate::game::api_values::install_parts(&[ModEntry::new("players", Box::new(PlayerMod))])
        .unwrap();
    with_game(|game| {
        zombie(game, 84, [3.5, 65.0, 3.5]);
        game.mobs[0].ai.goals.clear();
        mobs::part_goals::attach(&mut game.mobs[0]);
        // A server's tick lends a guest's view, with the guest's real
        // identity captured beside it — no player of the game's own exists.
        let view = view_player([9.5, 65.0, 9.5]);
        tick_goals_as(game, 0, Some(view), Some(([1, 2, 3, 4], 5000)));
        let reads = READS.lock().unwrap();
        assert_eq!(
            reads.last().copied(),
            Some((true, Some([9.5, 65.0, 9.5]), false)),
            "the guest resolves by uuid and id, reads from its view, and is no MobRef"
        );
        drop(reads);
        // A view with no identity behind it — a fake player's — resolves as
        // no player at all.
        let fake = view_player([9.5, 65.0, 9.5]);
        tick_goals_as(game, 0, Some(fake), None);
        let reads = READS.lock().unwrap();
        assert_eq!(
            reads.last().copied(),
            Some((false, None, false)),
            "a view with no identity behind it resolves as no player"
        );
    });
}

/// What one mod-goal tick costs against a plain engine goal, in nanoseconds:
/// printed once for the PR body (arismoko/horizon#1077). Run with `--ignored --nocapture`.
#[test]
#[ignore = "cost probe"]
fn mod_goal_tick_cost() {
    use horizon_client::mobs::goals::GoalSelector;
    use std::time::Instant;

    fn spin(selector: &mut GoalSelector, m: &mut Mob, cx: &mut Cx, rounds: usize) -> f64 {
        // Warm up, then the best of three: a debug-build probe, but a stable one.
        for _ in 0..rounds / 4 {
            selector.tick(m, cx);
        }
        (0..3)
            .map(|_| {
                let start = Instant::now();
                let mut held = 0u64;
                for _ in 0..rounds {
                    selector.tick(m, cx);
                    held = held.wrapping_add(1);
                }
                std::hint::black_box(held);
                start.elapsed().as_nanos() as f64 / rounds as f64
            })
            .fold(f64::INFINITY, f64::min)
    }

    EVENTS.lock().unwrap().clear();
    install_game_mods();
    with_game(|game| {
        zombie(game, 60, [3.5, 65.0, 3.5]);
        let mut mobs = game.mobs.lift();
        let snapshot: Vec<_> = mobs
            .iter()
            .map(|m| creatures::seen(m, game.level.player_uuid, game.items()))
            .collect();
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        let mut events = Vec::new();
        game.options.dedicated = Some(Vec::new());
        let mut w = game.mob_world_base(&snapshot);
        w.others_index = None;
        let mut cx = Cx {
            w: &w,
            level: &mut level,
            events: &mut events,
        };
        // One plain engine goal of the same shape (always can_use, MOVE).
        let mut plain = GoalSelector::default();
        plain.add(3, Vanilla);
        let per_plain = spin(&mut plain, &mut mobs[0], &mut cx, 20_000);
        // The mod goal through its wrapper and context: a fresh zombie's
        // registered Counter, alone in its selector.
        let per_mod = {
            let mut mob = Mob::new(
                60,
                game.res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone(),
                Vec3::new(3.5, 65.0, 3.5),
                0.0,
            );
            mob.ai.goals.clear();
            mobs::part_goals::attach(&mut mob);
            let mut wrapped = std::mem::take(&mut mob.ai.goals);
            spin(&mut wrapped, &mut mobs[0], &mut cx, 20_000)
        };
        println!(
            "per tick: plain engine goal {per_plain:.1} ns, mod goal {per_mod:.1} ns, \
             wrapper overhead {:+.1} ns",
            per_mod - per_plain
        );
    });
}

/// Registers the zombie's mod goal and listens to `finalize_spawn` without
/// changing anything.
struct GoalsAndFinalize;
#[mod_api::__subscriptions]
impl Hooks for GoalsAndFinalize {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        r.entity_part("minecraft:zombie", ZombieGoals);
    }
    fn finalize_spawn(
        &self,
        _: &dyn mod_api::Host,
        _: &mut mod_api::FinalizeSpawnEvent<'_>,
    ) -> mod_api::Flow {
        mod_api::Flow::Continue
    }
}

/// A `finalize_spawn` listener lends the mob for its call: the mod goal
/// attached when the mob was made (`part_goals::attach`, whose copies carry
/// no goal and name themselves `ModGoal`) is still the mob's own afterwards.
#[test]
#[ignore = "daily"]
fn a_finalize_spawn_listener_keeps_the_mod_goals() {
    // The mods install before the game opens: a game that opens resolves
    // the parts of whatever is installed, and the worker block metadata
    // with them (installing inside the game leaves it unpublished).
    let entries = vec![ModEntry::new("finalize-goals", Box::new(GoalsAndFinalize))];
    mod_platform::__private::install(entries.into());
    crate::game::api_values::install_parts(&[ModEntry::new(
        "finalize-goals",
        Box::new(GoalsAndFinalize),
    )])
    .unwrap();
    with_game(|game| {
        game.resolve_parts().unwrap();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut m = Mob::new(60, kind, Vec3::new(3.5, 65.0, 3.5), 0.0);
        m.on_ground = true;
        let reason = horizon_client::mobs::spawn_rules::Reason::Natural;
        game.finalize_with(&mut m, reason, None);
        assert!(
            m.ai.goals.entries().any(|(_, name, _)| name == "Counter"),
            "the mod goal survives the lend"
        );
    });
}
