//! Exercise the server's attack entry points with real enchantment data.
use super::*;
use crate::game::*;
use horizon_client::{
    mobs::{Damage, DamageKind, Mob, MobEvent, Who, finalize as f},
    projectiles::{Hit, Projectile, ProjectileKind},
};

fn with_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut g = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
    )
    .unwrap();
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    g.player = Some(PlayerMotion::new(g.motion, Vec3::new(30.0, 64.0, 0.5)).unwrap());
    g.player.as_mut().unwrap().entity_id = 77;
    g.audio.capture = Some(Vec::new());
    test(&mut g);
}
fn thorny() -> ItemStack {
    let mut stack = ItemStack::new("minecraft:diamond_chestplate", 1);
    stack.set_crafting_enchantments(vec![("minecraft:thorns".into(), 255)]);
    stack
}
fn pair(g: &mut Game<'_>, slot: usize) -> Damage {
    g.mobs.clear();
    g.audio.capture.as_mut().unwrap().clear();
    let kind = g.res.mob_types().get("minecraft:husk").unwrap().clone();
    for (id, x) in [(41, 10.5), (42, 0.5)] {
        let mut m = Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 0.5), 0.0);
        m.health = 100.0;
        g.mobs.push(m);
    }
    g.mobs[0].random.set_seed(1);
    f::set_item_stack(&mut g.mobs[0], f::MAINHAND, thorny()); // Retaliation must not recurse.
    f::set_item_stack(&mut g.mobs[1], slot, thorny());
    Damage::mob(41, [10.5, 64.0, 0.5])
}
fn check_answer(g: &Game<'_>, slot: usize, answers: bool) {
    let health = g.mobs[0].health;
    assert_eq!(health < 100.0, answers, "attacker health: {health}");
    assert_eq!(
        f::item_stack(&g.mobs[1], slot).unwrap().damage,
        if answers { 2 } else { 0 }
    );
    assert_eq!(f::item_stack(&g.mobs[0], f::MAINHAND).unwrap().damage, 0);
    let sounds: Vec<_> = g
        .audio
        .capture
        .as_ref()
        .unwrap()
        .iter()
        .filter(|s| s.event == "minecraft:enchant.thorns.hit")
        .collect();
    assert_eq!(sounds.len(), usize::from(answers));
    if answers {
        assert!((95.0..=99.0).contains(&health));
        let source = g.mobs[0].last_damage.unwrap().0;
        assert_eq!(source.kind, DamageKind::Thorns);
        assert_eq!(source.attacker, Some(Who::Mob(42)));
        assert_eq!(sounds[0].position, g.mobs[0].center());
    }
}
#[test]
#[ignore = "requires HORIZON_TEST_RESOURCES and fresh Horizon homes"]
fn mob_thorns_melee_projectiles_and_non_attacks() {
    with_game(|g| {
        for slot in [f::MAINHAND, f::OFFHAND, f::FEET, f::LEGS, f::CHEST, f::HEAD] {
            let source = pair(g, slot);
            g.handle_mob_events(vec![MobEvent::AttackMob {
                target: 42,
                amount: 1.0,
                source,
            }])
            .unwrap();
            check_answer(g, slot, true);
            assert_eq!(g.mobs[0].health, 96.139_27); // Java Random(1), then the husk's 2 armor.
        }
        for case in 0..5 {
            let source = pair(g, f::CHEST);
            let event = match case {
                0 => MobEvent::AttackMobWithEffect {
                    target: 42,
                    amount: 1.0,
                    source,
                    effect: "minecraft:poison",
                    ticks: 20,
                    amplifier: 0,
                },
                1 => MobEvent::CubeHurtMob {
                    cube: 41,
                    target: 42,
                    amount: 1.0,
                    source,
                },
                2 => MobEvent::IronGolemHit {
                    target: Who::Mob(42),
                    amount: 1.0,
                    source,
                },
                3 => MobEvent::DamageMob {
                    target: 42,
                    amount: 1.0,
                    source,
                },
                _ => {
                    g.mobs[1].invulnerable = 20;
                    g.mobs[1].last_hurt = 10.0;
                    MobEvent::AttackMob {
                        target: 42,
                        amount: 1.0,
                        source,
                    }
                }
            };
            g.handle_mob_events(vec![event]).unwrap();
            check_answer(g, f::CHEST, case < 3);
        }
        for (kind, answers) in [
            (
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                true,
            ),
            (ProjectileKind::Trident, true),
            (ProjectileKind::SmallFireball, true),
            (ProjectileKind::LargeFireball { power: 0 }, true),
            (ProjectileKind::LlamaSpit, true),
            (
                ProjectileKind::ShulkerBullet {
                    target: None,
                    dir: None,
                    steps: 0,
                    delta: [0.0; 3],
                    seed: 1,
                },
                true,
            ),
            (ProjectileKind::WitherSkull { dangerous: false }, true),
            (ProjectileKind::WindCharge, true),
            (ProjectileKind::BreezeWindCharge, true),
            (ProjectileKind::Snowball, false),
            (ProjectileKind::Egg, false),
        ] {
            pair(g, f::CHEST);
            let mut p = Projectile::new(kind, g.mobs[1].position, Some(Who::Mob(41)));
            p.velocity = Vec3::new(-1.0, 0.0, 0.0);
            g.projectile_hit(
                &p,
                Hit::Entity {
                    who: Who::Mob(42),
                    at: p.position,
                },
            )
            .unwrap();
            check_answer(g, f::CHEST, answers);
        }
        pair(g, f::CHEST);
        g.insert_fangs(100, [0.5, 64.0, 0.5], 0.0, -7, 41);
        g.tick_fangs();
        check_answer(g, f::CHEST, true);
    });
}
#[test]
#[ignore = "requires HORIZON_TEST_RESOURCES and fresh Horizon homes"]
fn dragon_takes_player_thorns_through_its_body() {
    with_game(|g| {
        for wing in [false, true] {
            g.player = Some(PlayerMotion::new(g.motion, Vec3::new(30.0, 64.0, 0.5)).unwrap());
            g.player.as_mut().unwrap().entity_id = 77;
            g.armor[1] = Some(thorny());
            let mut d = Dragon::new(Vec3::new(0.0, 70.0, 0.0), 0.0, 1);
            d.id = SPECIAL;
            g.dragon.replace(d);
            g.audio.capture.as_mut().unwrap().clear();
            if wing {
                g.dragon_wing_knock_back(Vec3::ZERO, false);
            } else {
                g.dragon_hurts_player("minecraft:mob_attack", 10.0);
            }
            let d = g.dragon.as_ref().unwrap();
            // DamageEntity rolls [1,5); the body's cut is damage/4 + min(damage,1).
            assert_eq!(d.health, 198.019_12);
            assert_eq!(
                d.last_source,
                Some(("minecraft:thorns", Some(77), Some(77)))
            );
            assert_eq!(d.hurt_by_player, Some((FightWho::Host, 0)));
            // Player.hurtArmor adds floor(max(1, damage / 4)) before
            // Thorns' two durability: 10-point head hit = 4, 5-point wing = 3.
            assert_eq!(
                g.armor[1].as_ref().unwrap().damage,
                if wing { 3 } else { 4 }
            );
            assert_eq!(
                g.audio
                    .capture
                    .as_ref()
                    .unwrap()
                    .iter()
                    .filter(|s| s.event == "minecraft:enchant.thorns.hit")
                    .count(),
                1
            );
        }
        // Head contacts run effects through a refused player hit; breath does not.
        g.dragon.as_mut().unwrap().invulnerable = 0;
        g.player.as_mut().unwrap().health.advance_clock();
        let health = g.player.as_ref().unwrap().health.health();
        g.dragon_hurts_player("minecraft:mob_attack", 1.0);
        assert_eq!(g.player.as_ref().unwrap().health.health(), health);
        // The refused head hit adds only Thorns wear to the wing's 3;
        // the refused breath hit adds neither ordinary armor wear nor Thorns.
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 5);
        let before = g.dragon.as_ref().unwrap().health;
        g.dragon_hurts_player("minecraft:indirect_magic", 1.0);
        assert_eq!(g.dragon.as_ref().unwrap().health, before);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 5);

        // Move past knockBack's two-tick recent-attacker exclusion, but stay
        // inside the ten-tick damage cooldown so this is an actual refused hurt.
        for _ in 0..3 {
            g.player.as_mut().unwrap().health.advance_clock();
            g.world.game_time += 1;
        }
        // A refused wing still retaliates, but neither wears ordinary armor
        // nor retains its nonzero push or a new velocity-sync marker.
        let d = g.dragon.as_mut().unwrap();
        d.invulnerable = 0;
        d.last_source = None;
        let before = d.health;
        g.audio.capture.as_mut().unwrap().clear();
        let p = g.player.as_mut().unwrap();
        p.health.sync_velocity = false;
        let velocity = Vec3::new(0.25, -0.125, 0.75);
        p.velocity = velocity;
        g.dragon_wing_knock_back(Vec3::new(0.5, 0.2, -0.25), false);
        let p = g.player.as_ref().unwrap();
        assert_eq!(p.health.health(), health);
        assert_eq!(p.velocity, velocity);
        assert!(!p.health.sync_velocity);
        assert_eq!(g.armor[1].as_ref().unwrap().damage, 7);
        let d = g.dragon.as_ref().unwrap();
        assert!(d.health < before);
        assert_eq!(
            d.last_source,
            Some(("minecraft:thorns", Some(77), Some(77)))
        );
        // Attribution stores the dragon's age, not the world's game time.
        assert_eq!(d.hurt_by_player, Some((FightWho::Host, 0)));
        assert_eq!(
            g.audio
                .capture
                .as_ref()
                .unwrap()
                .iter()
                .filter(|s| s.event == "minecraft:enchant.thorns.hit")
                .count(),
            1
        );
    });
}

/// `Guardian.hurtServer`: a still guardian's spikes are 2 of
/// `minecraft:thorns` on the player whose blow it was, dealt inside that hurt
/// (before any tick runs) — not at the end of the tick, where the outbox
/// sweep has no player in place: a serving thread's players are all guests.
#[test]
#[ignore = "daily"]
fn a_still_guardian_spikes_the_player_who_hit_it() {
    with_game(|g| {
        g.mobs.clear();
        let kind = g.res.mob_types().get("minecraft:guardian").unwrap().clone();
        let mut guardian = Mob::new(41, kind, Vec3::new(10.5, 64.0, 0.5), 0.0);
        guardian.health = 30.0;
        g.mobs.push(guardian);
        g.player.as_mut().unwrap().health.advance_clock();
        let before = g.player.as_ref().unwrap().health.health();
        assert!(
            g.hurt_by_player(0, 2.0, Damage::player([10.5, 64.0, 0.5])),
            "the blow lands on the guardian"
        );
        g.player.as_mut().unwrap().health.advance_clock();
        assert_eq!(
            g.player.as_ref().unwrap().health.health(),
            before - 2.0,
            "the spikes hurt the player who hit it, in the hurt itself"
        );
        // Dealt once: the sweep at the end of the tick finds nothing left.
        assert!(
            !g.mobs[0]
                .outbox
                .iter()
                .any(|e| matches!(e, MobEvent::DamagePlayer { .. })),
            "{:?}",
            g.mobs[0].outbox
        );
    });
}
