//! `living_damage_pre` and `living_damage_post` heard by a trait mod, through the game's own
//! hooks: the damage container each hit fills, the hit's level host, and the mob and dragon
//! hits that fire them.
use super::dragon::{Dragon, SPECIAL};
use super::entity::damage_level_of;
use super::native_entity_refs::tests::{bind_local_player, with_game};
use horizon_client::command::args::GameMode;
use horizon_client::mobs::{Damage, DamageKind, Mob};
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, random::RandomSource};
use mod_api::{
    self, Hooks, HurtSource, LevelRandomHost, LivingDamagePostEvent, LivingDamagePreEvent,
    LivingIncomingDamageEvent, LivingShieldBlockEvent, Reduction,
};
use std::sync::{Arc, Mutex};

/// What the probe heard: Pre's entity, original damage, new damage and armor's reduction, and
/// Post's health damage, then its entity, health damage, absorption reduction and blocked damage.
#[derive(Debug, Default, PartialEq)]
struct Heard {
    pre: Vec<(i32, f32, f32, f32)>,
    post: Vec<(i32, f32)>,
    post_values: Vec<(i32, f32, f32, f32)>,
    sources: Vec<HitSource>,
    side_effects: Vec<(i32, bool)>,
}

#[derive(Debug, PartialEq)]
struct HitSource {
    phase: &'static str,
    entity: i32,
    causing: i32,
    direct: i32,
}

impl Heard {
    fn source(&mut self, phase: &'static str, entity: i32, source: &HurtSource<'_>) {
        self.sources.push(HitSource {
            phase,
            entity,
            causing: source.causing,
            direct: source.direct,
        });
    }
}

/// The test mod: 225001's incoming hit becomes 4, Pre sets 225003's and 225008's damage to 2,
/// and draws once from the level's random for 225010.
struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .lock()
            .unwrap()
            .source("incoming", event.entity, event.source);
        if event.entity == 225001 {
            event.container.set_new_damage(4.0);
        }
        mod_api::Flow::Continue
    }
    fn living_shield_block(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingShieldBlockEvent<'_>,
    ) {
        if event.entity == 227002 {
            event.block.blocked = true;
            event.block.blocked_damage = event.damage;
        }
    }
    fn living_damage_pre(
        &self,
        level: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePreEvent<'_>,
    ) {
        let container = &mut *event.container;
        if matches!(event.entity, 225003 | 225008) {
            container.set_new_damage(2.0);
        }
        if event.entity == 225010 {
            level.with_random(|random| random.next_int()).unwrap();
        }
        let heard = (
            event.entity,
            container.original(),
            container.new_damage(),
            container.reduction(Reduction::Armor),
        );
        let mut log = self.0.lock().unwrap();
        log.pre.push(heard);
        log.source("pre", event.entity, event.source);
    }
    fn living_damage_post(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePostEvent<'_>,
    ) {
        let heard = (event.entity, event.container.health_damage());
        let mut log = self.0.lock().unwrap();
        log.post.push(heard);
        log.post_values.push((
            event.entity,
            event.container.health_damage(),
            event.container.reduction(Reduction::Absorption),
            event.container.blocked(),
        ));
        log.source("post", event.entity, event.source);
        log.side_effects
            .push((event.entity, event.container.should_cause_side_effects()));
    }
}

/// Runs `test` in a game with the probe installed, and returns what it heard.
fn heard(test: impl FnOnce(&mut super::Game<'_>)) -> Heard {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(test));
    std::mem::take(&mut *log.lock().unwrap())
}

fn mob(game: &super::Game<'_>, id: u32, kind: &str) -> Mob {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

#[test]
#[ignore = "daily"]
fn packaged_damage_type_reaches_the_api_and_damage_command() {
    use horizon_mod_package::Package;

    let scratch = tempfile::tempdir().unwrap();
    let package = scratch.path().join("damage_fixture.hmod");
    let manifest_text = "
id = 'damage_fixture'
name = 'Damage fixture'
version = '1'
api = '0.2'
capabilities = ['registries']
";
    let files: &[(&str, &[u8])] = &[
        (
            "data/damage_fixture/damage_type/shock.json",
            br#"{"message_id":"damage_fixture.shock","scaling":"never","exhaustion":0.1}"#,
        ),
        (
            "data/minecraft/tags/damage_type/bypasses_invulnerability.json",
            br#"{"values":["damage_fixture:shock"]}"#,
        ),
    ];
    std::fs::write(
        &package,
        Package::write_files(manifest_text, files).unwrap(),
    )
    .unwrap();
    let manifest = Package::read(&package).unwrap().manifest;
    horizon_mod_package::Content::read(&package, &manifest).unwrap();

    with_game(|game| {
        // Publish the admitted archive's data as the content loader does.
        game.pack.data_changes =
            horizon_assets::resource_source::ResourceSource::archive(&package).unwrap();
        let mut request = super::world_loading::WorldRequest::new(
            "registered damage",
            Some("registered damage"),
            Some(1),
            false,
            None,
            Arc::new(horizon_client::gamerules::RuleRegistry::vanilla()),
        )
        .unwrap();
        request.save_root = scratch.path().join("world");
        game.load_world(request).unwrap();
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        bind_local_player(game, 20, 123);
        game.set_allow_commands(true);

        let shock = "damage_fixture:shock";
        assert!(
            game.health_resources.damage.id(shock).is_some(),
            "the world loaded the packaged type"
        );

        game.mobs.push(mob(game, 225015, "minecraft:pig"));
        let victim = game.entity(225015).unwrap();
        assert!(game.entity_set_invulnerable(victim, true));
        assert_eq!(
            victim.hurt(game, "minecraft:generic", 1.0, None, None),
            Some(false)
        );
        assert!(!game.run_command("damage @s 1 damage_fixture:missing"));

        game.spawn_item(
            horizon_client::items::ItemStack::new("minecraft:stone", 1),
            [0.0; 3],
            [0.0; 3],
            0,
        );
        let item_id = game.just_spawned_item().unwrap().id;
        let item = game.entity(item_id).unwrap();
        let native_hit = victim.hurt(game, shock, 1.0, None, None);
        let command_hit = game.run_command("damage @s 1 damage_fixture:shock");
        let item_hit = item.hurt(game, shock, 1.0, None, None);
        assert_eq!(
            (native_hit, command_hit, item_hit),
            (Some(true), true, Some(true))
        );
        assert_eq!(game.mobs[0].health, 9.0, "the custom tag bypassed the flag");
        assert_eq!(game.dropped[0].health, 4);
        assert_eq!(victim.invulnerable_to_damage(game, shock), Some(false));

        let player = game.player.as_mut().unwrap();
        assert_eq!(player.health.health(), 19.0);
        assert_eq!(player.health.food.exhaustion, 0.1);
        let events = std::mem::take(&mut player.events);
        game.record_combat(&events);
        let name = horizon_client::text::Text::literal("Player");
        let death = game.combat_death_message_for(name).component();
        assert_eq!(death["translate"], "death.attack.damage_fixture.shock");
    });
}

#[test]
#[ignore = "daily"]
fn the_original_damage_is_the_hit_before_the_incoming_hook() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225001, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game),
            10.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 6.0);
    });
    assert_eq!(heard.pre, [(225001, 10.0, 4.0, 0.0)]);
    assert_eq!(heard.post, [(225001, 4.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_sets_the_health_lost_and_the_last_hurt_the_cooldown_compares() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225003, "minecraft:pig");
        assert!(pig.damage(damage_level_of!(game), 6.0, Damage::of(DamageKind::Generic)));
        assert_eq!(pig.health, 8.0);
        // The inflicted damage reaches `lastHurt`.
        assert_eq!(pig.last_hurt, 2.0);
    });
    assert_eq!(heard.pre, [(225003, 6.0, 2.0, 0.0)]);
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_fires_for_a_hit_its_reductions_took_to_zero() {
    let heard = heard(|game| {
        let mut witch = mob(game, 225008, "minecraft:witch");
        // Witch.getDamageAfterMagicAbsorb: none from its own doing.
        let own = Damage::mob(225008, [2.0, 0.0, 2.0]);
        assert!(witch.damage(damage_level_of!(game), 6.0, own));
        assert_eq!(witch.health, 24.0);
    });
    assert_eq!(heard.pre, [(225008, 6.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_draws_from_the_level_random_of_the_hit() {
    heard(|game| {
        let mut expected = game.world.random.clone();
        expected.next_int();
        let mut pig = mob(game, 225010, "minecraft:pig");
        assert!(pig.damage(damage_level_of!(game), 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!(game.world.random.next_int(), expected.next_int());
    });
}

#[test]
#[ignore = "daily"]
fn a_hit_on_the_dragon_fires_pre_and_post() {
    let heard = heard(|game| {
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225009;
        game.dragon.replace(dragon);
        let _ = game.hit_special(SPECIAL, 6.0, true, DamageKind::Generic, None);
        assert_eq!(game.dragon.as_ref().unwrap().health, 194.0);
    });
    assert_eq!(heard.pre, [(225009, 6.0, 6.0, 0.0)]);
    assert_eq!(heard.post, [(225009, 6.0)]);
}

#[test]
#[ignore = "daily"]
fn mob_and_dragon_post_records_fresh_hit_decision() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225014, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(damage_level_of!(game), 2.0, source));
        assert!(pig.damage(damage_level_of!(game), 4.0, source));
        assert!(!pig.damage(damage_level_of!(game), 3.0, source));
        assert_eq!(pig.health, 6.0);
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225015;
        game.dragon.replace(dragon);
        for amount in [2.0, 4.0, 3.0] {
            let _ = game.hit_special(SPECIAL, amount, true, DamageKind::Generic, None);
        }
        assert_eq!(game.dragon.as_ref().unwrap().health, 196.0);
    });
    assert_eq!(
        heard.post,
        [(225014, 2.0), (225014, 2.0), (225015, 2.0), (225015, 2.0)]
    );
    assert_eq!(
        heard.side_effects,
        [
            (225014, true),
            (225014, false),
            (225015, true),
            (225015, false)
        ]
    );
}

fn survival_player(game: &mut super::Game<'_>, id: u32) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    bind_local_player(game, id, u128::from(id));
    game.set_game_mode(GameMode::Survival);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_that_absorption_took_whole() {
    let heard = heard(|game| {
        survival_player(game, 227001);
        // Absorption is cut to `max_absorption`, 0 without a modifier.
        let player = game.player.as_mut().unwrap();
        player
            .set_modifier(
                "minecraft:max_absorption",
                "test:absorption",
                10.0,
                "add_value",
                true,
            )
            .unwrap();
        player.set_health(2.0, true).unwrap();
        assert!(game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false));
        let health = &game.player.as_ref().unwrap().health;
        assert_eq!((health.health(), health.absorption()), (20.0, 0.0));
    });
    assert_eq!(heard.post_values, [(227001, 0.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_a_shield_blocked_whole() {
    let heard = heard(|game| {
        survival_player(game, 227002);
        game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false);
        assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
    });
    assert_eq!(heard.post_values, [(227002, 0.0, 0.0, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn a_hit_inside_the_cooldown_no_stronger_than_the_last_fires_no_post() {
    let heard = heard(|game| {
        // Pre sets 225003's damage to 2, which `lastHurt` keeps.
        let mut pig = mob(game, 225003, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(damage_level_of!(game), 4.0, source));
        // Inside the cooldown and no stronger than the last hit: refused, no Post.
        assert!(!pig.damage(damage_level_of!(game), 2.0, source));
    });
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_once_for_a_killing_blow_on_a_mob() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225016, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game),
            100.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 0.0);
    });
    // `living_death` is settled by the game's next tick, after the hurt and its Post.
    assert_eq!(heard.post, [(225016, 100.0)]);
}

/// What a killing blow tells a mod, in order: `("post", entity)` and `("death", entity)`.
#[derive(Default)]
struct DeathOrder {
    heard: Mutex<Vec<(&'static str, i64)>>,
    cancel: std::sync::atomic::AtomicBool,
}

struct DeathProbe(Arc<DeathOrder>);

#[mod_api::__subscriptions]
impl Hooks for DeathProbe {
    fn living_damage_post(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePostEvent<'_>,
    ) {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("post", entity));
    }
    fn living_death(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("death", entity));
        if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// A pig killed by a 100-damage hit through the game, then the creature tick that settles its
/// death and one more: what the probe heard, and the pig's health after.
fn killing_blow(cancel: bool) -> (Vec<(&'static str, i64)>, f32) {
    let probe = Arc::new(DeathOrder::default());
    probe
        .cancel
        .store(cancel, std::sync::atomic::Ordering::Relaxed);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(probe.clone())),
    )]
    .into();
    let mut health = f32::NAN;
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let pig = mob(game, 225017, "minecraft:pig");
            game.mobs.push(pig);
            let victim = game.entity(225017).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:magic", 100.0, None, None),
                Some(true)
            );
            game.tick_creature_entities().unwrap();
            game.tick_creature_entities().unwrap();
            health = game.mobs.iter().find(|m| m.id == 225017).unwrap().health;
        });
    });
    let heard = std::mem::take(&mut *probe.heard.lock().unwrap());
    (heard, health)
}

/// Horizon's order, not NeoForge's (whose Post follows the death processing): a killing blow's
/// Post fires once, then the death hooks, as the maintainers ruled on arismoko/horizon#3963.
#[test]
#[ignore = "daily"]
fn a_killing_blows_post_fires_once_before_the_death_hooks() {
    assert_eq!(
        killing_blow(false),
        (vec![("post", 225017), ("death", 225017)], 0.0)
    );
}

/// A death a mod cancels: the pig remains cancelled at 0 health with `death_canceled`,
/// without a second Post or death on the next tick.
#[test]
#[ignore = "daily"]
fn a_cancelled_deaths_post_fires_once_before_the_death_hooks() {
    assert_eq!(
        killing_blow(true),
        (vec![("post", 225017), ("death", 225017)], 0.0)
    );
}

#[test]
#[ignore = "daily"]
fn the_container_names_what_armor_took() {
    let heard = heard(|game| {
        let mut zombie = mob(game, 225011, "minecraft:zombie");
        // A mob's attack: armor takes its share, where a generic hit bypasses it.
        let attack = Damage::mob(225012, [2.0, 0.0, 2.0]);
        assert!(zombie.damage(damage_level_of!(game), 10.0, attack));
    });
    // A zombie's two armor points against 10 damage: `CombatRules.getDamageAfterAbsorb`
    // clamps 2 - 10 / 2 up to a fifth of the armor, 0.4, and keeps 1 - 0.4 / 25 of the hit.
    let after = 10.0 * (1.0 - 0.4_f32 / 25.0);
    assert_eq!(heard.pre, [(225011, 10.0, after, 10.0 - after)]);
}

/// A level built the normal way lends the game's hooks to its hits: a path that built it
/// without them would run no mod's damage hook.
#[test]
#[ignore = "daily"]
fn a_game_built_level_has_every_damage_hook() {
    with_game(|game| {
        let hooks = &game.damage_hooks;
        assert!(hooks.incoming.is_some() && hooks.pre.is_some() && hooks.post.is_some());
        assert!(hooks.knockback.is_some() && hooks.totem.is_some() && hooks.shield.is_some());
    });
}

/// The real projectile hit routes (not hand-built HurtSources) preserve the direct
/// entity through incoming, pre and post, independently of whether there is a shooter.
#[test]
#[ignore = "daily"]
fn arrow_hits_name_the_projectile_on_players_mobs_and_the_dragon() {
    use horizon_client::command::args::GameMode;
    use horizon_client::health::NO_ATTACK;
    use horizon_client::mobs::Who;
    use horizon_client::motion::PlayerMotion;
    use horizon_client::projectiles::{Hit, Projectile, ProjectileKind};

    for (target, owner, entity, causing) in [
        (Who::Player, Some(Who::Mob(42)), 7, 42),
        (Who::Player, None, 7, 0),
        (Who::Mob(43), Some(Who::Player), 43, 7),
        (Who::Mob(43), None, 43, 0),
        (Who::Mob(SPECIAL), Some(Who::Player), 44, 7),
    ] {
        let heard = heard(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.set_game_mode(GameMode::Survival);
            game.mobs.push(mob(game, 42, "minecraft:skeleton"));
            game.mobs.push(mob(game, 43, "minecraft:pig"));
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            // Hover is a sitting phase and correctly shrugs off arrows.
            dragon.set_native_phase(mod_api::DragonPhase::HoldingPattern);
            dragon.id = 44;
            game.dragon.replace(dragon);
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.0, 1.0, 0.5),
                owner,
            );
            arrow.id = 912;
            arrow.velocity = Vec3::new(0.0, 0.0, -2.0);
            game.projectile_hit(
                &arrow,
                Hit::Entity {
                    who: target,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            assert_eq!(game.current_attack(), NO_ATTACK, "projectile scope leaked");
            match target {
                Who::Player => assert!(game.player.as_ref().unwrap().health.health() < 20.0),
                Who::Mob(SPECIAL) => assert!(game.dragon.as_ref().unwrap().health < 200.0),
                Who::Mob(_) => assert!(game.mobs[1].health < 10.0),
            }
        });
        let expected = ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity,
            causing,
            direct: 912,
        });
        assert_eq!(
            heard.sources, expected,
            "target {target:?}, owner {owner:?}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn snapshot_crosses_the_real_damage_hook_with_reductions() {
    let log = heard(|game| {
        let mut container = horizon_world::living_damage::DamageContainer::new(10.0);
        container.record_reduction(horizon_world::living_damage::Reduction::Armor, 3.5);
        container.enter_pre(6.5);
        damage_level_of!(game).damage_pre(
            horizon_client::health::Hurting::Player(
                225003,
                HurtSource {
                    damage_type: "minecraft:generic",
                    causing: 0,
                    direct: 0,
                    position: None,
                },
            ),
            &mut container,
        );
        assert_eq!(container.capture_inflicted(), 2.0);
        assert_eq!(
            container.reduction(horizon_world::living_damage::Reduction::Armor),
            3.5
        );
    });
    assert_eq!(log.pre, [(225003, 10.0, 2.0, 3.5)]);
}

#[test]
#[ignore = "daily"]
fn unsubscribed_damage_does_not_convert() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        with_game(|game| {
            super::api_values::damage::CONVERSIONS.with(|count| count.set(0));
            let mut container = horizon_world::living_damage::DamageContainer::new(10.0);
            damage_level_of!(game).damage_pre(
                horizon_client::health::Hurting::Player(
                    225003,
                    HurtSource {
                        damage_type: "minecraft:generic",
                        causing: 0,
                        direct: 0,
                        position: None,
                    },
                ),
                &mut container,
            );
            super::api_values::damage::CONVERSIONS.with(|count| assert_eq!(count.get(), 0));
        })
    });
}
