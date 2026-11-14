use super::*;
use std::sync::{Arc, Mutex};
#[derive(Clone)]
struct Accept23(Arc<Mutex<Vec<u32>>>);
#[mod_api::horizon_part]
impl mod_api::ItemPart for Accept23 {
    fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
        assert_eq!(event.stack.count(), 3);
        assert_eq!(event.entity.entity().position(host).unwrap()[1], 64.0);
        assert!(event.entity.entity().kind(host).is_some());
        assert!(
            event
                .entity
                .equipment(host, mod_api::EquipmentSlot::Head)
                .is_none()
        );
        self.0.lock().unwrap().push(event.entity.id());
        event.can_equip = event.entity.id() == 23;
    }
}
struct Register(Accept23);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Register {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let helmet = r.item_key("minecraft:iron_helmet");
        r.item_part(helmet, self.0.clone());
    }
}
#[test]
#[ignore = "daily"]
fn crown_is_prepared_once_for_three_and_eight_dispenser_candidates() {
    for ids in [vec![21, 22, 23], vec![16, 17, 18, 19, 20, 21, 22, 23]] {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "equip_counter",
            Box::new(Register(Accept23(asked.clone()))),
        )]
        .into();
        crate::entry::install_client_hooks();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let at = horizon_core::geometry::Vec3::new(6.5, 64.0, 5.5);
                for id in &ids {
                    let mut mob = horizon_client::mobs::Mob::new(*id, kind.clone(), at, 0.0);
                    mob.extra.pick_up_loot = true;
                    game.mobs.push(mob);
                }
                let mut helmet = ItemStack::new("minecraft:iron_helmet", 3);
                helmet.set_crafting_enchantments(vec![("minecraft:protection".into(), 4)]);
                helmet.data_mut().custom_name =
                    Some(horizon_core::nbt::Tag::String("Crown".into()));
                crate::game::equip_answers::take_equip_conversions();
                assert!(game.dispense_equipment((5, 64, 5), Direction::East, &mut helmet));
                assert_eq!(helmet.count, 2);
                assert_eq!(*asked.lock().unwrap(), ids);
                assert_eq!(crate::game::equip_answers::take_equip_conversions(), 1);
                for mob in game.mobs.iter().filter(|mob| ids.contains(&mob.id)) {
                    assert_eq!(mob_slot_item(mob, Slot::Head).is_some(), mob.id == 23);
                }
            });
        });
    }
}
