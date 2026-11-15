//! More of step 4a's acceptance list over LAN TCP, from a server in the dedicated role: a
//! negotiated peer that does not see the holder is sent nothing, and a message keeps its
//! place among the player's held gameplay packets.
use super::transport_tests::{
    Route, VALUE, peer, set_level, store, sync_packets_within, values_until, wait_server_ticks,
};
use horizon_core::nbt::Tag;
use horizon_mod_runtime::attachments::{AttachmentSync, Holder};
use horizon_protocol::{
    attachment_sync,
    buf::{Codec, Ident, Reader},
    connection::Connection,
    packets::play,
};
use std::time::{Duration, Instant};

const NAME: &str = "game::attachment_sync::route_cases_tests::a_dedicated_server_sends_only_trackers_in_packet_order";

/// What a peer is sent, reduced to what this test orders: system chat text and the level's
/// synced values, until it has seen `count` of them.
#[derive(Debug, PartialEq)]
enum Seen {
    Chat(String),
    Sync(Holder, i32),
}
fn seen_until(c: &mut Connection, count: usize) -> Vec<Seen> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while seen.len() < count {
        assert!(Instant::now() < deadline, "only {seen:?}");
        let (id, body) = match c.recv() {
            Ok(packet) => packet,
            Err(error) if error.is_timed_out() => continue,
            Err(error) => panic!("peer receive failed: {error}"),
        };
        if id == horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
            let mut reader = Reader::new(&body);
            if Ident::read(&mut reader).unwrap() == attachment_sync::CHANNEL {
                let message = attachment_sync::decode(reader.rest()).unwrap();
                for change in message
                    .changes
                    .iter()
                    .filter(|_| message.holder == Holder::Level)
                {
                    if let Some(Tag::Int(n)) = change
                        .value
                        .as_ref()
                        .and_then(|value| value.get(&"n".into()))
                    {
                        seen.push(Seen::Sync(message.holder, *n));
                    }
                }
                continue;
            }
        }
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::SystemChat(p) => {
                if let Tag::String(text) = p.content {
                    seen.push(Seen::Chat(text.to_string().unwrap_or_default()));
                }
            }
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    seen
}

#[test]
#[ignore = "daily"]
fn a_dedicated_server_sends_only_trackers_in_packet_order() {
    std::env::var_os("HORIZON_TEST_RESOURCES")
        .expect("missing HORIZON_TEST_RESOURCES: a prepared 26.3 resource root");
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_ATTACHMENT_SYNC_CASES_CHILD",
        &["cases"],
    ) {
        return;
    }
    let mods = vec![mod_api::ModEntry::new("sync_route", Box::new(Route))].into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_client_game(|game| {
            game.settings.render_distance = 2;
            game.settings.simulation_distance = 2;
            let options = horizon_client::title::WorldOptions {
                world_type: horizon_client::save::WorldPreset::Flat,
                ..Default::default()
            };
            game.open_integrated(
                "sync_cases",
                Some("Sync cases"),
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
            // The server takes the dedicated role before it opens its socket, as a
            // dedicated server runs the same publication, hub and connections.
            let port = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    server.options.execution_role =
                        crate::game::execution_role::ExecutionRole::DedicatedServer;
                    server.publish(super::super::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    })
                })
                .unwrap()
                .unwrap();
            let mut near = peer(port, "SyncNear", true);
            let mut far = peer(port, "SyncFar", true);
            set_level(game, 20, true);
            assert_eq!(values_until(&mut near, Holder::Level, 20), [20]);
            assert_eq!(values_until(&mut far, Holder::Level, 20), [20]);

            // The far peer leaves the near one's chunks; only the near one sees its chunk.
            let chunk = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    assert!(server.native_run_command("tp SyncFar 4000 100 4000"));
                    let lan = server.lan.as_ref().unwrap();
                    let near = lan.hub.guests.iter().find(|g| g.name == "SyncNear");
                    near.unwrap().chunk()
                })
                .unwrap();
            wait_server_ticks(game, 20);
            // What the move itself sends it (its new chunks) is drained first.
            sync_packets_within(&mut far, Duration::from_secs(1));
            let holder = Holder::Chunk([chunk.0, chunk.1]);
            game.integrated
                .as_ref()
                .unwrap()
                .call(move |server| VALUE.set(store(server), holder, 21).unwrap())
                .unwrap();
            assert_eq!(values_until(&mut near, holder, 21), [21]);
            // Negotiated, but tracking no holder that changed: 0 updates.
            assert_eq!(sync_packets_within(&mut far, Duration::from_secs(2)), 0);

            // Held gameplay packets keep their place around a message: before, the
            // message, after, on the wire in that order.
            game.integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    let lan = server.lan.as_ref().unwrap();
                    let guest = lan.hub.guests.iter().find(|g| g.name == "SyncNear");
                    let sender = guest.unwrap().sender.clone();
                    let chat = |text: &str| {
                        play::Clientbound::SystemChat(play::SystemChat {
                            content: Tag::String(text.into()),
                            overlay: false,
                        })
                        .encode()
                    };
                    let holding = horizon_client::lan::Holding::start();
                    sender.send(chat("before"));
                    let message = attachment_sync::Message {
                        holder: Holder::Level,
                        replace: false,
                        changes: vec![AttachmentSync {
                            namespace: "sync_route".into(),
                            name: "value".into(),
                            value: Some([("n".into(), Tag::Int(22))].into()),
                        }],
                    };
                    server.send_attachment_sync(std::slice::from_ref(&sender), &message);
                    sender.send(chat("after"));
                    drop(holding);
                    horizon_client::lan::flush_all([&sender]);
                })
                .unwrap();
            near.reader
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let order: Vec<_> = seen_until(&mut near, 3);
            assert_eq!(
                order,
                [
                    Seen::Chat("before".into()),
                    Seen::Sync(Holder::Level, 22),
                    Seen::Chat("after".into()),
                ]
            );
            drop((near, far));
            game.leave_server(None).unwrap();
        });
    });
}
