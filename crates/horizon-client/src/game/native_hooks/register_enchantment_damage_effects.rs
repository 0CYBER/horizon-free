use super::*;
impl Game<'_> {
    /// Register damage value effects before loading enchantment content.
    pub(in crate::game) fn native_register_enchantment_damage_effects(
        context: &mod_api::RegistrationContext,
        effects: &mut dyn EnchantmentDamageEffects,
    ) {
        Self::fire_register_enchantment_damage_effects(context, effects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{mobs::Mob, motion::PlayerMotion};
    use horizon_core::geometry::Vec3;
    use mod_api::{
        EnchantmentDamageContext, EnchantmentDamageEffect, Hooks, ModEntry,
        RegisterEnchantmentDamageEffectsEvent, ServerHost,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    type Observations = Arc<Mutex<Vec<(u32, u32, String, u32, f32)>>>;
    struct Effect {
        reads: Observations,
        scale: bool,
    }
    impl EnchantmentDamageEffect for Effect {
        fn apply(
            &self,
            host: &dyn ServerHost,
            context: &EnchantmentDamageContext<'_>,
            value: f32,
        ) -> f32 {
            assert_eq!(
                host.entity(context.attacker.id()),
                Some(context.attacker.entity())
            );
            assert_eq!(host.entity(context.victim.id()), Some(context.victim));
            assert_eq!(context.weapon.count(), 1);
            assert_eq!(context.weapon.damage(), 11);
            self.reads.lock().unwrap().push((
                context.attacker.id(),
                context.victim.id(),
                mod_api::__private::item_key_id(context.weapon.item())
                    .unwrap()
                    .to_string(),
                context.level,
                value,
            ));
            if self.scale {
                value * 2.0
            } else {
                value + context.level as f32
            }
        }
    }
    struct TestMod(Observations);
    #[mod_api::__subscriptions]
    impl Hooks for TestMod {
        fn register_enchantment_damage_effects(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut RegisterEnchantmentDamageEffectsEvent<'_>,
        ) {
            for (id, scale) in [("damage_test:double", true), ("damage_test:level", false)] {
                event
                    .effects
                    .register(
                        mod_api::Identifier::parse(id).unwrap(),
                        Arc::new(Effect {
                            reads: self.0.clone(),
                            scale,
                        }),
                    )
                    .unwrap();
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn custom_enchantment_changes_real_hit_damage_and_observes_hit_context() {
        let reads: Observations = Default::default();
        with_mods(
            vec![ModEntry::new(
                "damage_test",
                Box::new(TestMod(reads.clone())),
            )]
            .into(),
            || {
                // Exercise the generated registrar dispatch, then the same publication as startup.
                super::super::super::mods::register_enchantment_damage_effects();
                crate::game::native_entity_refs::tests::with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let root = std::env::temp_dir()
                        .join(format!("horizon-damage-effects-{}", std::process::id()));
                    let pack = root.join("datapacks/effects");
                    let definitions = pack.join("data/damage_test/enchantment");
                    std::fs::create_dir_all(&definitions).unwrap();
                    std::fs::write(
                        pack.join("pack.mcmeta"),
                        r#"{"pack":{"description":"damage fixture"}}"#,
                    )
                    .unwrap();
                    std::fs::write(
                        root.join("datapacks.json"),
                        r#"{"enabled":["file/effects"],"disabled":[]}"#,
                    )
                    .unwrap();
                    let definition = serde_json::json!({
                        "description":"Damage fixture", "supported_items":"minecraft:stick", "weight":1, "max_level":2,
                        "min_cost":{"base":1,"per_level_above_first":0}, "max_cost":{"base":1,"per_level_above_first":0},
                        "anvil_cost":1,"slots":["mainhand"],
                        "effects":{"minecraft:damage":[{"effect":{"type":"minecraft:all_of","effects":[
                            {"type":"minecraft:add","value":2}, {"type":"damage_test:double"},
                            {"type":"minecraft:all_of","effects":[{"type":"damage_test:level"}]}
                        ]}}]}
                    });
                    std::fs::write(
                        definitions.join("force.json"),
                        serde_json::to_vec(&definition).unwrap(),
                    )
                    .unwrap();
                    std::fs::write(definitions.join("unknown.json"), serde_json::to_vec(&serde_json::json!({"effects":{"minecraft:damage":[{"effect":{"type":"absent:damage"}}]}})).unwrap()).unwrap();
                    game.world_root = root.clone();
                    let stack = game.reload_datapacks(false, &[]).unwrap();
                    let resources = game.options.resources.clone();
                    game.world_tables = game
                        .stage_world_tables(&stack, false, &resources)
                        .unwrap()
                        .0;
                    assert!(
                        game.stations()
                            .enchantments
                            .get("damage_test:force")
                            .is_some()
                    );
                    assert!(
                        game.stations()
                            .enchantments
                            .get("damage_test:unknown")
                            .is_none()
                    );
                    let mut player =
                        PlayerMotion::new(game.motion, Vec3::new(0.0, 64.0, 0.0)).unwrap();
                    player.entity_id = 7;
                    player.ground.on_ground = true;
                    game.player = Some(player);
                    crate::game::native_entity_refs::tests::bind_local_player(game, 7, 1);
                    game.lan_acting_uuid = Some(1);
                    game.lan_acting_entity = Some(7);
                    game.lan_acting_name = Some("attacker".into());
                    let golem = game
                        .res
                        .mob_types()
                        .get("minecraft:iron_golem")
                        .unwrap()
                        .clone();
                    for (levels, expected) in [
                        (vec![], 1.0),
                        (vec![("minecraft:sharpness", 3)], 3.0),
                        (vec![("damage_test:force", 2)], 8.0),
                        (
                            vec![("minecraft:sharpness", 3), ("damage_test:force", 2)],
                            // Effective component maps put damage_test:force before sharpness:
                            // ((1 + 2) * 2 + level 2) + Sharpness III's 2 = 10.
                            10.0,
                        ),
                    ] {
                        game.mobs.clear();
                        game.mobs
                            .push(Mob::new(21, golem.clone(), Vec3::new(1.0, 64.0, 0.0), 0.0));
                        let mut weapon = ItemStack::new("minecraft:stick", 1);
                        weapon.damage = 11;
                        weapon.data_mut().enchantments = levels
                            .into_iter()
                            .map(|(id, level)| (id.into(), level))
                            .collect();
                        game.inventory[game.selected] = Some(weapon);
                        game.attack_ticks = 100;
                        let before = game.mobs[0].health;
                        game.attack_mob(0);
                        assert_eq!(before - game.mobs[0].health, expected);
                    }
                    let mut trident = horizon_client::projectiles::Projectile::new(
                        horizon_client::projectiles::ProjectileKind::Trident,
                        Vec3::ZERO,
                        Some(horizon_client::mobs::Who::Player),
                    );
                    let mut weapon = ItemStack::new("minecraft:trident", 1);
                    weapon.damage = 11;
                    weapon.data_mut().enchantments = vec![("damage_test:force".into(), 2)];
                    trident.arrow.as_mut().unwrap().pickup_item = weapon;
                    assert_eq!(
                        game.shot_damage(
                            &mut trident,
                            Some("minecraft:iron_golem"),
                            horizon_client::mobs::Who::Mob(21)
                        ),
                        22.0
                    );
                    let mut arrow = horizon_client::projectiles::Projectile::new(
                        horizon_client::projectiles::ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        },
                        Vec3::ZERO,
                        Some(horizon_client::mobs::Who::Player),
                    );
                    arrow.velocity = Vec3::new(3.0, 0.0, 0.0);
                    let mut bow = ItemStack::new("minecraft:bow", 1);
                    bow.damage = 11;
                    bow.data_mut().enchantments = vec![("damage_test:force".into(), 2)];
                    arrow.arrow.as_mut().unwrap().fired_from_weapon = Some(bow);
                    assert_eq!(
                        game.shot_damage(
                            &mut arrow,
                            Some("minecraft:iron_golem"),
                            horizon_client::mobs::Who::Mob(21)
                        ),
                        30.0
                    );
                    if let horizon_client::projectiles::ProjectileKind::Arrow { critical, .. } =
                        &mut arrow.kind
                    {
                        *critical = true;
                    }
                    arrow.random.0 = horizon_core::random::EntityRandom::own(horizon_core::random::LegacyRandom::new(42));
                    assert_eq!(
                        game.shot_damage(
                            &mut arrow,
                            Some("minecraft:iron_golem"),
                            horizon_client::mobs::Who::Mob(21)
                        ),
                        39.0
                    );
                    // java.util.Random(42): nextInt(17) = 9, then nextInt() = 234785527.
                    use horizon_core::random::RandomSource;
                    assert_eq!(arrow.random.0.next_int(), 234785527);
                    assert_eq!(
                        *reads.lock().unwrap(),
                        [
                            (7, 21, "minecraft:stick".into(), 2, 3.0),
                            (7, 21, "minecraft:stick".into(), 2, 6.0),
                            (7, 21, "minecraft:stick".into(), 2, 3.0),
                            (7, 21, "minecraft:stick".into(), 2, 6.0),
                            (7, 21, "minecraft:trident".into(), 2, 10.0),
                            (7, 21, "minecraft:trident".into(), 2, 20.0),
                            (7, 21, "minecraft:bow".into(), 2, 4.0),
                            (7, 21, "minecraft:bow".into(), 2, 8.0),
                            (7, 21, "minecraft:bow".into(), 2, 4.0),
                            (7, 21, "minecraft:bow".into(), 2, 8.0),
                        ]
                    );
                    std::fs::remove_dir_all(root).unwrap();
                });
            },
        );
    }
}
