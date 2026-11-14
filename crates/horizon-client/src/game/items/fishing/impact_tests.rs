//! Tick-level bobber impact payload, cancellation, gate and removal coverage.
use super::*;
use crate::game::native_projectiles::impact_probe::{Action, point, with_probe};
use horizon_client::mobs::Mob;
use std::sync::atomic::Ordering;

fn cast(game: &mut Game<'_>) {
    let mut h = Hook::remote(71, Vec3::new(0.5, 64.5, 0.5), [0, 0, 0, 71]);
    h.velocity = Vec3::new(4.0, 0.0, 0.0);
    game.install_fishing_hook(h);
}

fn target(game: &mut Game<'_>) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:target")
        .unwrap();
    game.world.set(2, 64, 0, state).unwrap();
}

#[test]
#[ignore = "daily"]
fn block_impact_reports_geometry_and_cancel_preserves_solid_collision() {
    for (action, power) in [(Action::Allow, 1), (Action::Cancel, 0)] {
        with_probe(action, true, |game, observed| {
            target(game);
            cast(game);
            game.tick_fishing();
            let hits = observed.hits.lock().unwrap();
            assert_eq!(hits.len(), 1);
            let hit = &hits[0];
            assert_eq!(hit.projectile, 71);
            assert_eq!(hit.kind, "minecraft:fishing_bobber");
            assert_eq!(hit.owner, 7);
            assert_eq!(hit.dimension, "minecraft:overworld");
            assert_eq!(hit.entity, None);
            assert_eq!(hit.block, [2, 64, 0]);
            assert_eq!(hit.face, [-1, 0, 0]);
            point(hit.at, [2.0, 64.5, 0.5]);
            assert_eq!(game.stats.get("custom", "minecraft:target_hit"), power);
            let h = game.hook.as_ref().unwrap();
            assert_eq!(h.state, State::Flying);
            assert_eq!(h.velocity, Vec3::ZERO);
            assert!((h.position.x - 1.875).abs() < 1.0e-6);
        });
    }
}

#[test]
#[ignore = "daily"]
fn entity_impact_includes_items_and_cancel_skips_hooking() {
    for item in [false, true] {
        for action in [Action::Allow, Action::Cancel] {
            with_probe(action, true, |game, observed| {
                let (id, kind, x) = if item {
                    game.spawn_at_location(
                        Vec3::new(3.0, 64.375, 0.5),
                        0.0,
                        ItemStack::new("minecraft:stone", 1),
                    );
                    (game.dropped[0].id, "minecraft:item", 2.875)
                } else {
                    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs
                        .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
                    (72, "minecraft:pig", 2.55)
                };
                cast(game);
                game.tick_fishing();
                let hits = observed.hits.lock().unwrap();
                assert_eq!(hits.len(), 1);
                assert_eq!(hits[0].entity, Some(id));
                assert_eq!(hits[0].entity_kind, kind);
                point(hits[0].at, [x, 64.5, 0.5]);
                let h = game.hook.as_ref().unwrap();
                if matches!(action, Action::Cancel) {
                    assert_eq!(h.state, State::Flying);
                    assert!((h.position.x - 4.5).abs() < 1.0e-6);
                } else {
                    assert_eq!(h.state, State::Hooked(id));
                    assert_eq!(h.position, Vec3::new(0.5, 64.5, 0.5));
                }
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn bobbers_notify_before_entity_deflection_and_cancel_skips_it() {
    for action in [Action::Allow, Action::Cancel] {
        with_probe(action, true, |game, observed| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:breeze")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(72, kind, Vec3::new(3.0, 64.0, 0.5), 0.0));
            cast(game);
            game.tick_fishing();
            let hits = observed.hits.lock().unwrap();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].entity, Some(72));
            let h = game.hook.as_ref().unwrap();
            assert_eq!(h.state, State::Flying);
            assert_eq!(
                h.last_deflected_by,
                if matches!(action, Action::Cancel) {
                    None
                } else {
                    Some(72)
                }
            );
        });
    }
}

#[test]
#[ignore = "daily"]
fn misses_water_and_the_closed_gate_emit_no_bobber_impact() {
    for case in 0..3 {
        with_probe(Action::Allow, case != 2, |game, observed| {
            if case == 1 {
                let water = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:water")
                    .unwrap();
                game.world.set(0, 64, 0, water).unwrap();
            } else if case == 2 {
                target(game);
            }
            cast(game);
            game.tick_fishing();
            assert!(observed.hits.lock().unwrap().is_empty());
            if case == 1 {
                assert_eq!(game.hook.as_ref().unwrap().state, State::Bobbing);
            } else if case == 2 {
                assert_eq!(game.stats.get("custom", "minecraft:target_hit"), 1);
            }
        });
    }
}

#[test]
#[ignore = "daily"]
fn removing_a_bobber_in_its_impact_clears_the_line_without_rewards() {
    for action in [Action::Discard, Action::Remove] {
        with_probe(action, true, |game, observed| {
            target(game);
            cast(game);
            let handle = game.entity(71).unwrap();
            game.tick_fishing();
            game.native_removals_flush();
            assert_eq!(observed.hits.lock().unwrap().len(), 1);
            assert!(game.entity(71).is_none());
            assert_eq!(game.entity_position(handle), None);
            assert!(game.hook.is_none(), "the owner's fishing line remains");
            assert!(game.dropped.is_empty());
            assert!(game.orbs.is_empty());
            assert_eq!(observed.catches.load(Ordering::Relaxed), 0);
            assert_eq!(game.stats.get("custom", "minecraft:fish_caught"), 0);
            assert_eq!(game.stats.get("custom", "minecraft:target_hit"), 0);
        });
    }
}
