//! `register_entity_renderers` for living types (arismoko/horizon#860 stage 3): a renderer registered for a type
//! drawn as a mob draws in place of the type's model and vanilla layers, reading the living
//! frame; the mods' layers and the render hook pair still run around it, a canceled
//! `render_living_pre` skips it, and a failing renderer leaves the type its own model until a
//! resource reload resolves it again.
use super::Game;
use horizon_client::{mesher::Vertex, mobs::Mob};
use mod_api::{
    BlockEntityDrawContext, EntityRenderLayer, EntityRenderState, EntityRenderer, Hooks,
    LayerTarget, Pose, PosedParts, RegisterEntityLayersEvent, RegisterEntityRenderersEvent,
    RegistrationContext, RenderLivingPostEvent, RenderLivingPreEvent,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

/// What the scene's mod heard, in order, and the entities whose `render_living_pre` it cancels.
#[derive(Default)]
struct Heard {
    events: Vec<String>,
    cancel: Vec<u32>,
}

type Shared = Arc<Mutex<Heard>>;

/// One stone quad a block wide, `height` blocks above the entity's feet.
fn raised_quad(draw: &mut dyn BlockEntityDrawContext, height: f32) {
    let (low, high) = (height, height + 1.0);
    let corners = [
        [0.0, high, 0.0],
        [0.0, low, 0.0],
        [1.0, low, 0.0],
        [1.0, high, 0.0],
    ];
    draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
}

/// The zombie's renderer: draws a quad 100 blocks up and records the living frame it read.
struct Zombie(Shared);
impl EntityRenderer for Zombie {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let living = entity.living.ok_or("no living frame")?;
        self.0.lock().unwrap().events.push(format!(
            "renderer {} body {} baby {} {:?}",
            entity.id, living.body_rot, living.is_baby, living.pose
        ));
        raised_quad(draw, 100.0);
        Ok(())
    }
}

/// The skeleton's renderer: refuses its first call with an `Err`, then draws a quad 400 blocks
/// up; `calls` counts the calls.
#[derive(Default)]
struct Flaky {
    calls: AtomicU32,
}
impl EntityRenderer for Flaky {
    fn submit(
        &self,
        _: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let first = self.calls.fetch_add(1, Ordering::Relaxed) == 0;
        raised_quad(draw, 400.0);
        if first {
            return Err("refused its first frame".into());
        }
        Ok(())
    }
}

/// A layer on the zombie: draws a quad 200 blocks up and records whether it had posed parts.
struct Layer(Shared);
impl EntityRenderLayer for Layer {
    fn submit(
        &self,
        entity: &EntityRenderState,
        parts: Option<&PosedParts>,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let event = format!("layer {} parts {}", entity.id, parts.is_some());
        self.0.lock().unwrap().events.push(event);
        raised_quad(draw, 200.0);
        Ok(())
    }
}

/// The scene's mod: the zombie's and the skeleton's renderers, a layer on the zombie, and the
/// living render hook pair.
struct Scene {
    heard: Shared,
    skeleton: Arc<Flaky>,
}

#[mod_api::__subscriptions]
impl Hooks for Scene {
    fn register_entity_renderers(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterEntityRenderersEvent<'_>,
    ) {
        let zombie = Arc::new(Zombie(self.heard.clone()));
        event.renderers.register("minecraft:zombie".into(), zombie);
        let skeleton = self.skeleton.clone();
        event
            .renderers
            .register("minecraft:skeleton".into(), skeleton);
    }
    fn register_entity_layers(
        &self,
        _: &RegistrationContext,
        event: &mut RegisterEntityLayersEvent<'_>,
    ) {
        let target = LayerTarget::Entity("minecraft:zombie".into());
        let layer = Arc::new(Layer(self.heard.clone()));
        event.layers.add(target, layer).unwrap();
    }
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.heard.lock().unwrap();
        heard.events.push(format!("living_pre {}", event.entity.id));
        event.cancelled = heard.cancel.contains(&event.entity.id);
        mod_api::Flow::Continue
    }
    fn render_living_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPostEvent<'_>,
    ) {
        let mut heard = self.heard.lock().unwrap();
        heard
            .events
            .push(format!("living_post {}", event.entity.id));
    }
}

/// The camera, south of the entity and looking at it.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);

/// How high above the entity's feet (at y 64) a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (64.0 - CAMERA.y) as f32
}

/// The heights the quads `out` holds start at, in order.
fn quads(out: &[Vertex]) -> Vec<f32> {
    out.chunks(4)
        .map(|quad| quad.iter().map(above).fold(f32::MAX, f32::min).round())
        .collect()
}

/// One frame of the level's mobs with only a mob of `kind`, id `id`, its body turned 30 degrees.
fn frame(game: &mut Game<'_>, kind: &str, id: u32) -> Vec<Vertex> {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut mob = Mob::new(id, kind, at, 30.0);
    (mob.body_yaw, mob.previous_body_yaw) = (30.0, 30.0);
    game.mobs.clear();
    game.mobs.push(mob);
    game.prepare_entity_culling(glam::DVec3::ZERO);
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

fn events(heard: &Shared) -> Vec<String> {
    std::mem::take(&mut heard.lock().unwrap().events)
}

/// Runs `check` on a client game with prepared resources and the [`Scene`] mod, resolved as the
/// first load does. In a child process running only `test`: the registrations are kept once per
/// process.
fn in_scene(test: &str, check: impl FnOnce(&mut Game<'_>, &Shared, &Flaky)) {
    if std::env::var_os("HORIZON_LIVING_RENDERERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("game::native_living_renderers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("HORIZON_LIVING_RENDERERS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let heard = Shared::default();
    let skeleton = Arc::new(Flaky::default());
    let scene = Scene {
        heard: heard.clone(),
        skeleton: skeleton.clone(),
    };
    let mods = vec![mod_api::ModEntry::new("scene", Box::new(scene))].into();
    mod_platform::testing::with_mods(mods, || {
        super::mods::register_entity_renderers(super::execution_role::ExecutionRole::Client);
        super::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &heard, &skeleton);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_living_type_draws_its_renderer_in_place_of_its_model() {
    in_scene(
        "a_registered_living_type_draws_its_renderer_in_place_of_its_model",
        |game, heard, _| {
            let zombie = frame(game, "minecraft:zombie", 91);
            assert_eq!(
                quads(&zombie),
                [100.0, 200.0],
                "the renderer's quad, then the layer's, and no vanilla model"
            );
            assert_eq!(
                events(heard),
                [
                    "living_pre 91",
                    "renderer 91 body 30 baby false Standing",
                    "layer 91 parts false",
                    "living_post 91"
                ]
            );
            let cow = frame(game, "minecraft:cow", 92);
            assert!(!cow.is_empty(), "an unregistered type draws its model");
            assert!(cow.iter().all(|v| above(v) < 3.0), "{cow:?}");
            assert_eq!(events(heard), ["living_pre 92", "living_post 92"]);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_screens_entity_keeps_the_layers_beside_a_registered_renderer() {
    in_scene(
        "a_screens_entity_keeps_the_layers_beside_a_registered_renderer",
        |game, heard, _| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
            let zombie = Mob::new_remote(93, kind, at, 0.0);
            let mut out = Vec::new();
            game.gui_entity_geometry(&zombie, &mut out);
            // Seen from its feet: the renderer's quad 100 up, then the layer's 200 up.
            let heights: Vec<f32> = out
                .chunks(4)
                .map(|q| q.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min).round())
                .collect();
            assert_eq!(
                heights,
                [100.0, 200.0],
                "both geometries, and no vanilla model"
            );
            let heard = events(heard);
            assert_eq!(heard.len(), 2, "{heard:?}");
            assert!(heard[0].starts_with("renderer 93 "), "{heard:?}");
            assert_eq!(heard[1], "layer 93 parts false");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_canceled_living_pre_skips_a_registered_renderer() {
    in_scene(
        "a_canceled_living_pre_skips_a_registered_renderer",
        |game, heard, _| {
            heard.lock().unwrap().cancel.push(91);
            let zombie = frame(game, "minecraft:zombie", 91);
            assert_eq!(zombie.len(), 0, "{zombie:?}");
            assert_eq!(events(heard), ["living_pre 91"]);
        },
    );
}

/// A failing renderer leaves the type its model and is asked no more; only a reload's fresh table
/// asks it again.
#[test]
#[ignore = "daily"]
fn a_failed_renderer_draws_the_types_model_until_a_resource_reload() {
    in_scene(
        "a_failed_renderer_draws_the_types_model_until_a_resource_reload",
        |game, _, skeleton| {
            for frame_name in ["the failing frame", "the next frame"] {
                let drawn = frame(game, "minecraft:skeleton", 93);
                assert!(!drawn.is_empty(), "{frame_name}: its own model");
                assert!(
                    drawn.iter().all(|v| above(v) < 3.0),
                    "{frame_name}: no quad kept: {drawn:?}"
                );
                let calls = skeleton.calls.load(Ordering::Relaxed);
                assert_eq!(calls, 1, "{frame_name}: asked once, then no more");
            }
            super::model_baking_completed_tests::reload(game);
            let drawn = frame(game, "minecraft:skeleton", 93);
            assert_eq!(skeleton.calls.load(Ordering::Relaxed), 2, "resolved again");
            assert_eq!(quads(&drawn), [400.0], "its quad alone, no bow or model");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_sleeping_or_baby_frame_reads_as_the_model_lays_it() {
    super::native_entity_refs::tests::with_client_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
        let mut villager = Mob::new(94, kind, at, 0.0);
        // A baby asleep in a bed facing east.
        villager.extra.age = -24000;
        villager.extra.ints[1] = 4;
        let state = super::native_mob_draw::entity_render_state(&villager, 94, 0.5);
        let living = state.living.unwrap();
        assert_eq!(living.pose, Pose::Sleeping);
        assert_eq!(living.bed_orientation, Some(mod_api::Direction::East));
        assert!(living.is_baby);
        assert_eq!(living.age_scale, 0.5);
    });
}

#[test]
#[ignore = "daily"]
fn own_yaw_is_independent_of_body_rotation_across_a_wrap() {
    super::native_entity_refs::tests::with_client_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(
            95,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            10.0,
        );
        mob.previous_yaw = 350.0;
        mob.previous_body_yaw = 30.0;
        mob.body_yaw = 50.0;
        mob.previous_head_yaw = 70.0;
        mob.head_yaw = 90.0;
        let state = super::native_mob_draw::entity_render_state(&mob, 95, 0.5);
        assert_eq!(state.yaw, 360.0); // The shortest interpolation reaches 0 degrees modulo 360.
        assert_eq!(state.living.unwrap().body_rot, 40.0);
        assert_eq!(state.living.unwrap().y_rot, 40.0);
    });
}

#[test]
#[ignore = "daily"]
fn extracted_living_signs_death_time_and_hurt_overlay_are_literal() {
    super::native_entity_refs::tests::with_client_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(
            96,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        mob.body_yaw = 20.0;
        mob.previous_body_yaw = 20.0;
        mob.head_yaw = 50.0;
        mob.previous_head_yaw = 50.0;
        mob.head_pitch = 15.0;
        mob.previous_head_pitch = 15.0;
        mob.custom_name = Some("Dinnerbone".into());
        let frame = |mob: &Mob| {
            super::native_mob_draw::entity_render_state(mob, 96, 0.5)
                .living
                .unwrap()
        };
        let living = frame(&mob);
        assert_eq!((living.y_rot, living.x_rot), (-30.0, -15.0));
        assert!(living.is_upside_down);
        assert_eq!(living.death_time, 0.0);
        assert!(!living.has_red_overlay);
        mob.hurt_time = 2;
        assert!(frame(&mob).has_red_overlay);
        mob.hurt_time = 0;
        mob.death_time = 7;
        assert_eq!(frame(&mob).death_time, 7.5);
        assert!(frame(&mob).has_red_overlay);
    });
}
