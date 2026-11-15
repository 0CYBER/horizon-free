//! A mod's datapack registry over a real connection (arismoko/horizon#811, stage 4): a host
//! (`Lan` and `Hub`) and a client (`net::Session`) on loopback negotiate `horizon:datapack_offer`
//! and `horizon:datapack_registries` through `lan/conn.rs` and `net.rs`. The host serves the
//! set `datapack_net::serve` was given; the world open and `/reload` that publish it are in
//! `Game` and are not run here, so each test publishes the sets itself. No prepared resources
//! are needed. The process holds one set of declarations and one served and received set, so
//! the tests take turns.
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    chunk::MIN_Y,
    datapack_net,
    lan::{
        Lan, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        hub::{HostEvent, Hub, HubConfig, TickCtx},
    },
    net::{Event, Session},
    world::World,
};
use horizon_protocol::{
    PROTOCOL_VERSION,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation},
        handshake, login, play,
    },
};
use horizon_world::{
    item_defaults::ItemData,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    registry::BlockStateId,
};
use mod_api::{
    DatapackCandidate, DatapackField, DatapackFieldKind, DatapackReadError,
    DatapackRegistryDeclaration, DatapackRegistrySet, DatapackRegistrySync, DatapackValue,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

static TURN: Mutex<()> = Mutex::new(());

fn declarations() -> Vec<DatapackRegistryDeclaration> {
    vec![DatapackRegistryDeclaration {
        mod_id: "example".into(),
        key: "example:spell".into(),
        fields: vec![DatapackField::required(
            "power",
            DatapackFieldKind::Int { min: 0, max: 100 },
        )],
        sync: DatapackRegistrySync::Required,
    }]
}

/// Declares the registry for the whole process, takes the turn and forgets earlier sets.
fn begin() -> MutexGuard<'static, ()> {
    let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    mod_api::__private::install_process_datapack_registry_declarations(declarations());
    datapack_net::serve(None);
    datapack_net::clear_received();
    turn
}

/// The candidate whose one entry, `addon:fire` of `example:spell`, has this power.
fn candidate(power: i64) -> DatapackCandidate {
    let file = format!(r#"{{"power": {power}}}"#).into_bytes();
    DatapackCandidate::build(
        &declarations(),
        &[(
            "data/addon/example/spell/fire.json".to_owned(),
            vec![("pack".to_owned(), file)],
        )],
    )
    .unwrap()
}

fn first_set(power: i64) -> Arc<DatapackRegistrySet> {
    Arc::new(DatapackRegistrySet::first(
        candidate(power),
        datapack_net::next_session(),
    ))
}

fn power(set: &DatapackRegistrySet) -> Option<DatapackValue> {
    let handle = set.handle("example:spell", "addon:fire").ok()?;
    set.get(&handle).ok()?.get("power").cloned()
}

fn start_lan() -> Lan {
    let net = NetConfig {
        announce: false,
        ..NetConfig::loopback(0, "Datapack join".into(), 8)
    };
    Lan::start_with(Arc::new(SyncData::empty()), net).unwrap()
}

/// A hub over an empty world, ticked by the test as the game loop would.
struct Host {
    hub: Hub,
    world: World,
    next_entity: u32,
    joined: Vec<u32>,
}

impl Host {
    fn start() -> Self {
        let sync = Arc::new(SyncData::empty());
        let net = NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "Datapack join".into(), 8)
        };
        let lan = Lan::start_with(sync.clone(), net).unwrap();
        let encoder = ChunkEncoder::new(
            [] as [(Arc<str>, bool); 0],
            &horizon_protocol::registry::Builtin::default(),
            &[],
            &[],
        );
        let config = HubConfig {
            allow_flight: false,
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 3,
            simulation_distance: 3,
            integrated: true,
            dimension: "minecraft:overworld".into(),
            seed: 1,
            max_players: 8,
            levels: Default::default(),
            level_seeds: Default::default(),
        };
        let load = Arc::new(
            load_context(DataLoadOrigin::ServerDataLoad)
                .loop_context(horizon_world::native_load_context::LoopSide::Server, []),
        );
        Self {
            hub: Hub::new(lan, sync, encoder, config, load),
            world: World::new(BlockStateId(0), 0),
            next_entity: 100,
            joined: Vec::new(),
        }
    }

    fn port(&self) -> u16 {
        self.hub.lan.port
    }

    fn tick(&mut self) {
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut self.world,
            entities: &[],
            next_entity: &mut self.next_entity,
            spawn: [8.5, f64::from(MIN_Y + 5), 8.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &horizon_client::worldborder::WorldBorder::default(),
            streaming: None,
        };
        for event in self.hub.tick(&mut ctx) {
            if let HostEvent::Joined(conn) = event {
                self.joined.push(conn);
            }
        }
    }
}

fn load_context(origin: DataLoadOrigin) -> LoadContext {
    LoadContext::new(
        origin,
        Arc::new(LoadInputs::new(BTreeMap::new(), Default::default())),
        [],
    )
}

/// A folder with the empty item data a client session loads, removed when dropped.
struct Resources(std::path::PathBuf);

impl Resources {
    fn new(name: &str) -> (Self, Arc<ItemData>) {
        let root = std::env::temp_dir().join(format!(
            "horizon-datapack-join-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = ItemData::load(&root, load_context(DataLoadOrigin::ClientPreload)).unwrap();
        (Self(root), Arc::new(data))
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn connect(port: u16, name: &str, data: &Arc<ItemData>) -> Session {
    Session::connect(
        &format!("127.0.0.1:{port}"),
        name,
        None,
        ClientInformation::new(8),
        Arc::new(Default::default()),
        Arc::clone(data),
    )
}

/// Ticks the host and polls the session until `done` takes an event; fails after 30 seconds.
fn drive<T>(
    host: &mut Host,
    session: &Session,
    mut done: impl FnMut(&mut Host, Event) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        host.tick();
        for receipt in session.poll() {
            let (event, _credit) = receipt.into_parts();
            if let Some(found) = done(host, event) {
                return found;
            }
        }
        assert!(Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn join(host: &mut Host, session: &Session) {
    drive(host, session, |_, event| match event {
        Event::Joined { .. } => Some(()),
        Event::Disconnected { reason, .. } => panic!("the join ended: {reason}"),
        _ => None,
    });
}

#[test]
#[ignore = "daily"]
fn a_client_that_offers_the_registry_joins_and_reads_the_entry() {
    let _turn = begin();
    datapack_net::serve(Some(first_set(7)));
    let (_resources, data) = Resources::new("offers");
    let mut host = Host::start();
    let session = connect(host.port(), "Mage", &data);
    join(&mut host, &session);
    let received = datapack_net::received().expect("the snapshot arrived before the join ended");
    assert_eq!(received.generation(), 1);
    assert_eq!(power(&received), Some(DatapackValue::Int(7)));
}

/// What a client without the mod sees: the reason it is refused with, and whether any
/// datapack payload came before it.
fn refused_client(brand: &str) -> (String, bool) {
    let lan = start_lan();
    let mut c = Connection::connect("127.0.0.1", lan.port, Duration::from_secs(5)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port: lan.port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "Plain".into(),
            uuid: offline_uuid("Plain"),
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, payload) = c.recv().unwrap();
        match login::Clientbound::decode(id, &payload).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .unwrap();
                break;
            }
            other => panic!("unexpected login packet {other:?}"),
        }
    }
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(4)).encode())
        .unwrap();
    let mut name = horizon_protocol::buf::Writer::new();
    name.string(brand);
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: name.data,
        })
        .encode(),
    )
    .unwrap();
    let mut registry_payload = false;
    loop {
        let (id, payload) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &payload).unwrap() {
            cfg::Clientbound::SelectKnownPacks(_) => c
                .send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap(),
            cfg::Clientbound::CustomPayload(p)
                if brand == "horizon"
                    && p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
            {
                use horizon_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                if Control::decode(&p.data).unwrap() == Control::Query {
                    c.send(
                        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Offer(Manifest::default()).encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            cfg::Clientbound::CustomPayload(p) if p.channel == datapack_net::CHANNEL => {
                registry_payload = true;
            }
            cfg::Clientbound::RegistryData(_) => registry_payload = true,
            cfg::Clientbound::Disconnect(d) => {
                let reason = horizon_protocol::text::plain(&d.reason, &Default::default());
                return (reason, registry_payload);
            }
            cfg::Clientbound::FinishConfiguration(_) => panic!("the client was let in"),
            _ => {}
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_required_registry_refuses_a_client_without_the_mod_before_any_registry_data() {
    let _turn = begin();
    datapack_net::serve(Some(first_set(7)));
    // A Horizon client that sends no offer, and a vanilla one.
    for brand in ["horizon", "vanilla"] {
        let (reason, registry_payload) = refused_client(brand);
        assert!(
            reason.contains("example") && reason.contains("example:spell"),
            "{brand}: {reason}"
        );
        assert!(!registry_payload, "{brand}: data came before the refusal");
    }
}

#[test]
#[ignore = "daily"]
fn a_reload_snapshot_gives_the_client_generation_two_and_stales_old_handles() {
    let _turn = begin();
    let first = first_set(7);
    datapack_net::serve(Some(first.clone()));
    let (_resources, data) = Resources::new("reload");
    let mut host = Host::start();
    let session = connect(host.port(), "Mage", &data);
    join(&mut host, &session);
    let joined = datapack_net::received().unwrap();
    assert_eq!(joined.generation(), 1);
    let held = joined.handle("example:spell", "addon:fire").unwrap();
    assert_eq!(
        joined.get(&held).unwrap().get("power"),
        Some(&DatapackValue::Int(7))
    );
    // The host registers the guest on a tick after the client joined.
    let deadline = Instant::now() + Duration::from_secs(30);
    while host.joined.is_empty() {
        assert!(
            Instant::now() < deadline,
            "host did not admit the joined client"
        );
        host.tick();
        std::thread::sleep(Duration::from_millis(10));
    }

    // What `/reload` does: publish the next generation, then send each joined client its snapshot.
    datapack_net::serve(Some(Arc::new(first.reload(candidate(9)).unwrap())));
    let datapack_net::Step::Send(data) = datapack_net::reload_payload(
        host.hub
            .guests
            .iter()
            .find(|g| g.uuid == offline_uuid("Mage"))
            .unwrap()
            .sender
            .datapack_offer
            .as_deref(),
    ) else {
        panic!("a snapshot for Mage");
    };
    for data in datapack_net::chunks(&data).unwrap() {
        host.hub.send_to(
            host.joined[0],
            &play::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: datapack_net::CHANNEL.into(),
                data,
            }),
        );
    }
    // The game keeps what its play packet carries with `datapack_net::accept`.
    drive(&mut host, &session, |_, event| match event {
        Event::Packet(packet) => match *packet {
            play::Clientbound::CustomPayload(p) if p.channel == datapack_net::CHANNEL => {
                datapack_net::accept_chunk(&p.data).unwrap();
                Some(())
            }
            _ => None,
        },
        Event::Disconnected { reason, .. } => panic!("the connection ended: {reason}"),
        _ => None,
    });
    let next = datapack_net::received().unwrap();
    assert_eq!(next.generation(), 2);
    assert_eq!(power(&next), Some(DatapackValue::Int(9)));
    assert_eq!(next.get(&held), Err(DatapackReadError::StaleGeneration));
}

#[test]
#[ignore = "daily"]
fn a_refused_reload_disconnects_instead_of_retaining_the_old_generation() {
    let _turn = begin();
    let first = first_set(7);
    datapack_net::serve(Some(first.clone()));
    let (_resources, data) = Resources::new("refused_reload");
    let mut host = Host::start();
    let session = connect(host.port(), "Mage", &data);
    join(&mut host, &session);
    let deadline = Instant::now() + Duration::from_secs(30);
    while host.joined.is_empty() {
        assert!(
            Instant::now() < deadline,
            "host did not admit the joined client"
        );
        host.tick();
        std::thread::yield_now();
    }
    let mut sender = host
        .hub
        .guests
        .iter()
        .find(|g| g.uuid == offline_uuid("Mage"))
        .unwrap()
        .sender
        .clone();
    sender.datapack_offer = Some(Arc::from([255_u8]));
    datapack_net::serve(Some(Arc::new(first.reload(candidate(9)).unwrap())));
    datapack_net::reload_connection(&sender);
    drive(&mut host, &session, |_, event| match event {
        Event::Disconnected { reason, .. } => {
            assert!(
                format!("{reason:?}").contains("registries are unreadable"),
                "{reason:?}"
            );
            Some(())
        }
        _ => None,
    });
}
