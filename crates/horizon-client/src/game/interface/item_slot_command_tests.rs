//! `/item` keeps `LivingEntity.createEquipmentSlotAccess`'s predicate on the
//! chest, legs, feet, body and saddle slots of an armor stand and the dragon:
//! an empty stack, or one `getEquipmentSlotForItem` puts in that slot. A stand's
//! `canUseSlot` refuses body and saddle, so only clearing works there. A mod's
//! `set_equipment` is not asked.
use horizon_client::{entities::ArmorStand, items::ItemStack};
use horizon_core::geometry::Vec3;
use horizon_items::equipment::{BODY, CHEST, SADDLE};
use mod_api::EquipmentSlot;

use super::native_entity_refs::tests::with_game;

#[test]
#[ignore = "daily"]
fn item_replace_asks_the_slot_predicate_of_a_stand_and_the_dragon() {
    with_game(|game| {
        game.build_registries();
        game.ui.registries.permission = 4;
        game.armor_stands.push(ArmorStand::new(16, Vec3::ZERO, 0.0));
        let mut dragon = super::dragon::Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 17;
        game.dragon.replace(dragon);
        let stand = "@e[type=minecraft:armor_stand,limit=1]";
        let dragon_sel = "@e[type=minecraft:ender_dragon,limit=1]";
        let replace = |game: &mut super::Game<'_>, who: &str, slot: &str, item: &str| {
            game.run_command(&format!("item replace entity {who} {slot} with {item}"));
        };
        let stand_slot =
            |game: &super::Game<'_>, slot| game.armor_stands[0].equipment[slot].clone();
        let dragon_slot =
            |game: &super::Game<'_>, slot| game.dragon.as_ref().unwrap().equipment[slot].clone();

        // A stand refuses body and saddle items and a chestplate in the wrong slot.
        replace(game, stand, "armor.body", "minecraft:diamond_horse_armor");
        replace(game, stand, "saddle", "minecraft:saddle");
        replace(game, stand, "armor.legs", "minecraft:iron_chestplate");
        assert!(game.armor_stands[0].equipment.is_empty());
        replace(game, stand, "armor.chest", "minecraft:iron_chestplate");
        assert_eq!(
            stand_slot(game, CHEST),
            Some(ItemStack::new("minecraft:iron_chestplate", 1))
        );

        // A mod's write is permissive; the command may clear what it wrote.
        let stack = ItemStack::new("minecraft:stone", 1);
        let living = game.living(game.entity(16).unwrap()).unwrap();
        assert!(game.living_set_equipment(living, EquipmentSlot::Body, Some(stack.clone())));
        assert_eq!(stand_slot(game, BODY), Some(stack));
        replace(game, stand, "armor.body", "minecraft:air");
        assert_eq!(stand_slot(game, BODY), None);

        // The dragon takes what is made for the slot, and nothing else.
        replace(game, dragon_sel, "armor.body", "minecraft:stone");
        replace(game, dragon_sel, "armor.chest", "minecraft:iron_helmet");
        replace(game, dragon_sel, "saddle", "minecraft:stone");
        assert!(game.dragon.as_ref().unwrap().equipment.is_empty());
        replace(game, dragon_sel, "armor.chest", "minecraft:iron_chestplate");
        replace(
            game,
            dragon_sel,
            "armor.body",
            "minecraft:diamond_horse_armor",
        );
        replace(game, dragon_sel, "saddle", "minecraft:saddle");
        assert_eq!(
            dragon_slot(game, CHEST),
            Some(ItemStack::new("minecraft:iron_chestplate", 1))
        );
        assert_eq!(
            dragon_slot(game, BODY),
            Some(ItemStack::new("minecraft:diamond_horse_armor", 1))
        );
        assert_eq!(
            dragon_slot(game, SADDLE),
            Some(ItemStack::new("minecraft:saddle", 1))
        );
    });
}
