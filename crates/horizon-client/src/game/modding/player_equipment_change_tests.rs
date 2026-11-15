//! `living_equipment_change` for a server's players (arismoko/horizon#3417): a hotbar selection, a pickup into
//! the selected slot and a helmet put on are each one change on the next tick, and nothing is
//! told while no mod sets `living_equipment_listened`.
use std::sync::{Arc, Mutex};

use horizon_client::items::ItemStack;
use mod_api::{self, Hooks, LivingEquipmentChangeEvent, LivingEquipmentListenedEvent, ServerHost};

use super::api_values::TryIntoApi;
use super::native_guest_turn_tests::with_guests;

/// What each call saw: the entity, its kind, the slot, and the stacks before and after.
type Heard = Vec<(
    u32,
    String,
    String,
    Option<mod_api::ItemStack>,
    Option<mod_api::ItemStack>,
)>;

struct Counter {
    heard: Arc<Mutex<Heard>>,
    listens: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Counter {
    fn living_equipment_listened(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LivingEquipmentListenedEvent,
    ) {
        event.listened |= self.listens;
    }
    fn living_equipment_change(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LivingEquipmentChangeEvent<'_>,
    ) {
        self.heard.lock().unwrap().push((
            event.entity,
            event.kind.to_owned(),
            event.slot.to_owned(),
            event.from.cloned(),
            event.to.cloned(),
        ));
    }
}

fn with_counter(listens: bool, test: impl FnOnce(&mut crate::game::Game<'_>, &Mutex<Heard>)) {
    let heard = Arc::new(Mutex::new(Heard::new()));
    let counter = Counter {
        heard: heard.clone(),
        listens,
    };
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("counter", Box::new(counter))].into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            // A joined guest has its play state from the login on.
            let host = game.lan.as_mut().unwrap();
            for conn in [12, 13] {
                host.play.entry(conn).or_default();
            }
            test(game, &heard)
        })
    });
}

fn api(stack: &ItemStack) -> Option<mod_api::ItemStack> {
    Some(stack.try_into_api().unwrap())
}

fn take(heard: &Mutex<Heard>) -> Heard {
    std::mem::take(&mut *heard.lock().unwrap())
}

fn player(slot: &str, from: Option<mod_api::ItemStack>, to: Option<mod_api::ItemStack>) -> Heard {
    vec![(12, "minecraft:player".into(), slot.into(), from, to)]
}

#[test]
#[ignore = "daily"]
fn a_players_hotbar_selection_pickup_and_helmet_are_each_one_change() {
    with_counter(true, |game, heard| {
        let sword = ItemStack::new("minecraft:iron_sword", 1);
        let stone = ItemStack::new("minecraft:stone", 1);
        let helmet = ItemStack::new("minecraft:iron_helmet", 1);
        // Both guests start empty-handed in slot 0: counting from empty, nothing changed.
        game.native_players_equipment();
        assert_eq!(take(heard), Heard::new());

        // Selecting hotbar slot 1, which holds a sword, moves it into the main hand.
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.inventory[1] = Some(sword.clone());
            g.selected = 1;
        });
        game.native_players_equipment();
        assert_eq!(take(heard), player("mainhand", None, api(&sword)));

        // Back to the empty slot 0, then a stone picked up into it.
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| g.selected = 0);
        game.native_players_equipment();
        assert_eq!(take(heard), player("mainhand", api(&sword), None));
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| g.inventory[0] = Some(stone.clone()));
        game.native_players_equipment();
        assert_eq!(take(heard), player("mainhand", None, api(&stone)));

        // A helmet put on from the inventory screen (`armor` runs head to feet).
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| g.armor[0] = Some(helmet.clone()));
        game.native_players_equipment();
        assert_eq!(take(heard), player("head", None, api(&helmet)));

        // A tick with no change tells nothing.
        game.native_players_equipment();
        assert_eq!(take(heard), Heard::new());
    });
}

#[test]
#[ignore = "daily"]
fn players_are_not_told_without_the_listened_opt_in() {
    with_counter(false, |game, heard| {
        let guests = &mut game.lan.as_mut().unwrap().hub.guests;
        guests.update(0, |g| {
            g.inventory[1] = Some(ItemStack::new("minecraft:iron_sword", 1));
            g.selected = 1;
            g.armor[0] = Some(ItemStack::new("minecraft:iron_helmet", 1));
        });
        game.native_players_equipment();
        assert_eq!(take(heard), Heard::new());
    });
}
