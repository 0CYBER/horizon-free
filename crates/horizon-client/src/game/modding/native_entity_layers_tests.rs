//! `register_entity_layers` on the client (arismoko/horizon#860 stage 2): the event tells what renderers this load
//! built and refuses a target that is no living entity type by name; a living type's layers draw
//! over it in the order they were added, a player's are its model's, a type with none draws none,
//! and a failing layer is dropped until a resource reload adds it again.
use super::Game;
use super::entity_type_rows::EntityTypeRows;
use super::native_entity_layers::LayerRegistration;
use horizon_client::{mesher::Vertex, mobs::Mob};
use mod_api::{
    BlockEntityDrawContext, EntityLayers, EntityRenderLayer, EntityRenderState, EntityRenderer,
    LayerTarget, PlayerModel, PosedParts,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

/// A load's entity types: a boat, a zombie, an armour stand, the ender dragon, the player and a mod's
/// living type.
fn rows() -> EntityTypeRows {
    EntityTypeRows::parse(
        br#"{"minecraft:oak_boat": {"width": 1.375},
             "minecraft:zombie": {"attributes": {}},
             "minecraft:armor_stand": {"attributes": {}},
             "minecraft:ender_dragon": {"attributes": {}},
             "minecraft:player": {"attributes": {}},
             "ruby:golem": {"template": "minecraft:zombie", "attributes": {}}}"#,
    )
}

/// The living types drawn as mobs, where layers run: not the armour stand or the ender dragon.
fn routed(id: &str) -> bool {
    ["minecraft:zombie", "ruby:golem"].contains(&id)
}

/// Draws one stone quad a block wide, `height` blocks above the entity's feet; refuses its first
/// `refuse` calls with an `Err`. `calls` counts the calls.
struct Raised {
    height: f32,
    refuse: u32,
    calls: AtomicU32,
    /// The entity id of the last call.
    id: AtomicU32,
}
impl Raised {
    fn new(height: f32) -> Arc<Self> {
        Self::refusing(height, 0)
    }
    fn refusing(height: f32, refuse: u32) -> Arc<Self> {
        Arc::new(Self {
            height,
            refuse,
            calls: AtomicU32::new(0),
            id: AtomicU32::new(u32::MAX),
        })
    }
    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}
impl EntityRenderLayer for Raised {
    fn submit(
        &self,
        entity: &EntityRenderState,
        _: Option<&PosedParts>,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        self.id.store(entity.id, Ordering::Relaxed);
        let (low, high) = (self.height, self.height + 1.0);
        let corners = [
            [0.0, high, 0.0],
            [0.0, low, 0.0],
            [1.0, low, 0.0],
            [1.0, high, 0.0],
        ];
        draw.quad("block/stone", corners, [0.0, 0.0, 16.0, 16.0], 0xFF_FFFF);
        if call < self.refuse {
            return Err("refused this frame".into());
        }
        Ok(())
    }
}

#[test]
fn the_event_tells_the_renderers_this_load_built() {
    let rows = rows();
    let mut renderers: BTreeMap<String, Arc<dyn EntityRenderer>> = BTreeMap::new();
    struct Nothing;
    impl EntityRenderer for Nothing {
        fn submit(
            &self,
            _: &EntityRenderState,
            _: &mut dyn BlockEntityDrawContext,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    renderers.insert("ruby:golem".into(), Arc::new(Nothing));
    renderers.insert("nope:ghost".into(), Arc::new(Nothing));
    let event = LayerRegistration::new(&rows, &renderers, &routed);
    assert_eq!(
        event.entity_types(),
        [
            "minecraft:armor_stand",
            "minecraft:ender_dragon",
            "minecraft:oak_boat",
            "minecraft:zombie",
            "ruby:golem"
        ]
    );
    assert!(
        event.is_living("minecraft:armor_stand"),
        "living, whatever its route"
    );
    assert!(event.is_living("minecraft:zombie"));
    assert!(event.is_living("ruby:golem"));
    assert!(!event.is_living("minecraft:oak_boat"));
    assert!(!event.is_living("minecraft:player"));
    assert!(!event.is_living("nope:ghost"));
    assert_eq!(
        event.player_models(),
        [PlayerModel::Wide, PlayerModel::Slim]
    );
    assert!(event.has_mod_renderer("ruby:golem"));
    assert!(!event.has_mod_renderer("minecraft:zombie"));
    assert!(!event.has_mod_renderer("nope:ghost"), "no entity type");
}

#[test]
fn a_target_that_is_no_living_entity_type_is_refused_by_name() {
    let rows = rows();
    let renderers = BTreeMap::new();
    let mut event = LayerRegistration::new(&rows, &renderers, &routed);
    let mut add = |target: &str| event.add(LayerTarget::Entity(target.into()), Raised::new(1.0));
    assert_eq!(
        add("minecraft:oak_boat"),
        Err("the entity layer for minecraft:oak_boat is refused: its renderer is not a living entity's, and only those take layers".into())
    );
    assert_eq!(
        add("nope:ghost"),
        Err("the entity layer for nope:ghost is refused: it is no entity type".into())
    );
    assert_eq!(
        add("minecraft:player"),
        Err("the entity layer for minecraft:player is refused: a player takes layers through its model (wide or slim)".into())
    );
    assert_eq!(
        add("minecraft:armor_stand"),
        Err("the entity layer for minecraft:armor_stand is refused: it draws through a route of its own, which does not run layers yet".into())
    );
    assert_eq!(
        add("minecraft:ender_dragon"),
        Err("the entity layer for minecraft:ender_dragon is refused: it draws through a route of its own, which does not run layers yet".into())
    );
    assert_eq!(add("minecraft:zombie"), Ok(()));
    assert_eq!(add("ruby:golem"), Ok(()), "a mod's living type");
    assert_eq!(
        event.add(LayerTarget::Player(PlayerModel::Slim), Raised::new(1.0)),
        Ok(())
    );
}

/// The layers the scene's mods add on every load: the first mod `zombie` 100 blocks up, `wide`
/// 300 up, `slim` 400 up and `pig` (refusing its first call) 500 up; the second mod `second`, on
/// the zombie too, 200 up.
struct Scene {
    zombie: Arc<Raised>,
    second: Arc<Raised>,
    wide: Arc<Raised>,
    slim: Arc<Raised>,
    pig: Arc<Raised>,
}

/// The camera, south of the entity and looking at it.
const CAMERA: glam::DVec3 = glam::DVec3::new(0.5, 64.0, -4.5);
/// Where the scene's entity stands.
const FEET: [f64; 3] = [0.5, 64.0, 0.5];
/// Full light, as the entity pass packs it.
const LIGHT: u32 = 0x00F0_00F0;

/// How high above the entity's feet a vertex is, in blocks.
fn above(v: &Vertex) -> f32 {
    v.pos[1] - (FEET[1] - CAMERA.y) as f32
}

/// The heights the quads `out` holds start at, in order.
fn quads(out: &[Vertex]) -> Vec<f32> {
    out.chunks(4)
        .map(|quad| quad.iter().map(above).fold(f32::MAX, f32::min).round())
        .collect()
}

/// A mob of `kind` (a player of the slim model when `slim`), standing at [`FEET`].
fn mob(game: &Game<'_>, kind: &str, slim: bool) -> Mob {
    let kind = if kind == "minecraft:player" {
        game.res.player_type().clone().unwrap()
    } else {
        game.res.mob_types().get(kind).unwrap().clone()
    };
    let at = horizon_core::geometry::Vec3::new(FEET[0], FEET[1], FEET[2]);
    let mut mob = Mob::new(90, kind, at, 0.0);
    mob.slim = slim;
    mob
}

/// The mods' layers one frame draws over a mob of `kind` (a player of the slim model when
/// `slim`).
fn layer_frame(game: &Game<'_>, kind: &str, slim: bool) -> Vec<Vertex> {
    let mut out = Vec::new();
    let mob = mob(game, kind, slim);
    let layers = game.mod_layers(&mob);
    game.mod_layer_geometry(&mob, None, None, layers, 1.0, CAMERA, LIGHT, &mut out);
    out
}

/// Runs `check` on a client game with prepared resources and the [`Scene`]'s mods, resolved as
/// the first load does. In a child process running only `test`: the registrations of a process
/// are its own.
fn in_scene(test: &str, check: impl FnOnce(&mut Game<'_>, &Scene)) {
    if std::env::var_os("HORIZON_ENTITY_LAYERS_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                &format!("game::native_entity_layers_tests::{test}"),
                "--include-ignored",
                "--nocapture",
            ])
            .env("HORIZON_ENTITY_LAYERS_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use mod_api::{Hooks, RegisterEntityLayersEvent, RegistrationContext};
    let scene = Scene {
        zombie: Raised::new(100.0),
        second: Raised::new(200.0),
        wide: Raised::new(300.0),
        slim: Raised::new(400.0),
        pig: Raised::refusing(500.0, 1),
    };
    /// A mod adding `layers` on every load.
    struct Layers(Vec<(LayerTarget, Arc<Raised>)>);
    #[mod_api::__subscriptions]
    impl Hooks for Layers {
        fn register_entity_layers(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterEntityLayersEvent<'_>,
        ) {
            for (target, layer) in &self.0 {
                event.layers.add(target.clone(), layer.clone()).unwrap();
            }
        }
    }
    let entity = |id: &str| LayerTarget::Entity(id.into());
    let first = Layers(vec![
        (entity("minecraft:zombie"), scene.zombie.clone()),
        (LayerTarget::Player(PlayerModel::Wide), scene.wide.clone()),
        (LayerTarget::Player(PlayerModel::Slim), scene.slim.clone()),
        (entity("minecraft:pig"), scene.pig.clone()),
    ]);
    let second = Layers(vec![(entity("minecraft:zombie"), scene.second.clone())]);
    let mods = vec![
        mod_api::ModEntry::new("first", Box::new(first)),
        mod_api::ModEntry::new("second", Box::new(second)),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_client_game(|game| {
            game.resolve_entity_renderers();
            check(game, &scene);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_living_type_draws_its_layers_in_the_order_they_were_added() {
    in_scene(
        "a_living_type_draws_its_layers_in_the_order_they_were_added",
        |game, scene| {
            let zombie = layer_frame(game, "minecraft:zombie", false);
            assert_eq!(
                quads(&zombie),
                [100.0, 200.0],
                "the first mod's, then the second's"
            );
            assert_eq!((scene.zombie.calls(), scene.second.calls()), (1, 1));
            assert!(
                layer_frame(game, "minecraft:cow", false).is_empty(),
                "a type with none"
            );
        },
    );
}

/// The one lookup a mob gets a frame finds its own type's or player model's layers and nothing for a
/// type without: what decides whether the draw keeps its posed parts, and what draws over it.
#[test]
#[ignore = "daily"]
fn a_mob_looks_up_only_its_own_types_or_models_layers() {
    in_scene(
        "a_mob_looks_up_only_its_own_types_or_models_layers",
        |game, _| {
            let count = |kind: &str, slim: bool| {
                game.mod_layers(&mob(game, kind, slim))
                    .map(|layers| layers.len())
            };
            assert_eq!(count("minecraft:zombie", false), Some(2));
            assert_eq!(
                count("minecraft:cow", false),
                None,
                "no parts kept, nothing drawn"
            );
            assert_eq!(
                count("minecraft:player", false),
                Some(1),
                "the wide model's"
            );
            assert_eq!(count("minecraft:player", true), Some(1), "the slim model's");
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_player_draws_the_layers_of_its_model() {
    in_scene("a_player_draws_the_layers_of_its_model", |game, scene| {
        assert_eq!(
            quads(&layer_frame(game, "minecraft:player", false)),
            [300.0]
        );
        assert_eq!(quads(&layer_frame(game, "minecraft:player", true)), [400.0]);
        assert_eq!((scene.wide.calls(), scene.slim.calls()), (1, 1));
        assert_eq!(
            scene.zombie.calls(),
            0,
            "the zombie's layers are not a player's"
        );
    });
}

#[test]
#[ignore = "daily"]
fn this_windows_player_is_told_its_own_entity_id() {
    in_scene(
        "this_windows_player_is_told_its_own_entity_id",
        |game, scene| {
            // This window's own entity id, as the server gave it.
            match game.remote.as_mut() {
                Some(remote) => remote.entity_id = 4321,
                None => {
                    let at = horizon_core::geometry::Vec3::ZERO;
                    let mut player =
                        horizon_client::motion::PlayerMotion::new(game.motion, at).unwrap();
                    player.entity_id = 4321;
                    game.player = Some(player);
                }
            }
            let own = u32::try_from(game.own_entity_id()).unwrap();
            assert_eq!(own, 4321);
            // Drawn from a model made for the frame, with id 0, as `tick_player_model` makes it.
            let mut model = mob(game, "minecraft:player", false);
            model.id = 0;
            let who = super::native_mob_draw::PlayerRender {
                name: "own",
                local: true,
            };
            let layers = game.mod_layers(&model);
            let mut out = Vec::new();
            game.mod_layer_geometry(
                &model,
                Some(who),
                None,
                layers,
                1.0,
                CAMERA,
                LIGHT,
                &mut out,
            );
            assert_eq!(scene.wide.id.load(Ordering::Relaxed), own);
            // Another player keeps its own mob's id.
            let other = mob(game, "minecraft:player", false);
            let layers = game.mod_layers(&other);
            let who = super::native_mob_draw::PlayerRender {
                name: "other",
                local: false,
            };
            game.mod_layer_geometry(
                &other,
                Some(who),
                None,
                layers,
                1.0,
                CAMERA,
                LIGHT,
                &mut out,
            );
            assert_eq!(scene.wide.id.load(Ordering::Relaxed), 90);
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_failing_layer_is_dropped_until_a_resource_reload_adds_it_again() {
    in_scene(
        "a_failing_layer_is_dropped_until_a_resource_reload_adds_it_again",
        |game, scene| {
            for frame in ["the failing frame", "the next frame"] {
                assert!(
                    layer_frame(game, "minecraft:pig", false).is_empty(),
                    "{frame}"
                );
                assert_eq!(scene.pig.calls(), 1, "{frame}: asked once, then no more");
            }
            super::model_baking_completed_tests::reload(game);
            assert_eq!(quads(&layer_frame(game, "minecraft:pig", false)), [500.0]);
            assert_eq!(scene.pig.calls(), 2, "the reload added it again");
        },
    );
}
