//! One address can't hold the relay's bridges: loopback is one address, so a host and its guests
//! here are the case of an address bridging to itself, and each bridge is an end at both sides.
use horizon_friends::wire::{self, ErrorCode, FromRelay, ToRelay};
use horizon_relay::{Config, Relay, limits::Rates};
use std::io::{BufReader, Read};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const PROTOCOL: i32 = 777;

fn connect(address: SocketAddr) -> TcpStream {
    let s = TcpStream::connect(address).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s
}

/// A registered host's control reader, its connection (kept open) and nameplate and token.
fn host(address: SocketAddr) -> (BufReader<TcpStream>, TcpStream, String, String) {
    let mut s = connect(address);
    wire::send(
        &mut s,
        &ToRelay::Host {
            v: wire::VERSION,
            protocol: PROTOCOL,
            game: "26.3".into(),
        },
    )
    .unwrap();
    let mut reader = BufReader::new(s.try_clone().unwrap());
    loop {
        match wire::recv_buffered::<_, FromRelay>(&mut reader).unwrap() {
            FromRelay::Registered {
                nameplate, token, ..
            } => return (reader, s, nameplate, token),
            FromRelay::Ping => {}
            other => panic!("{other:?}"),
        }
    }
}

/// One guest joining and the host attaching: the guest's answer, and the host's stream when it
/// was bridged.
fn pair(
    address: SocketAddr,
    control: &mut BufReader<TcpStream>,
    nameplate: &str,
    token: &str,
) -> (FromRelay, Option<TcpStream>, TcpStream) {
    let guest = join(address, nameplate);
    pair_joined(guest, address, control, token)
}

fn join(address: SocketAddr, nameplate: &str) -> TcpStream {
    let mut guest = connect(address);
    wire::send(
        &mut guest,
        &ToRelay::Join {
            v: wire::VERSION,
            nameplate: nameplate.into(),
            protocol: PROTOCOL,
            game: "26.3".into(),
        },
    )
    .unwrap();
    guest
}

/// A guest that was let in, and the host attaching.
fn pair_joined(
    mut guest: TcpStream,
    address: SocketAddr,
    control: &mut BufReader<TcpStream>,
    token: &str,
) -> (FromRelay, Option<TcpStream>, TcpStream) {
    let session = loop {
        match wire::recv_buffered::<_, FromRelay>(control).unwrap() {
            FromRelay::Join { session } => break session,
            FromRelay::Ping => {}
            other => panic!("{other:?}"),
        }
    };
    let mut attach = connect(address);
    wire::send(
        &mut attach,
        &ToRelay::Attach {
            v: wire::VERSION,
            session,
            token: token.into(),
        },
    )
    .unwrap();
    let answer = wire::recv::<_, FromRelay>(&mut guest).unwrap();
    let attach = matches!(answer, FromRelay::Bridged { .. }).then_some(attach);
    (answer, attach, guest)
}

fn settles_to_zero(relay: &Relay) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while relay.counts().bridges != 0 {
        assert!(Instant::now() < deadline, "bridges stuck");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "daily"]
fn an_address_bridging_to_itself_stops_at_its_cap_and_gets_places_back() {
    let (relay, address, _) = Relay::spawn_local(Config {
        rates: Rates {
            bridges_per_address: 4,
            ..Rates::default()
        },
        ..Config::default()
    })
    .unwrap();
    let (mut control, _keep, nameplate, token) = host(address);
    // Two bridges are four ends on this address; the global cap (200) is nowhere near.
    let mut bridged = Vec::new();
    for _ in 0..2 {
        let (answer, attach, guest) = pair(address, &mut control, &nameplate, &token);
        assert!(matches!(answer, FromRelay::Bridged { .. }), "{answer:?}");
        bridged.push((attach.unwrap(), guest));
    }
    // The third finds the address full, though the relay is not.
    let mut third = connect(address);
    wire::send(
        &mut third,
        &ToRelay::Join {
            v: wire::VERSION,
            nameplate: nameplate.clone(),
            protocol: PROTOCOL,
            game: "26.3".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        wire::recv::<_, FromRelay>(&mut third).unwrap(),
        FromRelay::Error {
            code: ErrorCode::Full,
            ..
        }
    ));
    assert_eq!(relay.counts().bridges, 2);
    // One bridge ends: its two ends are given back, and another guest gets in.
    let (attach, guest) = bridged.pop().unwrap();
    guest.shutdown(Shutdown::Both).unwrap();
    drop((attach, guest));
    let deadline = Instant::now() + Duration::from_secs(5);
    while relay.counts().bridges != 1 {
        assert!(Instant::now() < deadline, "the bridge never ended");
        std::thread::sleep(Duration::from_millis(10));
    }
    // The bridge count drops when the bridge returns, a moment before the host's end gives its
    // address place back, so a join in that gap is refused as full: ask again, within a deadline.
    let deadline = Instant::now() + Duration::from_secs(5);
    let guest = loop {
        let mut guest = join(address, &nameplate);
        guest
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        match wire::recv::<_, FromRelay>(&mut guest) {
            Ok(FromRelay::Error {
                code: ErrorCode::Full,
                ..
            }) => {
                assert!(Instant::now() < deadline, "the places never came back");
                std::thread::sleep(Duration::from_millis(20));
            }
            // Nothing yet: the guest was let in and waits for its host.
            Err(_) => {
                guest
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                break guest;
            }
            Ok(other) => panic!("{other:?}"),
        }
    };
    let (answer, attach, _guest) = pair_joined(guest, address, &mut control, &token);
    assert!(matches!(answer, FromRelay::Bridged { .. }), "{answer:?}");
    drop((attach, bridged));
    settles_to_zero(&relay);
}

#[test]
#[ignore = "daily"]
fn the_host_side_attach_connection_stays_counted_while_the_bridge_lives() {
    let (_relay, address, _) = Relay::spawn_local(Config {
        rates: Rates {
            open_per_address: 4,
            ..Rates::default()
        },
        ..Config::default()
    })
    .unwrap();
    // The host's control connection, the guest's and the host's attach connection: three of four.
    let (mut control, _keep, nameplate, token) = host(address);
    let (answer, attach, _guest) = pair(address, &mut control, &nameplate, &token);
    assert!(matches!(answer, FromRelay::Bridged { .. }), "{answer:?}");
    let _attach = attach.unwrap();
    let closed_at_once = |s: &mut TcpStream| {
        s.set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        match s.read(&mut [0u8; 1]) {
            Err(e) => !matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ),
            Ok(n) => n == 0,
        }
    };
    let mut fourth = connect(address);
    assert!(!closed_at_once(&mut fourth), "the fourth fits");
    let mut fifth = connect(address);
    assert!(closed_at_once(&mut fifth), "the fifth is over the cap");
}
