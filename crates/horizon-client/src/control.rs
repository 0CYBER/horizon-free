//! Local control socket: lets a script or `horizon-ctl` drive a running game.
//!
//! Each connection sends commands as text lines and receives one JSON line per
//! command. Commands are executed on the game thread between frames; replies
//! that depend on a later frame (screenshots, benchmarks) arrive when ready.
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

#[cfg(unix)]
pub use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(not(unix))]
pub use unsupported::{UnixListener, UnixStream};

/// Windows' standard library has no Unix domain sockets: there the control
/// socket reports itself unavailable and the game runs without it.
#[cfg(not(unix))]
mod unsupported {
    use std::{io, path::Path};

    fn unavailable() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "the control socket needs Unix domain sockets (Linux or macOS)",
        )
    }
    pub struct UnixStream;
    impl UnixStream {
        pub fn connect<P: AsRef<Path>>(_: P) -> io::Result<Self> {
            Err(unavailable())
        }
        pub fn try_clone(&self) -> io::Result<Self> {
            Err(unavailable())
        }
    }
    impl io::Read for UnixStream {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(unavailable())
        }
    }
    impl io::Write for UnixStream {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(unavailable())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    pub struct UnixListener;
    impl UnixListener {
        pub fn bind<P: AsRef<Path>>(_: P) -> io::Result<Self> {
            Err(unavailable())
        }
        pub fn incoming(&self) -> std::iter::Empty<io::Result<UnixStream>> {
            std::iter::empty()
        }
    }
}

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

/// Frame durations since the last reset, summarized like the Java recorder,
/// with the CPU part of each frame and GPU times from timestamp queries.
#[derive(Default)]
pub struct FrameStats {
    millis: Vec<f32>,
    cpu: Vec<f32>,
    gpu: Vec<f32>,
    /// Summed milliseconds per frame phase (`PHASES`) and the frames counted.
    phases: [f64; PHASES.len()],
    phase_frames: u64,
}

/// The parts of a frame's CPU time: world streaming and uploads, game ticks,
/// building the frame's geometry and HUD, and the renderer's own work.
pub const PHASES: [&str; 4] = ["update", "tick", "prepare", "render"];

fn percentiles(values: &[f32]) -> serde_json::Value {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len();
    if n == 0 {
        return serde_json::Value::Null;
    }
    let total: f64 = sorted.iter().map(|&m| f64::from(m)).sum();
    let percentile = |p: f64| sorted[((n as f64 * p).ceil() as usize).clamp(1, n) - 1];
    serde_json::json!({
        "avg_ms": total / n as f64,
        "p50_ms": percentile(0.5),
        "p95_ms": percentile(0.95),
        "p99_ms": percentile(0.99),
        "max_ms": sorted[n - 1],
    })
}

impl FrameStats {
    const LIMIT: usize = 1 << 20;
    pub fn record(&mut self, millis: f32) {
        if self.millis.len() < Self::LIMIT {
            self.millis.push(millis);
        }
    }
    /// The frame's own work: its duration minus waits for the GPU, the
    /// display and the frame limiter.
    pub fn record_cpu(&mut self, millis: f32) {
        if self.cpu.len() < Self::LIMIT {
            self.cpu.push(millis);
        }
    }
    pub fn record_phases(&mut self, millis: [f32; PHASES.len()]) {
        for (sum, m) in self.phases.iter_mut().zip(millis) {
            *sum += f64::from(m);
        }
        self.phase_frames += 1;
    }
    pub fn record_gpu(&mut self, millis: &[f32]) {
        if self.gpu.len() < Self::LIMIT {
            self.gpu.extend_from_slice(millis);
        }
    }
    pub fn reset(&mut self) {
        self.millis.clear();
        self.cpu.clear();
        self.gpu.clear();
        self.phases = [0.0; PHASES.len()];
        self.phase_frames = 0;
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
            "cpu": percentiles(&self.cpu),
            "gpu": percentiles(&self.gpu),
            "phases_avg_ms": PHASES
                .iter()
                .zip(self.phases)
                .map(|(name, sum)| (name.to_string(), (sum / self.phase_frames.max(1) as f64).into()))
                .collect::<serde_json::Map<_, _>>(),
        })
    }
}
