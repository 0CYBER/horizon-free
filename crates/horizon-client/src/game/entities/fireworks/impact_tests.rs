//! Real rocket tick coverage for impact payloads and canceled explosions.
use super::*;
use crate::game::native_projectiles::impact_probe::{Action, point, with_probe};
use horizon_client::mobs::Mob;

fn launch(game: &mut Game<'_>, lifetime: i32) {
    let mut r = Rocket::remote(71, Vec3::new(0.5, 64.5, 0.5), Vec3::new(4.0, 0.0, 0.0));
    r.angled = true;
    r.owner = Some(Who::Player);
    r.left_owner = true;
    r.lifetime = lifetime;
    r.explosions.push(FireworkExplosion::default());
    game.rockets.push(r);
}

#[test]
#[ignore = "daily"]
fn rocket_hits_report_geometry_and_cancellation_skips_the_explosion() {
    for entity in [false, true] {
        for action in [Action::Allow, Action::Cancel] {
            with_probe(action, true, |game, observed| {
                if entity {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs
                        .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
                } else {
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(2, 64, 0, stone).unwrap();
                }
                launch(game, 100);
                game.tick_rockets();
                let hits = observed.hits.lock().unwrap();
                assert_eq!(hits.len(), 1);
                let hit = &hits[0];
                assert_eq!(hit.projectile, 71);
                assert_eq!(hit.kind, "minecraft:firework_rocket");
                assert_eq!(hit.owner, 7);
                assert_eq!(hit.dimension, "minecraft:overworld");
                if entity {
                    assert_eq!(hit.entity, Some(72));
                    assert_eq!(hit.entity_kind, "minecraft:pig");
                    point(hit.at, [2.55, 64.5, 0.5]);
                } else {
                    assert_eq!(hit.entity, None);
                    assert_eq!(hit.block, [2, 64, 0]);
                    assert_eq!(hit.face, [-1, 0, 0]);
                    point(hit.at, [2.0, 64.5, 0.5]);
                }
                if matches!(action, Action::Cancel) {
                    assert_eq!(game.rockets.len(), 1);
                    assert!(game.entity(71).is_some());
                    assert_eq!(game.rockets[0].life, 1);
                    if entity {
                        assert_eq!(game.mobs[0].health, 10.0);
                    }
                } else {
                    assert!(game.rockets.is_empty());
                    if entity {
                        assert!(game.mobs[0].health < 10.0);
                    }
                }
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn rockets_skip_misses_deflections_and_a_closed_gate_but_keep_lifetime_expiry() {
    for case in 0..5 {
        let action = if case == 4 {
            Action::Allow
        } else {
            Action::Cancel
        };
        with_probe(action, case != 2, |game, observed| {
            if case == 1 {
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:breeze")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
            } else if case >= 2 {
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
            }
            launch(game, if case >= 3 { 0 } else { 100 });
            if case == 4 {
                // A plain rocket still reports its block hit, but does not
                // explode on that hit even when it is allowed.
                game.rockets[0].explosions.clear();
                game.rockets[0].lifetime = 100;
            }
            game.tick_rockets();
            assert_eq!(observed.hits.lock().unwrap().len(), usize::from(case >= 3));
            match case {
                1 => {
                    assert_eq!(game.rockets.len(), 1);
                    assert_eq!(game.rockets[0].velocity.x, -2.0);
                }
                2 | 3 => assert!(game.rockets.is_empty()),
                _ => assert_eq!(game.rockets.len(), 1),
            }
        });
    }
}
