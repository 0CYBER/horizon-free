//! Living screen scenes (`native_scene_live`): the input checks and caps, the frame's
//! animating budget, the catch-up bound, the particles' facing, and (daily, with the
//! game's resources) the block-only fast path, a lit furnace's particles replayed, paused,
//! stepped and caught up, the live particle cap, the budget's last state, entity refusals
//! and two mods' scenes of the same id.
use super::{
    native_scene::{SceneBlock, scene_matrix},
    native_scene_live::{LiveScenes, check, facing_viewer, lives, scene_key, ticks_to_run},
};
use glam::Vec3;
use horizon_world::registry::BlockStateId;
use mod_api::{
    Compound, GuiEntity, SCENE_CATCH_UP, SCENE_MAX_ANIMATING, SCENE_MAX_ENTITIES,
    SCENE_MAX_PARTICLES, Scene, SceneClock, SceneEntity, SceneError, SceneParticle, SceneView,
};

fn cow(pos: [f64; 3]) -> SceneEntity {
    SceneEntity::new(GuiEntity::kind("minecraft:cow"), pos)
}

/// Smoke, by the key the game's resources resolved (a game must be loaded).
fn smoke(pos: [f64; 3], velocity: [f64; 3]) -> SceneParticle {
    let key = mod_api::__private::particle_named_key("minecraft:smoke").expect("smoke's key");
    SceneParticle::new(mod_api::ParticleOptions::simple(key), pos, velocity)
}

const RECT: [f32; 4] = [10.0, 10.0, 110.0, 90.0];

fn view() -> SceneView {
    SceneView::new([0.5, 0.5, 0.5], 30.0, 25.0, 20.0)
}

#[test]
fn a_particle_faces_the_viewer_upright_through_any_turn() {
    for (yaw, pitch) in [(0.0, 0.0), (30.0, 25.0), (-135.0, 60.0), (210.0, -10.0)] {
        let v = SceneView::new([0.0; 3], yaw, pitch, 16.0);
        let m = scene_matrix(RECT, v);
        let q = facing_viewer(v);
        let up = m.transform_vector3(q * Vec3::Y);
        let right = m.transform_vector3(q * Vec3::X);
        // Flat to the screen: no depth across the quad; up the screen and to the right.
        assert!(up.z.abs() < 1e-3 && right.z.abs() < 1e-3, "{yaw} {pitch}");
        assert!(
            (up.y + 16.0).abs() < 1e-3 && up.x.abs() < 1e-3,
            "{yaw} {pitch}: {up}"
        );
        assert!(
            (right.x - 16.0).abs() < 1e-3 && right.y.abs() < 1e-3,
            "{yaw} {pitch}"
        );
    }
}

#[test]
fn the_caps_and_mod_input_are_refused_whole_and_named() {
    assert_eq!(SCENE_MAX_ENTITIES, 16);
    assert_eq!(SCENE_MAX_PARTICLES, 512);
    assert_eq!(SCENE_MAX_ANIMATING, 4);
    assert_eq!(mod_api::SCENE_ANIMATE_PICKS, 64);
    let clock = SceneClock::new(1, 0, 0.0);
    let ok = Scene::new(Vec::new())
        .with_clock(clock)
        .with_entities(vec![cow([0.5, 1.0, 0.5])]);
    assert_eq!(check(RECT, &ok, view()), Ok(clock));
    // Sixteen entities are a scene; seventeen are refused by their own name.
    let mut crowd = ok.clone();
    crowd.entities = vec![cow([0.0; 3]); 16];
    assert_eq!(check(RECT, &crowd, view()), Ok(clock));
    crowd.entities.push(cow([0.0; 3]));
    assert_eq!(
        check(RECT, &crowd, view()),
        Err(SceneError::TooManyEntities)
    );
    let mut nan_rect = RECT;
    nan_rect[2] = f32::NAN;
    assert_eq!(check(nan_rect, &ok, view()), Err(SceneError::BadView));
    let turned = SceneView::new([0.0; 3], f32::INFINITY, 0.0, 20.0);
    assert_eq!(check(RECT, &ok, turned), Err(SceneError::BadView));
    let mut far = ok.clone();
    far.entities.push(cow([3.0e7 + 1.0, 0.0, 0.0]));
    assert_eq!(
        check(RECT, &far, view()),
        Err(SceneError::BadEntity { index: 1 })
    );
    let mut spun = ok.clone();
    spun.entities[0] = cow([0.0; 3]).turned(0.0, f32::NAN, 0.0);
    assert_eq!(
        check(RECT, &spun, view()),
        Err(SceneError::BadEntity { index: 0 })
    );
    let mut clockless = ok.clone();
    clockless.clock = None;
    assert_eq!(check(RECT, &clockless, view()), Err(SceneError::NoClock));
    let mut nan_clock = ok;
    nan_clock.clock = Some(SceneClock::new(1, 0, f32::NAN));
    assert_eq!(check(RECT, &nan_clock, view()), Err(SceneError::BadClock));
}

#[test]
fn only_a_scene_with_entities_particles_or_animating_blocks_lives() {
    let blocks = Scene::new(Vec::new());
    assert!(!lives(&blocks));
    assert!(!lives(
        &blocks
            .clone()
            .with_clock(SceneClock::new(1, 5, 0.0).animate_blocks(false))
    ));
    assert!(lives(
        &blocks.clone().with_clock(SceneClock::new(1, 5, 0.0))
    ));
    assert!(lives(&blocks.with_entities(vec![cow([0.0; 3])])));
}

#[test]
fn four_scenes_animate_a_frame_and_a_fifth_waits_for_the_next() {
    let mut scenes = LiveScenes::default();
    for id in 0..4 {
        assert!(scenes.admit(7, "guide", id), "scene {id}");
    }
    assert!(!scenes.admit(7, "guide", 4), "a fifth in the same frame");
    // Another mod's scene of an id already admitted is another scene.
    assert!(!scenes.admit(7, "ponder", 0));
    // A scene drawn twice in a frame counts once.
    assert!(scenes.admit(7, "guide", 2));
    assert!(scenes.admit(8, "guide", 4), "the next frame");
    assert_eq!(scenes.len(), 0, "admitting keeps no scene");
}

#[test]
fn a_clock_runs_the_ticks_between_and_a_long_jump_starts_over_from_the_last_twenty() {
    assert_eq!(SCENE_CATCH_UP, 20);
    assert_eq!(ticks_to_run(5, 5), None, "paused");
    assert_eq!(ticks_to_run(5, 4), None, "behind: the caller starts over");
    assert_eq!(ticks_to_run(5, 6), Some((false, 6, 6)), "one step");
    assert_eq!(
        ticks_to_run(5, 25),
        Some((false, 6, 25)),
        "twenty, caught up"
    );
    assert_eq!(ticks_to_run(5, 26), Some((true, 7, 26)), "twenty-one: over");
    assert_eq!(ticks_to_run(5, 1000), Some((true, 981, 1000)));
    assert_eq!(
        ticks_to_run(0, u64::MAX),
        Some((true, u64::MAX - 19, u64::MAX))
    );
}

#[test]
fn a_scene_starts_over_when_its_blocks_change() {
    let blocks = |state| {
        vec![SceneBlock {
            pos: [0, 0, 0],
            state: BlockStateId(state),
            entity: None,
        }]
    };
    assert_eq!(scene_key(&blocks(5)), scene_key(&blocks(5)));
    assert_ne!(scene_key(&blocks(6)), scene_key(&blocks(5)));
    let mut moved = blocks(5);
    moved[0].pos = [1, 0, 0];
    assert_ne!(scene_key(&moved), scene_key(&blocks(5)));
}

/// The game's lit furnace state, as the API names it.
fn lit_furnace(game: &super::Game<'_>) -> mod_api::BlockState {
    use crate::game::api_values::TryIntoApi as _;
    let registry = game.pack.assets.registry;
    let first = registry
        .default_state("minecraft:furnace")
        .expect("a furnace")
        .0;
    let lit = (first.saturating_sub(8)..first + 8)
        .map(BlockStateId)
        .find(|&id| {
            registry.state(id).is_some_and(|s| {
                s.name == "minecraft:furnace"
                    && s.properties.get("lit").is_some_and(|v| v == "true")
            })
        })
        .expect("a lit furnace");
    lit.try_into_api().expect("the furnace's state")
}

/// Draws `scene` as `DrawContext::draw_scene` does: the result and the vertices drawn.
fn draw(game: &super::Game<'_>, scene: &Scene, view: SceneView) -> (Result<(), SceneError>, usize) {
    let mut batch = horizon_client::hud::HudBatch::scratch([320, 240]);
    let result = crate::game::api_values::aggregates::scene_blocks(&scene.blocks)
        .and_then(|prepared| game.native_draw_scene(&mut batch, RECT, &prepared, scene, view));
    (result, batch.vertices.len())
}

/// A lit furnace's scene `id` run from tick 0 to each of `ticks` in turn, one frame each.
fn run(game: &mut super::Game<'_>, scene: &Scene, ticks: impl IntoIterator<Item = u64>) {
    for tick in ticks {
        let mut at = scene.clone();
        at.clock.as_mut().unwrap().tick = tick;
        game.frames += 1;
        assert_eq!(draw(game, &at, view()).0, Ok(()), "tick {tick}");
    }
}

#[test]
#[ignore = "daily"]
fn a_scene_of_blocks_alone_keeps_no_state_and_reads_no_clock() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let blocks = vec![mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None)];
        let alone = Scene::new(blocks.clone());
        let (result, drawn) = draw(game, &alone, view());
        assert_eq!(result, Ok(()));
        assert!(drawn > 0, "the furnace is drawn");
        assert_eq!(game.live_scenes.borrow().len(), 0);
        let still =
            Scene::new(blocks).with_clock(SceneClock::new(3, 40, 0.5).animate_blocks(false));
        assert_eq!(draw(game, &still, view()), (Ok(()), drawn));
        assert_eq!(
            game.live_scenes.borrow().len(),
            0,
            "a clock that animates nothing"
        );
        assert_eq!(game.scene_tick("", 3), None);
    });
}

#[test]
#[ignore = "daily"]
fn a_scene_accepts_4096_blocks_and_refuses_4097_without_drawing_or_changing_state() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let furnace = mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None);
        // Literal boundary counts keep the expectation independent of the cap constant.
        let blocks = (0..4096)
            .map(|i| {
                mod_api::SceneBlock::new([i % 16, i / 256, (i / 16) % 16], furnace.state, None)
            })
            .collect();
        let accepted = Scene::new(blocks);
        let (result, drawn) = draw(game, &accepted, view());
        assert_eq!(result, Ok(()), "4096 blocks are accepted");
        assert!(drawn > 0, "the accepted scene actually draws its blocks");

        // Keep a running scene so refusal must preserve existing state, not just
        // leave an already empty cache empty.
        let live = Scene::new(vec![furnace.clone()]).with_clock(SceneClock::new(41, 0, 0.0));
        run(game, &live, 0..=30);
        let particles = game.scene_particles("", 41);
        assert!(!particles.is_empty(), "the running furnace has particles");
        let kept = game.live_scenes.borrow().len();
        assert_eq!(game.scene_tick("", 41), Some(30));

        let mut refused = accepted.with_clock(SceneClock::new(41, 31, 0.0));
        refused.blocks.push(furnace);
        assert_eq!(refused.blocks.len(), 4097);
        assert_eq!(
            draw(game, &refused, view()),
            (Err(SceneError::TooManyBlocks), 0)
        );
        assert_eq!(
            game.live_scenes.borrow().len(),
            kept,
            "no scene added or removed"
        );
        assert_eq!(
            game.scene_tick("", 41),
            Some(30),
            "no clock advance or replacement"
        );
        assert_eq!(
            game.scene_particles("", 41),
            particles,
            "no particle mutation"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_furnaces_particles_replay_pause_step_and_start_over_from_the_last_twenty_ticks() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let blocks = vec![mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None)];
        let scene = Scene::new(blocks).with_clock(SceneClock::new(11, 0, 0.0));
        // Forty ticks of flames and smoke, then the same forty from a fresh start.
        run(game, &scene, 0..=40);
        let first = game.scene_particles("", 11);
        assert!(
            !first.is_empty(),
            "a lit furnace's animate ticks make particles"
        );
        assert_eq!(game.scene_tick("", 11), Some(40));
        game.live_scenes.borrow_mut().clear();
        run(game, &scene, 0..=40);
        assert_eq!(
            game.scene_particles("", 11),
            first,
            "the same ticks, the same particles"
        );
        // Paused: the same tick again moves nothing, a later frame too.
        run(game, &scene, [40, 40]);
        assert_eq!(game.scene_particles("", 11), first);
        // Stepped by one: every particle that lived on is a tick older.
        run(game, &scene, [41]);
        let stepped = game.scene_particles("", 11);
        assert_eq!(game.scene_tick("", 11), Some(41));
        let oldest = |ps: &[([f64; 3], [f64; 3], i32)]| ps.iter().map(|p| p.2).max().unwrap_or(0);
        assert!(oldest(&stepped) <= oldest(&first) + 1);
        assert_ne!(stepped, first);
        // A long jump starts over from no particles and runs only the last twenty ticks:
        // nothing older than twenty, and what a scene started at 980 shows at 1000.
        run(game, &scene, [1000]);
        let jumped = game.scene_particles("", 11);
        assert_eq!(game.scene_tick("", 11), Some(1000));
        assert!(jumped.iter().all(|p| p.2 <= 20), "{jumped:?}");
        game.live_scenes.borrow_mut().clear();
        run(game, &scene, [980, 1000]);
        assert_eq!(game.scene_particles("", 11), jumped);
        // The view never changes what lives, an extreme finite centre included.
        game.live_scenes.borrow_mut().clear();
        let far = SceneView::new([f32::MAX, -f32::MAX, f32::MAX], 0.0, 0.0, 20.0);
        for tick in [980, 1000] {
            let mut at = scene.clone();
            at.clock.as_mut().unwrap().tick = tick;
            game.frames += 1;
            let _ = draw(game, &at, far);
        }
        assert_eq!(game.scene_tick("", 11), Some(1000));
        assert_eq!(game.scene_particles("", 11), jumped);
    });
}

#[test]
#[ignore = "daily"]
fn a_scene_holds_at_most_512_live_particles_the_blocks_own_counted() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let blocks = vec![mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None)];
        let puffs = |n| vec![smoke([0.5, 1.5, 0.5], [0.0; 3]); n];
        let scene = Scene::new(blocks).with_clock(SceneClock::new(21, 0, 0.0));
        let too_many = scene.clone().with_particles(puffs(513));
        assert_eq!(
            draw(game, &too_many, view()).0,
            Err(SceneError::TooManyParticles)
        );
        assert_eq!(game.scene_tick("", 21), None, "nothing kept");
        run(game, &scene.clone().with_particles(puffs(512)), [0, 1]);
        assert_eq!(game.scene_particles("", 21).len(), 512);
        // More smoke and the furnace's own flames on the next ticks: still 512.
        run(game, &scene.with_particles(puffs(512)), [2, 3]);
        assert_eq!(game.scene_particles("", 21).len(), 512);
    });
}

#[test]
#[ignore = "daily"]
fn a_fifth_animating_scene_draws_its_last_state_and_is_refused_by_name() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let blocks = vec![mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None)];
        let scene = |id| Scene::new(blocks.clone()).with_clock(SceneClock::new(id, 0, 0.0));
        run(game, &scene(4), 0..=30);
        let last = game.scene_particles("", 4);
        game.frames += 1;
        for id in 0..4 {
            let mut at = scene(id);
            at.clock.as_mut().unwrap().tick = 31;
            assert_eq!(draw(game, &at, view()).0, Ok(()), "scene {id}");
        }
        let mut fifth = scene(4);
        fifth.clock.as_mut().unwrap().tick = 31;
        let (result, drawn) = draw(game, &fifth, view());
        assert_eq!(result, Err(SceneError::TooManyScenes));
        assert!(drawn > 0, "its blocks and last state are drawn");
        assert_eq!(
            game.scene_tick("", 4),
            Some(30),
            "its clock did not advance"
        );
        assert_eq!(game.scene_particles("", 4), last);
        game.frames += 1;
        assert_eq!(draw(game, &fifth, view()).0, Ok(()));
        assert_eq!(game.scene_tick("", 4), Some(31));
    });
}

#[test]
#[ignore = "daily"]
fn scene_entities_are_drawn_and_refused_as_draw_context_entity_refuses_them() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let blocks = vec![mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None)];
        let clock = SceneClock::new(31, 0, 0.0).animate_blocks(false);
        let bare = draw(game, &Scene::new(blocks.clone()), view()).1;
        let with_cow = Scene::new(blocks.clone())
            .with_clock(clock)
            .with_entities(vec![cow([1.5, 0.0, 0.5])]);
        let (result, drawn) = draw(game, &with_cow, view());
        assert_eq!(result, Ok(()));
        assert!(drawn > bare, "the cow's model is drawn");
        let refused = |entities: Vec<SceneEntity>| {
            let scene = Scene::new(blocks.clone())
                .with_clock(clock)
                .with_entities(entities);
            draw(game, &scene, view())
        };
        let arrow = SceneEntity::new(GuiEntity::kind("minecraft:arrow"), [0.0; 3]);
        assert_eq!(
            refused(vec![arrow]),
            (Err(SceneError::UnknownType { index: 0 }), 0)
        );
        let mut pig = Compound::new();
        pig.insert("id".into(), mod_api::Tag::String("minecraft:pig".into()));
        let not_a_cow = SceneEntity::new(GuiEntity::with_data("minecraft:cow", pig), [0.0; 3]);
        assert_eq!(
            refused(vec![cow([0.0; 3]), not_a_cow]),
            (Err(SceneError::BadData { index: 1 }), 0)
        );
        let gone = mod_api::EntityRef::new(mod_api::WorldEpoch::new(0), u32::MAX);
        let stale = SceneEntity::new(GuiEntity::existing(gone), [0.0; 3]);
        assert_eq!(
            refused(vec![stale]),
            (Err(SceneError::NoEntity { index: 0 }), 0)
        );
    });
}

#[test]
#[ignore = "daily"]
fn two_mods_scenes_of_the_same_id_keep_their_own_state() {
    use mod_api::content::crash::{ModCall, enter_native_mod};
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let blocks = vec![mod_api::SceneBlock::new([0, 0, 0], lit_furnace(game), None)];
        let scene = Scene::new(blocks.clone()).with_clock(SceneClock::new(1, 0, 0.0));
        let as_mod = |game: &mut super::Game<'_>, id: &'static str, ticks: Vec<u64>| {
            let _guard = enter_native_mod(ModCall {
                id,
                hook: "screen_draw",
                part: None,
            });
            run(game, &scene, ticks);
        };
        as_mod(game, "guide", (0..=30).collect::<Vec<u64>>());
        as_mod(game, "ponder", vec![0]);
        assert_eq!(game.scene_tick("guide", 1), Some(30));
        assert_eq!(game.scene_tick("ponder", 1), Some(0));
        assert!(!game.scene_particles("guide", 1).is_empty());
        assert!(game.scene_particles("ponder", 1).is_empty());
    });
}
