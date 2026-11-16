//! A client mod's hotbar selection (`ClientHost::select_hotbar_slot`): the local player picks a
//! slot as a number key does, and the game's own carried-item path tells the server when it does
//! for a key press. Nothing is sent here.
use super::Game;
use mod_api::HotbarSlotError;

impl Game<'_> {
    /// `Inventory.setSelectedSlot` for a client mod: `slot` (0 to 8) becomes the hotbar slot the
    /// local player holds. Refused above 8, without a client world, and for a spectator (its
    /// number keys open the spectator menu), with the selection unchanged. The packet follows the
    /// game's key-press path: `MultiPlayerGameMode.ensureHasSentCarriedItem` at the next tick's
    /// start at the latest, and before an attack, a use, an interact, a release, a drop and a
    /// continued block break.
    pub(super) fn select_hotbar_slot(&mut self, slot: u8) -> Result<(), HotbarSlotError> {
        if slot > 8 {
            return Err(HotbarSlotError::OutOfRange { slot });
        }
        if self.player.is_none() {
            return Err(HotbarSlotError::NoWorld);
        }
        if self.spectator() {
            return Err(HotbarSlotError::Refused);
        }
        self.selected = usize::from(slot);
        Ok(())
    }
}

#[cfg(test)]
mod hotbar_slot_tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_client_game;
    use crate::game::remote::tests::connect_menu_test;
    use horizon_client::command::args::GameMode;
    use horizon_protocol::connection::ConnReader;
    use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    /// What the window sent since the last call, as play packets.
    fn sent(reader: &mut ConnReader) -> Vec<S> {
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let mut packets = Vec::new();
        while let Ok((id, body)) = reader.recv() {
            packets.extend(S::decode(id, &body).ok());
        }
        packets
    }

    /// A window logged in to the overworld at (0.5, 65, 0.5), its player made by the server's
    /// position packet, with everything the login sent already drained.
    fn logged_in(game: &mut Game<'_>) -> ConnReader {
        let (mut reader, _writer) = connect_menu_test(game);
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        sent(&mut reader);
        reader
    }

    fn carried(packets: &[S]) -> Vec<i16> {
        packets
            .iter()
            .filter_map(|packet| match packet {
                S::SetCarriedItem(item) => Some(item.slot),
                _ => None,
            })
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn a_selection_reaches_the_server_at_the_next_tick_and_only_once() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            assert_eq!(game.selected, 4);
            assert!(
                sent(&mut reader).is_empty(),
                "select_hotbar_slot sends nothing itself"
            );

            game.tick_remote_level().unwrap();
            assert_eq!(carried(&sent(&mut reader)), [4]);

            // The same slot twice makes no second packet.
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            game.tick_remote_level().unwrap();
            assert!(carried(&sent(&mut reader)).is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_selection_goes_out_before_an_attack_in_the_same_tick() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            assert!(game.remote_attack_entity(123, 1.0));
            let packets = sent(&mut reader);
            let carried_at = packets
                .iter()
                .position(|packet| matches!(packet, S::SetCarriedItem(item) if item.slot == 4))
                .expect("the carried-item packet goes out");
            let attack_at = packets
                .iter()
                .position(|packet| matches!(packet, S::Attack(_)))
                .expect("the attack goes out");
            assert!(
                carried_at < attack_at,
                "the carried-item packet precedes the attack: {packets:?}"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_out_of_range_slot_is_refused_and_keeps_the_selection() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.select_hotbar_slot(2), Ok(()));
            assert_eq!(
                game.select_hotbar_slot(9),
                Err(HotbarSlotError::OutOfRange { slot: 9 })
            );
            assert_eq!(game.selected, 2, "the refused slot changes nothing");
            game.tick_remote_level().unwrap();
            assert_eq!(carried(&sent(&mut reader)), [2]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_spectator_gets_no_hotbar_selection() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.select_hotbar_slot(2).unwrap();
            game.mode = GameMode::Spectator;
            assert_eq!(game.select_hotbar_slot(4), Err(HotbarSlotError::Refused));
            assert_eq!(game.selected, 2, "a spectator keeps its slot");
            game.tick_remote_level().unwrap();
            assert_eq!(
                carried(&sent(&mut reader)),
                [2],
                "only the earlier change goes out"
            );
        });
    }
}
