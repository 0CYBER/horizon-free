//! Everything that is not UI: GitHub, installs, resource preparation, the game's processes and
//! updates. The GUI, the CLI and the MCP server all go through this module. Functions here
//! block; the GUI runs them on threads and reads their messages.
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub const REPO: &str = "ludovic111/horizon";
pub const SPONSOR_URL: &str = "https://github.com/sponsors/ludovic111";
pub const README_URL: &str = "https://github.com/ludovic111/horizon/blob/launcher/launcher/README.md";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Launcher releases are tagged `launcher-vX.Y.Z`; Horizon's own are `vYYYY.MM.DD`.
pub const LAUNCHER_TAG_PREFIX: &str = "launcher-v";
const AGENT: &str = concat!("horizon-launcher/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, serde::Serialize)]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const LAUNCHER_ASSET: Option<&str> = Some("horizon-launcher-linux-x86_64.tar.gz");
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const LAUNCHER_ASSET: Option<&str> = Some("horizon-launcher-macos-arm64.zip");
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub const LAUNCHER_ASSET: Option<&str> = Some("horizon-launcher-windows-x64.zip");
#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "windows", target_arch = "x86_64")
)))]
pub const LAUNCHER_ASSET: Option<&str> = None;

const LAUNCHER_BINARY: &str = if cfg!(windows) { "horizon-launcher.exe" } else { "horizon-launcher" };

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

/// A Horizon game tag: `v` then a digit (`v2026.10.13`). Launcher tags never match.
pub fn is_game_tag(tag: &str) -> bool {
    tag.strip_prefix('v').is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// The launcher version in a `launcher-vX.Y.Z` tag, or None for any other tag.
pub fn launcher_version(tag: &str) -> Option<&str> {
    tag.strip_prefix(LAUNCHER_TAG_PREFIX).filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// The numeric parts of a version or tag: `v2026.10.13` and `2026.10.13` give [2026, 10, 13].
/// Anything after a `-` (a pre-release) is ignored.
fn version_parts(v: &str) -> Vec<u64> {
    let v = v.trim_start_matches(LAUNCHER_TAG_PREFIX).trim_start_matches('v');
    v.split('-').next().unwrap_or("").split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Compares two versions part by part, missing parts counting as zero (1.2 == 1.2.0).
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (version_parts(a), version_parts(b));
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// Every published release of the repository, both kinds, as GitHub's JSON.
fn release_list() -> Result<serde_json::Value, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    get(&url)?.into_json().map_err(|e| e.to_string())
}

/// The releases in `list` whose tag passes `keep`, newest first, with the asset named `asset`.
fn parse_releases(list: &serde_json::Value, asset: Option<&str>, keep: fn(&str) -> bool) -> Vec<Release> {
    let mut out = Vec::new();
    for r in list.as_array().into_iter().flatten() {
        let tag = r["tag_name"].as_str().unwrap_or_default();
        if r["draft"].as_bool().unwrap_or(false) || !keep(tag) {
            continue;
        }
        let found = r["assets"].as_array().into_iter().flatten().find_map(|a| {
            (Some(a["name"].as_str()?) == asset)
                .then(|| Some((a["browser_download_url"].as_str()?.to_string(), a["size"].as_u64()?)))?
        });
        out.push(Release {
            tag: tag.into(),
            name: r["name"].as_str().unwrap_or_default().into(),
            body: r["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
            date: r["published_at"].as_str().unwrap_or_default().chars().take(10).collect(),
            asset: found,
        });
    }
    out.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
    out
}

fn is_launcher_tag(tag: &str) -> bool {
    launcher_version(tag).is_some()
}

/// Published Horizon releases, newest first. Drafts never reach the anonymous API, and launcher
/// releases are left out.
pub fn releases() -> Result<Vec<Release>, String> {
    Ok(parse_releases(&release_list()?, ASSET, is_game_tag))
}

/// Published launcher releases (`launcher-v…`), newest first, with this platform's archive.
pub fn launcher_releases() -> Result<Vec<Release>, String> {
    Ok(parse_releases(&release_list()?, LAUNCHER_ASSET, is_launcher_tag))
}

/// The newest Horizon release that has a build for this platform.
pub fn latest(releases: &[Release]) -> Option<&Release> {
    releases.iter().find(|r| r.asset.is_some())
}

/// The README's roadmap section from main (the project status section if it has none).
pub fn roadmap() -> Result<String, String> {
    let url = format!("https://raw.githubusercontent.com/{REPO}/main/README.md");
    let readme = get(&url)?.into_string().map_err(|e| e.to_string())?;
    let level = |l: &str| l.chars().take_while(|c| *c == '#').count();
    for title in ["Roadmap", "Project status"] {
        let lines: Vec<&str> = readme.lines().collect();
        if let Some(start) = lines.iter().position(|l| level(l) > 0 && l.trim_start_matches('#').trim() == title) {
            let depth = level(lines[start]);
            let section: Vec<&str> = lines[start + 1..]
                .iter()
                .take_while(|l| !(level(l) > 0 && level(l) <= depth))
                .copied()
                .collect();
            return Ok(section.join("\n").trim().to_string());
        }
    }
    Err("The README has no roadmap section yet.".into())
}

/// The launcher's data folder; `HORIZON_LAUNCHER_HOME` overrides it (for tests and scripts).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HORIZON_LAUNCHER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
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
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

fn download(url: &str, size: u64, to: &Path, progress: &impl Fn(f32)) -> Result<(), String> {
    let mut reader = get(url)?.into_reader();
    let mut file = fs::File::create(to).map_err(|e| e.to_string())?;
    let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            return Ok(());
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress((done as f32 / size.max(1) as f32).min(1.0) * 0.9);
    }
}

fn unpack(archive: &Path, url: &str, to: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    if url.ends_with(".tar.gz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(to).map_err(|e| e.to_string())
    } else {
        zip::ZipArchive::new(file).and_then(|mut z| z.extract(to)).map_err(|e| e.to_string())
    }
}

/// Downloads and unpacks one release; `progress` gets the fraction done.
pub fn install(tag: &str, url: &str, size: u64, progress: impl Fn(f32)) -> Result<(), String> {
    if !is_game_tag(tag) {
        return Err(format!("{tag} is not a Horizon release."));
    }
    let root = versions_dir();
    let partial = root.join(format!(".{tag}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let archive = partial.join("download");
    download(url, size, &archive, &progress)?;
    let unpacked = partial.join("unpacked");
    unpack(&archive, url, &unpacked)?;
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

/// Where the game keeps saves and options.txt: `HORIZON_HOME`, else `horizon` in the platform's
/// data folder (~/.local/share/horizon on Linux, as the release folders' horizon.sh uses). It is
/// never inside a version folder, so installing, updating or removing a version keeps the saves.
pub fn game_home() -> PathBuf {
    if let Some(home) = std::env::var_os("HORIZON_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("horizon")
}

pub fn client_command(tag: &str) -> Command {
    let home = game_home();
    let _ = fs::create_dir_all(&home);
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(home).arg("--resources").arg(resources(tag));
    cmd
}

pub fn server_command(tag: &str, folder: &Path) -> Command {
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(folder).arg("--resources").arg(resources(tag)).args(["--dedicated", "--nogui"]);
    cmd
}

pub fn eula_agreed(folder: &Path) -> bool {
    fs::read_to_string(folder.join("eula.txt")).is_ok_and(|s| s.lines().any(|l| l.trim() == "eula=true"))
}

/// Writes eula.txt as vanilla's server expects it, once the player agreed in the launcher.
pub fn agree_eula(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let text = "#By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).\neula=true\n";
    fs::write(folder.join("eula.txt"), text).map_err(|e| e.to_string())
}

// ----- Shared steps for the front ends -----------------------------------------------------------

/// The installed version to use: the one asked for, else the selected one, else the newest.
pub fn resolve_version(requested: Option<&str>) -> Result<String, String> {
    let installed = installed();
    if let Some(tag) = requested {
        return installed.iter().find(|t| *t == tag).cloned().ok_or_else(|| format!("{tag} is not installed."));
    }
    let selected = Settings::load().selected.filter(|s| installed.contains(s));
    selected.or_else(|| installed.first().cloned()).ok_or_else(|| "No version is installed yet.".into())
}

/// Removes an installed version and moves the selection to the newest one left if needed.
pub fn remove_version(tag: &str) -> Result<(), String> {
    if !installed().iter().any(|t| t == tag) {
        return Err(format!("{tag} is not installed."));
    }
    remove(tag)?;
    let mut settings = Settings::load();
    if settings.selected.as_deref() == Some(tag) {
        settings.selected = installed().first().cloned();
        settings.save();
    }
    Ok(())
}

/// The Minecraft folder to prepare from: the one given, the one chosen in the launcher, or the
/// official launcher's default.
pub fn minecraft_dir(requested: Option<PathBuf>) -> Result<PathBuf, String> {
    requested
        .or(Settings::load().minecraft_dir)
        .or_else(default_minecraft_dir)
        .ok_or_else(|| "No Minecraft folder found. Pass the official launcher's game folder.".into())
}

/// The server folder: the one given, the one chosen in the launcher, or `server/` in the data folder.
pub fn server_dir(requested: Option<PathBuf>) -> PathBuf {
    requested.or(Settings::load().server_dir).unwrap_or_else(|| data_dir().join("server"))
}

fn need_prepared(tag: &str) -> Result<(), String> {
    if is_prepared(tag) {
        Ok(())
    } else {
        Err(format!("{tag} is not prepared yet. Prepare it first (it needs your own Minecraft {}).", game_version(tag)))
    }
}

/// The client's command, once its resources are prepared.
pub fn client(tag: &str) -> Result<Command, String> {
    need_prepared(tag)?;
    Ok(client_command(tag))
}

/// The dedicated server's command in `folder`, once prepared and the EULA is agreed to.
pub fn server(tag: &str, folder: &Path, accept_eula: bool) -> Result<Command, String> {
    need_prepared(tag)?;
    if accept_eula {
        agree_eula(folder)?;
    }
    if !eula_agreed(folder) {
        return Err("Running a server needs your agreement to the Minecraft EULA (https://aka.ms/MinecraftEULA). Accept it to continue.".into());
    }
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    Ok(server_command(tag, folder))
}

/// One release's notes: the tag given, or the newest.
pub fn release_notes(tag: Option<&str>) -> Result<Release, String> {
    let releases = releases()?;
    match tag {
        Some(tag) => releases.into_iter().find(|r| r.tag == tag).ok_or_else(|| format!("No Horizon release is tagged {tag}.")),
        None => releases.into_iter().next().ok_or_else(|| "No release is published yet.".into()),
    }
}

/// A process whose output is kept: the last `LOG_KEEP` lines, and its result once it ends.
pub struct Logged {
    pub running: Running,
    pub log: Arc<Mutex<std::collections::VecDeque<String>>>,
    pub exit: Arc<Mutex<Option<Result<(), String>>>>,
}

pub const LOG_KEEP: usize = 2000;

impl Logged {
    pub fn start(job: Job, cmd: Command) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let running = spawn(job, cmd, tx)?;
        let log = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let exit = Arc::new(Mutex::new(None));
        let (l, e) = (log.clone(), exit.clone());
        std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    Msg::Line(_, line) => {
                        let mut log = l.lock().unwrap();
                        log.push_back(line);
                        if log.len() > LOG_KEEP {
                            log.pop_front();
                        }
                    }
                    Msg::Exit(_, result) => *e.lock().unwrap() = Some(result),
                    _ => {}
                }
            }
        });
        Ok(Self { running, log, exit })
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.exit.lock().unwrap().clone()
    }

    pub fn tail(&self, lines: usize) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter().skip(log.len().saturating_sub(lines)).cloned().collect()
    }

    pub fn wait(&self) -> Result<(), String> {
        loop {
            if let Some(result) = self.finished() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Asks a server to stop, and kills it if it has not after `grace`.
    pub fn stop(&mut self, grace: std::time::Duration) {
        self.running.send("stop");
        let start = std::time::Instant::now();
        while self.finished().is_none() && start.elapsed() < grace {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if self.finished().is_none() {
            self.running.kill();
        }
    }
}

// ----- Updates -----------------------------------------------------------------------------------

/// What a check found: a Horizon build newer than every installed one, and a newer launcher.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Updates {
    pub horizon: Option<Release>,
    pub launcher: Option<Release>,
}

/// The newest Horizon release with a build here, if it is newer than every installed version.
pub fn horizon_update(releases: &[Release], installed: &[String]) -> Option<Release> {
    let newest = latest(releases)?;
    let newer = installed.iter().all(|t| compare_versions(&newest.tag, t).is_gt());
    newer.then(|| newest.clone())
}

/// The newest launcher release with an archive for this platform, if newer than this launcher.
pub fn launcher_update(releases: &[Release]) -> Option<Release> {
    releases
        .iter()
        .find(|r| r.asset.is_some())
        .filter(|r| compare_versions(launcher_version(&r.tag).unwrap_or("0"), VERSION).is_gt())
        .cloned()
}

/// One request to GitHub, both kinds of update.
pub fn check_updates() -> Result<Updates, String> {
    let list = release_list()?;
    Ok(Updates {
        horizon: horizon_update(&parse_releases(&list, ASSET, is_game_tag), &installed()),
        launcher: launcher_update(&parse_releases(&list, LAUNCHER_ASSET, is_launcher_tag)),
    })
}

/// Whether a version installed as an update becomes the selected one: only when the player was
/// on the newest installed version before (or had none).
pub fn select_after_update(selected: Option<&str>, installed_before: &[String]) -> bool {
    match selected {
        None => true,
        Some(s) => installed_before.first().is_none_or(|newest| newest == s),
    }
}

/// Installs `tag` (or the newest release with "latest"), then updates the selection as
/// `select_after_update` says when `as_update`, or always selects it otherwise.
pub fn install_tag(tag: &str, as_update: bool, progress: impl Fn(f32)) -> Result<String, String> {
    let releases = releases()?;
    let release = if tag == "latest" {
        latest(&releases).ok_or("No release has a build for this platform yet.")?
    } else {
        releases.iter().find(|r| r.tag == tag).ok_or_else(|| format!("No Horizon release is tagged {tag}."))?
    };
    let (url, size) = release.asset.clone().ok_or_else(|| format!("{} has no build for this platform.", release.tag))?;
    let before = installed();
    install(&release.tag, &url, size, progress)?;
    let mut settings = Settings::load();
    if !as_update || select_after_update(settings.selected.as_deref(), &before) {
        settings.selected = Some(release.tag.clone());
        settings.save();
    }
    Ok(release.tag.clone())
}

/// The launcher's own path, read once at start: after a self-update the running file is gone.
pub fn exe_path() -> &'static Path {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from(LAUNCHER_BINARY)))
}

/// Development builds and `HORIZON_LAUNCHER_NO_SELF_UPDATE` never replace themselves.
pub fn self_update_allowed() -> bool {
    !cfg!(debug_assertions) && std::env::var_os("HORIZON_LAUNCHER_NO_SELF_UPDATE").is_none()
}

/// Downloads a launcher release and puts its binary in place of this one. The running process
/// keeps working; `restart` starts the new one.
pub fn self_update(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    let (url, size) = release.asset.clone().ok_or("This launcher release has no build for this platform.")?;
    let work = data_dir().join(".launcher-update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let archive = work.join("download");
    download(&url, size, &archive, &progress)?;
    let unpacked = work.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    let binary = find_file(&unpacked, LAUNCHER_BINARY).ok_or("The download has no launcher binary.")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&binary, fs::Permissions::from_mode(0o755));
    }
    self_replace::self_replace(&binary).map_err(|e| format!("Could not replace the launcher: {e}"))?;
    let _ = fs::remove_dir_all(&work);
    progress(1.0);
    Ok(())
}

fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file(&path, name) {
                return Some(found);
            }
        } else if entry.file_name() == name {
            return Some(path);
        }
    }
    None
}

/// Starts the launcher again (the updated binary) with the same arguments, and exits.
pub fn restart() -> ! {
    let _ = Command::new(exe_path()).args(std::env::args_os().skip(1)).spawn();
    std::process::exit(0)
}

/// Plain text from markdown, for terminals: headings on their own lines, bullets as "- ",
/// links followed by their address.
pub fn markdown_to_text(md: &str) -> String {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut out = String::new();
    let mut links = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new(md) {
        match event {
            Event::Start(Tag::Item) => {
                let depth = lists.len().saturating_sub(1);
                out.push_str(&"  ".repeat(depth));
                match lists.last_mut() {
                    Some(Some(n)) => {
                        out.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => out.push_str("- "),
                }
            }
            Event::Start(Tag::List(start)) => {
                if !out.ends_with('\n') && !out.is_empty() {
                    out.push('\n');
                }
                lists.push(start)
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                if let Some(url) = links.pop() {
                    out.push_str(&format!(" ({url})"));
                }
            }
            Event::End(TagEnd::Heading(_)) => out.push_str("\n\n"),
            Event::End(TagEnd::Paragraph) => out.push_str(if lists.is_empty() { "\n\n" } else { "\n" }),
            Event::End(TagEnd::Item) if !out.ends_with('\n') => out.push('\n'),
            Event::End(TagEnd::CodeBlock) => out.push('\n'),
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::Rule => out.push_str("---\n\n"),
            _ => {}
        }
    }
    out.trim_end().to_string() + "\n"
}

/// The first sentence-like line of a release's notes, for a one-line summary.
pub fn notes_summary(body: &str) -> Option<String> {
    let text = markdown_to_text(body);
    let line = text.lines().map(str::trim).find(|l| l.len() > 24 && !l.starts_with("---"))?;
    let line = line.trim_start_matches("- ");
    Some(match line.char_indices().nth(140) {
        Some((i, _)) => format!("{}…", line[..i].trim_end()),
        None => line.to_string(),
    })
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
    Updates(Result<Updates, String>),
    LauncherProgress(f32),
    LauncherUpdated(Result<(), String>),
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
    #[serde(default)]
    pub auto_install: bool,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn tags_never_mix() {
        assert!(is_game_tag("v2026.10.13"));
        assert!(!is_game_tag("launcher-v0.2.0"));
        assert!(!is_game_tag("vnext"));
        assert_eq!(launcher_version("launcher-v0.2.0"), Some("0.2.0"));
        assert_eq!(launcher_version("v2026.10.13"), None);
        assert_eq!(launcher_version("launcher-vx"), None);
        let list = serde_json::json!([
            {"tag_name": "v2026.10.12", "assets": [{"name": "horizon-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
            {"tag_name": "launcher-v9.0.0", "assets": [{"name": "horizon-launcher-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
            {"tag_name": "v2026.10.13", "assets": []},
            {"tag_name": "launcher-v0.0.1", "draft": true, "assets": []},
        ]);
        let game: Vec<_> = parse_releases(&list, ASSET, is_game_tag).into_iter().map(|r| r.tag).collect();
        assert_eq!(game, ["v2026.10.13", "v2026.10.12"]);
        let launcher: Vec<_> = parse_releases(&list, LAUNCHER_ASSET, is_launcher_tag).into_iter().map(|r| r.tag).collect();
        assert_eq!(launcher, ["launcher-v9.0.0"]);
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("v2026.10.13", "v2026.10.12"), Greater);
        assert_eq!(compare_versions("v2026.9.30", "v2026.10.1"), Less);
        assert_eq!(compare_versions("v2026.10.13.1", "v2026.10.13"), Greater);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("launcher-v1.2", "1.2.0"), Equal);
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Equal);
    }

    #[test]
    fn updates() {
        let r = |tag: &str| Release { tag: tag.into(), name: String::new(), body: String::new(), date: String::new(), asset: Some(("u".into(), 1)) };
        let releases = [r("v2026.10.13"), r("v2026.10.12")];
        assert_eq!(horizon_update(&releases, &["v2026.10.12".into()]).map(|r| r.tag), Some("v2026.10.13".into()));
        assert!(horizon_update(&releases, &["v2026.10.13".into()]).is_none());
        assert!(launcher_update(&[r("launcher-v0.0.1")]).is_none());
        assert!(launcher_update(&[r("launcher-v99.0.0")]).is_some());
        let installed = ["v2026.10.12".to_string(), "v2026.10.1".to_string()];
        assert!(select_after_update(Some("v2026.10.12"), &installed));
        assert!(!select_after_update(Some("v2026.10.1"), &installed));
        assert!(select_after_update(None, &installed));
    }

    #[test]
    fn plain_notes() {
        let text = markdown_to_text("# Title\n\nSome *words* and [a link](https://x.y).\n\n- one\n- two\n");
        assert_eq!(text, "Title\n\nSome words and a link (https://x.y).\n\n- one\n- two\n");
    }
}
