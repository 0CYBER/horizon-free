//! The held item's say on the drop key: `ItemPart::on_dropped_by_player` heard by a trait mod
//! through a guest's drop.
use super::native_guest_turn_tests::with_guests;
use horizon_client::items::ItemStack;
use mod_api::{
    self, Flow, Hooks, ItemPart, ItemTossEvent, OnDroppedByPlayerEvent, Registrar,
    RegistrationContext, ServerHost,
};
use std::sync::{Arc, Mutex};

/// What the Part and `item_toss` heard, and what the Part answers.
#[derive(Default)]
struct Heard {
    refuse: bool,
    /// Each drop asked about: the player, the item, the stack's count and `all`.
    drops: Vec<(u128, String, u32, bool)>,
    tosses: u32,
}

#[derive(Clone, Default)]
struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:stick");
        r.item_part(key, self.clone());
    }
    fn item_toss(&self, _: &mut dyn ServerHost, _: &mut ItemTossEvent) -> Flow {
        self.0.lock().unwrap().tosses += 1;
        Flow::Continue
    }
}
#[mod_api::horizon_part]
impl ItemPart for Probe {
    fn on_dropped_by_player(
        &self,
        _: &dyn mod_api::Host,
        event: &mut OnDroppedByPlayerEvent<'_>,
    ) -> Flow {
        let mut heard = self.0.lock().unwrap();
        let item = mod_api::__private::item_key_id(event.stack.item())
            .unwrap()
            .to_owned();
        heard
            .drops
            .push((event.player.uuid(), item, event.stack.count(), event.all));
        if heard.refuse {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn with_probe(probe: &Probe, test: impl FnOnce()) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drop_probe",
        Box::new(probe.clone()),
    )]
    .into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, test);
}

fn hold(game: &mut super::Game<'_>, item: &str, count: u32) {
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .update_guest(12, |guest| {
            guest.inventory[guest.selected] = Some(ItemStack::new(item, count));
        })
        .unwrap();
}

/// Guest 12's selected stack's count, and its menu state (bumped by a full inventory send).
fn held(game: &super::Game<'_>) -> (Option<u32>, i32) {
    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
    let count = guest.inventory[guest.selected].as_ref().map(|s| s.count);
    (count, guest.menu_state)
}

/// A refused drop keeps the whole stack in the slot, makes no item entity, fires no
/// `item_toss` and sends the guest its inventory back; an allowed one drops as before, and an
/// item without the Part is not asked.
#[test]
#[ignore = "daily"]
fn a_part_keeps_the_stack_in_the_slot() {
    let probe = Probe::default();
    with_probe(&probe, || {
        with_guests(|game| {
            let gi = game.guest_index(12).unwrap();
            let items = game.dropped.len();

            // Refused: nothing leaves the slot, the client is told so.
            probe.0.lock().unwrap().refuse = true;
            hold(game, "minecraft:stick", 5);
            let (_, state) = held(game);
            let stats = game
                .as_guest(12, |g| {
                    (
                        g.stats.get("dropped", "minecraft:stick"),
                        g.stats.get("custom", "minecraft:drop"),
                    )
                })
                .unwrap();
            game.guest_drop_held(gi, 12, true);
            assert_eq!(held(game), (Some(5), (state + 1) & 0x7FFF));
            assert_eq!(game.dropped.len(), items);
            assert_eq!(
                game.as_guest(12, |g| {
                    (
                        g.stats.get("dropped", "minecraft:stick"),
                        g.stats.get("custom", "minecraft:drop"),
                    )
                })
                .unwrap(),
                stats
            );
            {
                let heard = probe.0.lock().unwrap();
                assert_eq!(heard.drops, [(12, "minecraft:stick".to_owned(), 5, true)]);
                assert_eq!(heard.tosses, 0);
            }

            // Allowed: one item is thrown and `item_toss` hears it.
            probe.0.lock().unwrap().refuse = false;
            game.guest_drop_held(gi, 12, false);
            assert_eq!(held(game).0, Some(4));
            assert_eq!(game.dropped.len(), items + 1);
            {
                let heard = probe.0.lock().unwrap();
                assert_eq!(heard.drops.len(), 2);
                assert_eq!(heard.tosses, 1);
            }

            // An item without the Part drops unasked.
            hold(game, "minecraft:stone", 1);
            game.guest_drop_held(gi, 12, false);
            assert_eq!(held(game).0, None);
            assert_eq!(probe.0.lock().unwrap().drops.len(), 2);
        });
    });
}
