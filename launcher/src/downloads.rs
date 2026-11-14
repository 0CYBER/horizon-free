//! Bounded, streaming HTTP transfers shared by the launcher's Minecraft pipeline.
//!
//! `HORIZON_DOWNLOAD_*` settings are read once when the shared engine starts. Invalid values
//! retain defaults; `Config` also supports direct configuration. Sizes are bytes, times seconds.
//!
//! | Suffix | Default | Meaning / bounds |
//! | --- | ---: | --- |
//! | FILES | 16 | active files, 1–1024 |
//! | CONNECTIONS | 32 | global HTTP requests, 1–1024 |
//! | PER_HOST | 16 | requests per hostname, 1–1024 |
//! | SEGMENTS | 8 | range workers per large file, 1–16 |
//! | THRESHOLD | 33554432 | range-probe threshold (32 MiB) |
//! | MINIMUM_SEGMENT | 4194304 | minimum chunk (4 MiB), at least 1 |
//! | CONNECT_TIMEOUT | 10 | connection timeout, at least 1 |
//! | READ_TIMEOUT | 30 | read inactivity timeout, at least 1 |
//! | RETRIES | 5 | additional transient-error attempts |
//! | BUFFER | 131072 | buffer per stream, clamped to 64–256 KiB |
//!
//! Redirect targets must use HTTPS. Retry-After delays are capped at 60 seconds. Each destination
//! directory is swept once before use for generated partials whose owning process has exited.
use futures_util::{stream, StreamExt};
use reqwest::{header, Client, Response, StatusCode, Url};
use sha1::{Digest, Sha1};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant, SystemTime},
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter},
    sync::{OwnedSemaphorePermit, Semaphore},
};

pub const CANCELLED: &str = "Download cancelled. Download again to go on where it stopped.";
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
type Sweeps = HashMap<PathBuf, Arc<tokio::sync::OnceCell<()>>>;
#[derive(Clone, Debug)]
pub struct Config {
    pub files: usize,
    pub connections: usize,
    pub per_host: usize,
    pub segments: usize,
    pub threshold: u64,
    pub minimum_segment: u64,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub retries: u32,
    pub buffer: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            files: 16,
            connections: 32,
            per_host: 16,
            segments: 8,
            threshold: 32 << 20,
            minimum_segment: 4 << 20,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            retries: 5,
            buffer: 128 << 10,
        }
    }
}
impl Config {
    pub fn from_env() -> Self {
        let mut c = Self::default();
        macro_rules! setting {
            ($field:ident,$name:literal) => {
                if let Ok(s) = std::env::var(concat!("HORIZON_DOWNLOAD_", $name)) {
                    if let Ok(n) = s.parse() {
                        c.$field = n;
                    }
                }
            };
        }
        setting!(files, "FILES");
        setting!(connections, "CONNECTIONS");
        setting!(per_host, "PER_HOST");
        setting!(segments, "SEGMENTS");
        setting!(threshold, "THRESHOLD");
        setting!(minimum_segment, "MINIMUM_SEGMENT");
        setting!(retries, "RETRIES");
        setting!(buffer, "BUFFER");
        for (name, value) in [
            ("CONNECT_TIMEOUT", &mut c.connect_timeout),
            ("READ_TIMEOUT", &mut c.read_timeout),
        ] {
            if let Ok(n) = std::env::var(format!("HORIZON_DOWNLOAD_{name}"))
                .unwrap_or_default()
                .parse::<u64>()
            {
                *value = Duration::from_secs(n.max(1));
            }
        }
        c
    }
}
#[derive(Clone, Debug)]
pub struct Artifact {
    pub url: String,
    pub path: PathBuf,
    pub sha1: String,
    pub size: Option<u64>,
}
#[derive(Default)]
struct Counters {
    submitted: AtomicU64,
    active: AtomicU64,
    connections: AtomicU64,
    completed: AtomicU64,
    failed: AtomicU64,
    retried: AtomicU64,
    bytes: AtomicU64,
    total: AtomicU64,
    verified: AtomicU64,
    file_micros: AtomicU64,
}
#[derive(Clone, Debug)]
pub struct Progress {
    pub active: u64,
    pub connections: u64,
    pub completed: u64,
    pub pending: u64,
    pub failed: u64,
    pub retried: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub verified_bytes: u64,
    pub elapsed: Duration,
    pub last_file_elapsed: Duration,
    pub mib_per_second: f64,
}
struct Count<'a>(&'a AtomicU64);
impl<'a> Count<'a> {
    fn new(n: &'a AtomicU64) -> Self {
        n.fetch_add(1, Relaxed);
        Self(n)
    }
}
impl Drop for Count<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Relaxed);
    }
}
pub struct Downloader {
    pub config: Config,
    clients: Arc<Mutex<Clients>>,
    network: Arc<Semaphore>,
    files: Arc<Semaphore>,
    hosts: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    paths: Arc<Mutex<HashMap<PathBuf, Weak<Semaphore>>>>,
    sweeps: Arc<Mutex<Sweeps>>,
    metrics: Arc<Metrics>,
}
#[derive(Debug)]
struct Failure {
    message: String,
    retry: bool,
    after: Duration,
    range: bool,
}
impl Failure {
    fn permanent(message: impl ToString) -> Self {
        Self {
            message: message.to_string(),
            retry: false,
            after: Duration::ZERO,
            range: false,
        }
    }
    fn transient(message: impl ToString) -> Self {
        Self {
            retry: true,
            ..Self::permanent(message)
        }
    }
    fn range(message: impl ToString) -> Self {
        Self {
            range: true,
            ..Self::permanent(message)
        }
    }
}
impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self::permanent(e)
    }
}
impl From<reqwest::Error> for Failure {
    fn from(e: reqwest::Error) -> Self {
        Self {
            retry: e.is_timeout()
                || e.is_connect()
                || e.is_body()
                || e.is_decode()
                || e.is_request(),
            ..Self::permanent(e.without_url())
        }
    }
}
struct Connection<'a> {
    response: Response,
    _global: OwnedSemaphorePermit,
    _host: OwnedSemaphorePermit,
    _count: Count<'a>,
}
impl Downloader {
    pub fn new(mut config: Config) -> Result<Self, String> {
        config.files = config.files.clamp(1, 1024);
        config.connections = config.connections.clamp(1, 1024);
        config.per_host = config.per_host.clamp(1, 1024);
        config.segments = config.segments.clamp(1, 16);
        config.minimum_segment = config.minimum_segment.max(1);
        config.buffer = config.buffer.clamp(64 << 10, 256 << 10);
        let direct = build_client(&config, None).map_err(|e| e.message)?;
        Ok(Self {
            network: Arc::new(Semaphore::new(config.connections)),
            files: Arc::new(Semaphore::new(config.files)),
            config,
            clients: Arc::new(Mutex::new(Clients {
                direct,
                proxy: None,
            })),
            hosts: Arc::new(Mutex::new(HashMap::new())),
            paths: Arc::new(Mutex::new(HashMap::new())),
            sweeps: Arc::new(Mutex::new(HashMap::new())),
            metrics: Arc::new(Metrics {
                counters: Counters::default(),
                started: Instant::now(),
            }),
        })
    }
    pub fn progress(&self) -> Progress {
        progress_of(&self.metrics)
    }
    /// A job owns its metrics; all jobs still share the HTTP pool and resource budgets.
    pub fn session(&self) -> Self {
        let metrics = Arc::new(Metrics {
            counters: Counters::default(),
            started: Instant::now(),
        });
        *VISIBLE.lock().unwrap() = Some(Arc::downgrade(&metrics));
        self.with_metrics(metrics)
    }
    fn with_metrics(&self, metrics: Arc<Metrics>) -> Self {
        Self {
            config: self.config.clone(),
            clients: self.clients.clone(),
            network: self.network.clone(),
            files: self.files.clone(),
            hosts: self.hosts.clone(),
            paths: self.paths.clone(),
            sweeps: self.sweeps.clone(),
            metrics,
        }
    }
    /// A pool is tied to its route. Changing Settings must never reuse an old proxy tunnel.
    fn client_for_proxy(&self, proxy: Option<String>) -> Result<Client, Failure> {
        {
            let clients = self.clients.lock().unwrap();
            match &proxy {
                None => return Ok(clients.direct.clone()),
                Some(spec) => {
                    if let Some((kept, client)) = &clients.proxy {
                        if spec == kept {
                            return Ok(client.clone());
                        }
                    }
                }
            }
        }
        let client = build_client(&self.config, proxy.as_deref())?;
        let mut clients = self.clients.lock().unwrap();
        if let Some((spec, kept)) = &clients.proxy {
            if Some(spec) == proxy.as_ref() {
                return Ok(kept.clone());
            }
        }
        clients.proxy = proxy.map(|spec| (spec, client.clone()));
        Ok(client)
    }
    async fn request(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
        etag: Option<&str>,
    ) -> Result<Connection<'_>, Failure> {
        let mut url = Url::parse(url).map_err(Failure::permanent)?;
        for _ in 0..=10 {
            let host = url
                .host_str()
                .ok_or_else(|| Failure::permanent("URL has no host"))?
                .to_owned();
            let budget = {
                let mut hosts = self.hosts.lock().unwrap();
                hosts
                    .entry(host)
                    .or_insert_with(|| Arc::new(Semaphore::new(self.config.per_host)))
                    .clone()
            };
            let host = budget.acquire_owned().await.unwrap();
            let global = self.network.clone().acquire_owned().await.unwrap();
            let count = Count::new(&self.metrics.counters.connections);
            let client =
                self.client_for_proxy(crate::proxy::spec_for(url.as_str()).map(proxy_address))?;
            let mut request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if let Some((start, end)) = range {
                request = request.header(header::RANGE, format!("bytes={start}-{end}"));
            }
            if let Some(etag) = etag {
                request = request.header(header::IF_RANGE, etag);
            }
            let response = request.send().await?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|s| s.to_str().ok())
                    .ok_or_else(|| Failure::permanent("Redirect has no Location"))?;
                url = redirect_target(&url, location)?;
                continue;
            }
            if !response.status().is_success() {
                let status = response.status();
                let after = response
                    .headers()
                    .get(header::RETRY_AFTER)
                    .and_then(|s| s.to_str().ok())
                    .map(|s| retry_after(s, SystemTime::now()))
                    .unwrap_or_default();
                return Err(Failure {
                    message: format!("{url} answered {status}"),
                    retry: status == StatusCode::TOO_MANY_REQUESTS
                        || status == StatusCode::REQUEST_TIMEOUT
                        || status.is_server_error(),
                    after,
                    range: range.is_some()
                        && (status == StatusCode::RANGE_NOT_SATISFIABLE
                            || status == StatusCode::BAD_REQUEST),
                });
            }
            return Ok(Connection {
                response,
                _global: global,
                _host: host,
                _count: count,
            });
        }
        Err(Failure::permanent("Too many redirects"))
    }
    async fn pause(&self, attempt: u32, after: Duration) {
        self.metrics.counters.retried.fetch_add(1, Relaxed);
        let mut random = [0; 8];
        let _ = getrandom::fill(&mut random);
        let jitter = u64::from_ne_bytes(random) % 251;
        tokio::time::sleep(after.min(MAX_RETRY_AFTER).max(Duration::from_millis(
            (250u64.saturating_mul(1u64 << attempt.min(7)) + jitter).min(30_000),
        )))
        .await;
    }
    /// A failed stream retains only bytes flushed to disk. Reusing them requires a strong ETag.
    async fn piece(
        &self,
        artifact: &Artifact,
        path: &Path,
        bounds: Option<(u64, u64, u64)>,
        validator: Option<&str>,
    ) -> Result<(), Failure> {
        let mut copied = 0;
        let mut validator = validator.map(str::to_owned);
        let (start, end, total) = bounds.unwrap_or((
            0,
            artifact.size.unwrap_or(0).saturating_sub(1),
            artifact.size.unwrap_or(0),
        ));
        for attempt in 0..=self.config.retries {
            if attempt > 0 && (validator.is_none() || total == 0) {
                copied = 0;
            }
            if copied > 0 && total > 0 && copied == end - start + 1 {
                return Ok(());
            }
            let ranged = bounds.is_some() || (copied > 0 && validator.is_some() && total > 0);
            let result = async {
                let mut connection = self
                    .request(
                        &artifact.url,
                        ranged.then_some((start + copied, end)),
                        validator.as_deref(),
                    )
                    .await?;
                let response = &mut connection.response;
                if ranged {
                    let expected = format!("bytes {}-{end}/{total}", start + copied);
                    if response.status() != StatusCode::PARTIAL_CONTENT
                        || response
                            .headers()
                            .get(header::CONTENT_RANGE)
                            .and_then(|v| v.to_str().ok())
                            != Some(expected.as_str())
                    {
                        return Err(Failure::range(
                            "Server ignored or changed the requested byte range",
                        ));
                    }
                } else if response.status() != StatusCode::OK {
                    return Err(Failure::range("Unexpected partial response"));
                }
                let tag = strong_etag(response);
                if validator.as_ref().is_some_and(|v| tag.as_ref() != Some(v)) {
                    return Err(Failure::range("Resource changed during transfer"));
                }
                if validator.is_none() {
                    validator = tag;
                }
                let expected = if ranged {
                    Some(end - start + 1 - copied)
                } else {
                    artifact.size.or(response.content_length())
                };
                if expected
                    .zip(response.content_length())
                    .is_some_and(|(a, b)| a != b)
                {
                    return Err(Failure::range("Inconsistent Content-Length"));
                }
                let mut file = fs::OpenOptions::new().write(true).open(path).await?;
                if !ranged {
                    file.set_len(0).await?;
                }
                file.seek(std::io::SeekFrom::Start(start + copied)).await?;
                let mut writer = BufWriter::with_capacity(self.config.buffer, file);
                let mut received = 0;
                let body = async {
                    while let Some(chunk) = response.chunk().await? {
                        if expected.is_some_and(|n| received + chunk.len() as u64 > n) {
                            return Err(Failure::range("Response exceeds expected length"));
                        }
                        writer.write_all(&chunk).await?;
                        received += chunk.len() as u64;
                        self.metrics
                            .counters
                            .bytes
                            .fetch_add(chunk.len() as u64, Relaxed);
                    }
                    if expected.is_some_and(|n| received != n) {
                        return Err(Failure::transient("Truncated response"));
                    }
                    Ok(())
                }
                .await;
                writer.flush().await?;
                copied += received;
                body
            }
            .await;
            match result {
                Ok(()) => return Ok(()),
                Err(e) if e.retry && attempt < self.config.retries => {
                    self.pause(attempt, e.after).await
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
    async fn transfer(&self, artifact: &Artifact, path: &Path) -> Result<(), Failure> {
        if let Some(size) = artifact.size.filter(|&size| {
            size >= self.config.threshold
                && size > self.config.minimum_segment
                && self.config.segments > 1
        }) {
            let mut attempt = 0;
            let probe = loop {
                match self.request(&artifact.url, Some((0, 0)), None).await {
                    Err(e) if e.retry && attempt < self.config.retries => {
                        self.pause(attempt, e.after).await;
                        attempt += 1;
                    }
                    result => break result,
                }
            };
            if let Ok(mut probe) = probe {
                let expected = format!("bytes 0-0/{size}");
                if probe.response.status() == StatusCode::PARTIAL_CONTENT
                    && probe
                        .response
                        .headers()
                        .get(header::CONTENT_RANGE)
                        .and_then(|v| v.to_str().ok())
                        == Some(expected.as_str())
                {
                    let validator = strong_etag(&probe.response);
                    let valid = probe
                        .response
                        .chunk()
                        .await
                        .is_ok_and(|c| c.is_some_and(|c| c.len() == 1))
                        && probe.response.chunk().await.is_ok_and(|c| c.is_none());
                    drop(probe);
                    if valid {
                        fs::OpenOptions::new()
                            .write(true)
                            .open(path)
                            .await?
                            .set_len(size)
                            .await?;
                        // More chunks than connections let a fast stream take over a slow one's remaining work.
                        let chunk = self
                            .config
                            .minimum_segment
                            .max(size.div_ceil((self.config.segments * 4) as u64));
                        let result = stream::iter((0..size.div_ceil(chunk)).map(|i| {
                            self.piece(
                                artifact,
                                path,
                                Some((i * chunk, ((i + 1) * chunk).min(size) - 1, size)),
                                validator.as_deref(),
                            )
                        }))
                        .buffer_unordered(self.config.segments)
                        .collect::<Vec<_>>()
                        .await
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>();
                        match result {
                            Ok(_) => return Ok(()),
                            Err(e) if e.range => {}
                            Err(e) => return Err(e),
                        }
                    }
                }
            } else if let Err(e) = probe {
                if !e.range {
                    return Err(e);
                }
            }
        }
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .await?
            .set_len(0)
            .await?;
        match self.piece(artifact, path, None, None).await {
            Err(e) if e.range => {
                fs::OpenOptions::new()
                    .write(true)
                    .open(path)
                    .await?
                    .set_len(0)
                    .await?;
                self.piece(artifact, path, None, None).await
            }
            result => result,
        }
    }
    pub async fn download(&self, artifact: &Artifact, cancel: &AtomicBool) -> Result<(), String> {
        self.register(std::slice::from_ref(artifact));
        self.one(artifact, cancel).await
    }
    fn register(&self, artifacts: &[Artifact]) {
        self.metrics
            .counters
            .submitted
            .fetch_add(artifacts.len() as u64, Relaxed);
        self.metrics.counters.total.fetch_add(
            artifacts.iter().filter_map(|a| a.size).sum::<u64>(),
            Relaxed,
        );
    }
    async fn one(&self, artifact: &Artifact, cancel: &AtomicBool) -> Result<(), String> {
        let start = Instant::now();
        let operation = async {
            if artifact.sha1.len() != 40 || !artifact.sha1.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Failure::permanent("Artifact requires a 40-character SHA-1"));
            }
            let lock = {
                let mut paths = self.paths.lock().unwrap();
                match paths.get(&artifact.path).and_then(Weak::upgrade) {
                    Some(lock) => lock,
                    None => {
                        let lock = Arc::new(Semaphore::new(1));
                        paths.insert(artifact.path.clone(), Arc::downgrade(&lock));
                        lock
                    }
                }
            };
            let _unique = lock.acquire().await.unwrap();
            let _file = self.files.acquire().await.unwrap();
            let _active = Count::new(&self.metrics.counters.active);
            let parent = artifact
                .path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            self.sweep(parent).await;
            if valid(artifact, &artifact.path, self.config.buffer).await? {
                return Ok(());
            }
            fs::create_dir_all(parent).await?;
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).map_err(Failure::permanent)?;
            let temporary = parent.join(format!(
                ".horizon-download-{}-{:x}.part",
                std::process::id(),
                u64::from_ne_bytes(nonce)
            ));
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .await?;
            let scratch = Scratch(temporary);
            let result = async {
                self.transfer(artifact, &scratch.0).await?;
                if !valid(artifact, &scratch.0, self.config.buffer).await? {
                    return Err(Failure::permanent(
                        "Downloaded size or SHA-1 does not match the manifest",
                    ));
                }
                if cancel.load(Relaxed) {
                    return Err(Failure::permanent(CANCELLED));
                }
                fs::rename(&scratch.0, &artifact.path).await?;
                Ok(())
            }
            .await;
            let _ = fs::remove_file(&scratch.0).await;
            result
        };
        let result = tokio::select! {biased; _=cancelled(cancel)=>Err(Failure::permanent(CANCELLED)),result=operation=>result};
        self.metrics.counters.file_micros.store(
            start.elapsed().as_micros().min(u64::MAX as u128) as u64,
            Relaxed,
        );
        {
            let mut paths = self.paths.lock().unwrap();
            if paths
                .get(&artifact.path)
                .is_some_and(|p| p.strong_count() == 0)
            {
                paths.remove(&artifact.path);
            }
        }
        match result {
            Ok(()) => {
                self.metrics.counters.completed.fetch_add(1, Relaxed);
                self.metrics
                    .counters
                    .verified
                    .fetch_add(artifact.size.unwrap_or(0), Relaxed);
                Ok(())
            }
            Err(e) => {
                self.metrics.counters.failed.fetch_add(1, Relaxed);
                Err(if e.message == CANCELLED {
                    e.message
                } else {
                    format!(
                        "{}: {} (after {:.2}s)",
                        artifact.url,
                        e.message,
                        start.elapsed().as_secs_f64()
                    )
                })
            }
        }
    }
    pub async fn batch(
        &self,
        artifacts: &[Artifact],
        cancel: &AtomicBool,
        log: &dyn Fn(String),
    ) -> Result<(), String> {
        self.report(self.batch_policy(artifacts, cancel, true), log)
            .await
    }
    pub async fn install(
        &self,
        required: &[Artifact],
        optional: &[Artifact],
        cancel: &AtomicBool,
        log: &dyn Fn(String),
    ) -> Result<Result<(), String>, String> {
        self.report(
            async {
                let (_, optional) =
                    tokio::try_join!(self.batch_policy(required, cancel, true), async {
                        Ok::<_, String>(self.batch_policy(optional, cancel, false).await)
                    })?;
                Ok(optional)
            },
            log,
        )
        .await
    }
    async fn report<F: std::future::Future>(&self, future: F, log: &dyn Fn(String)) -> F::Output {
        tokio::pin!(future);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.tick().await;
        loop {
            tokio::select! {result=&mut future=>return result,_=tick.tick()=>{let p=self.progress();log(format!("{} files complete, {} active, {} pending; {:.2} MiB/s; {}/{} bytes",p.completed,p.active,p.pending,p.mib_per_second,p.verified_bytes,p.total_bytes));}}
        }
    }
    async fn batch_policy(
        &self,
        artifacts: &[Artifact],
        cancel: &AtomicBool,
        fail_fast: bool,
    ) -> Result<(), String> {
        self.register(artifacts);
        let mut first = None;
        let mut pending = BatchPending {
            metrics: self.metrics.clone(),
            remaining: artifacts.len() as u64,
        };
        let work = stream::iter(artifacts.iter().map(|a| self.one(a, cancel)))
            .buffer_unordered(self.config.files);
        tokio::pin!(work);
        while let Some(result) = work.next().await {
            pending.remaining -= 1;
            if let Err(e) = result {
                if fail_fast {
                    return Err(e);
                }
                first.get_or_insert(e);
            }
        }
        first.map_or(Ok(()), Err)
    }
    async fn sweep(&self, parent: &Path) {
        let once = self
            .sweeps
            .lock()
            .unwrap()
            .entry(parent.to_owned())
            .or_default()
            .clone();
        once.get_or_init(|| async {
            let parent = parent.to_owned();
            let _ = tokio::task::spawn_blocking(move || sweep_partials(&parent)).await;
        })
        .await;
    }
    /// Metadata is small and must be parsed before dependent work can be queued.
    pub async fn metadata(&self, url: &str, cancel: &AtomicBool) -> Result<Vec<u8>, String> {
        let operation = async {
            for attempt in 0..=self.config.retries {
                let result = async {
                    let mut connection = self.request(url, None, None).await?;
                    if connection.response.status() != StatusCode::OK {
                        return Err(Failure::permanent("Unexpected partial metadata response"));
                    }
                    let mut body = Vec::new();
                    while let Some(chunk) = connection.response.chunk().await? {
                        if body.len() + chunk.len() > 16 << 20 {
                            return Err(Failure::permanent("Metadata exceeds 16 MiB"));
                        }
                        self.metrics
                            .counters
                            .bytes
                            .fetch_add(chunk.len() as u64, Relaxed);
                        body.extend_from_slice(&chunk);
                    }
                    Ok(body)
                }
                .await;
                match result {
                    Ok(body) => return Ok(body),
                    Err(e) if e.retry && attempt < self.config.retries => {
                        self.pause(attempt, e.after).await
                    }
                    Err(e) => return Err(e.message),
                }
            }
            unreachable!()
        };
        tokio::select! {biased; _=cancelled(cancel)=>Err(CANCELLED.into()),result=operation=>result}
    }
}
fn redirect_target(url: &Url, location: &str) -> Result<Url, Failure> {
    let target = url.join(location).map_err(Failure::permanent)?;
    if target.scheme() != "https" {
        return Err(Failure::permanent("Redirect target must use HTTPS"));
    }
    Ok(target)
}
fn retry_after(value: &str, now: SystemTime) -> Duration {
    value
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(value)
                .ok()
                .map(|date| date.duration_since(now).unwrap_or_default())
        })
        .unwrap_or_default()
        .min(MAX_RETRY_AFTER)
}
fn sweep_partials(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some((pid, nonce)) = name
            .to_str()
            .and_then(|s| s.strip_prefix(".horizon-download-"))
            .and_then(|s| s.strip_suffix(".part"))
            .and_then(|s| s.split_once('-'))
        else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if pid == 0
            || nonce.is_empty()
            || nonce.len() > 16
            || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
        {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_file()) && !crate::backend::process_alive(pid)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
fn strong_etag(response: &Response) -> Option<String> {
    response
        .headers()
        .get(header::ETAG)
        .and_then(|s| s.to_str().ok())
        .filter(|s| s.starts_with('"') && s.ends_with('"'))
        .map(str::to_owned)
}
async fn cancelled(cancel: &AtomicBool) {
    loop {
        if cancel.load(Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn valid(artifact: &Artifact, path: &Path, buffer: usize) -> Result<bool, Failure> {
    let mut file = match fs::File::open(path).await {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    let size = file.metadata().await?.len();
    if artifact.size.is_some_and(|expected| size != expected) {
        return Ok(false);
    }
    let mut hash = Sha1::new();
    let mut buffer = vec![0; buffer];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&artifact.sha1))
}
/// On dropped/cancelled futures, unlink asynchronously after all stream handles have dropped.
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let path = self.0.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = fs::remove_file(path).await;
            });
        }
    }
}
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
pub fn run<F: std::future::Future>(future: F) -> F::Output {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("download runtime")
        })
        .block_on(future)
}
static SHARED: OnceLock<Result<Downloader, String>> = OnceLock::new();
pub fn shared() -> Result<&'static Downloader, String> {
    SHARED
        .get_or_init(|| Downloader::new(Config::from_env()))
        .as_ref()
        .map_err(Clone::clone)
}
pub fn get_bytes(url: &str) -> Result<Vec<u8>, String> {
    run(shared()?.metadata(url, &AtomicBool::new(false)))
}
pub fn install_metadata(url: &str) -> Result<Vec<u8>, String> {
    let engine = shared()?;
    let metrics = VISIBLE.lock().unwrap().as_ref().and_then(Weak::upgrade);
    match metrics {
        Some(metrics) => run(engine
            .with_metrics(metrics)
            .metadata(url, &crate::mojang::CANCEL)),
        None => run(engine.metadata(url, &crate::mojang::CANCEL)),
    }
}
fn proxy_address(spec: String) -> String {
    match spec.split_once("://") {
        Some(("socks" | "socks5", address)) => format!("socks5h://{address}"),
        Some(_) => spec,
        None => format!("http://{spec}"),
    }
}
struct Clients {
    direct: Client,
    proxy: Option<(String, Client)>,
}
fn build_client(config: &Config, proxy: Option<&str>) -> Result<Client, Failure> {
    let mut builder = Client::builder()
        .user_agent(concat!("horizon-launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(config.connect_timeout)
        .read_timeout(config.read_timeout)
        .pool_max_idle_per_host(config.per_host)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if let Some(proxy) = proxy {
        let url = Url::parse(proxy).map_err(Failure::permanent)?;
        if !matches!(
            url.scheme(),
            "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
        ) {
            return Err(Failure::permanent("Unsupported proxy scheme"));
        }
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    builder.build().map_err(Into::into)
}

pub fn current_progress() -> Option<Progress> {
    let metrics = VISIBLE.lock().unwrap().as_ref()?.upgrade()?;
    let p = progress_of(&metrics);
    (p.active + p.pending > 0).then_some(p)
}

struct Metrics {
    counters: Counters,
    started: Instant,
}
static VISIBLE: Mutex<Option<Weak<Metrics>>> = Mutex::new(None);
fn progress_of(metrics: &Metrics) -> Progress {
    let c = &metrics.counters;
    let active = c.active.load(Relaxed);
    let completed = c.completed.load(Relaxed);
    let failed = c.failed.load(Relaxed);
    let bytes = c.bytes.load(Relaxed);
    let elapsed = metrics.started.elapsed();
    Progress {
        active,
        connections: c.connections.load(Relaxed),
        completed,
        failed,
        pending: c
            .submitted
            .load(Relaxed)
            .saturating_sub(active + completed + failed),
        retried: c.retried.load(Relaxed),
        downloaded_bytes: bytes,
        total_bytes: c.total.load(Relaxed),
        verified_bytes: c.verified.load(Relaxed),
        elapsed,
        last_file_elapsed: Duration::from_micros(c.file_micros.load(Relaxed)),
        mib_per_second: bytes as f64 / (1 << 20) as f64 / elapsed.as_secs_f64().max(0.001),
    }
}

struct BatchPending {
    metrics: Arc<Metrics>,
    remaining: u64,
}
impl Drop for BatchPending {
    fn drop(&mut self) {
        self.metrics
            .counters
            .failed
            .fetch_add(self.remaining, Relaxed);
    }
}

#[cfg(test)]
#[path = "downloads/tests.rs"]
mod tests;
