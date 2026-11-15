use crate::game::native_entity_refs::tests::with_game;
use horizon_client::mobs::{Mob, path::Path};
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "daily"]
fn command_stops_navigation_and_dismounts() {
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut mob = Mob::new(12, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
        mob.ai.nav.path = Some(Path {
            nodes: Vec::new(),
            next: 0,
            target: [1, 64, 1],
            dist_to_target: 0.0,
            reached: true,
        });
        mob.extra.mount = Some(42);
        mob.extra.passenger = true;
        mob.velocity = Vec3::new(1.0, -2.0, 3.0);
        mob.fall_distance = 7.0;
        game.mobs.push(mob);
        let kind = game
            .res
            .mob_types()
            .get("minecraft:skeleton")
            .unwrap()
            .clone();
        let mut vehicle = Mob::new(42, kind, Vec3::new(0.0, 64.0, 0.0), 0.0);
        vehicle.extra.rider = Some(12);
        game.mobs.push(vehicle);
        game.ui.registries.permission = 4;
        assert!(game.run_command("tp @e[type=minecraft:zombie] 10 70 10"));
        let mob = &game.mobs[0];
        assert_eq!(mob.position, Vec3::new(10.5, 70.0, 10.5));
        assert!(mob.ai.nav.path.is_none());
        assert!(mob.extra.mount.is_none());
        assert!(!mob.extra.passenger);
        assert_eq!(mob.fall_distance, 7.0);
        assert_eq!(mob.velocity, Vec3::ZERO);
        assert!(mob.on_ground);
        assert!(game.mobs[1].extra.rider.is_none());
        let mob = &mut game.mobs[0];
        mob.extra.fall_flying = true;
        mob.velocity = Vec3::new(1.0, -3.0, 2.0);
        mob.on_ground = false;
        assert!(game.run_command("tp @e[type=minecraft:zombie] ~ 70 ~"));
        let mob = &game.mobs[0];
        assert_eq!(mob.velocity, Vec3::new(1.0, 0.0, 2.0));
        assert!(!mob.on_ground);
        game.mobs[0].velocity = Vec3::new(1.0, -3.0, 2.0);
        assert!(game.run_command("tp @e[type=minecraft:zombie] 12 ~ 12"));
        assert_eq!(game.mobs[0].velocity, Vec3::new(0.0, -3.0, 0.0));
        game.mobs[0].extra.fall_flying = false;
        assert!(game.run_command("tp @e[type=minecraft:zombie] ~ ~ ~"));
        assert_eq!(game.mobs[0].velocity.y, 0.0);
        assert!(game.mobs[0].on_ground);
        assert!(
            game.run_command("tp @e[type=minecraft:zombie] -30000000.0 -20000000.0 -30000000.0")
        );
        assert_eq!(
            game.mobs[0].position,
            Vec3::new(-30_000_000.0, -20_000_000.0, -30_000_000.0)
        );
        game.run_command("tp @e[type=minecraft:zombie] 30000000.0 70 0");
        assert_eq!(game.mobs[0].position.x, -30_000_000.0);
    });
}
