use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{chunk::Chunk, mobs::Mob};
use horizon_core::geometry::Vec3;
use mod_api::{EntityRef, MobRef, WantedPositionError, WorldEpoch};
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn wanted_position_moves_and_navigation_replaces_the_wish() {
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 64, z, stone).unwrap();
            }
        }
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut zombie = Mob::new(12, kind, Vec3::new(3.5, 65.0, 3.5), 270.0);
        zombie.on_ground = true;
        game.mobs.push(zombie);
        let mob = game.mob(game.entity(12).unwrap()).unwrap();
        assert_eq!(
            mob.set_wanted_position(game, [10.5, 65.0, 3.5], 1.0),
            Ok(true)
        );
        assert_eq!(mob.wanted_position(game), Some([10.5, 65.0, 3.5]));
        assert_eq!(mob.speed_modifier(game), Some(1.0));
        let mut taken = game.mobs.lift();
        let w = game.mob_world_base(&[]);
        let mut test_level = horizon_client::health::TestLevel::new(0);
        let mut level = test_level.level();
        let mut events = Vec::new();
        for _ in 0..10 {
            taken[0].move_control_tick(&w, &mut events);
            let input = Vec3::new(0.0, 0.0, f64::from(taken[0].ai.zza));
            taken[0].travel(&w, &mut level, input, &mut events);
        }
        assert!(
            taken[0].position.x > 3.55,
            "position: {:?}",
            taken[0].position
        );
        assert!((10.5 - taken[0].position.x).abs() < 6.95);
        game.mobs.restore(
            taken,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        let mut taken = game.mobs.lift();
        let w = game.mob_world_base(&[]);
        let mut goal =
            horizon_client::mobs::goals::Stroll::new(1.25).position(|_, _| Some([2.5, 65.0, 8.5]));
        goal.force = true;
        let mut selector = horizon_client::mobs::goals::GoalSelector::default();
        selector.add(1, goal);
        selector.tick(
            &mut taken[0],
            &mut horizon_client::mobs::goals::Cx {
                w: &w,
                level: &mut level,
                events: &mut events,
            },
        );
        taken[0].nav_tick(&w);
        game.mobs.restore(
            taken,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        assert_ne!(mob.wanted_position(game), Some([10.5, 65.0, 3.5]));
        assert_eq!(mob.speed_modifier(game), Some(1.25));
    });
}

#[test]
#[ignore = "daily"]
fn wanted_position_names_invalid_inputs_and_refuses_unavailable_mobs() {
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(Mob::new(12, kind, Vec3::ZERO, 0.0));
        let mob = game.mob(game.entity(12).unwrap()).unwrap();
        assert_eq!(
            mob.set_wanted_position(game, [10000.0, 80.0, -10000.0], -1.0),
            Ok(true)
        );
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for axis in 0..3 {
                let mut at = [0.0; 3];
                at[axis] = bad;
                assert_eq!(
                    mob.set_wanted_position(game, at, 1.0),
                    Err(WantedPositionError::NonFinitePosition)
                );
            }
            assert_eq!(
                mob.set_wanted_position(game, [0.0; 3], bad),
                Err(WantedPositionError::NonFiniteSpeed)
            );
        }
        assert_eq!(mob.wanted_position(game), Some([10000.0, 80.0, -10000.0]));
        assert_eq!(mob.speed_modifier(game), Some(-1.0));
        let stale = MobRef::new(EntityRef::new(
            WorldEpoch::new(mob.entity().epoch().get() + 1),
            12,
        ));
        assert_eq!(stale.set_wanted_position(game, [0.0; 3], 1.0), Ok(false));
        assert_eq!(stale.wanted_position(game), None);
        game.held_out.push(12);
        assert_eq!(mob.set_wanted_position(game, [0.0; 3], 1.0), Ok(false));
        assert_eq!(mob.wanted_position(game), None);
        game.held_out.retain(|id| *id != 12);
        let taken = game.mobs.lift();
        assert_eq!(mob.set_wanted_position(game, [0.0; 3], 1.0), Ok(false));
        assert_eq!(mob.wanted_position(game), None);
        game.mobs.restore(
            taken,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        let mut dragon = crate::game::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 15;
        game.dragon.replace(dragon);
        let dragon = game.mob(game.entity(15).unwrap()).unwrap();
        assert_eq!(dragon.set_wanted_position(game, [0.0; 3], 1.0), Ok(false));
        assert_eq!(dragon.wanted_position(game), None);
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        assert_eq!(mob.set_wanted_position(game, [0.0; 3], 1.0), Ok(false));
        assert_eq!(mob.wanted_position(game), None);
        assert_eq!(mob.speed_modifier(game), None);
    });
}
