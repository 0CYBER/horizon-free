//! The proxy against a stand-in game: a listener that does the game's side of the forwarded login, its
//! configuration and a play that echoes, on real sockets.

use super::{
    replay::body,
    wire::{Connection, Reader, handshake, ids, login, status},
    *,
};
use std::{
    net::{TcpListener, TcpStream},
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
    streams: Arc<Mutex<Vec<TcpStream>>>,
    stopped: Arc<std::sync::atomic::AtomicBool>,
    listener: Mutex<Option<thread::JoinHandle<()>>>,
}

/// A game that forwards-logs players in with `secret`, shows its status and echoes play packets of id
/// `ECHO` back as `ECHOED`.
pub const ECHO: i32 = ids::play::serverbound::CHAT_COMMAND;
pub const ECHOED: i32 = ids::play::clientbound::SYSTEM_CHAT;

impl FakeGame {
    pub fn start(secret: &[u8]) -> Self {
        Self::start_on(0, secret)
    }

    /// On `port` (0: any), as a game restarted where the last one was.
    pub fn start_on(port: u16, secret: &[u8]) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let streams: Arc<Mutex<Vec<TcpStream>>> = Arc::default();
        let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (s, secret, open, off) = (
            seen.clone(),
            secret.to_vec(),
            streams.clone(),
            stopped.clone(),
        );
        let listener = thread::spawn(move || {
            for stream in listener.incoming() {
                if off.load(std::sync::atomic::Ordering::SeqCst) {
                    return;
                }
                let (s, secret) = (s.clone(), secret.clone());
                let Ok(stream) = stream else { continue };
                open.lock().unwrap().push(stream.try_clone().unwrap());
                thread::spawn(move || {
                    let _ = serve(stream, &s, &secret);
                });
            }
        });
        Self {
            port,
            seen,
            streams,
            stopped,
            listener: Mutex::new(Some(listener)),
        }
    }

    /// The game ends: every connection closes without a word, as a crash does.
    pub fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(listener) = self.listener.lock().unwrap().take() {
            let _ = TcpStream::connect(("127.0.0.1", self.port));
            listener.join().unwrap();
        }
        for s in self.streams.lock().unwrap().drain(..) {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    /// Every player is kicked with `server_shutdown`, as a stopping game does, then the game ends.
    pub fn kick_and_stop(&self) {
        let mut w = wire::Writer::new();
        w.nbt(&wire::translated(
            "multiplayer.disconnect.server_shutdown",
            "Server closed",
        ));
        let packet = body(ids::play::clientbound::DISCONNECT, &w.data);
        let mut frame = Vec::new();
        wire::write_varint(&mut frame, packet.len() as i32);
        frame.extend_from_slice(&packet);
        for s in self.streams.lock().unwrap().iter_mut() {
            let _ = std::io::Write::write_all(s, &frame);
        }
        thread::sleep(Duration::from_millis(50));
        self.stop();
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
    proxy_holding(game, dir, 60)
}

pub fn proxy_holding(game: &FakeGame, dir: &Path, hold_seconds: u32) -> (Proxy, u16) {
    let mut config = Config::default();
    config.proxy.hold_seconds = hold_seconds;
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
    proxy.game_up();

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
    proxy.game_up();
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
        if up {
            proxy.game_up();
        }
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

// ---- a restart of the game, with the player on the same connection -----------------------------------

use super::replay::Phase;

/// A client that answers like the game's own: configuration's keep-alives, StartConfiguration and the end of
/// configuration. It is one `Connection` from its join to the end: one TCP connection.
struct Player {
    c: Connection,
    state: Phase,
    keep_alives: usize,
}

impl Player {
    fn join(port: u16, name: &str, said: &[(i32, Vec<u8>)]) -> Self {
        Self {
            c: join(port, name, said),
            state: Phase::Play,
            keep_alives: 0,
        }
    }

    /// Logged in and in configuration, with no game behind the proxy yet.
    fn login_only(port: u16, name: &str) -> Self {
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
                login::Clientbound::LoginFinished(_) => break,
                other => panic!("{other:?}"),
            }
        }
        c.send(&login::Serverbound::LoginAcknowledged(wire::Empty).encode())
            .unwrap();
        Self {
            c,
            state: Phase::Config,
            keep_alives: 0,
        }
    }

    /// The next packet the game's side sends, after what the client does on its own for it.
    fn next(&mut self) -> (i32, Vec<u8>) {
        loop {
            let (id, payload) = self
                .c
                .recv()
                .unwrap_or_else(|e| panic!("the connection ended: {e}"));
            match (self.state, id) {
                (Phase::Config, ids::configuration::clientbound::KEEP_ALIVE) => {
                    self.keep_alives += 1;
                    self.c
                        .send(&body(ids::configuration::serverbound::KEEP_ALIVE, &payload))
                        .unwrap();
                    continue;
                }
                (Phase::Config, ids::configuration::clientbound::FINISH_CONFIGURATION) => {
                    self.c
                        .send(&body(
                            ids::configuration::serverbound::FINISH_CONFIGURATION,
                            &[],
                        ))
                        .unwrap();
                    self.state = Phase::Play;
                }
                (Phase::Play, ids::play::clientbound::START_CONFIGURATION) => {
                    self.c
                        .send(&body(
                            ids::play::serverbound::CONFIGURATION_ACKNOWLEDGED,
                            &[],
                        ))
                        .unwrap();
                    self.state = Phase::Config;
                }
                _ => {}
            }
            return (id, payload);
        }
    }

    /// Reads (and answers) for `time`, returning what else arrived.
    fn wait(&mut self, time: Duration) -> Vec<(i32, Vec<u8>)> {
        self.c
            .reader
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let (end, mut got) = (std::time::Instant::now() + time, Vec::new());
        while std::time::Instant::now() < end {
            match self.c.reader.recv() {
                Ok((id, payload)) => {
                    let keep = (self.state == Phase::Config
                        && id == ids::configuration::clientbound::KEEP_ALIVE)
                        .then(|| {
                            self.keep_alives += 1;
                            self.c
                                .send(&body(ids::configuration::serverbound::KEEP_ALIVE, &payload))
                                .unwrap();
                        });
                    if keep.is_none() {
                        got.push((id, payload));
                    }
                }
                Err(e) if e.is_timed_out() => {}
                Err(e) => panic!("the connection ended: {e}"),
            }
        }
        self.c
            .reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        got
    }

    fn until(&mut self, id: i32) -> Vec<i32> {
        let mut seen = Vec::new();
        loop {
            let (got, _) = self.next();
            seen.push(got);
            if got == id {
                return seen;
            }
        }
    }

    fn echo(&mut self, text: &[u8]) {
        self.c.send(&body(ECHO, text)).unwrap();
        let (id, payload) = self.next();
        assert_eq!((id, payload.as_slice()), (ECHOED, text));
    }
}

fn restart_in_place(how: &str) {
    let dir = temp_dir(&format!("restart-{how}"));
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    proxy.game_up();
    let mut player = Player::join(
        port,
        "Steve",
        &[(
            ids::configuration::serverbound::CLIENT_INFORMATION,
            vec![1, 2, 3],
        )],
    );
    player
        .c
        .send(&body(ids::play::serverbound::CHAT_SESSION_UPDATE, &[5; 8]))
        .unwrap();
    player.echo(b"before");

    match how {
        // A planned restart: the supervisor says so first, then the game kicks everyone as it stops.
        "planned" => {
            proxy.game_stopping();
            assert_eq!(player.next().0, ids::play::clientbound::START_CONFIGURATION);
            game.kick_and_stop();
        }
        // The game's shutdown notice can reach a client before the supervisor observes the exit.
        "kick" => game.kick_and_stop(),
        // A crash: the connection to the game just ends.
        _ => game.stop(),
    }
    if how != "planned" {
        assert_eq!(player.next().0, ids::play::clientbound::START_CONFIGURATION);
    }
    proxy.game_exited(false);
    // Held: kept alive by the proxy, and never sent the game's kick or a disconnect.
    let during = player.wait(Duration::from_millis(900));
    assert!(
        player.keep_alives >= 2,
        "kept alive while the game is down ({})",
        player.keep_alives
    );
    assert!(
        !during
            .iter()
            .any(|(id, _)| *id == ids::configuration::clientbound::DISCONNECT
                || *id == ids::play::clientbound::DISCONNECT),
        "no disconnect: {during:?}"
    );

    // The game is back, where it was.
    let again = FakeGame::start_on(game.port, &secret);
    proxy.game_up();
    let seen = player.until(ids::play::clientbound::LOGIN);
    assert!(
        seen.contains(&ids::configuration::clientbound::FINISH_CONFIGURATION),
        "{seen:?}"
    );
    // Same player, same connection: back in play, and the game was told what the client had said.
    player.echo(b"after");
    let seen = again.seen.lock().unwrap();
    assert_eq!(seen.logins.len(), 1);
    assert_eq!(
        (seen.logins[0].0.as_str(), seen.logins[0].1),
        ("Steve", wire::offline_uuid("Steve"))
    );
    assert!(seen.config.iter().any(|(id, p)| *id
        == ids::configuration::serverbound::CLIENT_INFORMATION
        && p == &[1, 2, 3]));
    assert!(
        seen.play
            .iter()
            .any(|(id, p)| *id == ids::play::serverbound::CHAT_SESSION_UPDATE && p == &[5; 8])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_planned_restart_holds_the_player_on_the_same_connection_and_puts_them_back() {
    restart_in_place("planned");
}

#[test]
fn the_games_own_shutdown_kick_is_a_hold_not_a_disconnect() {
    restart_in_place("kick");
}

#[test]
fn a_crash_holds_the_player_too() {
    restart_in_place("crash");
}

#[test]
fn a_restart_waits_for_configuration_already_sent_to_the_client() {
    let dir = temp_dir("finish-in-flight");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    proxy.game_up();
    let mut player = Player::login_only(port, "Alex");
    while player.c.recv().unwrap().0 != ids::configuration::clientbound::FINISH_CONFIGURATION {}

    proxy.game_stopping();
    game.stop();
    let again = FakeGame::start_on(game.port, &secret);
    proxy.game_up();

    player
        .c
        .send(&body(
            ids::configuration::serverbound::FINISH_CONFIGURATION,
            &[],
        ))
        .unwrap();
    player.state = Phase::Play;
    assert_eq!(player.next().0, ids::play::clientbound::START_CONFIGURATION);
    player.until(ids::play::clientbound::LOGIN);
    player.echo(b"resumed");
    again.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_player_leaving_does_not_hold_other_players() {
    let dir = temp_dir("leave");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (left, departed) = std::sync::mpsc::channel();
    let mut config = Config::default();
    config.server.online_mode = false;
    config.server.bind = "127.0.0.1".into();
    config.server.port = free_port();
    config.proxy.game_port = game.port;
    let proxy = start(&dir, &config, move |_, line| {
        if line == "Alex left through the proxy" {
            let _ = left.send(());
        }
    })
    .unwrap();
    proxy.game_up();
    let first = Player::join(config.server.port, "Alex", &[]);
    let mut second = Player::join(config.server.port, "Steve", &[]);
    first.c.writer.shutdown();
    departed.recv_timeout(Duration::from_secs(5)).unwrap();
    second.echo(b"still playing");
    game.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_player_who_comes_while_the_game_is_down_is_held_until_it_is_up() {
    let dir = temp_dir("join-down");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    // The game has not said it is up yet.
    let mut player = Player::login_only(port, "Alex");
    player
        .c
        .send(&body(
            ids::configuration::serverbound::CLIENT_INFORMATION,
            &[7],
        ))
        .unwrap();
    player.wait(Duration::from_millis(700));
    assert!(player.keep_alives >= 2);
    proxy.game_up();
    player.until(ids::play::clientbound::LOGIN);
    player.echo(b"in");
    // What it said while it waited reached the game.
    assert!(
        game.seen
            .lock()
            .unwrap()
            .config
            .iter()
            .any(|(id, p)| *id == ids::configuration::serverbound::CLIENT_INFORMATION && p == &[7])
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_game_that_never_comes_back_ends_with_a_translated_disconnect() {
    let dir = temp_dir("timeout");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_holding(&game, &dir, 1);
    proxy.game_up();
    let mut player = Player::join(port, "Steve", &[]);
    proxy.game_stopping();
    game.kick_and_stop();
    assert_eq!(player.next().0, ids::play::clientbound::START_CONFIGURATION);
    let (id, payload) = player.next();
    assert_eq!(id, ids::configuration::clientbound::DISCONNECT);
    let text = String::from_utf8_lossy(&payload);
    assert!(text.contains("horizon.proxy.restart_timed_out"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stop_for_good_reaches_the_player_as_the_games_kick() {
    let dir = temp_dir("closing");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    proxy.game_up();
    let mut player = Player::join(port, "Steve", &[]);
    proxy.closing();
    game.kick_and_stop();
    let (id, payload) = player.next();
    assert_eq!(id, ids::play::clientbound::DISCONNECT);
    assert!(String::from_utf8_lossy(&payload).contains("server_shutdown"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// How many of `streams` the proxy still holds open: a closed one reads end-of-stream at once.
fn held_open(streams: &mut [TcpStream]) -> usize {
    streams
        .iter_mut()
        .filter_map(|s| {
            s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
            let mut byte = [0u8; 1];
            matches!(
                std::io::Read::read(s, &mut byte),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut
            )
            .then_some(())
        })
        .count()
}

#[test]
fn silent_connections_are_closed_past_the_most_the_proxy_holds_before_login() {
    let dir = temp_dir("flood");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (_proxy, port) = proxy_for(&game, &dir);
    // The player limit plus the 64 spare places, worked out here and not read back from the proxy.
    let most = Config::default().server.max_players as usize + 64;
    let mut silent: Vec<TcpStream> = (0..most + 100)
        .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
        .collect();
    thread::sleep(Duration::from_millis(500));
    assert_eq!(held_open(&mut silent), most);
}

#[test]
fn a_player_who_logged_in_no_longer_holds_a_place_before_login() {
    let dir = temp_dir("place");
    let secret = super::secret(&dir).unwrap();
    let game = FakeGame::start(&secret);
    let (proxy, port) = proxy_for(&game, &dir);
    proxy.game_up();
    let _player = join(port, "Alex", &[]);
    let open = &proxy.inner.admission.open;
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while open.load(Ordering::SeqCst) != 0 && std::time::Instant::now() < end {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(open.load(Ordering::SeqCst), 0);
}
