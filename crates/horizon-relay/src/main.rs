//! `horizon-relay`: listens on `$PORT` (7000 when unset), limits from the
//! `RELAY_*` variables (docs/play-with-friends.md). Prints counts every five
//! minutes, nothing else.
use horizon_relay::{Config, Relay};
use std::{net::TcpListener, time::Duration};

fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or(7000);
    // Dual-stack where the system allows it (Railway's private network is
    // IPv6), else IPv4.
    let listener = TcpListener::bind(("::", port))
        .or_else(|_| TcpListener::bind(("0.0.0.0", port)))
        .unwrap_or_else(|e| {
            eprintln!("horizon-relay: can't listen on port {port}: {e}");
            std::process::exit(1);
        });
    let config = Config::from_env();
    println!(
        "horizon-relay {} listening on port {port} (hosts {}, bridges {}, {} KiB/s per stream, per-address limits {})",
        env!("CARGO_PKG_VERSION"),
        config.max_hosts,
        config.max_bridges,
        (config.session_rate / 1024.0) as u64,
        if config.per_address { "on" } else { "off" },
    );
    let relay = Relay::new(config);
    let r = relay.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(300));
            println!("{}", r.counts().json());
        }
    });
    relay.serve(listener);
}
