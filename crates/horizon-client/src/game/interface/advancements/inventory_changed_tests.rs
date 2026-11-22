//! Run alone with private prepared resources: installs process-wide trait mods.
use crate::game::*;
use horizon_client::chunk::Chunk;
use mod_api::{Hooks, PlayerInventoryChangedEvent, ServerHost};
use std::sync::Mutex;

type Heard = (u128, Vec<Option<(String, u32)>>);

#[derive(Default)]
struct Probe {
    heard: Arc<Mutex<Vec<Heard>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn player_inventory_changed(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerInventoryChangedEvent<'_>,
    ) {
        let stacks = event
            .stacks
            .iter()
            .map(|s| {
                s.as_ref().map(|s| {
                    (
                        mod_api::__private::item_key_id(s.item())
                            .unwrap()
                            .to_owned(),
                        s.count(),
                    )
                })
            })
            .collect();
        self.heard.lock().unwrap().push((event.player, stacks));
    }
}

#[test]
#[ignore = "daily"]
fn a_changed_inventory_is_heard_once_with_its_41_slots() {
    crate::game::in_own_process(
        "game::advancements::inventory_changed_tests::a_changed_inventory_is_heard_once_with_its_41_slots",
        a_changed_inventory_is_heard_once_with_its_41_slots_isolated,
    );
}

fn a_changed_inventory_is_heard_once_with_its_41_slots_isolated() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let probe = Probe::default();
        let heard_values = probe.heard.clone();
        mod_platform::testing::with_mods(
            vec![mod_api::ModEntry::new("inventory_watch", Box::new(probe))].into(),
            || {
                game.world = World::new(game.res.facts().air, 1);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                let me = game.native_acting_id();
                let heard = || std::mem::take(&mut *heard_values.lock().unwrap());

                // The first check after joining reports what the player holds.
                game.check_inventory_trigger();
                let first = heard();
                assert_eq!(first.len(), 1);
                assert_eq!(first[0].0, me);
                assert_eq!(first[0].1.len(), 41);
                assert!(first[0].1.iter().all(Option::is_none));

                // Nothing changed: nothing heard.
                game.check_inventory_trigger();
                assert!(heard().is_empty());

                // An item picked up is heard once, in its slot.
                game.inventory[4] = Some(ItemStack::new("minecraft:diamond", 3));
                game.check_inventory_trigger();
                game.check_inventory_trigger();
                let added = heard();
                assert_eq!(added.len(), 1);
                assert_eq!(added[0].1[4], Some(("minecraft:diamond".into(), 3)));

                // Armour feet to head (36..40), the off hand at 40.
                game.armor[3] = Some(ItemStack::new("minecraft:iron_boots", 1));
                game.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
                game.offhand = Some(ItemStack::new("minecraft:shield", 1));
                game.check_inventory_trigger();
                let worn = heard();
                assert_eq!(worn.len(), 1);
                let slots = &worn[0].1;
                assert_eq!(slots[36], Some(("minecraft:iron_boots".into(), 1)));
                assert_eq!(slots[39], Some(("minecraft:iron_helmet".into(), 1)));
                assert_eq!(slots[40], Some(("minecraft:shield".into(), 1)));
                assert_eq!(slots[4], Some(("minecraft:diamond".into(), 3)));

                // A count change is a change.
                game.inventory[4].as_mut().unwrap().count = 1;
                game.check_inventory_trigger();
                assert_eq!(heard()[0].1[4], Some(("minecraft:diamond".into(), 1)));
            },
        );
    });
}

#[test]
fn the_slots_follow_inventory_numbering() {
    let mut inventory = vec![None; 36];
    inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
    let armor = [
        Some(ItemStack::new("minecraft:iron_helmet", 1)),
        None,
        None,
        Some(ItemStack::new("minecraft:iron_boots", 1)),
    ];
    let offhand = Some(ItemStack::new("minecraft:shield", 1));
    let slots = super::inventory_slots(&inventory, &armor, &offhand);
    assert_eq!(slots.len(), 41);
    assert_eq!(slots[0].as_ref().map(|s| &*s.item), Some("minecraft:stone"));
    assert_eq!(
        slots[36].as_ref().map(|s| &*s.item),
        Some("minecraft:iron_boots")
    );
    assert_eq!(
        slots[39].as_ref().map(|s| &*s.item),
        Some("minecraft:iron_helmet")
    );
    assert_eq!(
        slots[40].as_ref().map(|s| &*s.item),
        Some("minecraft:shield")
    );
    // A short inventory is padded, never shifted into the armour.
    assert_eq!(super::inventory_slots(&[], &armor, &None).len(), 41);
}

#[test]
fn empty_inventory_hashes_to_zero_and_starts_unseen() {
    use std::hash::{Hash, Hasher};
    let mut h = horizon_core::fast_hash::FxHasher::default();
    for _ in 0..41 {
        Option::<(&str, u32)>::None.hash(&mut h);
    }
    assert_eq!(h.finish(), 0);
    let state = super::AdvState::default();
    assert_eq!(state.last_inventory, None);
    assert_ne!(state.last_inventory, Some(0));
}
