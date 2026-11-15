use crate::game::native_entity_refs::tests::{with_client_game, with_game};
use horizon_client::mobs::{Mob, nav::NavKind};
use horizon_core::geometry::Vec3;
use mod_api::{EntityRef, MobRef, WorldEpoch};

fn zombie(game: &mut crate::game::Game<'_>) -> MobRef {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    game.mobs
        .push(Mob::new(99, kind, Vec3::new(3.5, 65.0, 3.5), 0.0));
    game.mob(game.entity(99).unwrap()).unwrap()
}
fn refused(game: &mut crate::game::Game<'_>, mob: MobRef) {
    assert_eq!(mob.can_open_doors(game), None);
    assert!(!mob.set_can_open_doors(game, true));
    assert_eq!(mob.can_pass_doors(game), None);
    assert!(!mob.set_can_pass_doors(game, true));
    assert_eq!(mob.can_float(game), None);
    assert!(!mob.set_can_float(game, true));
}
#[test]
#[ignore = "daily"]
fn navigation_flags_literal_values_and_current_path_unchanged() {
    with_game(|game| {
        let mob = zombie(game);
        use horizon_client::mobs::path::{Path, PathNode, PathType};
        let path = Path {
            nodes: vec![
                PathNode {
                    x: 3,
                    y: 65,
                    z: 3,
                    kind: PathType::Walkable,
                },
                PathNode {
                    x: 10,
                    y: 65,
                    z: 3,
                    kind: PathType::Walkable,
                },
            ],
            next: 1,
            target: [10, 65, 3],
            dist_to_target: 0.0,
            reached: true,
        };
        game.mobs[0].ai.nav.path = Some(path.clone());
        assert_eq!(mob.can_open_doors(game), Some(false));
        assert!(mob.set_can_open_doors(game, true));
        assert_eq!(mob.can_open_doors(game), Some(true));
        assert_eq!(mob.can_pass_doors(game), Some(true));
        assert!(mob.set_can_pass_doors(game, false));
        assert_eq!(mob.can_pass_doors(game), Some(false));
        assert_eq!(mob.can_float(game), Some(false));
        assert!(mob.set_can_float(game, true));
        assert_eq!(mob.can_float(game), Some(true));
        assert_eq!(game.mobs[0].ai.nav.path, Some(path));
    });
}
#[test]
#[ignore = "daily"]
fn navigation_flags_refuse_stale_handle() {
    with_game(|game| {
        let mob = zombie(game);
        let mob = MobRef::new(EntityRef::new(
            WorldEpoch::new(mob.entity().epoch().get() + 1),
            99,
        ));
        refused(game, mob);
    });
}
#[test]
#[ignore = "daily"]
fn navigation_flags_refuse_held_out_of_level() {
    with_game(|game| {
        let mob = zombie(game);
        let held = game.mobs.lift();
        refused(game, mob);
        game.mobs
            .restore(held, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
    });
}
#[test]
#[ignore = "daily"]
fn navigation_flags_refuse_no_ground_or_flying_navigation() {
    with_game(|game| {
        let mob = zombie(game);
        for kind in [
            NavKind::Water { breach: false },
            NavKind::Amphibious {
                prefers_shallow: false,
            },
        ] {
            game.mobs[0].ai.nav.kind = kind;
            refused(game, mob);
        }
    });
}
#[test]
#[ignore = "daily"]
fn navigation_flags_refuse_client() {
    with_client_game(|game| {
        let mob = MobRef::new(EntityRef::new(WorldEpoch::new(1), 99));
        refused(game, mob);
    });
}

// Ask for exact reach (0) so this fixture can assert the literal goal block;
// the public coordinate move_to normally allows a path within one block.
fn path_to_exact_goal(game: &mut crate::game::Game<'_>, id: u32) -> bool {
    let mut mobs = game.mobs.lift();
    let index = mobs.iter().position(|m| m.id == id).unwrap();
    let started = {
        let world = game.mob_world_base(&[]);
        let path = mobs[index].create_path_to(&world, [10, 65, 3], 0);
        mobs[index].nav_move_to_path(&world, path, 1.0)
    };
    game.mobs
        .restore(mobs, horizon_client::join_seq::RestoreOrder::SurvivorsFirst);
    started
}

#[test]
#[ignore = "daily"]
fn navigation_flags_closed_door_path_reaches_goal_only_when_passable() {
    for passable in [true, false] {
        with_game(|game| {
            use horizon_client::{
                chunk::Chunk,
                mobs::{doors::BreakDoor, goals::GoalSelector},
            };
            use horizon_core::position::ChunkPos;
            use std::sync::Arc;
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.load_ticking_neighbourhood(0, 0);
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("difficulty hard"));
            // Keep daylight burning out of this navigation comparison.
            game.world.day_time = 18000;
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let door = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_door")
                .unwrap();
            let door = game
                .with_props(door, &[("facing", "east"), ("open", "false")])
                .unwrap();
            for x in 1..=12 {
                for z in 2..=4 {
                    game.world.set(x, 64, z, stone).unwrap();
                    if z != 3 {
                        for y in 65..=68 {
                            game.world.set(x, y, z, stone).unwrap();
                        }
                    }
                }
                game.world.set(x, 68, 3, stone).unwrap();
            }
            game.world.set(6, 65, 3, door).unwrap();
            let upper = game.with_props(door, &[("half", "upper")]).unwrap();
            game.world.set(6, 66, 3, upper).unwrap();
            // Use the normal server insertion path for the moving mob.
            let id = game
                .spawn_mob_raw("minecraft:zombie", [3.5, 65.0, 3.5], true)
                .unwrap();
            let mob = game.mob(game.entity(id).unwrap()).unwrap(); // start block [3, 65, 3]
            game.mobs[0].on_ground = true;
            game.mobs[0].persistent = true;
            game.mobs[0].ai.goals = GoalSelector::default();
            game.mobs[0].ai.targets = GoalSelector::default();
            game.mobs[0].ai.goals.add(1, BreakDoor::zombie());
            game.mobs[0].extra.can_break_doors = true;
            assert!(mob.set_can_open_doors(game, true));
            assert!(mob.set_can_pass_doors(game, passable));
            path_to_exact_goal(game, id); // goal block [10, 65, 3]
            let mut reached = false;
            for tick in 0..600 {
                // A mod driving navigation retries after vanilla's stuck timeout;
                // the zombie's door-breaking goal takes 240 ticks.
                if mob.is_done(game) == Some(true) {
                    path_to_exact_goal(game, id);
                }
                let before = game
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .map(|m| (m.position, m.health));
                game.tick().unwrap();
                let at = game
                    .mobs
                    .iter()
                    .find(|m| m.id == id)
                    .unwrap_or_else(|| panic!("zombie removed at tick {tick}, before={before:?}"))
                    .position;
                if [
                    at.x.floor() as i32,
                    at.y.floor() as i32,
                    at.z.floor() as i32,
                ] == [10, 65, 3]
                {
                    reached = true;
                    break;
                }
            }
            assert_eq!(
                reached,
                passable,
                "600 ticks, can_pass_doors={passable}, final={:?}",
                game.mobs.iter().find(|m| m.id == id).map(|m| (
                    m.position,
                    m.health,
                    m.ai.nav.is_done()
                ))
            );
        });
    }
}
