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
    game.player_model_geometry(CAMERA, 1.0, &mut out);
    out
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
