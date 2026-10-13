//! Everything that is not UI: GitHub, installs, resource preparation and the game's processes.
//! Functions here block; the UI runs them on threads and reads their messages.
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub const REPO: &str = "ludovic111/horizon";
pub const SPONSOR_URL: &str = "https://github.com/sponsors/ludovic111";
const AGENT: &str = concat!("horizon-launcher/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub body: String,
    pub date: String,
    pub asset: Option<(String, u64)>,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const ASSET: Option<&str> = Some("horizon-linux-x86_64.tar.gz");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const ASSET: Option<&str> = Some("horizon-macos-arm64.zip");
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub const ASSET: Option<&str> = Some("horizon-windows-x64.zip");
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
pub const ASSET: Option<&str> = None;

const CLIENT: &str = if cfg!(windows) { "horizon-client.exe" } else { "horizon-client" };

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().user_agent(AGENT).build()
}

fn get(url: &str) -> Result<ureq::Response, String> {
    agent().get(url).call().map_err(|e| match e {
        ureq::Error::Status(403, _) => "GitHub's rate limit was reached. Try again in a few minutes.".into(),
        ureq::Error::Status(code, _) => format!("{url} answered {code}"),
        ureq::Error::Transport(t) => format!("No connection: {t}"),
    })
}

/// Published releases, newest first. Drafts never reach the anonymous API.
pub fn releases() -> Result<Vec<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=50");
    let list: serde_json::Value = get(&url)?.into_json().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in list.as_array().into_iter().flatten() {
        if r["draft"].as_bool().unwrap_or(false) {
            continue;
        }
        let asset = r["assets"].as_array().into_iter().flatten().find_map(|a| {
            (Some(a["name"].as_str()?) == ASSET)
                .then(|| Some((a["browser_download_url"].as_str()?.to_string(), a["size"].as_u64()?)))?
        });
        out.push(Release {
            tag: r["tag_name"].as_str().unwrap_or_default().into(),
            name: r["name"].as_str().unwrap_or_default().into(),
            body: r["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
            date: r["published_at"].as_str().unwrap_or_default().chars().take(10).collect(),
            asset,
        });
    }
    Ok(out)
}

/// The README's roadmap section from main (the project status section if it has none).
pub fn roadmap() -> Result<String, String> {
    let url = format!("https://raw.githubusercontent.com/{REPO}/main/README.md");
    let readme = get(&url)?.into_string().map_err(|e| e.to_string())?;
    for title in ["## Roadmap", "## Project status"] {
        if let Some(start) = readme.lines().position(|l| l.trim() == title) {
            let section: Vec<&str> =
                readme.lines().skip(start + 1).take_while(|l| !l.starts_with("## ")).collect();
            return Ok(section.join("\n").trim().to_string());
        }
    }
    Err("The README has no roadmap section yet.".into())
}

pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("horizon-launcher")
}

fn versions_dir() -> PathBuf {
    data_dir().join("versions")
}

pub fn version_dir(tag: &str) -> PathBuf {
    versions_dir().join(tag)
}

const COMPLETE: &str = ".installed";

/// Installed versions, newest tag first.
pub fn installed() -> Vec<String> {
    let mut tags: Vec<String> = fs::read_dir(versions_dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(COMPLETE).exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    tags.sort_by(|a, b| b.cmp(a));
    tags
}

/// Downloads and unpacks one release; `progress` gets the fraction done.
pub fn install(tag: &str, url: &str, size: u64, progress: impl Fn(f32)) -> Result<(), String> {
    let root = versions_dir();
    let partial = root.join(format!(".{tag}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let archive = partial.join("download");
    {
        let mut reader = get(url)?.into_reader();
        let mut file = fs::File::create(&archive).map_err(|e| e.to_string())?;
        let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
        loop {
            let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            done += n as u64;
            progress((done as f32 / size.max(1) as f32).min(1.0) * 0.9);
        }
    }
    let unpacked = partial.join("unpacked");
    let file = fs::File::open(&archive).map_err(|e| e.to_string())?;
    if url.ends_with(".tar.gz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(&unpacked).map_err(|e| e.to_string())?;
    } else {
        zip::ZipArchive::new(file).and_then(|mut z| z.extract(&unpacked)).map_err(|e| e.to_string())?;
    }
    progress(0.97);
    // The archives hold one top folder (horizon-linux-x86_64/...); the version folder is its contents.
    let entries: Vec<_> = fs::read_dir(&unpacked).map_err(|e| e.to_string())?.flatten().collect();
    let top = match entries.as_slice() {
        [only] if only.path().is_dir() => only.path(),
        _ => unpacked.clone(),
    };
    if !top.join(CLIENT).exists() {
        return Err(format!("The download has no {CLIENT}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(top.join(CLIENT), fs::Permissions::from_mode(0o755));
    }
    let target = version_dir(tag);
    let _ = fs::remove_dir_all(&target);
    fs::rename(&top, &target).map_err(|e| e.to_string())?;
    fs::write(target.join(COMPLETE), "").map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    progress(1.0);
    Ok(())
}

pub fn remove(tag: &str) -> Result<(), String> {
    fs::remove_dir_all(version_dir(tag)).map_err(|e| e.to_string())
}

/// The Minecraft version a Horizon build targets, from its research-lock.json.
pub fn game_version(tag: &str) -> String {
    fs::read_to_string(version_dir(tag).join("research-lock.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["minecraft"]["version"].as_str().map(String::from))
        .unwrap_or_else(|| "26.3".into())
}

pub fn resources(tag: &str) -> PathBuf {
    version_dir(tag).join(".local/horizon-client").join(game_version(tag))
}

pub fn is_prepared(tag: &str) -> bool {
    resources(tag).join("horizon-resources.json").exists()
}

/// The official launcher's game directory, where prepare_native_client.py looks by default.
pub fn default_minecraft_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/minecraft")
    } else if cfg!(windows) {
        dirs::config_dir()?.join(".minecraft")
    } else {
        home.join(".minecraft")
    };
    dir.is_dir().then_some(dir)
}

fn found(program: &str, arg: &str) -> bool {
    Command::new(program).arg(arg).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

fn python() -> Option<&'static str> {
    let candidates: &[&'static str] = if cfg!(windows) { &["py", "python"] } else { &["python3", "python"] };
    candidates.iter().copied().find(|p| found(p, "--version"))
}

/// Checks what the preparation needs and builds its command. PLAY.txt describes the same steps:
/// the player's own launcher install, the server jar from that version's own json, Java and Python.
pub fn prepare_command(tag: &str, minecraft: &Path, log: &dyn Fn(String)) -> Result<Command, String> {
    let version = game_version(tag);
    let manifest = minecraft.join("versions").join(&version).join(format!("{version}.json"));
    if !manifest.exists() {
        return Err(format!(
            "Minecraft {version} is not in {}. Play {version} once from the official launcher, or choose another folder.",
            minecraft.display()
        ));
    }
    let python = python().ok_or("Python 3.10 or newer is needed. Install it and try again.")?;
    if !found("java", "-version") {
        return Err("Java 25 is needed (a JDK, e.g. Eclipse Temurin 25). Install it and try again.".into());
    }
    let jar = data_dir().join("server-jars").join(format!("{version}.jar"));
    if !jar.exists() {
        let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let url = json["downloads"]["server"]["url"]
            .as_str()
            .ok_or_else(|| format!("{} has no server download", manifest.display()))?;
        log(format!("Downloading the {version} server jar from Mojang"));
        fs::create_dir_all(jar.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut body = Vec::new();
        get(url)?.into_reader().read_to_end(&mut body).map_err(|e| e.to_string())?;
        let partial = jar.with_extension("partial");
        fs::write(&partial, body).and_then(|_| fs::rename(&partial, &jar)).map_err(|e| e.to_string())?;
    }
    let mut cmd = Command::new(python);
    cmd.current_dir(version_dir(tag))
        .args(["-u", "tools/prepare_native_client.py", "--prepare", "--server-jar"])
        .arg(&jar)
        .arg("--minecraft-dir")
        .arg(minecraft);
    Ok(cmd)
}

pub fn client_command(tag: &str) -> Command {
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(version_dir(tag));
    cmd
}

pub fn server_command(tag: &str, folder: &Path) -> Command {
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(folder).arg("--resources").arg(resources(tag)).args(["--dedicated", "--nogui"]);
    cmd
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Job {
    Prepare,
    Client,
    Server,
}

pub enum Msg {
    Releases(Result<Vec<Release>, String>),
    Roadmap(Result<String, String>),
    Progress(String, f32),
    Installed(String, Result<(), String>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
}

/// A running process; its output arrives as `Msg::Line`, its end as `Msg::Exit`.
pub struct Running {
    pub child: Arc<Mutex<Child>>,
    pub stdin: Option<ChildStdin>,
}

impl Running {
    pub fn send(&mut self, line: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }
    pub fn kill(&self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

pub fn spawn(job: Job, mut cmd: Command, tx: Sender<Msg>) -> Result<Running, String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
    let stdin = child.stdin.take();
    let mut readers = Vec::new();
    let out = child.stdout.take().map(|o| Box::new(o) as Box<dyn Read + Send>);
    let err = child.stderr.take().map(|o| Box::new(o) as Box<dyn Read + Send>);
    for stream in [out, err].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Line(job, line));
            }
        }));
    }
    let child = Arc::new(Mutex::new(child));
    let waited = child.clone();
    std::thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
        let status = loop {
            if let Some(s) = waited.lock().unwrap().try_wait().transpose() {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let result = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Exited with {s}")),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Msg::Exit(job, result));
    });
    Ok(Running { child, stdin })
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub selected: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
}

impl Settings {
    pub fn load() -> Self {
        fs::read_to_string(data_dir().join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(data_dir().join("settings.json"), serde_json::to_string_pretty(self).unwrap());
    }
}
