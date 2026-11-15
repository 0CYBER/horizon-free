use super::*;
use crate::game::{
    commands::Target,
    native_entity_refs::tests::{with_client_game, with_game},
};
use mod_api::{EntityEnteringSectionEvent, Hooks, HostSide};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What each told crossing carried: client side, entity id, old, new, chunk changed.
type Calls = Arc<Mutex<Vec<(bool, u32, [i32; 3], [i32; 3], bool)>>>;

struct Listener(Calls);

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
        self.0.lock().unwrap().push((
            matches!(host, HostSide::Client(_)),
            event.entity.id(),
            event.old_section,
            event.new_section,
            event.chunk_changed,
        ));
    }
}

type Positions = Arc<Mutex<Vec<Option<[f64; 3]>>>>;

struct PositionListener(Positions);

#[mod_api::__subscriptions]
impl Hooks for PositionListener {
    fn entity_entering_section(&self, host: HostSide<'_>, event: &mut EntityEnteringSectionEvent) {
        let position = match host {
            HostSide::Server(host) => event.entity.position(host),
            HostSide::Client(host) => event.entity.position(host),
        };
        self.0.lock().unwrap().push(position);
    }
}

fn observe(test: impl FnOnce(Calls)) {
    let calls = Calls::default();
    let mods = vec![mod_api::ModEntry::new(
        "sections",
        Box::new(Listener(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(calls));
}

fn observe_positions(test: impl FnOnce(Positions)) {
    let positions = Positions::default();
    let mods = vec![mod_api::ModEntry::new(
        "section-positions",
        Box::new(PositionListener(positions.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(positions));
}

type CallbackCount = Arc<AtomicUsize>;

struct CountListener(CallbackCount);

#[mod_api::__subscriptions]
impl Hooks for CountListener {
    fn entity_entering_section(&self, _: HostSide<'_>, _: &mut EntityEnteringSectionEvent) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn observe_count(test: impl FnOnce(CallbackCount)) {
    let count = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "section-count",
        Box::new(CountListener(count.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || test(count));
}

fn scope(level: crate::game::Dim) -> Scope {
    Scope {
        epoch: WorldEpoch::new(1),
        level: level.key(),
    }
}

use crate::game::Dim::{Nether, Overworld};

/// The crossings queued since the last call, as a telling takes them.
fn told(sections: &mut EntitySections) -> Vec<Crossing> {
    match sections.begin_telling() {
        Some(batch) => {
            let out = batch.clone();
            sections.end_telling(batch);
            out
        }
        None => Vec::new(),
    }
}

fn crossing(id: u32, old: [i32; 3], new: [i32; 3]) -> Crossing {
    Crossing { id, old, new }
}

#[test]
fn a_section_is_the_floored_block_shifted_by_four() {
    // Blocks 0..=15 are section 0, -1..=-16 section -1, -17 section -2.
    assert_eq!(section_of([0.0, 15.999, 15.0]), [0, 0, 0]);
    assert_eq!(section_of([16.0, 64.0, 31.5]), [1, 4, 1]);
    assert_eq!(section_of([-0.001, -16.0, -16.001]), [-1, -1, -2]);
    // Java's cast saturates and NaN is 0; `Mth.floor` then wraps `MIN - 1` to `MAX`.
    assert_eq!(section_coord(f64::NAN), 0);
    assert_eq!(section_coord(1e300), i32::MAX >> 4);
    assert_eq!(section_coord(f64::from(i32::MIN)), -134_217_728);
    assert_eq!(section_coord(f64::from(i32::MIN) - 0.5), 134_217_727);
    assert_eq!(section_coord(f64::NEG_INFINITY), 134_217_727);
}

#[test]
fn moving_inside_a_section_is_quiet_and_each_edge_is_one_crossing() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 5, [15.5, 70.0, 3.0]);
    s.moved(w, 5, None, [1.0, 64.0, 15.9]);
    assert!(told(&mut s).is_empty());
    // East over x = 16, then down over y = 64, then north into negative z.
    s.moved(w, 5, None, [16.2, 64.0, 15.9]);
    s.moved(w, 5, None, [16.2, 63.9, 15.9]);
    s.moved(w, 5, None, [16.2, 63.9, -0.1]);
    assert_eq!(
        told(&mut s),
        [
            crossing(5, [0, 4, 0], [1, 4, 0]),
            crossing(5, [1, 4, 0], [1, 3, 0]),
            crossing(5, [1, 3, 0], [1, 3, -1]),
        ]
    );
}

#[test]
fn two_moves_in_one_tick_through_three_sections_are_two_crossings() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    // A, then B and C before anything is told: A -> B and B -> C, not A -> C.
    s.placed(w, 9, [8.0, 70.0, 8.0]);
    s.moved(w, 9, None, [24.0, 70.0, 8.0]);
    s.moved(w, 9, None, [-1000.0, -40.0, 5000.0]);
    assert_eq!(
        told(&mut s),
        [
            crossing(9, [0, 4, 0], [1, 4, 0]),
            crossing(9, [1, 4, 0], [-63, -3, 312]),
        ]
    );
}

#[test]
fn a_move_out_and_back_in_one_tick_is_two_crossings() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 3, [0.5, 70.0, 0.5]);
    s.moved(w, 3, None, [-0.5, 70.0, 0.5]);
    s.moved(w, 3, None, [0.5, 70.0, 0.5]);
    assert_eq!(
        told(&mut s),
        [
            crossing(3, [0, 4, 0], [-1, 4, 0]),
            crossing(3, [-1, 4, 0], [0, 4, 0]),
        ]
    );
}

#[test]
fn a_move_before_the_first_tick_is_told() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    // Filed where it joined, then moved before its first tick.
    s.placed(w, 1, [0.5, 70.0, 0.5]);
    s.moved(w, 1, None, [0.5, 90.0, 0.5]);
    // Moved before its join was filed: the boundary's own start is where it was.
    s.moved(w, 2, Some([40.0, 0.0, 40.0]), [56.0, 0.0, 40.0]);
    // Neither filed nor a known start: filed untold.
    s.moved(w, 4, None, [56.0, 0.0, 40.0]);
    assert_eq!(
        told(&mut s),
        [
            crossing(1, [0, 4, 0], [0, 5, 0]),
            crossing(2, [2, 0, 2], [3, 0, 2]),
        ]
    );
}

#[test]
fn another_level_a_respawn_and_another_world_start_untold() {
    let mut s = EntitySections::default();
    s.placed(scope(Overworld), 4, [0.5, 70.0, 0.5]);
    // Changing level files it in the new one, untold; a move there is told.
    s.moved(scope(Nether), 4, None, [100.0, 70.0, 0.5]);
    s.moved(scope(Nether), 4, None, [116.0, 70.0, 0.5]);
    assert_eq!(told(&mut s), [crossing(4, [6, 4, 0], [7, 4, 0])]);
    // Forgotten (a respawn, a removal): filed again untold.
    s.forget(4);
    s.moved(scope(Nether), 4, None, [0.5, 70.0, 0.5]);
    assert!(told(&mut s).is_empty());
    // Another world open reuses ids: what the last one filed is dropped.
    let next = Scope {
        epoch: WorldEpoch::new(2),
        level: Nether.key(),
    };
    s.moved(next, 4, None, [100.0, 70.0, 0.5]);
    assert!(told(&mut s).is_empty());
}

#[test]
fn what_a_listener_moves_waits_for_the_next_telling() {
    let mut s = EntitySections::default();
    let w = scope(Overworld);
    s.placed(w, 6, [0.5, 70.0, 0.5]);
    s.moved(w, 6, None, [16.5, 70.0, 0.5]);
    let batch = s.begin_telling().unwrap();
    assert_eq!(batch, [crossing(6, [0, 4, 0], [1, 4, 0])]);
    // A listener moves it on while the first crossing is told: not told from inside.
    s.moved(w, 6, None, [32.5, 70.0, 0.5]);
    assert!(s.begin_telling().is_none());
    s.end_telling(batch);
    assert_eq!(told(&mut s), [crossing(6, [1, 4, 0], [2, 4, 0])]);
}

#[test]
#[ignore = "daily"]
fn no_listener_files_no_entity() {
    with_game(|game| {
        let id = game
            .spawn_mob_spawned(
                "cow",
                [1.0, 70.0, 3.0],
                crate::game::creatures::SpawnSite::Plain,
            )
            .unwrap();
        game.teleport_to(Target::Mob(id), [40.0, 70.0, 3.0], None, true);
        assert!(game.native_lifecycle.entity_sections.filed.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_server_mob_moved_twice_in_one_tick_is_told_each_move() {
    observe(|calls| {
        with_game(|game| {
            let id = game
                .spawn_mob_spawned(
                    "cow",
                    [1.0, 70.0, 3.0],
                    crate::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            // Before its first tick, A -> B -> C by two teleports.
            game.teleport_to(Target::Mob(id), [-0.25, 70.0, 3.0], None, true);
            game.teleport_to(Target::Mob(id), [-0.25, 130.0, 3.0], None, true);
            // Out of the section and back: two more.
            game.teleport_to(Target::Mob(id), [-0.25, 130.0, 20.0], None, true);
            game.teleport_to(Target::Mob(id), [-0.25, 130.0, 3.0], None, true);
            // Inside the section: none.
            game.teleport_to(Target::Mob(id), [-8.0, 129.0, 9.0], None, true);
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 4, 0], [-1, 4, 0], true),
                    (false, id, [-1, 4, 0], [-1, 8, 0], false),
                    (false, id, [-1, 8, 0], [-1, 8, 1], true),
                    (false, id, [-1, 8, 1], [-1, 8, 0], true),
                ]
            );
        })
    });
}

/// A cow joined at `at` in a loaded, empty chunk (0, 0), floating (`NoGravity`): filed by its
/// join, as `addEntity` files it.
fn floating_cow(game: &mut crate::game::Game<'_>, at: [f64; 3]) -> u32 {
    if !game.world.chunks.contains_key(&(0, 0)) {
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
    }
    let id = game
        .spawn_mob_spawned("cow", at, crate::game::creatures::SpawnSite::Plain)
        .unwrap();
    assert!(game.native_joins_flush(false, 0));
    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
    game.mobs.update(i, |m| m.no_gravity = true);
    id
}

/// A goal that, when it starts, teleports its mob to each of its targets in turn, once: the
/// `snapTo` an enderman's, a pet's or a shulker's teleport makes in its tick.
#[derive(Clone, Debug)]
struct Teleports(Vec<[f64; 3]>);

impl horizon_client::mobs::goals::Goal for Teleports {
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(
        &mut self,
        _: &mut horizon_client::mobs::Mob,
        _: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        !self.0.is_empty()
    }
    fn start(
        &mut self,
        m: &mut horizon_client::mobs::Mob,
        cx: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) {
        for [x, y, z] in self.0.drain(..) {
            let at = horizon_core::geometry::Vec3::new(x, y, z);
            horizon_client::mobs::living::teleports::snap_to_with(
                m,
                at,
                cx.w.section_writer.clone(),
            );
        }
    }
    fn box_clone(&self) -> Box<dyn horizon_client::mobs::goals::Goal> {
        Box::new(self.clone())
    }
}

/// The cow `id`, moving at `velocity`, runs real mob ticks until one has teleported it through
/// `targets` (its [`Teleports`] goal starting) and then walked it on, in that same tick.
fn teleport_then_walk(
    game: &mut crate::game::Game<'_>,
    id: u32,
    targets: &[[f64; 3]],
    velocity: [f64; 3],
) {
    use horizon_core::geometry::Vec3;
    let i = game.mobs.iter().position(|m| m.id == id).unwrap();
    let start = game.mobs[i].position.y;
    game.mobs.update(i, |m| {
        m.ai.goals.add(0, Teleports(targets.to_vec()));
        m.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
    });
    for _ in 0..4 {
        game.tick_creature_entities().unwrap();
        let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
        if (at.y - start).abs() > 5.0 {
            return;
        }
    }
    panic!("the cow's tick never teleported it");
}

#[test]
#[ignore = "daily"]
fn a_mobs_teleport_and_its_walk_in_one_tick_are_two_crossings() {
    observe(|calls| {
        with_game(|game| {
            // Section y 6 (blocks 96 to 111), teleported near the top of y 7, then 0.6 up into y 8.
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            teleport_then_walk(game, id, &[[8.5, 127.75, 8.5]], [0.0, 0.6, 0.0]);
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(at.y >= 128.0, "the walk took it into y 8: {at:?}");
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                    (false, id, [0, 7, 0], [0, 8, 0], false),
                ]
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_real_mob_tick_exposes_each_intermediate_position_to_the_listener() {
    observe_positions(|positions| {
        with_game(|game| {
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            teleport_then_walk(game, id, &[[8.5, 127.75, 8.5]], [0.0, 0.6, 0.0]);
            let positions = positions.lock().unwrap();
            assert_eq!(
                positions.first().copied().flatten(),
                Some([8.5, 127.75, 8.5])
            );
            assert_eq!(positions.len(), 2);
            assert!(positions[1].is_some_and(|at| at[1] >= 128.0));
        })
    });
}

#[derive(Clone, Debug)]
struct TeleportsAndChecks {
    callbacks: CallbackCount,
    failed: Arc<AtomicBool>,
    ran: bool,
}

impl horizon_client::mobs::goals::Goal for TeleportsAndChecks {
    fn flags(&self) -> u8 {
        0
    }

    fn can_use(
        &mut self,
        _: &mut horizon_client::mobs::Mob,
        _: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        !self.ran
    }

    fn start(
        &mut self,
        m: &mut horizon_client::mobs::Mob,
        cx: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) {
        self.ran = true;
        let first = horizon_core::geometry::Vec3::new(8.5, 120.0, 8.5);
        horizon_client::mobs::living::teleports::set_pos_with(
            m,
            first,
            cx.w.section_writer.clone(),
        );
        if self.callbacks.load(Ordering::SeqCst) != 1 {
            self.failed.store(true, Ordering::SeqCst);
        }
        let second = horizon_core::geometry::Vec3::new(8.5, 143.75, 8.5);
        horizon_client::mobs::living::teleports::set_pos_with(
            m,
            second,
            cx.w.section_writer.clone(),
        );
        if self.callbacks.load(Ordering::SeqCst) != 2 {
            self.failed.store(true, Ordering::SeqCst);
        }
    }

    fn box_clone(&self) -> Box<dyn horizon_client::mobs::goals::Goal> {
        Box::new(self.clone())
    }
}

#[test]
#[ignore = "daily"]
fn a_real_mob_tick_tells_inside_each_position_write() {
    observe_count(|callbacks| {
        with_game(|game| {
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            game.tick_creature_entities().unwrap();
            callbacks.store(0, Ordering::SeqCst);
            let failed = Arc::new(AtomicBool::new(false));
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs.update(i, |m| {
                m.ai.goals.add(
                    0,
                    TeleportsAndChecks {
                        callbacks: callbacks.clone(),
                        failed: failed.clone(),
                        ran: false,
                    },
                )
            });
            // Mob.serverAiStep only scans for new goals on alternating ticks.
            // The first tick after installing this goal may tick running goals only.
            for _ in 0..2 {
                game.tick_creature_entities().unwrap();
                if callbacks.load(Ordering::SeqCst) != 0 {
                    break;
                }
            }
            assert!(!failed.load(Ordering::SeqCst));
            assert_eq!(callbacks.load(Ordering::SeqCst), 2);
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_mobs_teleport_out_and_its_walk_back_in_one_tick_are_two_crossings() {
    observe(|calls| {
        with_game(|game| {
            // Teleported just over y = 112 into section y 7, then 0.6 down, back into y 6.
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            teleport_then_walk(game, id, &[[8.5, 112.25, 8.5]], [0.0, -0.6, 0.0]);
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(at.y < 112.0, "the walk took it back into y 6: {at:?}");
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                    (false, id, [0, 7, 0], [0, 6, 0], false),
                ]
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_mob_that_teleports_twice_in_its_tick_then_walks_is_three_crossings() {
    observe(|calls| {
        with_game(|game| {
            // y 6, then in one tick teleported into y 7 and on near the top of y 8, then 0.6 up
            // into y 9: A -> B, B -> C and C -> D, not only the last teleport.
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            let targets = [[8.5, 120.0, 8.5], [8.5, 143.75, 8.5]];
            teleport_then_walk(game, id, &targets, [0.0, 0.6, 0.0]);
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(at.y >= 144.0, "the walk took it into y 9: {at:?}");
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                    (false, id, [0, 7, 0], [0, 8, 0], false),
                    (false, id, [0, 8, 0], [0, 9, 0], false),
                ]
            );
        })
    });
}

/// A goal that, when it starts, runs `LivingEntity.refreshDimensions`' nudge for a mob that grew
/// from a 0.1 by 0.1 box (`fudgePositionAfterSizeChange`, a `setPos`), then sets the mob moving
/// up at 0.6, once.
#[derive(Clone, Debug)]
struct Grows(bool);

impl horizon_client::mobs::goals::Goal for Grows {
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(
        &mut self,
        _: &mut horizon_client::mobs::Mob,
        _: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        !self.0
    }
    fn start(
        &mut self,
        m: &mut horizon_client::mobs::Mob,
        cx: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) {
        self.0 = true;
        m.fudge_position_after_size_change(cx.w, 0.1, 0.1);
        m.velocity = horizon_core::geometry::Vec3::new(0.0, 0.6, 0.0);
    }
    fn box_clone(&self) -> Box<dyn horizon_client::mobs::goals::Goal> {
        Box::new(self.clone())
    }
}

#[test]
#[ignore = "daily"]
fn a_size_change_nudge_in_a_mobs_tick_then_its_walk_are_two_crossings() {
    observe(|calls| {
        with_game(|game| {
            // y 7, 0.3 over its floor. Growing to an adult cow's 1.4 from 0.1 keeps its centre, so
            // the nudge sets it 0.65 lower, into y 6; the same tick walks it 0.6 up, back into y 7.
            // Told only where the tick ended, it would be no crossing at all.
            let id = floating_cow(game, [8.5, 112.3, 8.5]);
            game.tick_creature_entities().unwrap();
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs.update(i, |m| m.ai.goals.add(0, Grows(false)));
            for _ in 0..4 {
                game.tick_creature_entities().unwrap();
                if !calls.lock().unwrap().is_empty() {
                    break;
                }
            }
            let at = game.mobs.iter().find(|m| m.id == id).unwrap().position;
            assert!(
                (at.y - 112.25).abs() < 1e-3,
                "nudged to 111.65, walked to 112.25: {at:?}"
            );
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (false, id, [0, 7, 0], [0, 6, 0], false),
                    (false, id, [0, 6, 0], [0, 7, 0], false),
                ]
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_mods_random_teleport_of_a_mob_is_told_at_once() {
    observe(|calls| {
        with_game(|game| {
            crate::game::native_living_position::tests::floor_and_zombies(game);
            // The zombie at (0.5, 64, 0.5) is filed where it stands, then sent to land on the
            // floor at (-3.5, 64, -3.5): one section west and one north.
            game.teleport_to(Target::Mob(12), [0.5, 64.0, 0.5], None, true);
            assert!(calls.lock().unwrap().is_empty());
            let zombie = game.living(game.entity(12).unwrap()).unwrap();
            assert!(game.living_random_teleport(zombie, [-3.5, 66.0, -3.5], None));
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[(false, 12, [0, 4, 0], [-1, 4, -1], true)]
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_server_entity_that_leaves_is_forgotten() {
    observe(|_| {
        with_game(|game| {
            let id = floating_cow(game, [8.5, 100.0, 8.5]);
            assert!(
                game.native_lifecycle
                    .entity_sections
                    .filed
                    .contains_key(&id)
            );
            // Despawned or unloaded: no longer in any list, so it leaves the level.
            game.mobs.clear();
            game.native_leaves_flush();
            assert!(
                !game
                    .native_lifecycle
                    .entity_sections
                    .filed
                    .contains_key(&id)
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_server_mobs_own_move_is_not_reconstructed_after_its_post() {
    use horizon_core::geometry::Vec3;
    observe(|calls| {
        with_game(|game| {
            let id = game
                .spawn_mob_spawned(
                    "cow",
                    [1.0, 70.0, 3.0],
                    crate::game::creatures::SpawnSite::Plain,
                )
                .unwrap();
            let kind = game.mobs.iter().find(|m| m.id == id).unwrap().kind.clone();
            // The mobs' round: Pre, the tick (its own move over z = 16), then the Post.
            let mut round = game.native_tick_round();
            assert!(game.native_tick_pre_batch(
                &mut round,
                id,
                crate::game::native_ticks::TickKind::Mob(kind)
            ));
            let i = game.mobs.iter().position(|m| m.id == id).unwrap();
            game.mobs
                .update(i, |m| m.position = Vec3::new(1.0, 70.0, 17.0));
            assert!(calls.lock().unwrap().is_empty());
            game.native_tick_post_batch(round);
            // A raw field edit is not a position-write boundary. The Post hook must not
            // synthesize a crossing after the fact; real ticks record their writes before Post.
            assert!(calls.lock().unwrap().is_empty());
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_client_is_told_each_teleport_and_where_its_entities_really_are() {
    use horizon_core::geometry::Vec3;
    use horizon_protocol::packets::play::{self, Clientbound as C};
    observe(|calls| {
        with_client_game(|game| {
            let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Steve").unwrap();
            game.handle_packet(C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: 8,
                simulation_distance: 8,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 1,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            }))
            .unwrap();
            let teleport = |id: i32, at: [f64; 3]| {
                C::PlayerPosition(play::PlayerPosition {
                    teleport: id,
                    position: at,
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                })
            };
            // The player is placed, then sent A -> B -> A before the client ticks.
            game.handle_packet(teleport(1, [8.0, 70.0, 8.0])).unwrap();
            game.handle_packet(teleport(2, [8.0, 70.0, 24.0])).unwrap();
            game.handle_packet(teleport(3, [8.0, 70.0, 8.0])).unwrap();
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[
                    (true, 7, [0, 4, 0], [0, 4, 1], true),
                    (true, 7, [0, 4, 1], [0, 4, 0], true),
                ]
            );
            calls.lock().unwrap().clear();
            let pig = game
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id("minecraft:pig")
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: 20,
                uuid: 20,
                kind: pig,
                position: [1.0, 70.0, 3.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            // Its own move (the interpolation the client steps), not the packet's position.
            let i = game.mobs.iter().position(|m| m.id == 20).unwrap();
            game.mobs
                .update(i, |m| m.position = Vec3::new(1.0, 70.0, -2.0));
            assert!(calls.lock().unwrap().is_empty());
            game.client_entity_sections_stepped();
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[(true, 20, [0, 4, 0], [0, 4, -1], true)]
            );
        })
    });
}

/// A mod's `teleport_to_level` into another level is a change of level, not a crossing: the
/// recreated entity is filed in the new level untold (its join), this call tells nothing of its
/// own, and its next move there over a section edge is told once.
#[test]
#[ignore = "daily"]
fn an_entity_moved_into_another_level_is_told_once_at_its_next_crossing() {
    observe(|calls| {
        with_game(|game| {
            game.native_lifecycle.world_open_ready().unwrap();
            game.summon_with_nbt(
                "minecraft:item",
                [8.5, 80.0, 8.5],
                r#"{Item:{id:"minecraft:stick",count:1},NoGravity:1b}"#,
            )
            .unwrap();
            game.lan_assign_ids();
            let item = game.entity(game.dropped[0].id).unwrap();
            assert!(game.entity_teleport_to_level(
                item,
                "minecraft:the_nether",
                [8.5, 80.0, 8.5],
                None
            ));
            assert!(
                calls.lock().unwrap().is_empty(),
                "the change of level is not told"
            );
            game.enter_level(Nether).unwrap();
            let moved = game.dropped[0].id;
            game.teleport_to(Target::Item(moved), [24.5, 80.0, 8.5], None, true);
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                &[(false, moved, [0, 5, 0], [1, 5, 0], true)]
            );
        })
    });
}
