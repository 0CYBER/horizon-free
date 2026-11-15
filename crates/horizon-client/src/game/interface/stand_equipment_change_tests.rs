//! `LivingEntity.collectEquipmentChanges` on an armor stand walks every
//! `EquipmentSlot`: a body or saddle a mod wrote is told to
//! `living_equipment_change` like the six hand and armor slots.
use std::sync::{Arc, Mutex};

use horizon_client::{entities::ArmorStand, items::ItemStack};
use horizon_core::geometry::Vec3;
use horizon_items::equipment::{BODY, HEAD, SADDLE};
use mod_api::{self, Hooks, LivingEquipmentChangeEvent, ServerHost};

use super::native_entity_refs::tests::with_game;

use super::api_values::TryIntoApi;
type Heard = Vec<(
    u32,
    String,
    Option<mod_api::ItemStack>,
    Option<mod_api::ItemStack>,
)>;

struct Counter(Arc<Mutex<Heard>>);
#[mod_api::__subscriptions]
impl Hooks for Counter {
    fn living_equipment_change(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LivingEquipmentChangeEvent<'_>,
    ) {
        self.0.lock().unwrap().push((
            event.entity,
            event.slot.to_owned(),
            event.from.cloned(),
            event.to.cloned(),
        ));
    }
}

#[test]
#[ignore = "daily"]
fn a_stands_body_and_saddle_writes_are_told_once_each() {
    let heard = Arc::new(Mutex::new(Heard::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "counter",
        Box::new(Counter(heard.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.armor_stands.push(ArmorStand::new(16, Vec3::ZERO, 0.0));
            let body = ItemStack::new("minecraft:diamond_horse_armor", 1);
            let saddle = ItemStack::new("minecraft:saddle", 1);
            let helmet = ItemStack::new("minecraft:iron_helmet", 1);
            let stand = &mut game.armor_stands[0];
            stand.equipment[BODY] = Some(body.clone());
            stand.equipment[SADDLE] = Some(saddle.clone());
            stand.equipment[HEAD] = Some(helmet.clone());
            game.armor_stand_location_effects(0, false);
            // Moving or landing later copies nothing unreported.
            game.armor_stand_location_effects(0, true);
            game.armor_stands[0].equipment[BODY] = None;
            game.armor_stand_location_effects(0, false);
            let heard = heard.lock().unwrap();
            let told: Vec<_> = heard
                .iter()
                .map(|(id, slot, from, to)| (*id, slot.as_str(), from.clone(), to.clone()))
                .collect();
            assert_eq!(
                told,
                vec![
                    (16, "head", None, Some(helmet.try_into_api().unwrap())),
                    (16, "body", None, Some(body.try_into_api().unwrap())),
                    (16, "saddle", None, Some(saddle.try_into_api().unwrap())),
                    (16, "body", Some(body.try_into_api().unwrap()), None),
                ]
            );
        })
    });
}
