//! `horizon-telemetry-server`: serves on `PORT` (default 8080), keeps its data in
//! `HORIZON_TELEMETRY_DB` (default `telemetry.sqlite3`), and does its upkeep every ten minutes.
use std::net::SocketAddr;
use std::time::Duration;

use horizon_telemetry_server::{App, Config, router, store::Store};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let path = std::env::var("HORIZON_TELEMETRY_DB").unwrap_or_else(|_| "telemetry.sqlite3".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let store = Store::open(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let app = App::new(store, Config::from_env());
    let upkeep = app.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(600));
        loop {
            every.tick().await;
            if let Err(e) = upkeep.maintain() {
                eprintln!("horizon-telemetry-server: upkeep failed: {e}");
            }
            // The hub's reports GitHub refused before: their issues, off the async workers.
            let filing = upkeep.clone();
            let _ = tokio::task::spawn_blocking(move || filing.file_pending()).await;
        }
    });
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)))
        .await
        .unwrap_or_else(|e| panic!("port {port}: {e}"));
    eprintln!("horizon-telemetry-server: listening on {port}, data in {path}");
    axum::serve(
        listener,
        router(app).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .unwrap_or_else(|e| panic!("{e}"));
}
