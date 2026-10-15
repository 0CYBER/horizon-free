//! The ingest server for opt-in data collection (docs/telemetry.md).
//!
//! - `POST /v1/batch`: a zstd batch from the launcher, checked by `horizon_telemetry::batch`.
//! - `DELETE /v1/installs/<id>`: everything an install sent goes.
//! - `GET /v1/aggregate?days=30`: the anonymous totals the website reads, cached.
//! - `GET /healthz`.
//!
//! Abuse limits: a token bucket per IP address (kept in memory for an hour, never stored), a
//! daily item cap per install, and the batch size limits. Raw rows expire after
//! `retention_days`; the daily totals stay. One small process and one SQLite file: see
//! `README.md` for running it.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use horizon_telemetry::batch;

pub mod store;

use store::Store;

pub struct Config {
    /// Days raw rows are kept.
    pub retention_days: u32,
    /// Groups in the public totals need at least this many install-days.
    pub min_install_days: u64,
    /// Requests an IP address may make an hour (with a burst of a tenth of it).
    pub requests_per_hour: f64,
    /// Items one install may send a day.
    pub items_per_day: u32,
    /// Behind a proxy (Railway, Fly…): the client's address is the first of X-Forwarded-For.
    pub trust_proxy: bool,
    /// The website allowed to read the totals from a browser.
    pub allow_origin: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            retention_days: 180,
            min_install_days: 5,
            requests_per_hour: 60.0,
            items_per_day: 100,
            trust_proxy: false,
            allow_origin: "https://playhorizon.dev".into(),
        }
    }
}

impl Config {
    /// The defaults, with `HORIZON_TELEMETRY_RETENTION_DAYS`, `…_MIN_INSTALL_DAYS`,
    /// `…_REQUESTS_PER_HOUR`, `…_ITEMS_PER_DAY`, `…_TRUST_PROXY` (1) and `…_ALLOW_ORIGIN`.
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(format!("HORIZON_TELEMETRY_{k}")).ok();
        let d = Self::default();
        Self {
            retention_days: var("RETENTION_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.retention_days),
            min_install_days: var("MIN_INSTALL_DAYS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.min_install_days),
            requests_per_hour: var("REQUESTS_PER_HOUR")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.requests_per_hour),
            items_per_day: var("ITEMS_PER_DAY")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.items_per_day),
            trust_proxy: var("TRUST_PROXY").is_some_and(|v| v == "1"),
            allow_origin: var("ALLOW_ORIGIN").unwrap_or(d.allow_origin),
        }
    }
}

pub struct App {
    store: Mutex<Store>,
    config: Config,
    /// Tokens left and when they were counted, per client address.
    buckets: Mutex<HashMap<IpAddr, (f64, Instant)>>,
    /// `/v1/aggregate` answers by `days`, with when they were made.
    cache: Mutex<HashMap<u32, (Instant, String)>>,
    /// Today, in days since 1970 (a test moves it).
    today: Box<dyn Fn() -> u32 + Send + Sync>,
}

const CACHE_FOR: Duration = Duration::from_secs(600);

fn status(code: StatusCode, message: &str) -> Response {
    (code, message.to_owned()).into_response()
}

impl App {
    pub fn new(store: Store, config: Config) -> Arc<Self> {
        Self::with_clock(store, config, Box::new(horizon_telemetry::today))
    }

    pub fn with_clock(
        store: Store,
        config: Config,
        today: Box<dyn Fn() -> u32 + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(store),
            config,
            buckets: Mutex::default(),
            cache: Mutex::default(),
            today,
        })
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a token for `ip`; false when it has none left.
    fn allow(&self, ip: IpAddr) -> bool {
        let rate = self.config.requests_per_hour / 3600.0;
        let burst = (self.config.requests_per_hour / 10.0).max(1.0);
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let (tokens, at) = buckets.entry(ip).or_insert((burst, now));
        *tokens = (*tokens + now.duration_since(*at).as_secs_f64() * rate).min(burst);
        *at = now;
        if *tokens < 1.0 {
            return false;
        }
        *tokens -= 1.0;
        true
    }

    fn client(&self, headers: &HeaderMap, peer: Option<SocketAddr>) -> IpAddr {
        let forwarded = self
            .config
            .trust_proxy
            .then(|| {
                headers
                    .get("x-forwarded-for")?
                    .to_str()
                    .ok()?
                    .split(',')
                    .next()?
                    .trim()
                    .parse()
                    .ok()
            })
            .flatten();
        forwarded
            .or(peer.map(|p| p.ip()))
            .unwrap_or(IpAddr::from([0, 0, 0, 0]))
    }

    /// The day an install's newest row came in, if it has any left.
    pub fn newest_day(&self, install: &[u8]) -> Option<u32> {
        self.store().newest_day(install).ok().flatten()
    }

    /// Daily upkeep: totals rebuilt, expired rows removed, idle rate-limit entries forgotten.
    pub fn maintain(&self) -> rusqlite::Result<usize> {
        let oldest = (self.today)().saturating_sub(self.config.retention_days);
        let removed = self.store().expire(oldest)?;
        self.buckets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(3600));
        if removed > 0 {
            self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        Ok(removed)
    }
}

fn peer(req: &Request) -> Option<SocketAddr> {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
}

async fn receive(State(app): State<Arc<App>>, req: Request) -> Response {
    let ip = app.client(req.headers(), peer(&req));
    if !app.allow(ip) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let zstd = req
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"zstd"));
    if !zstd {
        return status(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "send zstd (Content-Encoding: zstd)",
        );
    }
    let Ok(body) = axum::body::to_bytes(req.into_body(), batch::MAX_COMPRESSED).await else {
        return status(StatusCode::PAYLOAD_TOO_LARGE, "a batch is at most 256 KiB");
    };
    let batch = match batch::decode(&body) {
        Ok(batch) => batch,
        Err(e) => return status(StatusCode::BAD_REQUEST, &e),
    };
    let today = (app.today)();
    let install = store::install_key(&batch.install_id).unwrap_or_default();
    let mut store = app.store();
    let sent = store.count_on(&install, today).unwrap_or(u32::MAX);
    if sent.saturating_add(batch.items.len() as u32) > app.config.items_per_day {
        return status(
            StatusCode::TOO_MANY_REQUESTS,
            "this install's daily limit is reached",
        );
    }
    match store.insert(&batch, today) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            eprintln!("horizon-telemetry-server: could not store a batch: {e}");
            status(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

async fn forget(State(app): State<Arc<App>>, Path(id): Path<String>, req: Request) -> Response {
    if !app.allow(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let Some(install) = store::install_key(&id) else {
        return status(StatusCode::BAD_REQUEST, "not an install ID");
    };
    match app.store().delete_install(&install) {
        Ok(_) => {
            app.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            eprintln!("horizon-telemetry-server: could not delete an install: {e}");
            status(StatusCode::SERVICE_UNAVAILABLE, "try again later")
        }
    }
}

#[derive(serde::Deserialize)]
struct Window {
    days: Option<u32>,
}

async fn aggregate(State(app): State<Arc<App>>, Query(window): Query<Window>) -> Response {
    let days = window.days.unwrap_or(30).clamp(1, 366);
    let cached = app
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&days)
        .filter(|(at, _)| at.elapsed() < CACHE_FOR)
        .map(|(_, json)| json.clone());
    let json = match cached {
        Some(json) => json,
        None => {
            let to = (app.today)();
            let made =
                app.store()
                    .aggregate(to + 1 - days.min(to + 1), to, app.config.min_install_days);
            match made {
                Ok(value) => {
                    let json = value.to_string();
                    app.cache
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(days, (Instant::now(), json.clone()));
                    json
                }
                Err(e) => {
                    eprintln!("horizon-telemetry-server: could not build the totals: {e}");
                    return status(StatusCode::SERVICE_UNAVAILABLE, "try again later");
                }
            }
        }
    };
    let origin =
        HeaderValue::from_str(&app.config.allow_origin).unwrap_or(HeaderValue::from_static("*"));
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=600"),
            ),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, origin),
        ],
        json,
    )
        .into_response()
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/v1/batch", post(receive))
        .route("/v1/installs/{id}", delete(forget))
        .route("/v1/aggregate", get(aggregate))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(app)
}
