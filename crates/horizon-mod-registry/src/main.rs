//! `horizon-mod-registry`: serves on `HOST`:`PORT` (default 0.0.0.0:8080). Configured by the
//! environment (see `Config::from_env`); keeps the ratings in `REGISTRY_DB`.
use std::net::SocketAddr;

use horizon_mod_registry::{App, Config, router};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let config = Config::from_env().unwrap_or_else(|e| panic!("horizon-mod-registry: {e}"));
    let repo = format!("{} as {}", config.github.repo, config.github.bot());
    // `HOST`: the address to listen on (127.0.0.1 on the host, where only the Cloudflare Tunnel reaches it).
    let host: std::net::IpAddr = std::env::var("HOST")
        .ok()
        .and_then(|h| h.parse().ok())
        .unwrap_or(std::net::IpAddr::from([0, 0, 0, 0]));
    let listener = tokio::net::TcpListener::bind(SocketAddr::new(host, port))
        .await
        .unwrap_or_else(|e| panic!("{host}:{port}: {e}"));
    eprintln!("horizon-mod-registry: listening on {port}, submitting to {repo}");
    let app = App::new(config).unwrap_or_else(|e| panic!("horizon-mod-registry: {e}"));
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
