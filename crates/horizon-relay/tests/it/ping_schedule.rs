//! A host that answers its pings stays registered however often guests join: a join line sent to the
//! host must not push the next ping back (docs/play-with-friends.md: pings every 20 s, answered within 60 s).
use horizon_friends::wire::{self, ErrorCode, FromRelay, ToRelay};
use horizon_relay::{Config, Relay, limits::Rates};
use std::{
    net::TcpStream,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[test]
#[ignore = "daily"]
fn guest_joins_do_not_starve_a_hosts_pings() {
    // The real 20 s ping and 60 s silence, scaled down 100 times and 60 times: a ping is due every 200 ms
    // and a host silent for 1 s is gone, so joins every ~150 ms would hold the old schedule off for good.
    let (relay, address, _) = Relay::spawn_local(Config {
        ping_every: Duration::from_millis(200),
        host_timeout: Duration::from_secs(1),
        attach_wait: Duration::from_millis(100),
        per_address: false,
        rates: Rates {
            register_per_min: 600.0,
            register_burst: 100.0,
            join_per_min: 60_000.0,
            join_burst: 10_000.0,
            ..Rates::default()
        },
        ..Config::default()
    })
    .unwrap();

    // A host that answers every ping and ignores the joins it is told about (so each guest is refused
    // `host_unreachable` after `attach_wait`).
    let mut host = TcpStream::connect(address).unwrap();
    host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let hello = ToRelay::Host {
        v: wire::VERSION,
        protocol: 777,
        game: "26.3".into(),
    };
    wire::send(&mut host, &hello).unwrap();
    let nameplate = match wire::recv::<_, FromRelay>(&mut host).unwrap() {
        FromRelay::Registered { nameplate, .. } => nameplate,
        other => panic!("unexpected answer {other:?}"),
    };
    let pings = Arc::new(AtomicUsize::new(0));
    let listener = {
        let (pings, mut host) = (pings.clone(), host.try_clone().unwrap());
        std::thread::spawn(move || {
            while let Ok(line) = wire::recv::<_, FromRelay>(&mut host) {
                if line == FromRelay::Ping {
                    pings.fetch_add(1, Ordering::Relaxed);
                    if wire::send(&mut host, &ToRelay::Pong).is_err() {
                        break;
                    }
                }
            }
        })
    };

    // Guests join every ~150 ms for 3 s: each join line goes to the host.
    let started = Instant::now();
    let mut joins = 0;
    while started.elapsed() < Duration::from_secs(3) {
        let mut guest = TcpStream::connect(address).unwrap();
        guest
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let join = ToRelay::Join {
            v: wire::VERSION,
            nameplate: nameplate.clone(),
            protocol: 777,
            game: "26.3".into(),
        };
        wire::send(&mut guest, &join).unwrap();
        match wire::recv::<_, FromRelay>(&mut guest).unwrap() {
            FromRelay::Error {
                code: ErrorCode::HostUnreachable,
                ..
            } => {}
            other => panic!("join {joins} after {:?}: {other:?}", started.elapsed()),
        }
        joins += 1;
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(joins >= 15, "{joins} joins");
    // The host answered every ping, so it is still registered and was pinged all along.
    assert_eq!(relay.counts().hosts, 1);
    assert!(
        pings.load(Ordering::Relaxed) >= 5,
        "{} pings in {:?}",
        pings.load(Ordering::Relaxed),
        started.elapsed()
    );
    let _ = host.shutdown(std::net::Shutdown::Both);
    let _ = listener.join();
    relay.stop();
}
