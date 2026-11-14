use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::TcpListener,
};
type Request = (String, Option<(u64, u64)>);
#[derive(Default)]
struct State {
    active: AtomicUsize,
    peak: AtomicUsize,
    accepted: AtomicUsize,
    requests: Mutex<Vec<Request>>,
    faults: AtomicUsize,
}
struct Server {
    url: String,
    state: Arc<State>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(data: Vec<u8>, delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        let kept = state.clone();
        let data = Arc::new(data);
        let task = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                kept.accepted.fetch_add(1, Relaxed);
                let state = kept.clone();
                let data = data.clone();
                tokio::spawn(async move {
                    let (read, mut write) = socket.into_split();
                    let mut read = BufReader::new(read);
                    loop {
                        let mut line = String::new();
                        if read.read_line(&mut line).await.unwrap_or(0) == 0 {
                            break;
                        }
                        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                        let mut range = None;
                        loop {
                            line.clear();
                            if read.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            if line == "\r\n" {
                                break;
                            }
                            if let Some(value) =
                                line.to_ascii_lowercase().strip_prefix("range: bytes=")
                            {
                                range = value.trim().split_once('-').and_then(|(a, b)| {
                                    Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?))
                                });
                            }
                        }
                        state.requests.lock().unwrap().push((path.clone(), range));
                        let active = state.active.fetch_add(1, Relaxed) + 1;
                        state.peak.fetch_max(active, Relaxed);
                        tokio::time::sleep(delay).await;
                        let fault = path == "/retry"
                            && range.is_some_and(|(start, _)| start > 0)
                            && state.faults.fetch_add(1, Relaxed) == 0;
                        let throttle =
                            path == "/throttle" && state.faults.fetch_add(1, Relaxed) == 0;
                        let reject = path == "/reject" && range.is_some();
                        let (status, extra, body) = if path == "/missing" {
                            ("404 Not Found", String::new(), Vec::new())
                        } else if path == "/redirect" {
                            ("302 Found", "Location: /file\r\n".into(), Vec::new())
                        } else if throttle {
                            (
                                "429 Too Many Requests",
                                "Retry-After: 1\r\n".into(),
                                Vec::new(),
                            )
                        } else if fault {
                            ("503 Unavailable", String::new(), Vec::new())
                        } else if reject {
                            ("416 Range Not Satisfiable", String::new(), Vec::new())
                        } else if let Some((start, end)) = range.filter(|_| path != "/ignore") {
                            if end >= data.len() as u64 || start > end {
                                return;
                            }
                            (
                                "206 Partial Content",
                                format!(
                                    "Content-Range: bytes {}-{end}/{}\r\n",
                                    if path == "/bad-range" {
                                        start + 1
                                    } else {
                                        start
                                    },
                                    data.len()
                                ),
                                data[start as usize..=end as usize].to_vec(),
                            )
                        } else {
                            ("200 OK", String::new(), data.to_vec())
                        };
                        let header=format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nETag: \"fixture\"\r\n{extra}\r\n",body.len());
                        if write.write_all(header.as_bytes()).await.is_err() {
                            state.active.fetch_sub(1, Relaxed);
                            return;
                        }
                        if path == "/truncate"
                            && range.is_none()
                            && state.faults.fetch_add(1, Relaxed) == 0
                        {
                            let _ = write.write_all(&body[..body.len() / 2]).await;
                            state.active.fetch_sub(1, Relaxed);
                            return;
                        }
                        let mut body = body;
                        if path == "/corrupt" && !body.is_empty() {
                            body[0] ^= 1;
                        }
                        for chunk in body.chunks(1024) {
                            if path == "/slow" {
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                            if write.write_all(chunk).await.is_err() {
                                break;
                            }
                        }
                        state.active.fetch_sub(1, Relaxed);
                    }
                });
            }
        });
        Self { url, state, task }
    }
    fn artifact(&self, path: &Path, route: &str, data: &[u8]) -> Artifact {
        Artifact {
            url: format!("{}{route}", self.url),
            path: path.into(),
            sha1: format!("{:x}", Sha1::digest(data)),
            size: Some(data.len() as u64),
        }
    }
}
fn data(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 251) as u8).collect()
}
fn config() -> Config {
    Config {
        threshold: 32 << 10,
        minimum_segment: 4 << 10,
        read_timeout: Duration::from_millis(200),
        retries: 2,
        ..Config::default()
    }
}
#[tokio::test]
async fn thousands_of_assets_are_bounded_and_connections_reused() {
    let body = data(256);
    let server = Server::new(body.clone(), Duration::from_millis(2)).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(Config {
        files: 16,
        connections: 7,
        per_host: 3,
        ..config()
    })
    .unwrap();
    let cancel = AtomicBool::new(false);
    let artifacts: Vec<_> = (0..2000)
        .map(|i| server.artifact(&dir.path().join(i.to_string()), "/file", &body))
        .collect();
    engine.batch(&artifacts, &cancel, &|_| {}).await.unwrap();
    assert_eq!(fs::read(&artifacts[1999].path).await.unwrap(), body);
    assert!(server.state.peak.load(Relaxed) <= 3);
    assert!(server.state.peak.load(Relaxed) > 1);
    assert!(server.state.accepted.load(Relaxed) < 20);
    let progress = engine.progress();
    assert_eq!(
        (progress.completed, progress.pending, progress.failed),
        (2000, 0, 0)
    );
    assert_eq!(progress.verified_bytes, 2000 * 256);
    assert_eq!(progress.connections, 0);
}
#[tokio::test]
async fn segmented_transfers_validate_and_fallback_without_corrupt_output() {
    let body = data(128 << 10);
    let server = Server::new(body.clone(), Duration::from_millis(2)).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(Config {
        connections: 4,
        per_host: 3,
        ..config()
    })
    .unwrap();
    let cancel = AtomicBool::new(false);
    for route in ["/file", "/ignore", "/reject", "/bad-range", "/retry"] {
        let artifact = server.artifact(&dir.path().join(&route[1..]), route, &body);
        engine.download(&artifact, &cancel).await.unwrap();
        assert_eq!(fs::read(&artifact.path).await.unwrap(), body);
    }
    assert!(server.state.peak.load(Relaxed) <= 3);
    assert!(engine.progress().retried > 0);
    let requests = server.state.requests.lock().unwrap();
    assert!(requests
        .iter()
        .any(|(p, r)| p == "/file" && r.is_some_and(|(a, _)| a > 0)));
}
#[tokio::test]
async fn transient_disconnect_resumes_only_validated_ranges() {
    let body = data(16 << 10);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let artifact = server.artifact(&dir.path().join("resumed"), "/truncate", &body);
    engine
        .download(&artifact, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(fs::read(&artifact.path).await.unwrap(), body);
    assert!(server
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|(_, r)| r.is_some_and(|(start, _)| start == 8192)));
    assert!(engine.progress().retried > 0);
}
#[tokio::test]
async fn corrupt_cache_is_repaired_duplicates_coalesce_and_bad_hashes_never_publish() {
    let body = data(1024);
    let server = Server::new(body.clone(), Duration::from_millis(10)).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let artifact = server.artifact(&dir.path().join("cached"), "/file", &body);
    fs::write(&artifact.path, vec![0; 1024]).await.unwrap();
    let cancel = AtomicBool::new(false);
    let (a, b) = tokio::join!(
        engine.download(&artifact, &cancel),
        engine.download(&artifact, &cancel)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(server.state.requests.lock().unwrap().len(), 1);
    assert_eq!(fs::read(&artifact.path).await.unwrap(), body);
    let bad = server.artifact(&dir.path().join("bad"), "/corrupt", &body);
    assert!(engine
        .download(&bad, &cancel)
        .await
        .unwrap_err()
        .contains("SHA-1"));
    assert!(!bad.path.exists());
    engine.download(&artifact, &cancel).await.unwrap();
    assert_eq!(server.state.requests.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn cancellation_and_inactivity_release_resources_and_cleanup() {
    let body = data(128 << 10);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let artifact = server.artifact(&dir.path().join("cancelled"), "/slow", &body);
    let cancel = AtomicBool::new(false);
    let stop = async {
        tokio::time::sleep(Duration::from_millis(60)).await;
        cancel.store(true, Relaxed);
    };
    let (result, _) = tokio::join!(engine.download(&artifact, &cancel), stop);
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert!(!artifact.path.exists());
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert_eq!(engine.progress().connections, 0);
    let slow = Server::new(body.clone(), Duration::from_millis(80)).await;
    let stalled = slow.artifact(&dir.path().join("stalled"), "/file", &body);
    let engine = Downloader::new(Config {
        read_timeout: Duration::from_millis(10),
        retries: 0,
        ..config()
    })
    .unwrap();
    assert!(engine
        .download(&stalled, &AtomicBool::new(false))
        .await
        .is_err());
    assert!(!stalled.path.exists());
}
#[tokio::test]
async fn rate_limits_wait_and_permanent_failures_do_not_retry() {
    let body = data(64);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    engine
        .download(
            &server.artifact(&dir.path().join("limited"), "/throttle", &body),
            &cancel,
        )
        .await
        .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(engine.progress().retried, 1);
    assert!(engine
        .download(
            &server.artifact(&dir.path().join("gone"), "/missing", &body),
            &cancel
        )
        .await
        .unwrap_err()
        .contains("404"));
    assert_eq!(engine.progress().retried, 1);
}
#[test]
fn retry_after_seconds_and_dates_are_bounded() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for value in [
        "86400".to_owned(),
        u64::MAX.to_string(),
        httpdate::fmt_http_date(now + Duration::from_secs(86400)),
    ] {
        assert_eq!(retry_after(&value, now), Duration::from_secs(60));
    }
    assert_eq!(retry_after("5", now), Duration::from_secs(5));
    assert_eq!(
        retry_after(&httpdate::fmt_http_date(now + Duration::from_secs(7)), now),
        Duration::from_secs(7)
    );
    assert_eq!(
        retry_after(&httpdate::fmt_http_date(now - Duration::from_secs(1)), now),
        Duration::ZERO
    );
    assert_eq!(retry_after("invalid", now), Duration::ZERO);
}
#[tokio::test]
async fn metadata_never_follows_an_insecure_redirect() {
    let server = Server::new(data(64), Duration::ZERO).await;
    let engine = Downloader::new(config()).unwrap();
    let error = engine
        .metadata(&format!("{}/redirect", server.url), &AtomicBool::new(false))
        .await
        .unwrap_err();
    assert!(error.contains("HTTPS"));
    assert_eq!(
        server.state.requests.lock().unwrap().as_slice(),
        &[("/redirect".into(), None)]
    );
    let secure = Url::parse("https://example.invalid/version").unwrap();
    assert_eq!(
        redirect_target(&secure, "/next").unwrap().as_str(),
        "https://example.invalid/next"
    );
    for target in [
        "http://example.invalid/next",
        "file:///tmp/version",
        "ftp://example.invalid/version",
    ] {
        assert!(redirect_target(&secure, target).is_err());
    }
}
#[tokio::test]
async fn missing_or_malformed_hashes_never_reuse_cache_or_start_a_request() {
    let body = data(64);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let mut artifact = server.artifact(&dir.path().join("file"), "/file", &body);
    for cached in [false, true] {
        if cached {
            fs::write(&artifact.path, &body).await.unwrap();
        }
        for hash in ["", "abc", &"g".repeat(40)] {
            artifact.sha1 = hash.into();
            assert!(engine
                .download(&artifact, &AtomicBool::new(false))
                .await
                .unwrap_err()
                .contains("SHA-1"));
        }
        assert_eq!(artifact.path.exists(), cached);
    }
    assert!(server.state.requests.lock().unwrap().is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[tokio::test]
async fn startup_sweep_only_removes_generated_files_from_exited_processes() {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let dead = child.id();
    child.wait().unwrap();
    assert!(!crate::backend::process_alive(dead));
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join(format!(".horizon-download-{dead}-a.part"));
    let live = dir
        .path()
        .join(format!(".horizon-download-{}-b.part", std::process::id()));
    let unrelated = dir
        .path()
        .join(format!(".horizon-download-{dead}-not-hex.part"));
    let directory = dir.path().join(format!(".horizon-download-{dead}-c.part"));
    for path in [&stale, &live, &unrelated] {
        fs::write(path, b"partial").await.unwrap();
    }
    fs::create_dir(&directory).await.unwrap();
    #[cfg(unix)]
    let link = {
        let link = dir.path().join(format!(".horizon-download-{dead}-d.part"));
        std::os::unix::fs::symlink(&live, &link).unwrap();
        link
    };
    let server = Server::new(data(64), Duration::ZERO).await;
    let artifact = server.artifact(&dir.path().join("cached"), "/file", &data(64));
    fs::write(&artifact.path, data(64)).await.unwrap();
    let engine = Downloader::new(config()).unwrap();
    engine
        .download(&artifact, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(!stale.exists());
    assert!(live.exists() && unrelated.exists() && directory.is_dir());
    #[cfg(unix)]
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(server.state.requests.lock().unwrap().is_empty());
    // Sessions share the once-per-directory startup sweep.
    fs::write(&stale, b"later").await.unwrap();
    engine
        .session()
        .download(&artifact, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(stale.exists());
}
/// Run each mode in a fresh test process; /usr/bin/time measures its peak RSS separately.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn benchmark_download_pipeline() {
    let mode = std::env::var("HORIZON_DOWNLOAD_BENCH_MODE").unwrap_or_else(|_| "parallel".into());
    let body = data(1024);
    let server = Server::new(body.clone(), Duration::from_millis(5)).await;
    let dir = tempfile::tempdir().unwrap();
    let artifacts: Vec<_> = (0..2000)
        .map(|i| server.artifact(&dir.path().join(i.to_string()), "/file", &body))
        .collect();
    let start = Instant::now();
    if mode == "parallel" {
        Downloader::new(Config::default())
            .unwrap()
            .batch(&artifacts, &AtomicBool::new(false), &|_| {})
            .await
            .unwrap();
    } else {
        let workers = if mode == "original" { 8 } else { 1 };
        let files = artifacts.clone();
        tokio::task::spawn_blocking(move || {
            let next = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    let files = &files;
                    let next = &next;
                    scope.spawn(move || {
                        while let Some(file) = files.get(next.fetch_add(1, Relaxed)) {
                            let mut body = Vec::new();
                            std::io::Read::read_to_end(
                                &mut ureq::AgentBuilder::new()
                                    .build()
                                    .get(&file.url)
                                    .call()
                                    .unwrap()
                                    .into_reader(),
                                &mut body,
                            )
                            .unwrap();
                            assert_eq!(format!("{:x}", Sha1::digest(&body)), file.sha1);
                            std::fs::write(&file.path, body).unwrap();
                        }
                    });
                }
            });
        })
        .await
        .unwrap();
    }
    let elapsed = start.elapsed().as_secs_f64();
    for file in artifacts {
        assert_eq!(fs::read(file.path).await.unwrap(), body);
    }
    println!("DOWNLOAD_BENCH mode={mode} files=2000 bytes={} seconds={elapsed:.4} mib_s={:.4} accepted_connections={} peak_requests={}",2000*body.len(),(2000*body.len()) as f64/(1<<20) as f64/elapsed,server.state.accepted.load(Relaxed),server.state.peak.load(Relaxed));
}
#[tokio::test]
async fn global_budget_covers_two_hosts_metadata_and_segment_streams() {
    let body = data(64 << 10);
    let a = Server::new(body.clone(), Duration::from_millis(5)).await;
    let b = Server::new(body.clone(), Duration::from_millis(5)).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(Config {
        connections: 3,
        per_host: 2,
        ..config()
    })
    .unwrap();
    let cancel = AtomicBool::new(false);
    let artifacts: Vec<_> = (0..12)
        .map(|i| {
            if i % 2 == 0 { &a } else { &b }.artifact(
                &dir.path().join(i.to_string()),
                "/file",
                &body,
            )
        })
        .collect();
    let done = AtomicBool::new(false);
    let peak = AtomicUsize::new(0);
    let monitor = async {
        while !done.load(Relaxed) {
            peak.fetch_max((engine.progress().connections) as usize, Relaxed);
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    };
    let download = async {
        let metadata_url = format!("{}/file", a.url);
        let (files, metadata) = tokio::join!(
            engine.batch(&artifacts, &cancel, &|_| {}),
            engine.metadata(&metadata_url, &cancel)
        );
        files.unwrap();
        assert_eq!(metadata.unwrap(), body);
        done.store(true, Relaxed);
    };
    tokio::join!(download, monitor);
    assert!(peak.load(Relaxed) <= 3);
    assert!(peak.load(Relaxed) > 1);
    assert!(a.state.peak.load(Relaxed) <= 2);
    assert!(b.state.peak.load(Relaxed) <= 2);
}
#[tokio::test]
async fn range_probes_honor_retry_after_before_any_followup_request() {
    let body = data(64 << 10);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let artifact = server.artifact(&dir.path().join("probe-limited"), "/throttle", &body);
    let start = Instant::now();
    engine
        .download(&artifact, &AtomicBool::new(false))
        .await
        .unwrap();
    assert!(start.elapsed() >= Duration::from_secs(1));
    assert_eq!(engine.progress().retried, 1);
    assert_eq!(server.state.requests.lock().unwrap()[0].1, Some((0, 0)));
}
#[tokio::test]
async fn unknown_size_disconnect_restarts_without_appending_the_whole_response() {
    let body = data(16 << 10);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let mut artifact = server.artifact(&dir.path().join("unknown-size"), "/truncate", &body);
    artifact.size = None;
    engine
        .download(&artifact, &AtomicBool::new(false))
        .await
        .unwrap();
    assert_eq!(fs::read(&artifact.path).await.unwrap(), body);
    assert!(server
        .state
        .requests
        .lock()
        .unwrap()
        .iter()
        .all(|(_, range)| range.is_none()));
}
#[test]
fn cancelled_install_does_not_cancel_independent_metadata() {
    let server = run(Server::new(b"metadata".to_vec(), Duration::ZERO));
    crate::mojang::CANCEL.store(true, Relaxed);
    let result = get_bytes(&format!("{}/file", server.url));
    crate::mojang::CANCEL.store(false, Relaxed);
    assert_eq!(result.unwrap(), b"metadata");
}
#[tokio::test]
async fn socks_proxy_resolves_hostnames_remotely_like_the_previous_transport() {
    for scheme in ["socks", "socks5", "socks5h"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let proxy = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            socket.write_all(&[5, 0]).await.unwrap();
            let mut request = [0; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(request, [5, 1, 0, 3]);
            let length = socket.read_u8().await.unwrap();
            let mut host = vec![0; length as usize];
            socket.read_exact(&mut host).await.unwrap();
            let _port = socket.read_u16().await.unwrap();
            socket
                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 80])
                .await
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nproxy")
                .await
                .unwrap();
            String::from_utf8(host).unwrap()
        });
        let client = Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy_address(format!("{scheme}://{address}"))).unwrap())
            .build()
            .unwrap();
        assert_eq!(
            client
                .get("http://only-the-proxy-knows.invalid/file")
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            "proxy"
        );
        assert_eq!(proxy.await.unwrap(), "only-the-proxy-knows.invalid");
    }
}
#[tokio::test]
async fn successive_jobs_reset_progress_and_share_budgets() {
    let body = data(64 << 10);
    let server = Server::new(body.clone(), Duration::from_millis(10)).await;
    let dir = tempfile::tempdir().unwrap();
    let pool = Downloader::new(Config {
        connections: 2,
        files: 1,
        ..config()
    })
    .unwrap();
    let first = pool.session();
    let cancel = AtomicBool::new(false);
    first
        .download(
            &server.artifact(&dir.path().join("first"), "/file", &body),
            &cancel,
        )
        .await
        .unwrap();
    assert_eq!(first.progress().completed, 1);
    let second = pool.session();
    assert_eq!(
        (
            second.progress().completed,
            second.progress().downloaded_bytes
        ),
        (0, 0)
    );
    assert!(second.progress().elapsed < first.progress().elapsed);
    let one = server.artifact(&dir.path().join("next"), "/file", &body);
    let two = server.artifact(&dir.path().join("other"), "/file", &body);
    let third = pool.session();
    let (a, b) = tokio::join!(
        second.download(&one, &cancel),
        third.download(&two, &cancel)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(second.progress().completed, 1);
    assert_eq!(third.progress().completed, 1);
    assert_eq!(first.progress().completed, 1);
    assert!(server.state.peak.load(Relaxed) <= 2);
}
#[tokio::test]
async fn required_failures_abort_queued_and_active_siblings() {
    let body = data(128 << 10);
    let server = Server::new(body.clone(), Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let engine = Downloader::new(config()).unwrap();
    let cancel = AtomicBool::new(false);
    let artifacts = [
        server.artifact(&dir.path().join("missing"), "/missing", &body),
        server.artifact(&dir.path().join("slow"), "/slow", &body),
    ];
    let start = Instant::now();
    assert!(engine.batch(&artifacts, &cancel, &|_| {}).await.is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(engine.progress().connections, 0);
    assert_eq!(engine.progress().pending, 0);
    assert_eq!(engine.progress().failed, 2);
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn changing_proxy_routes_never_reuses_a_previous_hosts_keepalive_pool() {
    let direct = Server::new(b"direct".to_vec(), Duration::ZERO).await;
    let proxy_a = Server::new(b"proxy-a".to_vec(), Duration::ZERO).await;
    let proxy_b = Server::new(b"proxy-b".to_vec(), Duration::ZERO).await;
    let engine = Downloader::new(config()).unwrap();
    let url = format!("{}/file", direct.url);
    for (proxy, expected) in [
        (None, "direct"),
        (Some(proxy_a.url.clone()), "proxy-a"),
        (Some(proxy_b.url.clone()), "proxy-b"),
        (None, "direct"),
    ] {
        let client = engine.client_for_proxy(proxy).unwrap();
        assert_eq!(
            client.get(&url).send().await.unwrap().text().await.unwrap(),
            expected
        );
    }
    assert_eq!(direct.state.accepted.load(Relaxed), 1);
    assert_eq!(proxy_a.state.accepted.load(Relaxed), 1);
    assert_eq!(proxy_b.state.accepted.load(Relaxed), 1);
    assert!(proxy_a.state.requests.lock().unwrap()[0]
        .0
        .starts_with("http://"));
    assert!(engine
        .client_for_proxy(Some("socks3://127.0.0.1:9".into()))
        .is_err());
}
