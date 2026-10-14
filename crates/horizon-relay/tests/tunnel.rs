//! A relay, a host tunnel in front of an echo server standing in for the
//! integrated server, and guest tunnels, all on loopback in one process.
use horizon_friends::{
    Code, Host, HostConfig, HostEvent, JoinError, PathMode,
    guest::{self, GuestConfig},
};
use horizon_relay::{Config, Relay, limits::Rates};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    time::{Duration, Instant},
};

const PROTOCOL: i32 = 777;

/// Echoes every connection back until it closes.
fn echo_server() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { continue };
            std::thread::spawn(move || {
                let mut out = s.try_clone().unwrap();
                let _ = std::io::copy(&mut s, &mut out);
            });
        }
    });
    a
}

fn relay(config: Config) -> (Relay, String) {
    let (relay, address, _) = Relay::spawn_local(config).unwrap();
    (relay, address.to_string())
}

fn generous() -> Config {
    Config {
        rates: Rates {
            join_per_min: 600.0,
            join_burst: 100.0,
            ..Rates::default()
        },
        ..Config::default()
    }
}

fn host(relay: &str, lan: SocketAddr, path: PathMode) -> (Host, Code) {
    let host = Host::start(HostConfig {
        relay: relay.to_owned(),
        lan,
        protocol: PROTOCOL,
        game: "26.3".into(),
        path,
    })
    .unwrap();
    let code = wait_event(&host, |e| match e {
        HostEvent::Code(c) => Some(c.clone()),
        _ => None,
    });
    (host, code)
}

thread_local! {
    /// Events polled but not yet waited for.
    static BACKLOG: std::cell::RefCell<Vec<HostEvent>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The first event (in order) that `f` picks; the others stay queued.
fn wait_event<T>(host: &Host, mut f: impl FnMut(&HostEvent) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let found = BACKLOG.with_borrow_mut(|backlog| {
            backlog.extend(host.poll());
            let i = backlog.iter().position(|e| f(e).is_some())?;
            f(&backlog.remove(i))
        });
        if let Some(t) = found {
            return t;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no such host event");
}

fn join(relay: &str, code: &Code, path: PathMode) -> Result<guest::Tunnel, JoinError> {
    guest::join(&GuestConfig {
        relay: relay.to_owned(),
        code: code.clone(),
        protocol: PROTOCOL,
        game: "26.3".into(),
        path,
    })
}

/// What the game's client does: connect to the loopback port, send, read
/// the echo back.
fn round_trip(tunnel: guest::Tunnel, payload: &[u8]) {
    let (port, done) = tunnel.serve_loopback().unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut w = s.try_clone().unwrap();
    let send = payload.to_vec();
    let writer = std::thread::spawn(move || w.write_all(&send).unwrap());
    let mut got = vec![0u8; payload.len()];
    s.read_exact(&mut got).unwrap();
    writer.join().unwrap();
    assert!(got == payload, "echo differs");
    drop(s);
    done.join().unwrap();
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
}

#[test]
fn the_direct_path_carries_the_stream() {
    let (relay, address) = relay(generous());
    let (host, code) = host(&address, echo_server(), PathMode::Auto);
    assert_eq!(code.nameplate.len(), 4);
    let tunnel = join(&address, &code, PathMode::Auto).unwrap();
    assert!(
        tunnel.direct,
        "loopback ends should reach each other directly"
    );
    let data = payload(1 << 20);
    round_trip(tunnel, &data);
    assert!(wait_event(&host, |e| match e {
        HostEvent::Connected { direct, .. } => Some(*direct),
        _ => None,
    }));
    // Only the handshake went through the relay.
    let counts = relay.counts();
    assert!(counts.relayed_bytes < 16 * 1024, "{counts:?}");
    assert_eq!(counts.hosts, 1);
    wait_event(&host, |e| {
        matches!(e, HostEvent::Disconnected { .. }).then_some(())
    });
    assert_eq!(host.sessions(), 0);
}

#[test]
fn the_relay_path_carries_the_stream() {
    let (relay, address) = relay(generous());
    let (host, code) = host(&address, echo_server(), PathMode::Auto);
    let tunnel = join(&address, &code, PathMode::RelayOnly).unwrap();
    assert!(!tunnel.direct);
    let data = payload(1 << 20);
    round_trip(tunnel, &data);
    assert!(!wait_event(&host, |e| match e {
        HostEvent::Connected { direct, .. } => Some(*direct),
        _ => None,
    }));
    // Both directions, encrypted and framed: a little more than twice.
    let relayed = relay.counts().relayed_bytes;
    assert!(relayed > 2 * data.len() as u64, "{relayed}");
    // Two guests at once, one per path.
    let a = join(&address, &code, PathMode::RelayOnly).unwrap();
    let b = join(&address, &code, PathMode::Auto).unwrap();
    let ta = std::thread::spawn(move || round_trip(a, &payload(50_000)));
    round_trip(b, &payload(70_000));
    ta.join().unwrap();
}

#[test]
fn the_host_can_refuse_direct_connections() {
    let (_relay, address) = relay(generous());
    let (_host, code) = host(&address, echo_server(), PathMode::RelayOnly);
    let tunnel = join(&address, &code, PathMode::Auto).unwrap();
    assert!(!tunnel.direct);
    round_trip(tunnel, b"relay only");
    // A guest that insists on a direct path gets none.
    assert!(matches!(
        join(&address, &code, PathMode::DirectOnly),
        Err(JoinError::Failed(_))
    ));
}

#[test]
fn wrong_secrets_fail_and_retire_the_code() {
    let (_relay, address) = relay(generous());
    let (host, code) = host(&address, echo_server(), PathMode::Auto);
    let wrong = Code {
        nameplate: code.nameplate.clone(),
        secret: if code.secret == "0000" {
            "0001".into()
        } else {
            "0000".into()
        },
    };
    for i in 1..=horizon_friends::host::MAX_WRONG {
        let e = join(&address, &wrong, PathMode::Auto).err().unwrap();
        assert!(matches!(e, JoinError::WrongCode), "{e}");
        let count = wait_event(&host, |e| match e {
            HostEvent::WrongCode { count } => Some(*count),
            _ => None,
        });
        assert_eq!(count, i);
    }
    // A new code replaces the old one, which has ended.
    let fresh = wait_event(&host, |e| match e {
        HostEvent::Code(c) => Some(c.clone()),
        _ => None,
    });
    assert_ne!(fresh.nameplate, code.nameplate);
    assert!(matches!(
        join(&address, &code, PathMode::Auto),
        Err(JoinError::Ended)
    ));
    round_trip(join(&address, &fresh, PathMode::Auto).unwrap(), b"fresh");
}

#[test]
fn unknown_codes_versions_and_ended_invites() {
    let (relay, address) = relay(generous());
    let (mut host, code) = host(&address, echo_server(), PathMode::Auto);
    let other = guest::join(&GuestConfig {
        relay: address.clone(),
        code: code.clone(),
        protocol: PROTOCOL - 1,
        game: "26.2".into(),
        path: PathMode::Auto,
    });
    match other {
        Err(e @ JoinError::VersionMismatch { .. }) => {
            assert_eq!(
                e.to_string(),
                "Your friend plays Minecraft 26.3; you have 26.2."
            )
        }
        other => panic!("{:?}", other.err()),
    }
    host.stop();
    assert!(matches!(
        join(&address, &code, PathMode::Auto),
        Err(JoinError::Ended)
    ));
    // A nameplate nobody holds; five of them block this address for a while.
    let free = (0..)
        .map(|i| horizon_friends::code::nameplate(i * 7919))
        .find(|n| *n != code.nameplate)
        .unwrap();
    let unknown = Code {
        nameplate: free,
        secret: "0000".into(),
    };
    for _ in 0..5 {
        assert!(matches!(
            join(&address, &unknown, PathMode::Auto),
            Err(JoinError::UnknownCode)
        ));
    }
    assert!(matches!(
        join(&address, &code, PathMode::Auto),
        Err(JoinError::RateLimited)
    ));
    let counts = relay.counts();
    assert_eq!(counts.unknown, 5);
    assert!(counts.rate_limited >= 1);
    assert_eq!(counts.hosts, 0);
}

#[test]
fn a_host_that_never_attaches_is_unreachable() {
    use horizon_friends::wire::{self, FromRelay, ToRelay};
    let (_relay, address) = relay(Config {
        attach_wait: Duration::from_millis(500),
        ..generous()
    });
    let mut control = TcpStream::connect(&address).unwrap();
    wire::send(
        &mut control,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: PROTOCOL,
            game: "26.3".into(),
        },
    )
    .unwrap();
    let FromRelay::Registered {
        nameplate, token, ..
    } = wire::recv(&mut control).unwrap()
    else {
        panic!()
    };
    let code = Code {
        nameplate,
        secret: "0000".into(),
    };
    assert!(matches!(
        join(&address, &code, PathMode::Auto),
        Err(JoinError::HostUnreachable)
    ));
    // The relay announced the session; attaching late or with another token fails.
    let FromRelay::Join { session } = wire::recv(&mut control).unwrap() else {
        panic!()
    };
    let mut late = TcpStream::connect(&address).unwrap();
    wire::send(
        &mut late,
        &ToRelay::Attach {
            v: wire::VERSION,
            session,
            token,
        },
    )
    .unwrap();
    assert!(matches!(
        wire::recv::<_, FromRelay>(&mut late).unwrap(),
        FromRelay::Error { .. }
    ));
}

#[test]
fn relayed_bandwidth_is_capped() {
    let rate = 200_000.0;
    let (_relay, address) = relay(Config {
        session_rate: rate,
        ..generous()
    });
    let (_host, code) = host(&address, echo_server(), PathMode::RelayOnly);
    let tunnel = join(&address, &code, PathMode::RelayOnly).unwrap();
    let started = Instant::now();
    round_trip(tunnel, &payload(600_000));
    // The first 100 kB pass at once, the rest at 200 kB/s.
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn health_and_whoami_answer_http() {
    let (_relay, address) = relay(Config::default());
    let get = |path: &str| {
        let mut s = TcpStream::connect(&address).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: relay\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    };
    let health = get("/health");
    assert!(health.starts_with("HTTP/1.1 200 OK\r\n"), "{health}");
    assert!(health.ends_with("{\"ok\":true,\"hosts\":0,\"bridges\":0,\"registrations\":0,\"joins\":0,\"unknown\":0,\"rate_limited\":0,\"relayed_bytes\":0}"));
    assert!(get("/whoami").contains("\"address\":\"127.0.0.1:"));
    assert!(get("/").starts_with("HTTP/1.1 404"));
}
