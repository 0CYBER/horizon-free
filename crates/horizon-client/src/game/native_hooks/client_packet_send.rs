use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A play packet the window's game is about to send to the server
    /// (`Connection.send`): every packet of the play phase it sends, the
    /// selected slot that goes out before an attack or a use included, in the
    /// order it sends them. Not heard: what the network thread sends itself
    /// (keep-alives, configuration pongs, cookies, the configuration phase), the answers the
    /// connection sends in whichever phase it is in (a resource pack's status, a
    /// dialog's custom click action), and far terrain requests. Cancel drops the
    /// packet: the game goes on as if it had been sent. Client-side, on any
    /// server, single player's own included. Every packet exposes its protocol
    /// type id; custom payloads also expose their channel and raw bytes.
    /// Play-phase pongs pass through this hook; mods cancel by returning `Flow::Cancel`.
    pub(in crate::game) fn native_client_packet_send(
        &self,
        packet: &ClientPacket,
        canceled: &mut bool,
    ) {
        self.fire_client_packet_send(packet, canceled);
    }
}

#[cfg(test)]
mod client_packet_send_tests {
    use horizon_protocol::packets::{
        configuration::CustomPayload,
        play::{Serverbound, SetCarriedItem},
    };
    use mod_api::{ClientHost, ClientPacketSendEvent, Flow, Hooks, ModEntry};
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};

    /// Drops payloads on `example:secret`, logging every packet it hears.
    struct Filter(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Filter {
        fn client_packet_send(
            &self,
            _: &dyn ClientHost,
            event: &mut ClientPacketSendEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push(event.packet.id.to_string());
            match &event.packet.custom_payload {
                Some(payload) if payload.channel == "example:secret" => Flow::Cancel,
                _ => Flow::Continue,
            }
        }
    }

    fn payload(channel: &str) -> Serverbound {
        Serverbound::CustomPayload(CustomPayload {
            channel: channel.into(),
            data: vec![7],
        })
    }

    /// `Connection.send`: the mod hears each packet the game sends and cancels the one it
    /// filters; without a listener nothing is cancelled.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_hears_and_cancels_packets_the_game_sends() {
        let slot = Serverbound::SetCarriedItem(SetCarriedItem { slot: 2 });
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.execution_role = super::ExecutionRole::Client;
            assert!(!game.client_packet_send_canceled(&payload("example:secret")));
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods =
            vec![ModEntry::new("filter", Box::new(Filter(Arc::clone(&seen))))].into();
        with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.options.execution_role = super::ExecutionRole::Client;
                assert!(!game.client_packet_send_canceled(&slot));
                assert!(game.client_packet_send_canceled(&payload("example:secret")));
                assert!(!game.client_packet_send_canceled(&payload("example:open")));
            });
        });
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "minecraft:set_carried_item",
                "minecraft:custom_payload",
                "minecraft:custom_payload"
            ]
        );
    }
}
