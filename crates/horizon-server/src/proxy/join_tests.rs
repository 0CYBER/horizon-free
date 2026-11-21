//! The proxy against a stand-in game: a listener that does the game's side of the forwarded login, its
//! configuration and a play that echoes, on real sockets.

use super::{
    replay::body,
    wire::{Connection, Reader, handshake, ids, login, status},
    *,
};
use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
};

/// What the stand-in game saw.
#[derive(Default)]
pub struct Seen {
    /// (name, UUID, address) of every forwarded login, after its signature checked out.
    pub logins: Vec<(String, u128, String)>,
    /// The play packets it received, as (id, payload).
    pub play: Vec<(i32, Vec<u8>)>,
    /// The configuration packets it received.
    pub config: Vec<(i32, Vec<u8>)>,
}

pub struct FakeGame {
    pub port: u16,
    pub seen: Arc<Mutex<Seen>>,
}

/// A game that forwards-logs players in with `secret`, shows its status and echoes play packets of id
/// `ECHO` back as `ECHOED`.
pub const ECHO: i32 = ids::play::serverbound::CHAT_COMMAND;
pub const ECHOED: i32 = ids::play::clientbound::SYSTEM_CHAT;

impl FakeGame {
    pub fn start(secret: &[u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let (s, secret) = (seen.clone(), secret.to_vec());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let (s, secret) = (s.clone(), secret.clone());
                let Ok(stream) = stream else { continue };
                thread::spawn(move || {
                    let _ = serve(stream, &s, &secret);
                });
            }
        });
        Self { port, seen }
    }
}

fn serve(stream: TcpStream, seen: &Mutex<Seen>, secret: &[u8]) -> Result<(), wire::Error> {
    let mut c = Connection::from_stream(stream)?;
    let (id, payload) = c.recv()?;
    let hello = handshake::decode_intention(id, &payload, 32767)?;
    if hello.intent == handshake::STATUS {
        c.recv()?;
        c.send(
            &status::Clientbound::StatusResponse(status::StatusResponse {
                json: r#"{"description":"fake game"}"#.into(),
            })
            .encode(),
        )?;
        return Ok(());
    }
    let (id, payload) = c.recv()?;
    let login::Serverbound::Hello(hello) = login::Serverbound::decode(id, &payload)? else {
        panic!("hello")
    };
    c.send(
        &login::Clientbound::CustomQuery(login::CustomQuery {
            transaction: 1,
            channel: "velocity:player_info".into(),
            data: vec![1],
        })
        .encode(),
    )?;
    let (id, payload) = c.recv()?;
    let login::Serverbound::CustomQueryAnswer(answer) = login::Serverbound::decode(id, &payload)?
    else {
        panic!("answer")
    };
    let data = answer.payload.unwrap();
    let (signature, data) = data.split_at(32);
    assert_eq!(
        signature,
        super::backend::hmac_sha256(secret, data),
        "signed with the shared secret"
    );
    let mut r = Reader::new(data);
    assert_eq!(r.varint()?, 1);
    let address = r.string(64)?;
    let uuid = r.uuid()?;
    assert_eq!(r.string(16)?, hello.name);
    seen.lock()
        .unwrap()
        .logins
        .push((hello.name.clone(), uuid, address));
    c.send(
        &login::Clientbound::LoginFinished(login::LoginFinished {
            profile: login::GameProfile {
                uuid,
                name: hello.name,
                properties: vec![],
            },
            session: uuid,
        })
        .encode(),
    )?;
    let (id, payload) = c.recv()?;
    assert!(matches!(
        login::Serverbound::decode(id, &payload)?,
        login::Serverbound::LoginAcknowledged(_)
    ));
    // Configuration: nothing to say but the end of it, then what the client said is read until it finishes.
    c.send(&body(
        ids::configuration::clientbound::FINISH_CONFIGURATION,
        &[],
    ))?;
    loop {
        let (id, payload) = c.recv()?;
        seen.lock().unwrap().config.push((id, payload));
        if id == ids::configuration::serverbound::FINISH_CONFIGURATION {
            break;
        }
    }
    c.send(&body(ids::play::clientbound::LOGIN, b"login"))?;
    loop {
        let (id, payload) = c.recv()?;
        seen.lock().unwrap().play.push((id, payload.clone()));
        if id == ECHO {
            c.send(&body(ECHOED, &payload))?;
        }
    }
}

/// A client's whole join through the proxy at `port`, up to the game's Login packet; the connection is kept.
pub fn join(port: u16, name: &str, said: &[(i32, Vec<u8>)]) -> Connection {
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: wire::PROTOCOL_VERSION,
            host: "localhost".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.into(),
            uuid: 0,
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, payload) = c.recv().unwrap();
        match login::Clientbound::decode(id, &payload).unwrap() {
            login::Clientbound::Compression(t) => c.set_compression(t.threshold),
            login::Clientbound::LoginFinished(done) => {
                assert_eq!(done.profile.name, name);
                break;
            }
            other => panic!("{other:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(wire::Empty).encode())
        .unwrap();
    for (id, payload) in said {
        c.send(&body(*id, payload)).unwrap();
    }
    // Configuration ends when the game says so; the client then finishes, and play starts with the Login.
    loop {
        let (id, _) = c.recv().unwrap();
        if id == ids::configuration::clientbound::FINISH_CONFIGURATION {
            break;
        }
    }
    c.send(&body(
        ids::configuration::serverbound::FINISH_CONFIGURATION,
        &[],
    ))
    .unwrap();
    let (id, payload) = c.recv().unwrap();
    assert_eq!(
        (id, payload.as_slice()),
        (ids::play::clientbound::LOGIN, &b"login"[..])
    );
    c
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn proxy_for(game: &FakeGame, dir: &Path) -> (Proxy, u16) {
    let mut config = Config::default();
    config.server.online_mode = false;
    config.server.bind = "127.0.0.1".into();
    config.server.port = free_port();
    config.proxy.game_port = game.port;
    let proxy = start(dir, &config, |_, _| {}).unwrap();
    (proxy, config.server.port)
}

pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("horizon-proxy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_player_joins_chats_and_the_game_sees_who_they_are() {
    let dir = temp_dir("join");
    // The stand-in must know the secret before it starts: the proxy keeps the one it makes.
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    proxy.set_game_up(true);

    let mut c = join(
        port,
        "Steve",
        &[(
            ids::configuration::serverbound::CLIENT_INFORMATION,
            vec![1, 2, 3],
        )],
    );
    c.send(&body(ECHO, b"hello")).unwrap();
    let (id, payload) = c.recv().unwrap();
    assert_eq!((id, payload.as_slice()), (ECHOED, &b"hello"[..]));

    let seen = game.seen.lock().unwrap();
    let (name, uuid, address) = &seen.logins[0];
    assert_eq!(name, "Steve");
    // Offline mode: the UUID of the name, which is the real one for an offline server.
    assert_eq!(*uuid, wire::offline_uuid("Steve"));
    assert_eq!(address, "127.0.0.1");
    assert!(seen.config.iter().any(|(id, p)| *id
        == ids::configuration::serverbound::CLIENT_INFORMATION
        && p == &[1, 2, 3]));
    assert_eq!(seen.play.last().unwrap(), &(ECHO, b"hello".to_vec()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_server_list_is_the_games_and_says_restarting_while_it_is_down() {
    let dir = temp_dir("status");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    let ask = || {
        let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: wire::PROTOCOL_VERSION,
                host: "localhost".into(),
                port,
                intent: handshake::STATUS,
            })
            .encode(),
        )
        .unwrap();
        c.send(&status::Serverbound::StatusRequest(wire::Empty).encode())
            .unwrap();
        let (id, payload) = c.recv().unwrap();
        let status::Clientbound::StatusResponse(r) =
            status::Clientbound::decode(id, &payload).unwrap()
        else {
            panic!()
        };
        c.send(&status::Serverbound::PingRequest(status::Ping { time: 42 }).encode())
            .unwrap();
        let (id, payload) = c.recv().unwrap();
        assert!(
            matches!(status::Clientbound::decode(id, &payload).unwrap(), status::Clientbound::PongResponse(p) if p.time == 42)
        );
        r.json
    };
    assert!(ask().contains("restarting"), "down");
    proxy.set_game_up(true);
    assert!(ask().contains("fake game"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_proxy_keeps_the_server_list_disabled() {
    let dir = temp_dir("status-disabled");
    let mut config = Config::default();
    config.server.online_mode = false;
    config.server.bind = "127.0.0.1".into();
    config.server.port = free_port();
    config
        .properties
        .insert("enable-status".into(), false.into());
    let proxy = start(&dir, &config, |_, _| {}).unwrap();
    for up in [false, true] {
        proxy.set_game_up(up);
        let mut client =
            Connection::connect("127.0.0.1", config.server.port, Duration::from_secs(5)).unwrap();
        client
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client
            .send(
                &handshake::Serverbound::Intention(handshake::Intention {
                    protocol: wire::PROTOCOL_VERSION,
                    host: "localhost".into(),
                    port: config.server.port,
                    intent: handshake::STATUS,
                })
                .encode(),
            )
            .unwrap();
        let _ = client.send(&status::Serverbound::StatusRequest(wire::Empty).encode());
        assert!(client.recv().is_err());
    }
    let _ = std::fs::remove_dir_all(&dir);
}
