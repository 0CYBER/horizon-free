use super::MobEvent;
use horizon_client::items::ItemStack;

#[test]
#[ignore = "daily"]
fn body_armor_guards_and_break_event_are_preserved() {
    use horizon_client::mobs::Mob;
    use horizon_core::equipment_slot::EquipmentSlot;
    use horizon_core::geometry::Vec3;
    with_game(|game| {
        let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
        game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
        game.audio.capture = Some(Vec::new());
        let sword = ItemStack::new("minecraft:diamond_sword", 1);
        game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(sword.clone()));
        game.hurt_mob_armor(
            21,
            4.0,
            &horizon_client::health::NO_ATTACK.player_source("minecraft:generic", 0),
        );
        assert_eq!(game.mobs[0].item_by_slot(EquipmentSlot::Body), Some(sword));
        let resistant = ItemStack::new("minecraft:netherite_chestplate", 1);
        assert!(
            !game
                .stack_can_be_hurt_by(&resistant, "minecraft:lava")
                .unwrap()
        );
        game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(resistant.clone()));
        game.hurt_mob_armor(
            21,
            4.0,
            &horizon_client::health::NO_ATTACK.player_source("minecraft:lava", 0),
        );
        assert_eq!(
            game.mobs[0].item_by_slot(EquipmentSlot::Body),
            Some(resistant)
        );
        let source = mod_api::ItemDamageSource {
            damage_type: mod_api::Identifier::parse("minecraft:lava").unwrap(),
            direct: None,
            causing: None,
            position: None,
        };
        let mut vulnerable = ItemStack::new("minecraft:netherite_chestplate", 1);
        vulnerable.data_mut().set_other(
            "!minecraft:damage_resistant",
            Some(horizon_core::nbt::Tag::compound(
                [] as [(&str, horizon_core::nbt::Tag); 0]
            )),
        );
        assert_eq!(
            game.item_resource_can_be_hurt_by(&vulnerable, &source),
            Some(true)
        );
        game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(vulnerable));
        game.hurt_mob_armor(
            21,
            4.0,
            &horizon_client::health::NO_ATTACK.player_source("minecraft:lava", 0),
        );
        assert_eq!(
            game.mobs[0]
                .item_by_slot(EquipmentSlot::Body)
                .unwrap()
                .damage,
            1
        );
        let mut resistant = ItemStack::new("minecraft:netherite_chestplate", 1);
        resistant.data_mut().set_other(
            "minecraft:damage_resistant",
            Some(horizon_core::nbt::Tag::compound([(
                "types",
                horizon_core::nbt::Tag::String("#minecraft:is_fire".into()),
            )])),
        );
        assert_eq!(
            game.item_resource_can_be_hurt_by(&resistant, &source),
            Some(false)
        );
        game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(resistant));
        game.hurt_mob_armor(
            21,
            4.0,
            &horizon_client::health::NO_ATTACK.player_source("minecraft:lava", 0),
        );
        assert_eq!(
            game.mobs[0]
                .item_by_slot(EquipmentSlot::Body)
                .unwrap()
                .damage,
            0
        );
        let mut armor = ItemStack::new("minecraft:wolf_armor", 1);
        armor.damage = game.items().get(&armor.item).unwrap().max_damage - 1;
        game.mobs[0].set_equipment_stack(EquipmentSlot::Body as usize, Some(armor));
        game.hurt_mob_armor(
            21,
            4.0,
            &horizon_client::health::NO_ATTACK.player_source("minecraft:generic", 0),
        );
        assert!(game.mobs[0].item_by_slot(EquipmentSlot::Body).is_none());
        assert_eq!(game.lan_entity_events.last().map(|event| event.1), Some(65));
    });
}

#[test]
#[ignore = "daily"]
fn sun_protection_uses_mob_draw_and_broadcasts_break() {
    use horizon_client::mobs::Mob;
    use horizon_core::equipment_slot::EquipmentSlot;
    use horizon_core::{geometry::Vec3, random::RandomSource};
    with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(Mob::new(21, kind, Vec3::ZERO, 0.0));
        let mut helmet = ItemStack::new("minecraft:diamond_helmet", 1);
        helmet.damage = game.items().get(&helmet.item).unwrap().max_damage - 1;
        game.mobs[0].set_equipment_stack(EquipmentSlot::Head as usize, Some(helmet));
        game.audio.capture = Some(Vec::new());
        while game.mobs[0].random.clone().next_int_bound(2) != Ok(1) {
            let _ = game.mobs[0].random.next_int_bound(2);
        }
        game.wear_sun_protection(21);
        assert!(game.mobs[0].item_by_slot(EquipmentSlot::Head).is_none());
        assert_eq!(game.lan_entity_events.last().map(|event| event.1), Some(49));
    });
}
use crate::game::{answered_wear, durability_damage, native_entity_refs::tests::with_game};
use mod_api::EquipmentSlot;

fn entry(slot: EquipmentSlot, new_damage: f32) -> mod_api::ArmorHurt {
    mod_api::ArmorHurt {
        slot,
        stack: mod_api::ItemStack::new(
            mod_api::__private::item_key("minecraft:diamond_helmet").unwrap(),
            1,
        )
        .unwrap(),
        damage: 2.0,
        new_damage,
    }
}

/// A mod that clears the entries, or removes one, leaves vanilla's wear (no panic).
#[test]
fn a_cleared_armor_hurt_entry_keeps_vanilla_wear() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        mod_api::__private::resolve_items(["minecraft:diamond_helmet"]).unwrap();
        assert_eq!(answered_wear(&[], EquipmentSlot::Body, 3), 3);
        let entries = [entry(EquipmentSlot::Head, 5.0)];
        assert_eq!(answered_wear(&entries, EquipmentSlot::Chest, 2), 2);
        assert_eq!(answered_wear(&entries, EquipmentSlot::Head, 2), 5);
        assert_eq!(
            answered_wear(&[entry(EquipmentSlot::Head, -1.0)], EquipmentSlot::Head, 2),
            0
        );
    });
}

/// `(int) Math.max(1.0F, damage / 4.0F)`.
#[test]
fn durability_damage_is_vanillas() {
    assert_eq!(durability_damage(0.5) as u32, 1);
    assert_eq!(durability_damage(8.0) as u32, 2);
    assert_eq!(durability_damage(11.9) as u32, 2);
}

/// Without a canceled break the effects play; with no break applied the
/// `otherwise` events do (a canceled break plays no sound and the ravager jumps).
#[test]
#[ignore = "daily"]
fn a_break_effect_follows_only_an_applied_break() {
    with_game(|game| {
        let id = game
            .spawn_mob(
                "minecraft:ravager",
                [0.5, 70.0, 0.5],
                false,
                super::SpawnSite::Plain,
            )
            .unwrap();
        let jump = |game: &mut super::Game<'_>| {
            game.mobs.iter().find(|m| m.id == id).unwrap().ai.jumping
        };
        let ifbroke = || MobEvent::IfBroke {
            mob: id,
            effects: Vec::new(),
            otherwise: vec![MobEvent::Jump { mob: id }],
        };
        // No break of this mob in the batch: the otherwise events run.
        game.mobs
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .ai
            .jumping = false;
        game.handle_mob_events(vec![ifbroke()]).unwrap();
        assert!(jump(game));
        // An applied break (no mod cancels): the effects run, the jump does not.
        game.mobs
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .ai
            .jumping = false;
        game.handle_mob_events(vec![
            MobEvent::BreakBlock {
                pos: [0, 200, 0],
                drop: false,
                by: Some(id),
                breaker: None,
            },
            ifbroke(),
        ])
        .unwrap();
        assert!(!jump(game));
    });
}

/// An enderman's pick-up is refused when the block is no longer the one decided on.
#[test]
#[ignore = "daily"]
fn an_enderman_does_not_take_a_changed_block() {
    with_game(|game| {
        let id = game
            .spawn_mob(
                "minecraft:enderman",
                [0.5, 70.0, 0.5],
                false,
                super::SpawnSite::Plain,
            )
            .unwrap();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let dirt = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dirt")
            .unwrap();
        let pos = [0, 64, 0];
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        game.world.set(0, 64, 0, dirt).unwrap();
        game.mobs
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .extra
            .carried = Some(stone);
        game.handle_mob_events(vec![MobEvent::TakeBlock {
            mob: id,
            pos,
            state: stone,
        }])
        .unwrap();
        assert_eq!(game.world.block(0, 64, 0), dirt);
        assert!(
            game.mobs
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .extra
                .carried
                .is_none()
        );
    });
}
