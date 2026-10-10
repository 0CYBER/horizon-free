//! horizon-ctl: drive a running Horizon client through its control socket.
#![forbid(unsafe_code)]
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    thread,
    time::{Duration, Instant},
};

const USAGE: &str = "usage: horizon-ctl [--socket PATH] COMMAND [ARGS...]

Local commands:
  launch [GAME ARGS...]      start horizon-client with a control socket and wait for it
  hold KEY SECONDS           press KEY, wait, release it
  wait-ready [SECONDS]       wait until the player has spawned and no chunks are pending
  -                          run commands from stdin, one per line (\"sleep S\" waits locally)

Game commands (run \"horizon-ctl help\" against a running game for the full list):
  status, key, mouse, look, turn, tp, hotbar, give, render-distance, time,
  screenshot PATH, stats, bench SECONDS, open, save, quit

Replies are JSON lines; the exit status is 1 when the reply has an \"error\".
The socket defaults to $HORIZON_CONTROL or $XDG_RUNTIME_DIR/horizon-control.sock.";

struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path).map_err(|e| {
            format!(
                "cannot connect to {} ({e}); is the game running with --control?",
                path.display()
            )
        })?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        Ok(Self {
            writer: stream,
            reader,
        })
    }
    fn send(&mut self, line: &str) -> Result<String, String> {
        writeln!(self.writer, "{line}").map_err(|e| e.to_string())?;
        let mut reply = String::new();
        if self
            .reader
            .read_line(&mut reply)
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("the game closed the connection".into());
        }
        Ok(reply.trim_end().to_owned())
    }
}

fn is_error(reply: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(reply).map_or(true, |v| v.get("error").is_some())
}

/// Screenshot paths are written by the game process: make them absolute here.
fn absolute_paths(words: &mut [String]) {
    if words.first().map(String::as_str) == Some("screenshot")
        && let Some(path) = words.get_mut(1)
        && let Ok(cwd) = std::env::current_dir()
    {
        *path = cwd.join(&*path).display().to_string();
    }
}

fn run_line(client: &mut Client, words: &mut [String]) -> Result<bool, String> {
    match words.first().map(String::as_str) {
        Some("sleep") => {
            let seconds: f64 = words
                .get(1)
                .ok_or("sleep SECONDS")?
                .parse()
                .map_err(|_| "sleep SECONDS")?;
            thread::sleep(Duration::from_secs_f64(seconds));
            Ok(true)
        }
        Some("hold") => {
            let (key, seconds) = match words {
                [_, key, seconds] => (
                    key.clone(),
                    seconds.parse::<f64>().map_err(|_| "hold KEY SECONDS")?,
                ),
                _ => return Err("hold KEY SECONDS".into()),
            };
            let down = client.send(&format!("key {key} down"))?;
            if is_error(&down) {
                println!("{down}");
                return Ok(false);
            }
            thread::sleep(Duration::from_secs_f64(seconds));
            let up = client.send(&format!("key {key} up"))?;
            println!("{up}");
            Ok(!is_error(&up))
        }
        Some("wait-ready") => {
            let limit = words
                .get(1)
                .map_or(Ok(120.0), |s| s.parse::<f64>())
                .map_err(|_| "wait-ready [SECONDS]")?;
            let deadline = Instant::now() + Duration::from_secs_f64(limit);
            loop {
                let reply = client.send("status")?;
                let status: serde_json::Value =
                    serde_json::from_str(&reply).map_err(|e| e.to_string())?;
                if !status["player"].is_null() && status["pending_chunks"] == 0 {
                    println!("{reply}");
                    return Ok(true);
                }
                if Instant::now() > deadline {
                    println!(
                        "{{\"error\":\"not ready after {limit} seconds\",\"status\":{reply}}}"
                    );
                    return Ok(false);
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
        _ => {
            absolute_paths(words);
            let reply = client.send(&words.join(" "))?;
            println!("{reply}");
            Ok(!is_error(&reply))
        }
    }
}

fn launch(socket: &Path, args: &[String]) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let game = exe.with_file_name("horizon-client");
    let log = socket.with_extension("log");
    let file = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let child = Command::new(&game)
        .args(args)
        .arg("--control-socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(file.try_clone().map_err(|e| e.to_string())?)
        .stderr(file)
        .spawn()
        .map_err(|e| format!("{}: {e}", game.display()))?;
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        if UnixStream::connect(socket).is_ok() {
            println!(
                "{}",
                serde_json::json!({ "ok": true, "pid": child.id(), "socket": socket, "log": log })
            );
            return Ok(true);
        }
        thread::sleep(Duration::from_millis(100));
    }
    println!(
        "{}",
        serde_json::json!({ "error": "control socket did not open within 300 seconds", "log": log })
    );
    Ok(false)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut socket = horizon_client::control::default_path();
    if args.first().map(String::as_str) == Some("--socket") && args.len() >= 2 {
        socket = PathBuf::from(args.remove(1));
        args.remove(0);
    }
    let result = match args.first().map(String::as_str) {
        None | Some("-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("launch") => launch(&socket, &args[1..]),
        Some("-") => Client::connect(&socket).and_then(|mut client| {
            let mut all_ok = true;
            for line in std::io::stdin().lock().lines() {
                let line = line.map_err(|e| e.to_string())?;
                let mut words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
                if words.is_empty() || words[0].starts_with('#') {
                    continue;
                }
                all_ok &= run_line(&mut client, &mut words)?;
            }
            Ok(all_ok)
        }),
        Some(_) => Client::connect(&socket).and_then(|mut client| run_line(&mut client, &mut args)),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("horizon-ctl: {e}");
            ExitCode::from(2)
        }
    }
}
