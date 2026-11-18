//! arismoko/horizon#3717: a mod's collision exclusions on both sides of a guest's moves. Its client walks through
//! a wall where the boxes are, and the server checks each of those moves through its own copy of
//! the same boxes: it accepts them, and refuses the move into the wall once the boxes are gone.
use super::*;
use crate::motion::{MotionWorld, PlayerMotion};
use crate::sprint::Keys;
use horizon_core::geometry::Vec3;

/// The opening: the wall's block column at x 22, z 20, from the floor up two blocks.
const OPENING: [[f64; 6]; 1] = [[22.0, 5.0, 20.0, 23.0, 7.0, 21.0]];

/// A stone floor with its top at y 5, with a wall at x 22 three blocks high or none.
fn scene(fixture: &crate::test_fixtures::Vanilla, wall: bool) -> World {
    let assets = &fixture.assets;
    let mut scene = World::test_scene(assets, &fixture.render, 2).unwrap();
    let stone = assets.registry.default_state("minecraft:stone").unwrap();
    let air = assets.registry.default_state("minecraft:air").unwrap();
    for x in 10..=30 {
        for z in 10..=30 {
            scene.set(x, 4, z, stone).unwrap();
            for y in 5..=12 {
                let block = if wall && x == 22 && y < 8 { stone } else { air };
                scene.set(x, y, z, block).unwrap();
            }
        }
    }
    scene
}

/// Where a client walking east from (20.5, 5, 20.5) is after each of 20 ticks, and whether it
/// stands on the floor.
fn walk(
    fixture: &crate::test_fixtures::Vanilla,
    scene: &mut World,
    exclusions: &[[f64; 6]],
) -> Vec<([f64; 3], bool)> {
    let resources = &fixture.motion;
    let mut client = PlayerMotion::new(resources, Vec3::new(20.5, 5.0, 20.5)).unwrap();
    client.client_side = true;
    client.yaw = -90.0;
    client.collision_exclusions.set(exclusions).unwrap();
    let forward = Keys {
        forward: true,
        ..Keys::default()
    };
    (0..20)
        .map(|_| {
            MotionWorld {
                entities: &[],
                player: &mut client,
                scene: &mut *scene,
                assets: &fixture.assets,
                collision_binding: &fixture.collision,
                resources,
                health: &resources.health,
                tag_items: None,
                block_registries: None,
                loop_context: crate::test_resources::server_loop_context(),
                damage_hooks: &crate::health::DamageHooks::NONE,
            }
            .controlled_simulation_tick(forward)
            .unwrap();
            let p = client.position;
            ([p.x, p.y, p.z], client.ground.on_ground)
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn the_server_checks_a_guests_moves_through_the_same_exclusions_as_its_client() {
    let Some(fixture) = crate::test_fixtures::vanilla() else {
        return;
    };
    let mut walled = scene(fixture, true);
    // Without boxes the client stops at the wall, its east face at x 22.
    let stopped = walk(fixture, &mut walled, &[]);
    let end = stopped.last().unwrap().0[0];
    assert!((end - 21.7).abs() < 1e-6, "stopped at {end}");
    // With them it walks on as if there were no wall.
    let through = walk(fixture, &mut walled, &OPENING);
    let open = walk(fixture, &mut scene(fixture, false), &[]);
    assert_eq!(through, open);
    assert!(through.last().unwrap().0[0] > 23.3, "past the wall");

    let (mut hub, _rx) = hub_with_a_guest();
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut walled,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [20.5, 5.0, 20.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: Some(&fixture.collision),
        border: &border,
        streaming: None,
    };
    let start = ([20.5, 5.0, 20.5], true);
    let place = |hub: &mut Hub, (at, on_ground): ([f64; 3], bool)| {
        hub.guests.update(0, |g| {
            (g.position, g.on_ground) = (at, on_ground);
        });
    };
    hub.guests
        .update(0, |g| g.collision_exclusions.set(&OPENING))
        .unwrap();
    place(&mut hub, start);
    for &step in &through {
        let (wrongly, refused, _) = hub.collide(0, step.0, &ctx);
        assert_eq!(
            (wrongly, refused),
            (false, false),
            "accepted to {:?}",
            step.0
        );
        place(&mut hub, step);
    }
    // Without the boxes, the first move into the wall touches a block the body wasn't in.
    hub.guests.update(0, |g| g.collision_exclusions.clear());
    let into = through
        .iter()
        .position(|(at, _)| at[0] + 0.3 > 22.0)
        .unwrap();
    place(&mut hub, if into == 0 { start } else { through[into - 1] });
    let (wrongly, refused, _) = hub.collide(0, through[into].0, &ctx);
    assert_eq!((wrongly, refused), (false, true));
}

#[test]
fn a_change_of_level_and_a_respawn_clear_a_guests_exclusions() {
    let (mut hub, _rx) = hub_with_a_guest();
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let mut next_entity = 2;
    let border = crate::worldborder::WorldBorder::default();
    let ctx = TickCtx {
        sea_level: 63,
        world: &mut world,
        entities: &[],
        next_entity: &mut next_entity,
        spawn: [0.5, 64.0, 0.5],
        spawn_angle: [0.0; 2],
        spawn_dimension: "minecraft:overworld",
        difficulty: 2,
        difficulty_locked: false,
        clock_rate: 1.0,
        tick_rate: 20.0,
        tick_frozen: false,
        tick_steps: Default::default(),
        collision: None,
        border: &border,
        streaming: None,
    };
    let boxes = |hub: &Hub| hub.guests[0].collision_exclusions.boxes().len();
    hub.guests
        .update(0, |g| g.collision_exclusions.set(&OPENING))
        .unwrap();
    assert_eq!(boxes(&hub), 1);
    hub.change_level(0, [100.5, 49.0, 0.5], 0.0, None, false, 0, &ctx);
    assert_eq!(boxes(&hub), 0, "a change of level");
    hub.guests
        .update(0, |g| g.collision_exclusions.set(&OPENING))
        .unwrap();
    hub.respawn(0, [0.5, 64.0, 0.5], None, &ctx);
    assert_eq!(boxes(&hub), 0, "a respawn");
}
