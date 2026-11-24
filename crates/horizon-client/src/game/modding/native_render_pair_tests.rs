//! The render hook pair around a living entity (arismoko/horizon#3681): `render_living_pre` and
//! `render_living_post`, wrapped for a player by `render_player_pre` and `render_player_post`. A
//! canceled pre hook skips the entity's model, its layers, its post hooks and the frame's later
//! passes; a culled entity is asked neither; the hooks run in the draw queue's order.
use super::Game;
use horizon_client::{mesher::Vertex, mobs::Mob};
use mod_api::{
    BlockEntityDrawContext, Hooks, RenderLivingPostEvent, RenderLivingPreEvent,
    RenderPlayerPostEvent, RenderPlayerPreEvent,
};
use std::sync::{Arc, Mutex};

/// What the mod heard, in order, and which entities it cancels.
#[derive(Default)]
struct Heard {
    events: Vec<String>,
    /// Entity ids whose `render_living_pre` it cancels.
    cancel_living: Vec<u32>,
    /// Whether it cancels `render_player_pre` of this window's own player (`true`) or another.
    cancel_player: Vec<bool>,
}

/// A mod listening to the four hooks: it records each call and, in each pre hook, draws one stone
/// quad 100 blocks above the entity, so a test sees what a canceled entity still draws.
struct Pair(Arc<Mutex<Heard>>);

fn marker(draw: &mut dyn BlockEntityDrawContext) {
    let corners = [
        [0.0, 101.0, 0.0],
        [0.0, 100.0, 0.0],
        [1.0, 100.0, 0.0],
        [1.0, 101.0, 0.0],
    ];
    draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
}

#[mod_api::__subscriptions]
impl Hooks for Pair {
    fn render_player_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.events.push(format!("player_pre {}", event.who.local));
        event.cancelled = heard.cancel_player.contains(&event.who.local);
        mod_api::Flow::Continue
    }
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.events.push(format!("living_pre {}", event.entity.id));
        marker(event.draw);
        event.cancelled = heard.cancel_living.contains(&event.entity.id);
        mod_api::Flow::Continue
    }
    fn render_living_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard
            .events
            .push(format!("living_post {}", event.entity.id));
    }
    fn render_player_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard
            .events
            .push(format!("player_post {}", event.who.local));
    }
}

/// The camera, south of the entities and looking at them.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);

/// A mob of `kind` with id `id`, standing `x` blocks east of the camera's line.
fn mob(game: &Game<'_>, kind: &str, id: u32, x: f64) -> Mob {
    let kind = if kind == "minecraft:player" {
        game.res.player_type().clone().unwrap()
    } else {
        game.res.mob_types().get(kind).unwrap().clone()
    };
    let at = horizon_core::geometry::Vec3::new(0.5 + x, 64.0, 0.5);
    Mob::new(id, kind, at, 0.0)
}

/// How many of `out`'s vertices are 100 or more blocks up: the pre hooks' markers.
fn markers(out: &[Vertex]) -> usize {
    out.iter()
        .filter(|v| v.pos[1] - (64.0 - CAMERA.y) as f32 >= 99.0)
        .count()
}

/// One frame of the level's mobs, begun as the entity pass begins it.
fn mob_frame(game: &mut Game<'_>) -> Vec<Vertex> {
    game.prepare_entity_culling(glam::DVec3::ZERO);
    let mut out = Vec::new();
    game.mob_geometry(CAMERA, 1.0, &mut out);
    out
}

/// Runs `check` on a client game with prepared resources and the [`Pair`] mod.
fn with_pair(heard: &Arc<Mutex<Heard>>, check: impl FnOnce(&mut Game<'_>)) {
    let mods = vec![mod_api::ModEntry::new(
        "pair",
        Box::new(Pair(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.mobs.clear();
            check(game);
        });
    });
}

fn events(heard: &Arc<Mutex<Heard>>) -> Vec<String> {
    std::mem::take(&mut heard.lock().unwrap().events)
}

#[test]
#[ignore = "daily"]
fn two_entities_are_asked_in_the_draw_queues_order() {
    let heard = Arc::default();
    with_pair(&heard, |game| {
        game.mobs.push(mob(game, "minecraft:zombie", 91, 0.0));
        game.mobs.push(mob(game, "minecraft:cow", 92, 2.0));
        let out = mob_frame(game);
        assert_eq!(
            events(&heard),
            [
                "living_pre 91",
                "living_post 91",
                "living_pre 92",
                "living_post 92"
            ]
        );
        assert_eq!(markers(&out), 8, "each pre hook's marker is kept");
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_living_pre_skips_the_model_and_the_post_hook() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        game.mobs.push(mob(game, "minecraft:zombie", 91, 0.0));
        let drawn = mob_frame(game);
        assert!(drawn.len() > 4, "its model, beside the marker");
        events(&heard);
        heard.lock().unwrap().cancel_living.push(91);
        let canceled = mob_frame(game);
        assert_eq!(events(&heard), ["living_pre 91"], "no post hook");
        assert_eq!(
            canceled.len(),
            4,
            "the marker alone, no model: {}",
            canceled.len()
        );
        assert_eq!(markers(&canceled), 4);
        // The frame's later passes (name tag, shadow, flames, leash) leave it out.
        assert!(game.render_canceled(91));
        assert_eq!(game.drawn_mobs().count(), 0);
    });
}

#[test]
#[ignore = "daily"]
fn a_canceled_entity_draws_no_name_tag() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        let mut named = mob(game, "minecraft:zombie", 91, 0.0);
        named.custom_name = Some("Bob".into());
        named.name_visible = true;
        game.mobs.push(named);
        let tag = |game: &mut Game<'_>| {
            mob_frame(game);
            let (mut out, mut see_through) = (Vec::new(), Vec::new());
            game.name_tag_geometry(CAMERA, 1.0, &mut out, &mut see_through);
            out.len() + see_through.len()
        };
        assert!(tag(game) > 0, "a named zombie's tag");
        heard.lock().unwrap().cancel_living.push(91);
        assert_eq!(tag(game), 0, "none once its pre hook cancels");
    });
}

#[test]
#[ignore = "daily"]
fn a_culled_entity_is_asked_neither_hook() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        // Far past any render distance.
        game.mobs.push(mob(game, "minecraft:zombie", 91, 100_000.0));
        let out = mob_frame(game);
        assert_eq!(events(&heard), Vec::<String>::new());
        assert_eq!(markers(&out), 0);
    });
}

/// This window's player in third person, drawn by the game's own path for it, with its own entity
/// id 4321; spectating when `spectator`.
fn own_player_frame(game: &mut Game<'_>, spectator: bool) -> Vec<Vertex> {
    let at = horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5);
    let mut player = horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap();
    player.entity_id = 4321;
    game.player = Some(player);
    if let Some(remote) = game.remote.as_mut() {
        remote.entity_id = 4321;
    }
    game.camera_type = horizon_client::camera::CameraType::ThirdPersonBack;
    game.tick_player_model();
    game.player_model.as_mut().unwrap().extra.spectator = spectator;
    game.prepare_entity_culling(glam::DVec3::ZERO);
    let mut out = Vec::new();
    game.player_model_geometry(CAMERA, 1.0, None, &mut out);
    out
}

struct FirstPersonProbe(Arc<Mutex<Option<([f32; 3], bool)>>>);

#[mod_api::__subscriptions]
impl Hooks for FirstPersonProbe {
    fn render_first_person_body(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::RenderFirstPersonBodyEvent<'_>,
    ) {
        let Some((offset, hide_helmet)) = *self.0.lock().unwrap() else {
            return;
        };
        assert_eq!(event.player.id, 4321);
        event.body.show = true;
        event.body.offset = offset;
        event.body.hidden.hide(mod_api::HumanoidPart::Head);
        if hide_helmet {
            event.body.hide_layer(mod_api::FirstPersonBodyLayer::Helmet);
        }
    }
}

#[test]
#[ignore = "daily"]
fn first_person_listener_admits_offsets_and_filters_the_local_world_model() {
    use horizon_client::{camera::CameraType, items::ItemStack};

    // Share the fixture's answer with the registered listener, not with the render path.
    let answer = Arc::new(Mutex::new(None));
    let mods = vec![mod_api::ModEntry::new(
        "body",
        Box::new(FirstPersonProbe(answer.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
            let third_person = own_player_frame(game, false);
            assert!(!third_person.is_empty());
            let draw = |game: &Game<'_>| {
                let mut out = Vec::new();
                let body = game.first_person_body(1.0);
                game.player_model_geometry(CAMERA, 1.0, body.as_ref(), &mut out);
                out
            };
            game.camera_type = CameraType::FirstPerson;
            assert!(
                draw(game).is_empty(),
                "a listener that doesn't opt in draws nothing"
            );

            *answer.lock().unwrap() = Some(([0.0; 3], false));
            let with_helmet = draw(game);
            assert!(
                with_helmet.len() < third_person.len(),
                "the head was hidden"
            );
            *answer.lock().unwrap() = Some(([0.0; 3], true));
            let bare = draw(game);
            assert!(
                !bare.is_empty() && bare.len() < with_helmet.len(),
                "only the helmet was removed"
            );

            let delta = glam::Vec3::new(0.25, -0.125, -0.5);
            *answer.lock().unwrap() = Some((delta.to_array(), true));
            let shifted = draw(game);
            assert_eq!(shifted.len(), bare.len());
            for (base, moved) in bare.iter().zip(shifted) {
                let actual = glam::Vec3::from_array(moved.pos) - glam::Vec3::from_array(base.pos);
                assert!((actual - delta).length() < 1e-5);
            }

            game.camera_type = CameraType::ThirdPersonBack;
            let third_after = draw(game);
            assert!(
                third_person
                    .iter()
                    .map(|v| v.pos)
                    .eq(third_after.iter().map(|v| v.pos))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn the_player_pair_wraps_the_living_pair() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            [
                "player_pre true",
                "living_pre 4321",
                "living_post 4321",
                "player_post true"
            ]
        );
        heard.lock().unwrap().cancel_living.push(4321);
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            ["player_pre true", "living_pre 4321", "player_post true"],
            "a canceled living pre still asks the player post"
        );
        assert!(game.own_render_canceled());
        heard.lock().unwrap().cancel_player.push(true);
        own_player_frame(game, false);
        assert_eq!(
            events(&heard),
            ["player_pre true"],
            "a canceled player pre skips the rest"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_spectator_draws_no_layers_but_gets_its_post_hooks() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    with_pair(&heard, |game| {
        // This window's player, spectating, in third person.
        own_player_frame(game, true);
        assert_eq!(
            events(&heard),
            [
                "player_pre true",
                "living_pre 4321",
                "living_post 4321",
                "player_post true"
            ]
        );
        // A spectating mannequin in the level's draw.
        let mut mannequin = mob(game, "minecraft:mannequin", 94, 0.0);
        mannequin.extra.spectator = true;
        game.mobs.push(mannequin);
        mob_frame(game);
        assert_eq!(events(&heard), ["living_pre 94", "living_post 94"]);
    });
}

/// A mod subscribed only to `render_living_pre` (no post hooks, no player hooks).
struct LivingPreOnly(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for LivingPreOnly {
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.events.push(format!("living_pre {}", event.entity.id));
        event.cancelled = heard.cancel_living.contains(&event.entity.id);
        mod_api::Flow::Continue
    }
}

/// A mod subscribed only to `render_living_pre` and `render_player_post` (no `render_living_post`,
/// no `render_player_pre`).
struct LivingPreAndPlayerPost(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for LivingPreAndPlayerPost {
    fn render_living_pre(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderLivingPreEvent<'_>,
    ) -> mod_api::Flow {
        let mut heard = self.0.lock().unwrap();
        heard.events.push(format!("living_pre {}", event.entity.id));
        event.cancelled = heard.cancel_living.contains(&event.entity.id);
        mod_api::Flow::Continue
    }
    fn render_player_post(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut RenderPlayerPostEvent<'_>,
    ) {
        let mut heard = self.0.lock().unwrap();
        heard
            .events
            .push(format!("player_post {}", event.who.local));
    }
}

#[test]
#[ignore = "daily"]
fn single_pre_hook_runs_pre_only_and_skips_post_phase() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    let mods = vec![mod_api::ModEntry::new(
        "living_pre_only",
        Box::new(LivingPreOnly(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.mobs.clear();
            game.mobs.push(mob(game, "minecraft:zombie", 91, 0.0));
            mob_frame(game);
            let hooks = game.mob_render_listened();
            assert!(hooks.any_pre());
            assert!(!hooks.any_post());
            assert!(!hooks.any_post_for(false));
            assert!(!hooks.any_post_for(true));
            assert_eq!(events(&heard), ["living_pre 91"]);

            // Entering `mod_render_post` directly with no post hooks active is a no-op.
            let mut out = Vec::new();
            game.mod_render_post(&game.mobs[0], None, None, 1.0, CAMERA, 0, &mut out);
            assert!(out.is_empty());
            assert!(events(&heard).is_empty());

            // Player pass also runs only `living_pre` and skips player/living post hooks.
            own_player_frame(game, false);
            assert_eq!(events(&heard), ["living_pre 4321"]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn living_pre_and_player_post_gating_matches_per_hook_mask() {
    let heard: Arc<Mutex<Heard>> = Arc::default();
    let mods = vec![mod_api::ModEntry::new(
        "living_pre_and_player_post",
        Box::new(LivingPreAndPlayerPost(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.mobs.clear();
            game.mobs.push(mob(game, "minecraft:zombie", 91, 0.0));
            mob_frame(game);
            let hooks = game.mob_render_listened();
            assert!(hooks.any_pre());
            assert!(hooks.any_post());
            assert!(!hooks.any_post_for(false));
            assert!(hooks.any_post_for(true));
            assert_eq!(
                events(&heard),
                ["living_pre 91"],
                "non-player mob does not fire player_post or unsubscribed living_post"
            );

            // Uncanceled player fires living_pre then player_post.
            own_player_frame(game, false);
            assert_eq!(events(&heard), ["living_pre 4321", "player_post true"]);

            // Canceled living_pre on a player still triggers player_post.
            heard.lock().unwrap().cancel_living.push(4321);
            own_player_frame(game, false);
            assert_eq!(events(&heard), ["living_pre 4321", "player_post true"]);
            assert!(game.own_render_canceled());
        });
    });
}
