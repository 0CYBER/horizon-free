use super::tests::{WeaponProbe, isolated};
use super::*;
use std::sync::{Arc, Mutex};

fn fixture(expected: Option<ItemStack>, run: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        game.save = Arc::new(horizon_client::save::SaveDir::new(
            scratch.path().join("save"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        game.build_registries();
        game.next_entity = 1000;
        let loot = horizon_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "review_weapon",
            Box::new(WeaponProbe {
                expected: expected.map(|stack| stack.try_into_api().unwrap()),
                swap: None,
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || run(game));
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}
fn victim(game: &Game<'_>) -> Mob {
    Mob::new(
        70,
        game.res.mob_types().get("minecraft:pig").unwrap().clone(),
        Vec3::new(2.0, 64.0, 0.0),
        0.0,
    )
}
fn arrow(weapon: Option<ItemStack>) -> Projectile {
    let mut shot = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::ZERO,
        None,
    );
    shot.arrow.as_mut().unwrap().fired_from_weapon = weapon;
    shot.id = 912;
    shot
}
fn hit(game: &mut Game<'_>, shot: &Projectile, amount: f32) -> bool {
    let mut damage = Damage::of(DamageKind::Arrow);
    damage.from = Some([shot.position.x, shot.position.y, shot.position.z]);
    game.damage_projectile_with_weapon(shot, 70, amount, damage, None)
}

#[test]
#[ignore = "daily"]
fn death_weapon_immune_crossbow_cannot_replace_accepted_crossbow() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let mut expected = ItemStack::new("minecraft:crossbow", 1);
        expected.enchant("minecraft:piercing", 1);
        fixture(Some(expected), |game| {
            let mut target = victim(game);
            target.kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            game.mobs.push(target);
            let mut first = ItemStack::new("minecraft:crossbow", 1);
            first.enchant("minecraft:piercing", 1);
            assert!(hit(game, &arrow(Some(first)), 1.0));
            let target = game.mobs.iter_mut().find(|m| m.id == 70).unwrap();
            let committed = target.last_damage.unwrap();
            target.extra.set_body(Some("minecraft:stone".into()));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:bow", 1))),
                2.0
            ));
            let mut dead = game.mobs.iter().find(|m| m.id == 70).unwrap().clone();
            assert_eq!(dead.last_damage.unwrap().1, committed.1);
            dead.health = 0.0;
            game.mob_died(&dead);
        });
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_refused_crossbow_cannot_replace_accepted_longbow() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let mut bow = ItemStack::new("minecraft:bow", 1);
        bow.enchant("minecraft:power", 2);
        bow.data_mut()
            .set_loot_name("minecraft:custom_name", &serde_json::json!("Longbow"));
        fixture(Some(bow.clone()), |game| {
            game.mobs.push(victim(game));
            let first = arrow(Some(bow));
            assert!(hit(game, &first, 100.0));
            let victim = game.mobs.iter().find(|mob| mob.id == 70).unwrap();
            assert_eq!(victim.health, 0.0);
            assert!(victim.invulnerable > 0);
            let committed = victim.last_damage;
            let second = arrow(Some(ItemStack::new("minecraft:crossbow", 1)));
            // Damage calculation must not capture; the subsequent hit is refused.
            let mut attempted = second.clone();
            game.shot_damage(&mut attempted, Some("minecraft:pig"), Who::Mob(70));
            assert!(!hit(game, &second, 1.0));
            let dead = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
            assert_eq!(dead.last_damage.unwrap().1, committed.unwrap().1);
            game.mob_died(&dead);
        });
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_dispenser_arrow_without_recorded_weapon_is_none() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        game.mobs.push(victim(game));
        // First accept a recorded weapon, then accept an unrecorded arrow after cooldown.
        assert!(hit(
            game,
            &arrow(Some(ItemStack::new("minecraft:bow", 1))),
            1.0
        ));
        game.mobs
            .iter_mut()
            .find(|mob| mob.id == 70)
            .unwrap()
            .invulnerable = 0;
        game.mobs.iter_mut().find(|mob| mob.id == 70).unwrap().age += 1;
        assert!(hit(game, &arrow(None), 100.0));
        let dead = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
        game.mob_died(&dead);
    });
}

fn player(game: &mut Game<'_>) {
    game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    crate::game::native_entity_refs::tests::bind_local_player(game, 11, 123);
}
#[test]
#[ignore = "daily"]
fn death_weapon_riptide_iii_uses_spin_trident() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let mut trident = ItemStack::new("minecraft:trident", 1);
        trident.enchant("minecraft:riptide", 3);
        fixture(Some(trident.clone()), |game| {
            player(game);
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
            let motion = game.player.as_mut().unwrap();
            motion.auto_spin_attack_ticks = 20;
            motion.auto_spin_attack_item = Some(trident);
            let mut dead = victim(game);
            dead.last_damage = Some((Damage::player([0.0; 3]), 0));
            dead.health = 0.0;
            dead.ai.last_hurt_by_player = 100;
            game.remember_mob_player_credit(dead.id);
            game.mob_died(&dead);
        });
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_harming_potion_does_not_read_thrower_sword() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        player(game);
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
        let mut target = victim(game);
        target.health = 1.0;
        let at = target.position;
        game.mobs.push(target);
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.data_mut().potion = Some("minecraft:harming".into());
        let owner = game.potion_owner(Some(Who::Player));
        game.splash(&potion, at, Some(at), None, 3, &owner, true, Vec3::ZERO);
        let dead = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
        assert_eq!(dead.health, 0.0);
        assert_eq!(dead.last_direct, Some("minecraft:splash_potion"));
        assert_eq!(
            game.mob_damage_source(&dead),
            ("minecraft:indirect_magic", 11)
        );
        game.mob_died(&dead);
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_different_killing_source_discards_projectile() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        game.mobs.push(victim(game));
        assert!(hit(
            game,
            &arrow(Some(ItemStack::new("minecraft:bow", 1))),
            1.0
        ));
        assert!(game.mobs[0].accepted_hit.is_some());
        let mut dead = game.mobs[0].clone();
        dead.last_damage = Some((Damage::of(DamageKind::Fall), i64::from(dead.age) + 1));
        dead.health = 0.0;
        game.mob_died(&dead);
        assert!(game.mobs[0].accepted_hit.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_without_subscriber_does_not_capture() {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            game.mobs.push(victim(game));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:bow", 1))),
                1.0
            ));
            assert!(game.mobs[0].accepted_hit.is_none());
        });
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_removal_clears_capture_and_vehicle_transfer_keeps_its_owner() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        for transfer in [false, true] {
            game.mobs.clear();
            game.mobs.push(victim(game));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:bow", 1))),
                1.0
            ));
            assert!(game.mobs[0].accepted_hit.is_some());
            if transfer {
                let mut state = game.take_transferred_entity_state(70);
                game.detach_entity(70);
                assert!(
                    game.mobs[0].accepted_hit.is_none(),
                    "the transfer token took ownership"
                );
                game.mobs[0].last_damage = None;
                state.restore_mob(&mut game.mobs[0]);
                assert!(
                    game.mobs[0]
                        .accepted_hit
                        .as_ref()
                        .unwrap()
                        .belongs_to(game.mobs[0].last_damage)
                );
                game.adopt_transferred_entity_state(70, state);
            } else {
                game.forget_entity(70);
            }
            assert_eq!(game.mobs[0].accepted_hit.is_some(), transfer);
        }
        let mut dead = victim(game);
        dead.health = 0.0;
        game.mob_died(&dead);
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_same_tick_stronger_arrow_replaces_weapon() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let expected = ItemStack::new("minecraft:bow", 1);
        fixture(Some(expected.clone()), |game| {
            game.mobs.push(victim(game));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:crossbow", 1))),
                1.0
            ));
            let first = game.mobs.iter().find(|m| m.id == 70).unwrap().last_damage;
            assert!(hit(game, &arrow(Some(expected.clone())), 2.0));
            let mut dead = game.mobs.iter().find(|m| m.id == 70).unwrap().clone();
            assert_eq!(
                dead.last_damage, first,
                "equal source values still commit a new hit"
            );
            assert_eq!(
                dead.accepted_hit.as_ref().unwrap().weapon,
                Some(expected.clone())
            );
            dead.health = 0.0;
            game.mob_died(&dead);
        });
    });
}
