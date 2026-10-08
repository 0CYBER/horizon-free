//! Local control socket: lets a script or `horizon-ctl` drive a running game.
//!
//! Each connection sends commands as text lines and receives one JSON line per
//! command. Commands are executed on the game thread between frames; replies
//! that depend on a later frame (screenshots, benchmarks) arrive when ready.
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

/// One command line from a client, with the channel for its JSON reply.
pub struct Request {
    pub line: String,
    pub reply: mpsc::Sender<String>,
}

pub struct Server {
    path: PathBuf,
    requests: mpsc::Receiver<Request>,
}

/// `$HORIZON_CONTROL`, else `horizon-control.sock` in the user's runtime directory.
pub fn default_path() -> PathBuf {
    if let Some(path) = std::env::var_os("HORIZON_CONTROL") {
        return path.into();
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    dir.join("horizon-control.sock")
}

impl Server {
    pub fn start(path: &Path) -> std::io::Result<Self> {
        // A socket left by a crashed game refuses connections; replace it.
        if path.exists() && UnixStream::connect(path).is_err() {
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        let (sender, requests) = mpsc::channel();
        thread::Builder::new()
            .name("horizon-control".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let sender = sender.clone();
                    let _ = thread::Builder::new()
                        .name("horizon-control-client".into())
                        .spawn(move || serve(stream, &sender));
                }
            })?;
        Ok(Self {
            path: path.to_owned(),
            requests,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn try_recv(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn serve(stream: UnixStream, sender: &mpsc::Sender<Request>) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let (reply, response) = mpsc::channel();
        if sender.send(Request { line, reply }).is_err() {
            return;
        }
        // The game dropping the reply sender means it exited mid-command.
        let text = response
            .recv()
            .unwrap_or_else(|_| r#"{"error":"game exited"}"#.to_owned());
        if writeln!(writer, "{text}").is_err() {
            return;
        }
    }
}

/// Frame durations since the last reset, summarized like the Java recorder.
#[derive(Default)]
pub struct FrameStats {
    millis: Vec<f32>,
}

impl FrameStats {
    const LIMIT: usize = 1 << 20;
    pub fn record(&mut self, millis: f32) {
        if self.millis.len() < Self::LIMIT {
            self.millis.push(millis);
        }
    }
    pub fn reset(&mut self) {
        self.millis.clear();
    }
    pub fn summary(&self) -> serde_json::Value {
        let mut sorted = self.millis.clone();
        sorted.sort_by(f32::total_cmp);
        let n = sorted.len();
        if n == 0 {
            return serde_json::json!({ "frames": 0 });
        }
        let total: f64 = sorted.iter().map(|&m| f64::from(m)).sum();
        let percentile = |p: f64| sorted[((n as f64 * p).ceil() as usize).clamp(1, n) - 1];
        // Average frame time of the slowest 1% of frames, as frames per second.
        let slowest = n.div_ceil(100);
        let slow: f64 = sorted[n - slowest..]
            .iter()
            .map(|&m| f64::from(m))
            .sum::<f64>()
            / slowest as f64;
        serde_json::json!({
            "frames": n,
            "seconds": total / 1000.0,
            "avg_fps": n as f64 * 1000.0 / total,
            "low_1pct_fps": 1000.0 / slow,
            "p50_ms": percentile(0.5),
            "p95_ms": percentile(0.95),
            "p99_ms": percentile(0.99),
            "max_ms": sorted[n - 1],
        })
    }
}
