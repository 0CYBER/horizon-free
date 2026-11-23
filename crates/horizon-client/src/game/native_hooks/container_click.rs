use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `player` clicked slot `slot` of window `window` (0: their inventory menu, else an open
    /// block container) with `button` and click type `mode`, before the click runs
    /// (`handleContainerClick`). `carried` is their cursor stack and `clicked` the stack their
    /// client was last sent for that slot. Set `canceled` and the click changes nothing: the
    /// menu and the cursor are sent again. Their state is swapped in. Server-side.
    pub(in crate::game) fn native_container_click(
        &mut self,
        call: mod_api::__private::ContainerClickCall<'_>,
    ) {
        self.fire_container_click(call);
    }
}

#[cfg(test)]
mod container_click_tests {
    use super::*;
    use crate::game::native_guest_turn_tests::with_guests;
    use horizon_client::lan::menu::Slots;
    use horizon_protocol::packets::play::{ContainerClick, Serverbound};
    use mod_api::{ContainerClickEvent, Flow, Hooks, ModEntry, ServerHost};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// Records every click it hears (window, slot, button, mode, carried item and count) and
    /// cancels the ones in slot 0.
    struct Guard(Arc<Mutex<Vec<(i32, i32, i32, i32, Option<(String, u32)>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Guard {
        fn container_click(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ContainerClickEvent<'_>,
        ) -> Flow {
            let carried = event.carried.map(|c| {
                (
                    mod_api::__private::item_key_id(c.item())
                        .unwrap()
                        .to_owned(),
                    c.count(),
                )
            });
            self.0.lock().unwrap().push((
                event.window,
                event.slot,
                event.button,
                event.mode,
                carried,
            ));
            if event.slot == 0 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    /// A pickup click in slot 0 of an open chest, with 8 stone on the cursor, is heard with
    /// exactly those values and canceled: the chest stays empty and the cursor keeps its stone.
    /// The same click in slot 1 is heard and runs: the stone goes into the chest.
    #[test]
    #[ignore = "daily"]
    fn a_canceled_container_click_changes_nothing_and_resends_the_menu() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new("click_guard", Box::new(Guard(heard.clone())))];
        with_mods(mods.into(), || {
            with_guests(|game| {
                let pos = (1, 64, 1);
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(pos.0, pos.1, pos.2, stone).unwrap();
                game.world.block_entities.insert(
                    pos,
                    horizon_client::containers::BlockEntity::Chest(vec![None; 27]),
                );
                let host = game.lan.as_mut().unwrap();
                host.hub.open_container(
                    12,
                    "minecraft:generic_9x3",
                    horizon_core::nbt::Tag::String("test".into()),
                    pos,
                    Slots::Block(pos),
                    vec![None; 27],
                    None,
                );
                let window = host.hub.guests[0].menu.as_ref().unwrap().window;
                host.hub.guests.update(0, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:stone", 8))
                });
                let click = |slot: i16| {
                    Serverbound::ContainerClick(ContainerClick {
                        container: window,
                        state: 0,
                        slot,
                        button: 0,
                        input: 0,
                        changed: Vec::new(),
                        carried: None,
                    })
                };
                game.lan_guest_packet(12, click(0));
                let chest = |game: &crate::game::Game<'_>, slot: usize| {
                    game.world.block_entities.get(&pos).unwrap().slots()[slot]
                        .as_ref()
                        .map_or(0, |s| s.count)
                };
                assert_eq!(chest(game, 0), 0);
                let carried = game.lan.as_ref().unwrap().hub.guests[0].carried.clone();
                assert_eq!(carried.map(|c| c.count), Some(8));
                game.lan_guest_packet(12, click(1));
                assert_eq!(chest(game, 1), 8);
                assert_eq!(
                    *heard.lock().unwrap(),
                    [
                        (window, 0, 0, 0, Some(("minecraft:stone".to_owned(), 8))),
                        (window, 1, 0, 0, Some(("minecraft:stone".to_owned(), 8))),
                    ]
                );
            });
        });
    }
}
