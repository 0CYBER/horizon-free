//! `horizon-mod-registry`: serves on `PORT` (default 8080). Configured by the
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
    let repo = config.github.repo.clone();
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)))
        .await
        .unwrap_or_else(|e| panic!("port {port}: {e}"));
    eprintln!("horizon-mod-registry: listening on {port}, submitting to {repo}");
    let app = App::new(config).unwrap_or_else(|e| panic!("horizon-mod-registry: {e}"));
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"));
}
