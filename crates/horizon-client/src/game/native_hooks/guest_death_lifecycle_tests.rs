//! Cancelled guest bodies survive turn reconstruction without repeating die.
#![cfg(test)]
use crate::game::{Game, native_entity_refs::tests::with_game};
use horizon_client::{chunk::Chunk, items::ItemStack};
use mod_api::{Flow, Hooks, HostSide, LivingDeathEvent, ModEntry, PlayerTickPreEvent, ServerHost};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Seen {
    deaths: u32,
    ticks: u32,
}
struct Probe {
    seen: Arc<Mutex<Seen>>,
    heal: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_death(&self, host: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
        self.seen.lock().unwrap().deaths += 1;
        if self.heal {
            let entity = host.entity(event.entity).unwrap();
            let living = host.living(entity).unwrap();
            assert!(living.set_health(host, 5.0));
        }
        Flow::Cancel
    }
    fn player_tick_pre(&self, host: HostSide<'_>, event: &mut PlayerTickPreEvent) {
        let HostSide::Server(host) = host else {
            return;
        };
        let tick = {
            let mut seen = self.seen.lock().unwrap();
            seen.ticks += 1;
            seen.ticks
        };
        if tick == 8 {
            assert!(mod_api::EntityRef::from(event.player).teleport_to(
                host,
                [8.5, 80.0, 8.5],
                None
            ));
        }
    }
}

fn lifecycle(heal: bool) {
    crate::entry::install_client_hooks();
    let seen = Arc::new(Mutex::new(Seen::default()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "guest-death-lifecycle",
            Box::new(Probe {
                seen: seen.clone(),
                heal,
            }),
        )]
        .into(),
        || {
            with_game(|game: &mut Game<'_>| {
                game.build_registries();
                game.load_server_loot();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                crate::game::native_player_death::tests::add_guest(game);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.health = 0.0;
                    guest.position = [4.5, 80.0, 4.5];
                    guest.loaded = true;
                    guest.load_timeout = 0;
                    guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
                    guest.carried = Some(ItemStack::new("minecraft:diamond", 2));
                });
                game.guest_die(0, 1);
                for tick in 1..=20 {
                    if tick == 8 {
                        // A same-entity level restore has replaced the motion before
                        // the hook borrows it; entering the turn resets CombatTracker.
                        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                            let _ = guest.new_body(true);
                        });
                    }
                    game.tick_guests();
                    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                    assert_eq!(guest.death_time, if heal { 0 } else { tick });
                    assert_eq!(guest.body_present(), heal || tick < 20);
                    assert_eq!(seen.lock().unwrap().deaths, 1);
                    assert_eq!(
                        guest.inventory[0],
                        Some(ItemStack::new("minecraft:diamond", 3))
                    );
                    assert_eq!(guest.carried, Some(ItemStack::new("minecraft:diamond", 2)));
                    assert!(game.dropped.is_empty());
                }
                assert_eq!(seen.lock().unwrap().ticks, 20);
                let guest = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(guest.position, [8.5, 80.0, 8.5]);
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    let _ = guest.new_body(false);
                    assert!(!guest.death_canceled);
                    assert_eq!(guest.death_time, 0);
                    // Clearing lifecycle is required even when no old motion remains.
                    guest.death_canceled = true;
                    guest.death_time = 19;
                    assert!(guest.new_body(false).is_none());
                    assert!(!guest.death_canceled);
                    assert_eq!(guest.death_time, 0);
                });
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn cancelled_corpse_teleported_in_player_tick_keeps_original_removal_deadline() {
    crate::game::in_own_process(
        "game::native_hooks::guest_death_lifecycle_tests::cancelled_corpse_teleported_in_player_tick_keeps_original_removal_deadline",
        || lifecycle(false),
    );
}

#[test]
#[ignore = "daily"]
fn listener_healed_guest_stays_visible_through_player_tick_teleport() {
    crate::game::in_own_process(
        "game::native_hooks::guest_death_lifecycle_tests::listener_healed_guest_stays_visible_through_player_tick_teleport",
        || lifecycle(true),
    );
}
