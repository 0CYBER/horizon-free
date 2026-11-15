//! Physical sender identity and action ordering through the real server main inbox.
use horizon_protocol::{
    connection::Connection,
    packets::{Empty, configuration as cfg, play},
};
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadDirection, PayloadId, PayloadProtocol, Registrar,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const NAME: &str =
    "game::payload_transport::sender_tests::published_tcp_attributes_players_and_orders_actions";
const CHANNEL: &str = "sender_probe:marker";
const PLAYERS: [&str; 2] = ["SenderOne", "SenderTwo"];
#[derive(Default)]
struct Probe {
    seen: Mutex<Vec<(u128, u8)>>,
}
impl IPayloadHandler for Probe {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        assert_eq!(context.protocol(), PayloadProtocol::Play);
        assert_eq!(context.flow(), PayloadDirection::ToServer);
        assert_eq!(bytes.0.len(), 2);
        let peer = usize::from(bytes.0[0]);
        let slot = bytes.0[1];
        assert!(peer < PLAYERS.len());
        let player = context.player().unwrap();
        assert_eq!(
            player.uuid(),
            horizon_protocol::crypto::offline_uuid(PLAYERS[peer])
        );
        let host = context.server_host().unwrap();
        assert_eq!(
            player.selected_slot(host),
            Some(usize::from(slot)),
            "the handler must observe the preceding slot action, not a later action"
        );
        let mut seen = self.seen.lock().unwrap();
        let previous = seen
            .iter()
            .filter(|(uuid, _)| *uuid == player.uuid())
            .count();
        assert_eq!(
            previous,
            usize::from(slot - 2),
            "per-player raw order or duplication"
        );
        seen.push((player.uuid(), slot));
        drop(seen);
        context
            .reply(&PayloadId(CHANNEL.into()), bytes)
            .map_err(|e| mod_api::PayloadHandleError::Failed(format!("reply: {e:?}")))?;
        Ok(())
    }
}
struct Registration(Arc<Probe>);
#[mod_api::__subscriptions]
impl Hooks for Registration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(HandlerThread::Main)
            .play_bidirectional(
                PayloadId(CHANNEL.into()),
                self.0.clone(),
                Some(self.0.clone()),
            );
    }
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES; starts an integrated server and two physical TCP clients"]
fn published_tcp_attributes_players_and_orders_actions() {
    super::tests::require_resources();
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_SENDER_TRANSPORT_CHILD",
        &["two_players"],
    ) {
        return;
    }
    let probe = Arc::new(Probe::default());
    let mods = vec![ModEntry::new(
        "sender_probe",
        Box::new(Registration(probe.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "sender_transport",
                Some("Sender transport"),
                Some(1),
                true,
                Some(&options),
            )
            .unwrap();
            let deadline = Instant::now() + Duration::from_secs(60);
            while !game
                .remote
                .as_ref()
                .is_some_and(|r| r.logged_in && r.joined)
            {
                game.poll_remote().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            let port = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    server.publish(super::super::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    })
                })
                .expect("integrated server answered publication request")
                .unwrap();
            let mut peers = [
                connect_peer(port, PLAYERS[0]),
                connect_peer(port, PLAYERS[1]),
            ];
            // Both physical players remain connected. Each stream carries action -> raw ->
            // later action -> raw, with no round-trip barrier that could conceal reordering.
            for slot in [2, 3] {
                for (peer, c) in peers.iter_mut().enumerate() {
                    c.send(
                        &play::Serverbound::SetCarriedItem(play::SetCarriedItem { slot }).encode(),
                    )
                    .unwrap();
                    c.send(
                        &play::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: vec![peer as u8, slot as u8],
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            let reply_deadline = Instant::now() + Duration::from_secs(60);
            for (peer, c) in peers.iter_mut().enumerate() {
                for slot in [2, 3] {
                    loop {
                        assert!(
                            Instant::now() < reply_deadline,
                            "raw reply deadline: peer {peer}, slot {slot}"
                        );
                        let (id, body) = c.recv().unwrap();
                        // Only the vanilla Identifier is decoded before the body oracle.
                        if id == horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
                            use horizon_protocol::buf::{Codec, Ident, Reader};
                            let mut reader = Reader::new(&body);
                            if Ident::read(&mut reader).unwrap() == CHANNEL {
                                assert_eq!(
                                    reader.rest(),
                                    &[peer as u8, slot],
                                    "TCP reply must not contain an envelope inside the declared channel"
                                );
                            }
                        }
                        match play::Clientbound::decode(id, &body).unwrap() {
                            play::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                                assert_eq!(p.data, vec![peer as u8, slot]);
                                break;
                            }
                            play::Clientbound::KeepAlive(p) => {
                                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
                            }
                            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
                            _ => {}
                        }
                    }
                }
            }
            let seen = probe.seen.lock().unwrap();
            assert_eq!(seen.len(), 4);
            for name in PLAYERS {
                let uuid = horizon_protocol::crypto::offline_uuid(name);
                assert_eq!(
                    seen.iter()
                        .filter(|(id, _)| *id == uuid)
                        .map(|(_, slot)| *slot)
                        .collect::<Vec<_>>(),
                    [2, 3]
                );
            }
            drop(seen);
            drop(peers);
            game.leave_server(None).unwrap();
        });
    });
}

pub(super) fn connect_peer(port: u16, name: &str) -> Connection {
    use horizon_protocol::{
        packets::{handshake, login},
        payload_negotiation::Control,
    };
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    // One budget covers the entire login/configuration/play handshake, including
    // streams that keep producing unrelated packets without reaching the next phase.
    let handshake_deadline = Instant::now() + Duration::from_secs(60);
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: horizon_protocol::PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: horizon_protocol::crypto::offline_uuid(name),
        })
        .encode(),
    )
    .unwrap();
    loop {
        assert!(
            Instant::now() < handshake_deadline,
            "login deadline: {name}"
        );
        let (id, body) = c.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            p => panic!("unexpected login packet {p:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    let local =
        horizon_client::raw_payloads::Local::capture(horizon_client::raw_payloads::Side::Client)
            .unwrap();
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
            data: Control::Query.encode().unwrap(),
        })
        .encode(),
    )
    .unwrap();
    let mut known = None;
    let mut offered = false;
    let mut queried = false;
    while known.is_none() || !offered {
        assert!(
            Instant::now() < handshake_deadline,
            "payload negotiation deadline: {name}"
        );
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::SelectKnownPacks(p) => known = Some(p),
            cfg::Clientbound::CustomPayload(p)
                if p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
            {
                match Control::decode(&p.data).unwrap() {
                    Control::Query => {
                        assert!(!queried, "duplicate query");
                        assert!(!offered, "query after offer");
                        queried = true;
                        c.send(
                            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                                channel: horizon_protocol::payload_negotiation::CHANNEL.into(),
                                data: Control::Offer(local.manifest.clone()).encode().unwrap(),
                            })
                            .encode(),
                        )
                        .unwrap();
                    }
                    Control::Offer(_) => {
                        assert!(queried, "offer before initial server query");
                        assert!(!offered, "duplicate offer");
                        offered = true;
                    }
                }
            }
            cfg::Clientbound::FinishConfiguration(_) => panic!("finished before known packs"),
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::SelectKnownPacks(known.unwrap()).encode())
        .unwrap();
    loop {
        assert!(
            Instant::now() < handshake_deadline,
            "configuration finish deadline: {name}"
        );
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::FinishConfiguration(_) => break,
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    let mut logged_in = false;
    loop {
        assert!(
            Instant::now() < handshake_deadline,
            "play teleport deadline: {name}"
        );
        let (id, body) = c.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::Login(_) => {
                assert!(!logged_in);
                logged_in = true;
            }
            play::Clientbound::PlayerPosition(p) => {
                assert!(logged_in, "teleport before play login");
                c.send(
                    &play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                        id: p.teleport,
                        x: p.position[0],
                        y: p.position[1],
                        z: p.position[2],
                        yaw: p.yaw,
                        pitch: p.pitch,
                    })
                    .encode(),
                )
                .unwrap();
                c.send(&play::Serverbound::PlayerLoaded(Empty).encode())
                    .unwrap();
                break;
            }
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c
}
