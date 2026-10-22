//! The ingest server for opt-in data collection (docs/telemetry.md).
//!
//! - `POST /v1/batch`: a zstd batch from the launcher, checked by `horizon_telemetry::batch`.
//! - `DELETE /v1/installs/<id>`: everything an install sent goes.
//! - `GET /v1/aggregate?days=30`: the anonymous totals the website reads, cached.
//! - `POST /v1/live`: a game's anonymous "I'm playing" ping (`live`, in memory only).
//! - `GET /v1/live`: how many play now, rounded, cached briefly.
//! - `GET /healthz`.
//!
//! Abuse limits: a token bucket per IP address (kept in memory for an hour, never stored), a
//! daily item cap per install, and the batch size limits. The live count has its own, larger
//! bucket per address (several players can share one) and a cap on the sessions it holds. Raw
//! rows expire after `retention_days`; the daily totals stay. One small process and one SQLite
//! file: see `README.md` for running it.
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

pub mod live;
pub mod store;

use horizon_telemetry::live::{MAX_PING_BYTES, Ping};
use live::Live;
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
    /// Live count requests (pings and reads) an IP address may make an hour, with a burst of a
    /// tenth of it. A game pings once a minute, so the default lets ten play behind one address.
    pub live_per_hour: f64,
    /// How long a session counts after its last ping.
    pub live_window: Duration,
    /// Sessions the live count holds at most; a new one past that is refused.
    pub live_max_sessions: usize,
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
            live_per_hour: 600.0,
            live_window: horizon_telemetry::live::WINDOW,
            live_max_sessions: 200_000,
        }
    }
}

impl Config {
    /// The defaults, with `HORIZON_TELEMETRY_RETENTION_DAYS`, `…_MIN_INSTALL_DAYS`,
    /// `…_REQUESTS_PER_HOUR`, `…_ITEMS_PER_DAY`, `…_TRUST_PROXY` (1), `…_ALLOW_ORIGIN`,
    /// `…_LIVE_PER_HOUR`, `…_LIVE_WINDOW_SECONDS` and `…_LIVE_MAX_SESSIONS`.
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
            live_per_hour: var("LIVE_PER_HOUR")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.live_per_hour),
            live_window: var("LIVE_WINDOW_SECONDS")
                .and_then(|v| v.parse().ok())
                .map_or(d.live_window, Duration::from_secs),
            live_max_sessions: var("LIVE_MAX_SESSIONS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.live_max_sessions),
        }
    }
}

pub struct App {
    store: Mutex<Store>,
    config: Config,
    /// Tokens left and when they were counted, per client address.
    buckets: Mutex<Buckets>,
    /// The same for the live count's requests.
    live_buckets: Mutex<Buckets>,
    /// Who is playing now (`live`).
    live: Mutex<Live>,
    /// `GET /v1/live`'s answer and when it was made.
    live_cache: Mutex<Option<(Instant, String)>>,
    /// `/v1/aggregate` answers by `days`, with when they were made.
    cache: Mutex<HashMap<u32, (Instant, String)>>,
    /// Today, in days since 1970 (a test moves it).
    today: Box<dyn Fn() -> u32 + Send + Sync>,
}

type Buckets = HashMap<IpAddr, (f64, Instant)>;

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
        let live = Live::new(config.live_window, config.live_max_sessions);
        Arc::new(Self {
            store: Mutex::new(store),
            config,
            buckets: Mutex::default(),
            live_buckets: Mutex::default(),
            live: Mutex::new(live),
            live_cache: Mutex::default(),
            cache: Mutex::default(),
            today,
        })
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes a token for `ip`; false when it has none left.
    fn allow(&self, ip: IpAddr) -> bool {
        take(&self.buckets, self.config.requests_per_hour, ip)
    }

    /// [`Self::allow`] for the live count's requests.
    fn allow_live(&self, ip: IpAddr) -> bool {
        take(&self.live_buckets, self.config.live_per_hour, ip)
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Daily upkeep: totals rebuilt, expired rows removed, idle rate-limit entries and live
    /// sessions forgotten.
    pub fn maintain(&self) -> rusqlite::Result<usize> {
        let oldest = (self.today)().saturating_sub(self.config.retention_days);
        let removed = self.store().expire(oldest)?;
        for buckets in [&self.buckets, &self.live_buckets] {
            buckets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(3600));
        }
        self.live().forget_expired(Instant::now());
        if removed > 0 {
            self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        Ok(removed)
    }
}

/// Takes a token for `ip` from its bucket in `buckets` (`per_hour`, bursts of a tenth of it);
/// false when it has none left.
fn take(buckets: &Mutex<Buckets>, per_hour: f64, ip: IpAddr) -> bool {
    let rate = per_hour / 3600.0;
    let burst = (per_hour / 10.0).max(1.0);
    let mut buckets = buckets.lock().unwrap_or_else(|e| e.into_inner());
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
    json_response(&app, json, CACHE_FOR.as_secs())
}

/// `POST /v1/live`: one "I'm playing". 204, or 400 for a bad ping, 413 for one too large, 429
/// over the address's limit, 503 when the table is full.
async fn ping(State(app): State<Arc<App>>, req: Request) -> Response {
    if !app.allow_live(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let Ok(body) = axum::body::to_bytes(req.into_body(), MAX_PING_BYTES).await else {
        return status(StatusCode::PAYLOAD_TOO_LARGE, "a ping is at most 512 bytes");
    };
    let ping: Ping = match serde_json::from_slice(&body) {
        Ok(ping) => ping,
        Err(e) => return status(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if let Err(e) = ping.check() {
        return status(StatusCode::BAD_REQUEST, e);
    }
    match app.live().ping(&ping, Instant::now()) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(live::Full) => status(StatusCode::SERVICE_UNAVAILABLE, "try again later"),
    }
}

/// `GET /v1/live`: the rounded live count (`horizon_telemetry::live::Count`), made again at
/// most every eighth of the window.
async fn playing(State(app): State<Arc<App>>, req: Request) -> Response {
    if !app.allow_live(app.client(req.headers(), peer(&req))) {
        return status(StatusCode::TOO_MANY_REQUESTS, "too many requests");
    }
    let fresh_for = app.config.live_window / 8;
    let json = {
        let mut cache = app.live_cache.lock().unwrap_or_else(|e| e.into_inner());
        match &*cache {
            Some((at, json)) if at.elapsed() < fresh_for => json.clone(),
            _ => {
                let json =
                    serde_json::to_string(&app.live().count(Instant::now())).unwrap_or_default();
                *cache = Some((Instant::now(), json.clone()));
                json
            }
        }
    };
    json_response(&app, json, fresh_for.as_secs())
}

fn json_response(app: &App, json: String, max_age: u64) -> Response {
    let origin =
        HeaderValue::from_str(&app.config.allow_origin).unwrap_or(HeaderValue::from_static("*"));
    let cache = HeaderValue::from_str(&format!("public, max-age={max_age}"))
        .unwrap_or(HeaderValue::from_static("no-cache"));
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::CACHE_CONTROL, cache),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, origin),
        ],
        json,
    )
        .into_response()
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/v1/batch", post(receive))
        .route("/v1/live", post(ping).get(playing))
        .route("/v1/installs/{id}", delete(forget))
        .route("/v1/aggregate", get(aggregate))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(app)
}
