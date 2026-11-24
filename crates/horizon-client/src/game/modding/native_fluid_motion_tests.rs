//! A real native trait mod, driven by the prepared game's movement on each side.
use super::Game;
use super::native_entity_refs::tests::{with_client_game, with_game};
use horizon_client::{
    chunk::Chunk,
    motion::{MotionWorld, PlayerMotion},
    sprint::Keys,
};
use horizon_client_render::effects::EffectInstance;
use horizon_core::geometry::Vec3;
use horizon_world::chunk::BlockState;
use mod_api::{
    CanSwimInFluidEvent, Hooks, LivingFluidTravelEvent, LoopContext, LoopSide, ModEntry,
};
use std::sync::Arc;

struct LavaSwim {
    side: LoopSide,
    expected_fluid: &'static str,
}

fn lava(fluid: &str) -> bool {
    matches!(fluid, "minecraft:lava" | "minecraft:flowing_lava")
}

#[mod_api::__subscriptions]
impl Hooks for LavaSwim {
    fn can_swim_in_fluid(&self, context: &dyn LoopContext, event: &mut CanSwimInFluidEvent<'_>) {
        assert_eq!(context.side(), self.side);
        assert_eq!((event.entity, event.kind), (47, "minecraft:player"));
        assert_eq!(event.fluid, self.expected_fluid);
        if lava(event.fluid) {
            assert!(!event.can_swim);
            event.can_swim = event.fire_resistance;
        }
    }

    fn living_fluid_travel(
        &self,
        context: &dyn LoopContext,
        event: &mut LivingFluidTravelEvent<'_>,
    ) {
        assert_eq!(context.side(), self.side);
        assert_eq!((event.entity, event.kind), (47, "minecraft:player"));
        assert_eq!(event.fluid, self.expected_fluid);
        assert!(event.deep);
        if lava(event.fluid) {
            assert_eq!(event.acceleration, 0.02);
            assert_eq!(event.horizontal_drag, 0.5);
            assert_eq!(event.vertical_drag, 0.5);
            assert!(!event.falling_adjustment);
            assert_eq!(event.extra_gravity_factor, 0.25);
            assert!(event.climb_impulse.is_none());
            if event.fire_resistance {
                event.acceleration = 0.065;
                event.horizontal_drag = 0.7;
                event.vertical_drag = f64::from(0.8_f32);
                event.falling_adjustment = true;
                event.extra_gravity_factor = 0.0;
                *event.climb_impulse = Some(0.2);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
struct SwimRun {
    position: Vec3,
    velocity: Vec3,
    first_sprinting: bool,
    swimming: bool,
    dive_delta: f64,
    server_drag: Option<f64>,
}

fn motion(game: &mut Game<'_>, side: LoopSide, fluid: &str, resistant: bool) -> SwimRun {
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let registry = game.pack.assets.registry;
    let state = if fluid == "minecraft:flowing_lava" {
        registry
            .resolve(&BlockState {
                name: "minecraft:lava".into(),
                properties: [("level".into(), "1".into())].into(),
            })
            .unwrap()
    } else {
        registry.default_state(fluid).unwrap()
    };
    for x in 1..15 {
        for y in 60..=70 {
            for z in 1..15 {
                game.world.set(x, y, z, state).unwrap();
            }
        }
    }
    let at = Vec3::new(5.5, 65.0, 3.5);
    let mut player = PlayerMotion::new(game.motion, at).unwrap();
    player.entity_id = 47;
    player.client_side = true;
    player.invulnerable = true;
    player.set_walking_speed_base(0.1).unwrap();
    game.player = Some(player);
    game.remove_player_effects(None);
    if resistant {
        assert!(game.add_player_effect(EffectInstance::new("fire_resistance", -1, 0)));
    }
    let player = game.player.as_mut().unwrap();
    assert_eq!(player.status.fire_resistance, resistant);
    let mut world = MotionWorld {
        entities: &[],
        player,
        scene: &mut game.world,
        assets: &game.pack.assets,
        collision_binding: game.collision,
        resources: game.motion,
        health: &game.health_resources,
        tag_items: Some(game.res.items()),
        loop_context: &game.loop_context,
        damage_hooks: &game.damage_hooks,
        block_registries: Some(&game.ui.registries),
    };
    let keys = Keys {
        forward: true,
        sprint: true,
        ..Keys::default()
    };
    // Warm up with local keys to exercise shallow-water sprint admission and the
    // earlier eye sample required to enter swimming. Then replay with the sprint
    // flag the client would send (the server does not run the keyboard latch).
    world.controlled_simulation_tick(keys).unwrap();
    let first_sprinting = world.player.sprinting;
    for _ in 0..3 {
        world.controlled_simulation_tick(keys).unwrap();
    }
    let sprinting = world.player.sprinting;
    for _ in 0..28 {
        if side == LoopSide::Server {
            world.replayed_tick(keys, sprinting).unwrap();
        } else {
            world.controlled_simulation_tick(keys).unwrap();
        }
    }
    let mut run = SwimRun {
        position: world.player.position,
        velocity: world.player.velocity,
        first_sprinting,
        swimming: world.player.swimming,
        dive_delta: 0.0,
        server_drag: None,
    };
    let before_dive = world.player.clone();
    world.controlled_simulation_tick(keys).unwrap();
    let without_sneak = world.player.velocity.y;
    *world.player = before_dive.clone();
    world
        .controlled_simulation_tick(Keys {
            shift: true,
            ..keys
        })
        .unwrap();
    run.dive_delta = world.player.velocity.y - without_sneak;
    *world.player = before_dive;
    if side == LoopSide::Server && lava(fluid) {
        world.player.velocity = Vec3::new(0.0, 0.0, 0.2);
        world.server_travel().unwrap();
        assert_eq!(
            world.player.position, run.position,
            "server copy retains the client position"
        );
        run.server_drag = Some(world.player.velocity.z);
    }
    run
}

fn exercise(game: &mut Game<'_>, side: LoopSide) {
    let run = |game: &mut Game<'_>, enabled, fluid: &'static str, resistant| {
        let mods = if enabled {
            vec![ModEntry::new(
                "lava_swim",
                Box::new(LavaSwim {
                    side,
                    expected_fluid: fluid,
                }),
            )]
        } else {
            vec![]
        };
        mod_platform::testing::with_mods(mods.into(), || motion(game, side, fluid, resistant))
    };
    let vanilla_lava = run(game, false, "minecraft:lava", true);
    let no_effect = run(game, true, "minecraft:lava", false);
    assert_eq!(no_effect, vanilla_lava);
    assert!(!vanilla_lava.swimming);
    let modded_lava = run(game, true, "minecraft:lava", true);
    assert!(modded_lava.swimming);
    assert!(
        !modded_lava.first_sprinting,
        "the first eye sample is not submerged yet"
    );
    assert!(
        modded_lava.position.z > vanilla_lava.position.z + 1.0,
        "{modded_lava:?} vs {vanilla_lava:?}"
    );
    assert_eq!(vanilla_lava.dive_delta, 0.0);
    assert!(modded_lava.dive_delta < -0.02);
    if side == LoopSide::Server {
        assert_eq!(vanilla_lava.server_drag, Some(0.2 * 0.5));
        assert_eq!(modded_lava.server_drag, Some(0.2 * 0.7));
    }
    let flowing_lava = run(game, true, "minecraft:flowing_lava", true);
    assert!(flowing_lava.swimming);
    assert!(flowing_lava.position.z > vanilla_lava.position.z + 1.0);
    let vanilla_water = run(game, false, "minecraft:water", true);
    assert!(vanilla_water.swimming);
    assert_eq!(run(game, true, "minecraft:water", true), vanilla_water);
    // The listeners require an exact fluid identity; dry land must never visit them.
    assert_eq!(
        run(game, true, "minecraft:air", true),
        run(game, false, "minecraft:air", true),
    );
}

#[test]
#[ignore = "daily"]
fn native_lava_swim_policy_drives_client_replay_and_server_travel() {
    crate::entry::install_client_hooks();
    with_client_game(|game| exercise(game, LoopSide::Client));
    with_game(|game| exercise(game, LoopSide::Server));
}
