//! Ordinary effect blends through the public handle and the client's packet/countdown paths.
use super::super::{Game, execution_role::ExecutionRole, native_entity_refs::tests::with_game};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use horizon_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};
use mod_api::LivingRef;
use mod_platform::__private::part_tests::effect_key;

fn with_client(test: impl FnOnce(&mut Game<'_>, LivingRef, i32)) {
    with_game(|game| {
        game.options.execution_role = ExecutionRole::Client;
        game.options.dedicated = None;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "BlendLifecycleTest").unwrap();
        let remote = game.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        remote.profile = 42;
        let speed = remote
            .builtin
            .get("minecraft:mob_effect")
            .unwrap()
            .id("minecraft:speed")
            .unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let living = game.living(game.entity(7).unwrap()).unwrap();
        test(game, living, speed);
    });
}

fn update(game: &mut Game<'_>, effect: i32, duration: i32, flags: i8) {
    game.handle_packet(C::UpdateMobEffect(play::UpdateMobEffect {
        id: 7,
        effect,
        amplifier: 0,
        duration,
        flags,
    }))
    .unwrap();
}

fn remove(game: &mut Game<'_>, effect: i32) {
    game.handle_packet(C::RemoveMobEffect(play::RemoveMobEffect { id: 7, effect }))
        .unwrap();
}

fn assert_blend(game: &Game<'_>, living: LivingRef, expected: [f32; 6]) {
    // Vanilla Mth.lerp does not clamp partial ticks, even outside a frame.
    for (partial, expected) in [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0].into_iter().zip(expected) {
        assert_eq!(
            living.effect_blend_factor(game, effect_key("minecraft:speed"), partial),
            Some(expected),
            "partial tick {partial}"
        );
    }
    assert!(
        living
            .effect_blend_factor(game, effect_key("speed"), f32::NAN)
            .unwrap()
            .is_nan()
    );
}

#[test]
#[ignore = "daily"]
fn ordinary_effect_blends_follow_the_client_lifecycle() {
    with_client(|game, living, speed| {
        assert_eq!(
            living.effect_blend_factor(game, effect_key("speed"), f32::NAN),
            Some(0.0)
        );
        // Independent expected factors from vanilla 26.3 BlendState: its
        // constructor starts at (0, 0); tickClient counts down before blending.
        for duration in [3, -1] {
            update(game, speed, duration, 10);
            assert_blend(game, living, [0.0; 6]);
            // Repeated reads before ticking cannot start or advance a blend.
            assert_blend(game, living, [0.0; 6]);
            game.tick_client_effects();
            assert_blend(game, living, [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0]);
            game.tick_client_effects();
            assert_blend(game, living, [1.0; 6]);
            if duration == 3 {
                game.tick_client_effects();
                assert_eq!(
                    living.effect(game, effect_key("speed")).unwrap().duration,
                    0
                );
                assert_blend(game, living, [2.0, 1.0, 0.75, 0.5, 0.0, -1.0]);
                game.tick_client_effects();
                assert_blend(game, living, [0.0; 6]);
            }
            remove(game, speed);
            assert_eq!(
                living.effect_blend_factor(game, effect_key("speed"), 0.5),
                Some(0.0)
            );
        }
        // A duration of one expires before its first blend tick can rise.
        for duration in [0, 1] {
            update(game, speed, duration, 10);
            assert_blend(game, living, [0.0; 6]);
            game.tick_client_effects();
            assert_blend(game, living, [0.0; 6]);
            remove(game, speed);
        }
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_effect_first_read_does_not_restart_the_blend() {
    for duration in [100, -1] {
        for ticks in [1, 2] {
            with_client(|game, living, speed| {
                update(game, speed, duration, 10);
                for _ in 0..ticks {
                    game.tick_client_effects();
                }
                // No earlier query, even for a missing effect, in this Game.
                assert_blend(
                    game,
                    living,
                    if ticks == 1 {
                        [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0]
                    } else {
                        [1.0; 6]
                    },
                );
            });
        }
    }
}

#[test]
#[ignore = "daily"]
fn ordinary_effect_infinite_partial_ticks_follow_the_blend_direction() {
    with_client(|game, living, speed| {
        update(game, speed, 100, 10);
        // Mth.lerp: infinite partial ticks produce NaN for a flat blend,
        // but signed infinities when the two factors differ.
        for partial in [f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                living
                    .effect_blend_factor(game, effect_key("speed"), partial)
                    .unwrap()
                    .is_nan()
            );
        }
        game.tick_client_effects();
        assert_eq!(
            living.effect_blend_factor(game, effect_key("speed"), f32::INFINITY),
            Some(f32::INFINITY)
        );
        assert_eq!(
            living.effect_blend_factor(game, effect_key("speed"), f32::NEG_INFINITY),
            Some(f32::NEG_INFINITY)
        );
        game.tick_client_effects();
        for partial in [f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                living
                    .effect_blend_factor(game, effect_key("speed"), partial)
                    .unwrap()
                    .is_nan()
            );
        }
        update(game, speed, 1, 10);
        game.tick_client_effects();
        assert_eq!(
            living.effect_blend_factor(game, effect_key("speed"), f32::INFINITY),
            Some(f32::NEG_INFINITY)
        );
        assert_eq!(
            living.effect_blend_factor(game, effect_key("speed"), f32::NEG_INFINITY),
            Some(f32::INFINITY)
        );
        remove(game, speed);
        for partial in [f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                living.effect_blend_factor(game, effect_key("speed"), partial),
                Some(0.0)
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_effect_packets_preserve_or_skip_the_blend() {
    with_client(|game, living, speed| {
        // forceAddEffect copies the old blend, even when shouldBlend is false.
        update(game, speed, 100, 10);
        update(game, speed, 200, 2);
        assert_blend(game, living, [0.0; 6]);
        game.tick_client_effects();
        update(game, speed, -1, 2);
        assert_blend(game, living, [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0]);
        game.tick_client_effects();
        update(game, speed, 1, 10);
        assert_blend(game, living, [1.0; 6]);
        game.tick_client_effects();
        assert_blend(game, living, [2.0, 1.0, 0.75, 0.5, 0.0, -1.0]);
        update(game, speed, 100, 10);
        assert_blend(game, living, [2.0, 1.0, 0.75, 0.5, 0.0, -1.0]);
        game.tick_client_effects();
        assert_blend(game, living, [-1.0, 0.0, 0.25, 0.5, 1.0, 2.0]);
        remove(game, speed);
        // A genuinely new, unblended packet starts at its target immediately.
        for duration in [100, -1, 1, 0] {
            update(game, speed, duration, 2);
            assert_blend(game, living, [if duration == 0 { 0.0 } else { 1.0 }; 6]);
            if duration == 1 {
                game.tick_client_effects();
                assert_blend(game, living, [2.0, 1.0, 0.75, 0.5, 0.0, -1.0]);
            }
            remove(game, speed);
        }
        // Removing and re-adding the same id gets a fresh instance's blend.
        update(game, speed, 100, 10);
        assert_blend(game, living, [0.0; 6]);
    });
}
