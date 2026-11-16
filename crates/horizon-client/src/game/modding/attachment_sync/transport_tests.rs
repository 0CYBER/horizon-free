//! Attachment sync on the negotiated route (arismoko/horizon#813 step 4a), over the real integrated memory
//! connection and LAN TCP: who is sent values, in which order, what a rollback and a stale
//! epoch send, and what an oversized message does to the connection.
use super::super::Game;
use horizon_core::nbt::Tag;
use horizon_mod_runtime::attachments::Holder;
use horizon_protocol::{
    attachment_sync,
    buf::{Codec, Ident, Reader},
    connection::Connection,
    packets::{Empty, configuration as cfg, handshake, login, play},
    payload_negotiation::{self as negotiation, Control},
};
use mod_api::{AttachmentCodec, AttachmentStore, AttachmentType, Host};
use std::time::{Duration, Instant};

const NAME: &str =
    "game::attachment_sync::transport_tests::attachment_sync_follows_the_negotiated_route";

pub(super) static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "sync_route",
    name: "value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |c| match c.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(mod_api::AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: true,
};
/// A value past every network frame limit, and incompressible.
static BLOB: AttachmentType<Vec<u8>> = AttachmentType {
    namespace: "sync_route",
    name: "blob",
    default: Vec::new,
    codec: Some(AttachmentCodec {
        encode: |bytes| Ok([("b".into(), Tag::ByteArray(bytes.clone()))].into()),
        decode: |c| match c.get(&"b".into()) {
            Some(Tag::ByteArray(bytes)) => Ok(bytes.clone()),
            _ => Err(mod_api::AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: true,
};

/// A mod with no hooks: with a mod loaded, the server sends joining players snapshots.
pub(super) struct Route;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Route {}

/// The server's own attachment store.
pub(super) fn store<'a>(server: &'a mut Game<'_>) -> &'a mut dyn AttachmentStore {
    let host: &mut dyn Host = server;
    host.attachments()
}

/// The server sets the level's value, committed or rolled back.
pub(super) fn set_level(game: &Game<'_>, value: i32, commit: bool) {
    game.integrated
        .as_ref()
        .unwrap()
        .call(move |server| {
            let seal = server.transaction_open();
            VALUE.set(store(server), Holder::Level, value).unwrap();
            server.transaction_close(&seal, commit);
        })
        .unwrap();
}

/// The window's synced copy of the level's value.
fn window_value(game: &mut Game<'_>) -> Option<i32> {
    let host: &dyn Host = game;
    let rows = host
        .attachment_query("sync_route", "value")
        .query(&VALUE)
        .ok()?;
    rows.iter().next().map(|row| *row.unwrap().value)
}

/// Let the server's hub tick `n` times, so later edits land in a later pass.
pub(super) fn wait_server_ticks(game: &Game<'_>, n: u64) {
    let ticks = |game: &Game<'_>| {
        game.integrated
            .as_ref()
            .unwrap()
            .call(|server| server.lan.as_ref().unwrap().hub.ticks)
            .unwrap()
    };
    let start = ticks(game);
    let deadline = Instant::now() + Duration::from_secs(30);
    while ticks(game) < start + n {
        assert!(Instant::now() < deadline, "the server stopped ticking");
        std::thread::yield_now();
    }
}

fn wait_window(game: &mut Game<'_>, value: i32) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while window_value(game) != Some(value) {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline, "window never saw {value}");
        std::thread::yield_now();
    }
}

/// A TCP peer that offers the engine's declarations (`horizon`) or offers nothing, as a
/// vanilla client does, logged in and in play.
pub(super) fn peer(port: u16, name: &str, horizon: bool) -> Connection {
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
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
        assert!(Instant::now() < deadline, "login deadline: {name}");
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
    loop {
        assert!(Instant::now() < deadline, "configuration deadline: {name}");
        let (id, body) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::CustomPayload(p) if p.channel == negotiation::CHANNEL => {
                if horizon && Control::decode(&p.data).unwrap() == Control::Query {
                    let offer = Control::Offer(local.manifest.clone()).encode().unwrap();
                    c.send(
                        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: negotiation::CHANNEL.into(),
                            data: offer,
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            cfg::Clientbound::SelectKnownPacks(p) => {
                c.send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                    .unwrap();
            }
            cfg::Clientbound::FinishConfiguration(_) => break,
            cfg::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
        .unwrap();
    loop {
        assert!(Instant::now() < deadline, "play deadline: {name}");
        let (id, body) = c.recv().unwrap();
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::PlayerPosition(p) => {
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
                return c;
            }
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
}

/// The values of `holder` a peer is sent until it is sent `last`, in order. A disconnect
/// fails.
pub(super) fn values_until(c: &mut Connection, holder: Holder, last: i32) -> Vec<i32> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut seen = Vec::new();
    while seen.last() != Some(&last) {
        assert!(Instant::now() < deadline, "never sent {last}: {seen:?}");
        let (id, body) = match c.recv() {
            Ok(packet) => packet,
            Err(error) if error.is_timed_out() => continue,
            Err(error) => panic!("peer receive failed: {error}"),
        };
        if id == horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
            let mut reader = Reader::new(&body);
            if Ident::read(&mut reader).unwrap() == attachment_sync::CHANNEL {
                let message = attachment_sync::decode(reader.rest()).unwrap();
                if message.holder == holder {
                    seen.extend(message.changes.iter().filter_map(|change| {
                        match change.value.as_ref()?.get(&"n".into())? {
                            Tag::Int(n) if change.name.to_string().ok()? == "value" => Some(*n),
                            _ => None,
                        }
                    }));
                }
                continue;
            }
        }
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    seen
}

/// How many attachment sync payloads a peer is sent in `window`; a disconnect fails.
pub(super) fn sync_packets_within(c: &mut Connection, window: Duration) -> usize {
    c.reader
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let deadline = Instant::now() + window;
    let mut count = 0;
    while Instant::now() < deadline {
        let (id, body) = match c.recv() {
            Ok(packet) => packet,
            Err(error) if error.is_timed_out() => continue,
            Err(error) => panic!("peer receive failed: {error}"),
        };
        if id == horizon_protocol::ids::play::clientbound::CUSTOM_PAYLOAD {
            let mut reader = Reader::new(&body);
            if Ident::read(&mut reader).unwrap() == attachment_sync::CHANNEL {
                count += 1;
                continue;
            }
        }
        match play::Clientbound::decode(id, &body).unwrap() {
            play::Clientbound::KeepAlive(p) => {
                c.send(&play::Serverbound::KeepAlive(p).encode()).unwrap()
            }
            play::Clientbound::Disconnect(p) => panic!("disconnected: {p:?}"),
            _ => {}
        }
    }
    count
}

#[test]
#[ignore = "daily"]
fn attachment_sync_follows_the_negotiated_route() {
    std::env::var_os("HORIZON_TEST_RESOURCES")
        .expect("missing HORIZON_TEST_RESOURCES: a prepared 26.3 resource root");
    if super::super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "HORIZON_ATTACHMENT_SYNC_ROUTE_CHILD",
        &["route"],
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
                "sync_route",
                Some("Sync route"),
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
            // Integrated memory connection: a commit reaches the window, a rollback does not.
            set_level(game, 7, true);
            wait_window(game, 7);
            set_level(game, 9, false);
            set_level(game, 10, true);
            wait_window(game, 10);

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
                .unwrap()
                .unwrap();
            let mut horizon = peer(port, "SyncHorizon", true);
            let mut vanilla = peer(port, "SyncVanilla", false);
            // A joining Horizon peer is sent the level's whole value, then each commit in
            // order; a rolled-back edit sends nothing, and the authority keeps the old value.
            assert_eq!(values_until(&mut horizon, Holder::Level, 10), [10]);
            set_level(game, 11, false);
            set_level(game, 12, true);
            assert_eq!(values_until(&mut horizon, Holder::Level, 12), [12]);
            let authority = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| VALUE.get(store(server), Holder::Level))
                .unwrap();
            assert_eq!(authority, Ok(12));
            // A vanilla peer is sent nothing, before or after.
            assert_eq!(sync_packets_within(&mut vanilla, Duration::from_secs(2)), 0);

            // Past the network frame limits: refused by name, the peer stays joined and is
            // sent the next value. The memory connection has no frame limit.
            let refused = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    let blob: Vec<u8> = (0u32..9 << 20)
                        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
                        .collect();
                    let message = attachment_sync::Message {
                        holder: Holder::Level,
                        replace: false,
                        changes: vec![horizon_mod_runtime::attachments::AttachmentSync {
                            namespace: "sync_route".into(),
                            name: "blob".into(),
                            value: Some([("b".into(), Tag::ByteArray(blob.clone()))].into()),
                        }],
                    };
                    let body = attachment_sync::encode(&message).map_err(|e| e.to_string());
                    let peer = server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.name == "SyncHorizon")
                        .unwrap()
                        .sender
                        .clone();
                    let refused = server.queue_attachment_sync(&peer, &body);
                    // The same value through the level's own pass, as a mod's edit sends it.
                    BLOB.set(store(server), Holder::Level, blob).unwrap();
                    refused
                })
                .unwrap();
            assert!(
                matches!(
                    refused,
                    Err(mod_api::PayloadSendError::FrameTooLarge { .. })
                ),
                "{refused:?}"
            );
            wait_server_ticks(game, 2);
            set_level(game, 13, true);
            assert_eq!(values_until(&mut horizon, Holder::Level, 13), [13]);

            // A message prepared against an epoch that ended before the root commit publishes
            // nothing, on the TCP and the memory connection alike.
            let stale = game
                .integrated
                .as_ref()
                .unwrap()
                .call(|server| {
                    let senders: Vec<_> = server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .guests
                        .iter()
                        .filter(|g| g.name != "SyncVanilla")
                        .map(|g| g.sender.clone())
                        .collect();
                    let message = attachment_sync::Message {
                        holder: Holder::Level,
                        replace: false,
                        changes: vec![horizon_mod_runtime::attachments::AttachmentSync {
                            namespace: "sync_route".into(),
                            name: "value".into(),
                            value: Some([("n".into(), Tag::Int(99))].into()),
                        }],
                    };
                    let seal = server.transaction_open();
                    server.send_attachment_sync(&senders, &message);
                    for sender in &senders {
                        sender.payload_transport().begin_configuration();
                    }
                    server.transaction_close(&seal, true);
                    senders.len()
                })
                .unwrap();
            assert_eq!(stale, 2, "the window's and the Horizon peer's connections");
            assert_eq!(sync_packets_within(&mut horizon, Duration::from_secs(2)), 0);
            for _ in 0..20 {
                game.poll_remote().unwrap();
            }
            assert_eq!(window_value(game), Some(13));
            drop((horizon, vanilla));
            game.leave_server(None).unwrap();
        });
    });
}
