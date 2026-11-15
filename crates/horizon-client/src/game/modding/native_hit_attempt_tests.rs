//! The actual player wrappers, damage continuations, effects and explosions reach one listener.
use super::entity::damage_level_of;
use super::native_entity_refs::tests::{bind_local_player, with_game};
use horizon_client::{
    command::args::GameMode,
    effects::EffectAction,
    mobs::{Damage, DamageKind, Mob, Who},
    motion::PlayerMotion,
    projectiles::{Hit, Projectile, ProjectileKind},
};
use horizon_core::geometry::Vec3;
use mod_api::{EntityInvulnerabilityCheckEvent, Hooks, LevelSide};
use std::sync::{Arc, Mutex};

struct Probe(Arc<Mutex<Vec<bool>>>, Arc<Mutex<Option<bool>>>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn entity_invulnerability_check(
        &self,
        _: LevelSide<'_>,
        event: &mut EntityInvulnerabilityCheckEvent<'_>,
    ) {
        self.0.lock().unwrap().push(event.original_invulnerability);
        if let Some(answer) = *self.1.lock().unwrap() {
            event.invulnerable = answer;
        }
    }
}

fn fresh_player(game: &mut super::Game<'_>, mode: GameMode) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    bind_local_player(game, 7, 7);
    // A replacement player starts with survival abilities even when Game was
    // already creative; force the mode transition to apply them to the new body.
    game.set_game_mode(GameMode::Survival);
    game.set_game_mode(mode);
}

#[test]
#[ignore = "daily"]
fn hit_attempts_reach_the_listener_once_including_refusals() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let override_answer = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "attempts",
        Box::new(Probe(heard.clone(), override_answer.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let check = |expected: &[bool], case: &str| {
                assert_eq!(&*heard.lock().unwrap(), expected, "{case}");
                heard.lock().unwrap().clear();
            };
            fresh_player(game, GameMode::Survival);
            assert!(game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false));
            check(
                &[false],
                "accepted player, including equipment continuations",
            );
            assert!(!game.hurt_player_from(1.0, None, "minecraft:generic", Some(None), false));
            check(&[false], "player cooldown refusal");

            for (mode, immune) in [
                (GameMode::Creative, false),
                (GameMode::Survival, false),
                (GameMode::Survival, true),
            ] {
                for kind in [
                    ProjectileKind::SmallFireball,
                    ProjectileKind::WindCharge,
                    ProjectileKind::ShulkerBullet {
                        target: Some(Who::Player),
                        dir: None,
                        steps: 0,
                        delta: [0.0; 3],
                        seed: 0,
                    },
                ] {
                    fresh_player(game, mode);
                    game.remove_player_effects(None);
                    game.player
                        .as_mut()
                        .unwrap()
                        .health
                        .permanently_invulnerable = immune;
                    game.player.as_mut().unwrap().set_remaining_fire_ticks(37);
                    let fireball = matches!(kind, ProjectileKind::SmallFireball);
                    let shulker = matches!(kind, ProjectileKind::ShulkerBullet { .. });
                    let shot = Projectile::new(kind, Vec3::new(0.0, 1.0, 0.5), None);
                    game.projectile_hit(
                        &shot,
                        Hit::Entity {
                            who: Who::Player,
                            at: Vec3::ZERO,
                        },
                    )
                    .unwrap();
                    check(
                        &[immune],
                        "actual small-fireball or shulker-bullet player entry",
                    );
                    let creative = mode == GameMode::Creative;
                    let refused = creative || immune;
                    assert_eq!(
                        game.player.as_ref().unwrap().health.health(),
                        if refused {
                            20.0
                        } else if fireball {
                            15.0
                        } else if shulker {
                            16.0
                        } else {
                            19.0
                        }
                    );
                    assert_eq!(
                        game.player.as_ref().unwrap().fire_ticks,
                        if creative {
                            1
                        } else if fireball && !refused {
                            100
                        } else {
                            37
                        },
                        "refused fireball restores prior fire"
                    );
                    assert_eq!(
                        game.potions.effects.get("levitation").is_some(),
                        shulker && !refused,
                        "levitation requires successful shulker damage"
                    );
                }
            }
            game.remove_player_effects(None);

            fresh_player(game, GameMode::Survival);
            game.player
                .as_mut()
                .unwrap()
                .health
                .permanently_invulnerable = true;
            *override_answer.lock().unwrap() = Some(false);
            assert!(game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false));
            assert_eq!(game.player.as_ref().unwrap().health.health(), 18.0);
            check(
                &[true],
                "listener answer survives nested predicates and equipment continuations",
            );
            assert!(game.hurt_player_from(4.0, None, "minecraft:generic", Some(None), false));
            assert_eq!(game.player.as_ref().unwrap().health.health(), 16.0);
            check(
                &[true],
                "a new attempt asks again rather than reusing the previous observation",
            );
            *override_answer.lock().unwrap() = None;

            fresh_player(game, GameMode::Creative);
            assert!(!game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false));
            check(
                &[false],
                "creative wrapper: ability refusal follows the base observation",
            );
            game.hurt_player_by("minecraft:generic", 2.0);
            check(&[false], "creative environmental wrapper");
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);

            fresh_player(game, GameMode::Survival);
            game.combat.refuse_player_hit = true;
            assert!(!game.hurt_player_from(
                2.0,
                None,
                "minecraft:player_attack",
                Some(None),
                false
            ));
            check(&[false], "PvP refusal after the first predicate");
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
            game.combat.refuse_player_hit = false;

            fresh_player(game, GameMode::Survival);
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            game.hurt_player_by("minecraft:generic", 2.0);
            check(&[false], "dead player");
            fresh_player(game, GameMode::Survival);
            game.player.as_mut().unwrap().status.fire_resistance = true;
            game.hurt_player_by("minecraft:in_fire", 2.0);
            check(&[false], "fire-resistant player");
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);

            fresh_player(game, GameMode::Survival);
            let mut player = game.player.take().unwrap();
            let shield = horizon_client::health::Blocking {
                rules: horizon_world::blocking_rules::BlockingRules {
                    delay_seconds: 0.0,
                    disable_scale: 1.0,
                    reductions: vec![horizon_world::blocking_rules::DamageReduction {
                        horizontal_angle: std::f32::consts::PI,
                        types: None,
                        base: 0.0,
                        factor: 1.0,
                    }],
                    item_damage: horizon_world::blocking_rules::ItemDamageFunction {
                        threshold: 0.0,
                        base: 0.0,
                        factor: 1.0,
                    },
                },
                bypassed: false,
                pierce: false,
                angle: 0.0,
            };
            let id = game
                .motion
                .health
                .damage
                .id("minecraft:mob_attack")
                .unwrap();
            let rules = game.world.health_rules;
            let outcome = player
                .hurt_blocking(
                    damage_level_of!(game),
                    &game.motion.health,
                    rules,
                    id,
                    2.0,
                    Some(shield),
                )
                .unwrap();
            assert_eq!(outcome.blocked, 2.0);
            assert_eq!(player.health.health(), 20.0);
            check(&[false], "fully shield-blocked player");
            game.player = Some(player);

            fresh_player(game, GameMode::Creative);
            let mut player = game.player.take().unwrap();
            player.health.food.food = 0;
            player.health.food.saturation = 0.0;
            player.health.food.timer = 79;
            let rules = game.world.health_rules;
            player
                .food_tick(
                    &game.loop_context,
                    damage_level_of!(game),
                    &game.motion.health,
                    rules,
                )
                .unwrap();
            check(&[false], "creative starvation attempt");
            assert_eq!(player.health.health(), 20.0);
            game.player = Some(player);

            fresh_player(game, GameMode::Creative);
            game.apply_effect_action(EffectAction::Hurt("minecraft:wither", 1.0));
            check(&[false], "creative damaging effect");
            game.apply_effect_action(EffectAction::Poison);
            check(&[false], "creative poison above one health");
            fresh_player(game, GameMode::Survival);
            game.player
                .as_mut()
                .unwrap()
                .set_health(1.0, false)
                .unwrap();
            game.apply_effect_action(EffectAction::Poison);
            check(&[], "poison at one health generates no vanilla hurt call");

            fresh_player(game, GameMode::Creative);
            game.explode(
                None,
                [0.0, 1.0, 0.0],
                1.0,
                super::explosion::ExplosionSource::default(),
            )
            .unwrap();
            check(&[false], "creative explosion damage is attempted");
            fresh_player(game, GameMode::Survival);
            game.player
                .as_mut()
                .unwrap()
                .set_health(0.0, false)
                .unwrap();
            game.explode(
                None,
                [0.0, 1.0, 0.0],
                1.0,
                super::explosion::ExplosionSource::default(),
            )
            .unwrap();
            check(
                &[false],
                "resident dead player explosion is attempted before death refusal",
            );
            fresh_player(game, GameMode::Survival);
            game.player.as_mut().unwrap().client_loaded = false;
            game.explode(
                None,
                [0.0, 1.0, 0.0],
                1.0,
                super::explosion::ExplosionSource::default(),
            )
            .unwrap();
            check(
                &[false],
                "resident unloaded player explosion is attempted before client-loaded refusal",
            );
            assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
            fresh_player(game, GameMode::Spectator);
            game.explode(
                None,
                [0.0, 1.0, 0.0],
                1.0,
                super::explosion::ExplosionSource::default(),
            )
            .unwrap();
            check(&[], "spectators are excluded by vanilla's entity query");

            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut pig = Mob::new(42, kind, Vec3::ZERO, 0.0);
            assert!(pig.damage(damage_level_of!(game), 2.0, Damage::of(DamageKind::Generic)));
            check(&[false], "accepted mob");
            assert!(!pig.damage(damage_level_of!(game), 1.0, Damage::of(DamageKind::Generic)));
            check(&[false], "mob cooldown refusal");
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut resistant = Mob::new(46, kind, Vec3::ZERO, 0.0);
            resistant.add_effect("minecraft:fire_resistance", -1, 0);
            assert!(!resistant.damage(damage_level_of!(game), 2.0, Damage::of(DamageKind::InFire)));
            check(&[false], "fire-resistant mob");
            pig.health = 0.0;
            assert!(!pig.damage(damage_level_of!(game), 2.0, Damage::of(DamageKind::Generic)));
            check(&[false], "dead mob");

            let kind = game.res.mob_types().get("minecraft:ghast").unwrap().clone();
            let mut ghast = Mob::new(43, kind, Vec3::ZERO, 0.0);
            ghast.invulnerable_flag = true;
            assert!(!ghast.damage(damage_level_of!(game), 2.0, Damage::of(DamageKind::Generic)));
            check(&[true], "flagged ghast override");
            ghast.invulnerable_flag = false;
            let mut fireball = Damage::of(DamageKind::Fireball);
            fireball.attacker = Some(Who::Player);
            assert!(ghast.damage(damage_level_of!(game), 2.0, fireball));
            check(&[false], "reflected ghast fireball");

            let kind = game
                .res
                .mob_types()
                .get("minecraft:warden")
                .unwrap()
                .clone();
            let mut warden = Mob::new(44, kind, Vec3::ZERO, 0.0);
            warden.extra.counter = 2; // Existing warden pose encoding: digging.
            assert!(!warden.damage(damage_level_of!(game), 2.0, Damage::of(DamageKind::Generic)));
            check(&[true], "digging warden override");

            let kind = game
                .res
                .mob_types()
                .get("minecraft:wither")
                .unwrap()
                .clone();
            let mut wither = Mob::new(47, kind, Vec3::ZERO, 0.0);
            let health = wither.health;
            let mut blast = Damage::of(DamageKind::Explosion);
            blast.attacker_species = Some(horizon_client::mobs::Species::Wither);
            assert!(!wither.damage(damage_level_of!(game), 2.0, blast));
            assert_eq!(wither.health, health);
            check(&[false], "wither-friend refusal follows the predicate");

            let kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            let mut cube = Mob::new(45, kind, Vec3::ZERO, 0.0);
            cube.extra.set_body(Some("minecraft:dirt".into()));
            assert!(cube.damage(
                damage_level_of!(game),
                2.0,
                Damage::of(DamageKind::FallingBlock)
            ));
            check(
                &[],
                "sulfur-cube body immune override never enters the vanilla predicate",
            );
        })
    });
}
