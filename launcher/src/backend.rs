//! Everything that is not UI: GitHub, installs, resource preparation, the game's processes and
//! updates. The GUI, the CLI and the MCP server all go through this module. Functions here
//! block; the GUI runs them on threads and reads their messages.
use crate::{account, mojang, sources};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub const REPO: &str = "ludovic111/horizon";
pub const README_URL: &str = "https://github.com/ludovic111/horizon/blob/main/launcher/README.md";
/// This launcher's version: the release workflow sets `HORIZON_LAUNCHER_VERSION` from the
/// tag it builds (no version-bump commits); other builds say Cargo.toml's.
pub const VERSION: &str = version_or(option_env!("HORIZON_LAUNCHER_VERSION"));
/// Launcher releases are tagged `launcher@X.Y.Z` from 0.2 on (docs/versioning.md), and Horizon's
/// own `vX.Y.Z`. Before 0.2 they were `launcher-vX.Y.Z` and `vYYYY.MM.DD[.N]`: every 0.2 tag is
/// newer than every one of those ([`release_order`]).
pub const LAUNCHER_TAG_PREFIX: &str = "launcher@";
/// The launcher tags from before 0.2 (`launcher-v0.16.0`).
pub const OLD_LAUNCHER_TAG_PREFIX: &str = "launcher-v";

const fn version_or(injected: Option<&'static str>) -> &'static str {
    match injected {
        Some(v) if !v.is_empty() => v,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub body: String,
    pub date: String,
    pub asset: Option<(String, u64)>,
    /// A beta: a game release not promoted to stable yet (a GitHub pre-release, docs/release-flow.md).
    pub beta: bool,
}

/// The release assets of each system Horizon ships for: (`std::env::consts::OS`, `ARCH`, the game,
/// the launcher). release.yml and launcher.yml publish exactly these (`platform_assets_are_published`).
pub const PLATFORMS: [(&str, &str, &str, &str); 4] = [
    (
        "linux",
        "x86_64",
        "horizon-linux-x86_64.tar.gz",
        "horizon-launcher-linux-x86_64.tar.gz",
    ),
    (
        "macos",
        "aarch64",
        "horizon-macos-arm64.zip",
        "horizon-launcher-macos-arm64.zip",
    ),
    // Intel Macs (#1161).
    (
        "macos",
        "x86_64",
        "horizon-macos-x64.zip",
        "horizon-launcher-macos-x64.zip",
    ),
    (
        "windows",
        "x86_64",
        "horizon-windows-x64.zip",
        "horizon-launcher-windows-x64.zip",
    ),
];

/// `a == b`, in a const context.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// This system's entry of `PLATFORMS`: the game's asset and the launcher's.
const fn platform_assets(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    let mut i = 0;
    while i < PLATFORMS.len() {
        let (o, a, game, launcher) = PLATFORMS[i];
        if same(o, os) && same(a, arch) {
            return Some((game, launcher));
        }
        i += 1;
    }
    None
}

const OWN_ASSETS: Option<(&str, &str)> =
    platform_assets(std::env::consts::OS, std::env::consts::ARCH);
/// The game build this launcher installs, None where Horizon has none.
pub const ASSET: Option<&str> = match OWN_ASSETS {
    Some((game, _)) => Some(game),
    None => None,
};
/// The launcher build this launcher updates itself from.
pub const LAUNCHER_ASSET: Option<&str> = match OWN_ASSETS {
    Some((_, launcher)) => Some(launcher),
    None => None,
};

const LAUNCHER_BINARY: &str = if cfg!(windows) {
    "horizon-launcher.exe"
} else {
    "horizon-launcher"
};

const CLIENT: &str = if cfg!(windows) {
    "horizon-client.exe"
} else {
    "horizon-client"
};

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .user_agent(&format!("horizon-launcher/{VERSION}"))
        .build()
}

fn get(url: &str) -> Result<ureq::Response, String> {
    let mut request = agent().get(url);
    // CI runners share addresses and hit GitHub's anonymous rate limit; they pass their own token.
    if let Some(token) = std::env::var("HORIZON_GITHUB_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
    {
        if url.starts_with("https://api.github.com/") {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
    }
    request.call().map_err(|e| match e {
        ureq::Error::Status(403, _) => {
            "GitHub's rate limit was reached. Try again in a few minutes.".into()
        }
        ureq::Error::Status(code, _) => format!("{url} answered {code}"),
        ureq::Error::Transport(t) => format!("No connection: {t}"),
    })
}

/// A Horizon game tag: `v` then a digit (`v2026.10.13`). Launcher tags never match.
pub fn is_game_tag(tag: &str) -> bool {
    tag.strip_prefix('v')
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// The launcher version in a `launcher@X.Y.Z` (or `launcher-vX.Y.Z`) tag, or None for any other tag.
pub fn launcher_version(tag: &str) -> Option<&str> {
    tag.strip_prefix(LAUNCHER_TAG_PREFIX)
        .or_else(|| tag.strip_prefix(OLD_LAUNCHER_TAG_PREFIX))
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// The tag this launcher was released as. launcher.yml gives it (`HORIZON_LAUNCHER_TAG`); a release
/// built before it did gave only the version, of a `launcher-v` tag; a build from source is
/// Cargo.toml's version on the 0.2 line.
pub fn own_tag() -> String {
    own_tag_of(
        option_env!("HORIZON_LAUNCHER_TAG"),
        option_env!("HORIZON_LAUNCHER_VERSION"),
    )
}

fn own_tag_of(tag: Option<&str>, injected: Option<&str>) -> String {
    match (
        tag.filter(|t| !t.is_empty()),
        injected.filter(|v| !v.is_empty()),
    ) {
        (Some(tag), _) => tag.to_string(),
        (None, Some(version)) => format!("{OLD_LAUNCHER_TAG_PREFIX}{version}"),
        (None, None) => format!("{LAUNCHER_TAG_PREFIX}{}", env!("CARGO_PKG_VERSION")),
    }
}

/// Where a tag or version stands among Horizon's releases (docs/versioning.md): the tags from
/// before 0.2 (`vYYYY.MM.DD[.N]`, `launcher-vX.Y.Z`) and bare versions first, compared number by
/// number with missing parts as zero and anything after a `-` ignored, as launchers always did;
/// then the 0.2 tags (`vX.Y.Z`, `launcher@X.Y.Z`) in semver's order, a pre-release
/// (`v0.3.0-rc.1`) before its release.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReleaseOrder {
    /// Numbers without trailing zeros, so 1.2 and 1.2.0 are equal.
    Before02(Vec<u64>),
    Semver([u64; 3], Prerelease),
}

/// (is a release, the pre-release's identifiers): a release follows every pre-release of its
/// version; identifiers compare one by one, numbers before words, a shorter list first.
pub type Prerelease = (bool, Vec<Identifier>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Identifier {
    Number(u64),
    Word(String),
}

/// Semver's majors stay below this; a date tag's first number is a year.
const FIRST_YEAR: u64 = 1000;

pub fn release_order(tag: &str) -> ReleaseOrder {
    let semver = tag
        .strip_prefix(LAUNCHER_TAG_PREFIX)
        .or_else(|| tag.strip_prefix('v'))
        .and_then(semver_order);
    semver.unwrap_or_else(|| {
        let v = tag
            .strip_prefix(OLD_LAUNCHER_TAG_PREFIX)
            .or_else(|| tag.strip_prefix(LAUNCHER_TAG_PREFIX))
            .unwrap_or(tag);
        let v = v.strip_prefix('v').unwrap_or(v);
        let mut parts: Vec<u64> = v
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        while parts.last() == Some(&0) {
            parts.pop();
        }
        ReleaseOrder::Before02(parts)
    })
}

/// `MAJOR.MINOR.PATCH[-PRE]` with a major below [`FIRST_YEAR`], or None.
fn semver_order(version: &str) -> Option<ReleaseOrder> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<u64> = core
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let &[major, minor, patch] = numbers.as_slice() else {
        return None;
    };
    if major >= FIRST_YEAR {
        return None;
    }
    let pre = match pre {
        None => (true, Vec::new()),
        Some(pre) => (
            false,
            pre.split('.')
                .map(|id| {
                    (!id.is_empty()).then(|| {
                        id.parse()
                            .map_or_else(|_| Identifier::Word(id.to_string()), Identifier::Number)
                    })
                })
                .collect::<Option<_>>()?,
        ),
    };
    Some(ReleaseOrder::Semver([major, minor, patch], pre))
}

/// Compares two tags or versions by [`release_order`].
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    release_order(a).cmp(&release_order(b))
}

/// Every published release of the repository, both kinds, as GitHub's JSON.
fn release_list() -> Result<serde_json::Value, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases?per_page=100");
    get(&url)?.into_json().map_err(|e| e.to_string())
}

/// The releases in `list` whose tag passes `keep`, newest first, with the asset named `asset`.
fn parse_releases(
    list: &serde_json::Value,
    asset: Option<&str>,
    keep: fn(&str) -> bool,
) -> Vec<Release> {
    let mut out = Vec::new();
    for r in list.as_array().into_iter().flatten() {
        let tag = r["tag_name"].as_str().unwrap_or_default();
        if r["draft"].as_bool().unwrap_or(false) || !keep(tag) {
            continue;
        }
        let found = r["assets"].as_array().into_iter().flatten().find_map(|a| {
            (Some(a["name"].as_str()?) == asset).then(|| {
                Some((
                    a["browser_download_url"].as_str()?.to_string(),
                    a["size"].as_u64()?,
                ))
            })?
        });
        out.push(Release {
            tag: tag.into(),
            name: r["name"].as_str().unwrap_or_default().into(),
            body: r["body"].as_str().unwrap_or_default().replace("\r\n", "\n"),
            date: r["published_at"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(10)
                .collect(),
            asset: found,
            beta: r["prerelease"].as_bool().unwrap_or(false),
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
    Ok(launcher_releases_in(&release_list()?))
}

/// The launcher releases in `list` (the newest 100 releases). Game releases come out several times
/// a day, so the newest launcher release can be older than all of those: then the newest
/// `launcher@` and `launcher-v` tags are looked up one by one (a draft's tag is skipped: it has no
/// public release).
fn launcher_releases_in(list: &serde_json::Value) -> Vec<Release> {
    launcher_releases_in_with(list, |url| {
        get(&url)?.into_json().map_err(|e| e.to_string())
    })
}

fn launcher_releases_in_with(
    list: &serde_json::Value,
    json: impl Fn(String) -> Result<serde_json::Value, String>,
) -> Vec<Release> {
    let found = parse_releases(list, LAUNCHER_ASSET, is_launcher_tag);
    if found.iter().any(|r| r.asset.is_some()) {
        return found;
    }
    let api = format!("https://api.github.com/repos/{REPO}");
    let mut tags = Vec::new();
    for prefix in [LAUNCHER_TAG_PREFIX, OLD_LAUNCHER_TAG_PREFIX] {
        // Every `launcher@` tag is newer than every `launcher-v` one: those only when there is none.
        if tags.is_empty() {
            if let Ok(refs) = json(format!("{api}/git/matching-refs/tags/{prefix}")) {
                tags = newest_launcher_tags(&refs);
            }
        }
    }
    for tag in tags.into_iter().take(3) {
        if let Ok(release) = json(format!("{api}/releases/tags/{tag}")) {
            let one = parse_releases(
                &serde_json::json!([release]),
                LAUNCHER_ASSET,
                is_launcher_tag,
            );
            if one.iter().any(|r| r.asset.is_some()) {
                return one;
            }
        }
    }
    found
}

/// The launcher tags in GitHub's matching-refs answer, newest first.
fn newest_launcher_tags(refs: &serde_json::Value) -> Vec<String> {
    let mut tags: Vec<String> = refs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["ref"].as_str()?.strip_prefix("refs/tags/"))
        .filter(|t| is_launcher_tag(t))
        .map(String::from)
        .collect();
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

/// The newest Horizon release that has a build for this platform.
pub fn latest(releases: &[Release]) -> Option<&Release> {
    releases.iter().find(|r| r.asset.is_some())
}

/// The releases a launcher installs by itself: stable ones, and betas too when the player turned
/// "Beta builds" on in Settings.
pub fn channel(releases: &[Release], beta: bool) -> Vec<Release> {
    releases
        .iter()
        .filter(|r| beta || !r.beta)
        .cloned()
        .collect()
}

/// The community, sponsor and website links, from `launcher/links.json` on main, so they can change
/// without a launcher release. An empty address hides its button.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Links {
    pub discord: String,
    pub sponsor: String,
    pub website: String,
}

impl Default for Links {
    /// What the launcher uses until (or unless) it can read links.json.
    fn default() -> Self {
        Self {
            discord: "https://discord.gg/HcTeMZxe5E".into(),
            sponsor: "https://github.com/sponsors/ludovic111".into(),
            website: "https://playhorizon.dev".into(),
        }
    }
}

impl Links {
    /// Reads links.json. A key that is missing keeps its default, an empty one hides the link,
    /// and text that is not a JSON object gives the defaults.
    pub fn parse(text: &str) -> Self {
        let mut links = Self::default();
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(text) {
            for (key, slot) in [
                ("discord", &mut links.discord),
                ("sponsor", &mut links.sponsor),
                ("website", &mut links.website),
            ] {
                if let Some(url) = map.get(key).and_then(|v| v.as_str()) {
                    *slot = url.trim().to_string();
                }
            }
        }
        links
    }
}

/// The links from main, or the built-in ones when offline.
pub fn links() -> Links {
    let url = format!("https://raw.githubusercontent.com/{REPO}/main/launcher/links.json");
    get(&url)
        .ok()
        .and_then(|r| r.into_string().ok())
        .map_or_else(Links::default, |text| Links::parse(&text))
}

/// The launcher's data folder; `HORIZON_LAUNCHER_HOME` overrides it (for tests and scripts).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("HORIZON_LAUNCHER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("horizon-launcher")
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
    installed_in(&versions_dir())
}

fn installed_in(versions: &Path) -> Vec<String> {
    let mut tags: Vec<String> = fs::read_dir(versions)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(COMPLETE).exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

/// A small release file (its checksums or their signature), whole; None when it has none.
fn fetch_small(url: &str) -> Result<Option<Vec<u8>>, String> {
    let response = match get(url) {
        Ok(response) => response,
        Err(e) if e.ends_with("answered 404") => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut body = Vec::new();
    response
        .into_reader()
        .take(1 << 20)
        .read_to_end(&mut body)
        .map_err(|e| format!("Download interrupted: {e}"))?;
    Ok(Some(body))
}

/// The SHA-256 the release's signed checksums give for `asset`, or why it must not be installed.
fn signed_digest(tag: &str, asset: &str) -> Result<String, String> {
    crate::signed::digest(
        tag,
        asset,
        &horizon_release_sign::trusted_keys(),
        fetch_small,
    )
}

/// Downloads `url` into `to`; the file is kept only if its SHA-256 is `sha256`, the signed one.
fn download(
    url: &str,
    size: u64,
    sha256: &str,
    to: &Path,
    progress: &impl Fn(f32),
) -> Result<(), String> {
    let mut reader = get(url)?.into_reader();
    let mut file = fs::File::create(to).map_err(|e| e.to_string())?;
    let mut hash = horizon_release_sign::Sha256::default();
    let (mut done, mut buf) = (0u64, vec![0u8; 1 << 16]);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        progress((done as f32 / size.max(1) as f32).min(1.0) * 0.9);
    }
    let name = url.rsplit('/').next().unwrap_or(url);
    horizon_release_sign::check(name, sha256, &hash.hex()).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })
}

fn unpack(archive: &Path, url: &str, to: &Path) -> Result<(), String> {
    let file = fs::File::open(archive).map_err(|e| e.to_string())?;
    if url.ends_with(".tar.gz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file))
            .unpack(to)
            .map_err(|e| e.to_string())
    } else {
        zip::ZipArchive::new(file)
            .and_then(|mut z| z.extract(to))
            .map_err(|e| e.to_string())
    }
}

/// Downloads one release, checks it against its signed checksums and unpacks it; `progress` gets
/// the fraction done.
pub fn install(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    let tag = release.tag.as_str();
    if !is_game_tag(tag) {
        return Err(format!("{tag} is not a Horizon release."));
    }
    let (url, size) = release
        .asset
        .clone()
        .ok_or_else(|| format!("{tag} has no build for this platform."))?;
    let sha256 = signed_digest(tag, ASSET.unwrap_or_default())?;
    let root = versions_dir();
    let partial = root.join(format!(".{tag}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    let archive = partial.join("download");
    download(&url, size, &sha256, &archive, &progress)?;
    let unpacked = partial.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    progress(0.97);
    // The archives hold one top folder (horizon-linux-x86_64/...); the version folder is its contents.
    let entries: Vec<_> = fs::read_dir(&unpacked)
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
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
    reuse_prepared(&root, tag);
    progress(1.0);
    Ok(())
}

pub fn remove(tag: &str) -> Result<(), String> {
    fs::remove_dir_all(version_dir(tag)).map_err(|e| e.to_string())
}

/// The Minecraft version a Horizon build targets, from its research-lock.json.
pub fn game_version(tag: &str) -> String {
    game_version_at(&version_dir(tag))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn game_version_at(dir: &Path) -> String {
    read_json(&dir.join("research-lock.json"))
        .and_then(|v| v["minecraft"]["version"].as_str().map(String::from))
        .unwrap_or_else(|| horizon_core::VANILLA_VERSION.into())
}

pub fn resources(tag: &str) -> PathBuf {
    resources_at(&version_dir(tag))
}

fn resources_at(dir: &Path) -> PathBuf {
    dir.join(".local/horizon-client").join(game_version_at(dir))
}

const RESOURCES_MANIFEST: &str = "horizon-resources.json";

pub fn is_prepared(tag: &str) -> bool {
    resources(tag).join(RESOURCES_MANIFEST).exists()
}

/// Whether the resources `from` prepared are the ones preparing `to` would make. The output
/// depends on the Horizon version, not only on Minecraft's: the manifest records the hashes of
/// the vanilla exporters it was made with (mods/horizon/src/test/java/.../parity), and the
/// preparer writes every file and the manifest's `schema`. So both need the same pinned
/// Minecraft jars (research-lock.json), the same preparer and the same exporter sources. The
/// preparer is crates/horizon-prepare (its sources ship in the release folder beside the
/// binary) and tools/prepare_native_client.py (the Python preparer of older versions, a wrapper
/// of the binary since); a file only one of the two versions has is a different preparer. The
/// helpers the Python preparer imported (lab.py and the test_*_parity.py modules) change
/// between releases in how they find and check the player's files, which the output does not
/// depend on; they are not compared.
fn resources_compatible(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let pins = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).map(|lock| {
            ["version", "client_sha1", "server_sha1"].map(|k| lock["minecraft"][k].clone())
        })
    };
    let (Some(a), Some(b)) = (pins(from), pins(to)) else {
        return false;
    };
    if a != b || a.iter().any(|v| !v.is_string()) {
        return false;
    }
    let Some(fields) = manifest.as_object() else {
        return false;
    };
    let mut preparer = vec![PathBuf::from("tools/prepare_native_client.py")];
    for dir in [from, to] {
        preparer.extend(files_under(dir, Path::new(PREPARER_CRATE)));
    }
    let same_or_absent = |path: &Path| match (fs::read(from.join(path)), fs::read(to.join(path))) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => {
            a.kind() == std::io::ErrorKind::NotFound && b.kind() == std::io::ErrorKind::NotFound
        }
        _ => false,
    };
    if !preparer.iter().all(|p| same_or_absent(p)) {
        return false;
    }
    let mut sources = vec![
        "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java".to_string(),
    ];
    for (_, value) in fields
        .iter()
        .filter(|(key, _)| key.ends_with("_exporters_sha256"))
    {
        let Some(files) = value.as_object() else {
            return false;
        };
        sources.extend(files.keys().cloned());
    }
    let same = |source: &str| {
        let path = Path::new(source);
        path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && matches!((fs::read(from.join(path)), fs::read(to.join(path))), (Ok(a), Ok(b)) if a == b)
    };
    sources.iter().all(|s| same(s))
}

/// Where a version folder keeps the preparer's sources (the release package copies them).
const PREPARER_CRATE: &str = "crates/horizon-prepare";

/// The files under `dir/relative`, as paths relative to `dir`.
fn files_under(dir: &Path, relative: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir.join(relative)) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = relative.join(entry.file_name());
        match entry.file_type() {
            Ok(t) if t.is_dir() => files.extend(files_under(dir, &path)),
            Ok(_) => files.push(path),
            Err(_) => {}
        }
    }
    files
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// After an install: when `tag` is not prepared but another installed version made the same
/// resources, copies them (not hard links: preparing again rewrites files in place, which would
/// change both). The player then needs neither their Minecraft folder nor a new preparation to
/// play a release that only changed Horizon. Returns the version they came from.
fn reuse_prepared(versions: &Path, tag: &str) -> Option<String> {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if to.join(RESOURCES_MANIFEST).exists() {
        return None;
    }
    let version = game_version_at(&target);
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let source = versions.join(&other);
        if game_version_at(&source) != version {
            continue;
        }
        let from = resources_at(&source);
        let Some(manifest) = read_json(&from.join(RESOURCES_MANIFEST)) else {
            continue;
        };
        if !resources_compatible(&source, &target, &manifest) {
            continue;
        }
        // Copied beside it and renamed when complete: an interrupted copy is never "prepared".
        // The name is this call's own, as the window's start and an install can both copy.
        static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = to.with_file_name(format!(".{version}.partial-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        let copied = copy_dir(&from, &partial).and_then(|_| {
            if to.join(RESOURCES_MANIFEST).exists() {
                return Ok(false); // Prepared meanwhile, by another copy or the player.
            }
            let _ = fs::remove_dir_all(&to);
            fs::rename(&partial, &to).map(|_| true)
        });
        let _ = fs::remove_dir_all(&partial);
        match copied {
            Ok(true) => return Some(other),
            Ok(false) => return None,
            Err(_) => {}
        }
    }
    None
}

/// `reuse_prepared` for every installed version that is not prepared, oldest first, so a
/// version prepared this way can serve the next. Launchers before 0.1.4 installed new versions
/// without it; this catches those up. Returns each (version, where its resources came from).
/// Prepared versions also get the run-time reports they lack (`complete_reports`).
pub fn reuse_prepared_everywhere() -> Vec<(String, String)> {
    reuse_prepared_in(&versions_dir())
}

fn reuse_prepared_in(versions: &Path) -> Vec<(String, String)> {
    let mut tags = installed_in(versions);
    tags.reverse();
    let reused = tags
        .iter()
        .filter_map(|tag| reuse_prepared(versions, tag).map(|from| (tag.clone(), from)))
        .collect();
    for tag in &tags {
        complete_reports(versions, tag);
    }
    reused
}

/// The data generator reports the game reads at run time: the built-in registries' network ids
/// (without them no server can be joined or hosted) and the command tree.
const RUNTIME_REPORTS: [&str; 2] = ["registries.json", "commands.json"];

/// Puts the data generator reports the game reads at run time (`RUNTIME_REPORTS`) into a prepared version's
/// resources when they are missing. Preparers before they shipped there left them only in the
/// scratch output of the version that ran the data generator (`.local/datagen/out/reports`),
/// and resources copied from another version (`reuse_prepared`) came without them. They depend
/// only on the server jar, so the version's own scratch output or any installed version pinning
/// the same jar serves, and no new preparation is needed. Returns whether all are there.
fn complete_reports(versions: &Path, tag: &str) -> bool {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if !to.join(RESOURCES_MANIFEST).exists() {
        return false;
    }
    let missing: Vec<_> = RUNTIME_REPORTS
        .iter()
        .filter(|name| !to.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return true;
    }
    let server = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).and_then(|lock| {
            let pins = ["version", "server_sha1"].map(|k| lock["minecraft"][k].clone());
            pins.iter().all(|v| v.is_string()).then_some(pins)
        })
    };
    let Some(pins) = server(&target) else {
        return false;
    };
    let reports = |dir: &Path| dir.join(".local/datagen/out/reports");
    let mut sources = vec![reports(&target)];
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let dir = versions.join(&other);
        if server(&dir).as_ref() == Some(&pins) {
            sources.push(resources_at(&dir));
            sources.push(reports(&dir));
        }
    }
    let mut complete = true;
    for name in missing {
        let Some(from) = sources.iter().map(|s| s.join(name)).find(|p| p.is_file()) else {
            complete = false;
            continue;
        };
        // Copied beside it and renamed: an interrupted copy never looks complete.
        let partial = to.join(format!(".{name}.partial-{}", std::process::id()));
        let copied = fs::copy(&from, &partial).and_then(|_| fs::rename(&partial, to.join(name)));
        if copied.is_err() {
            let _ = fs::remove_file(&partial);
            complete = false;
        }
    }
    complete
}

/// The official launcher's game directory, else the first other launcher's found
/// (sources.rs); preparing looks in all of them anyway.
pub fn default_minecraft_dir() -> Option<PathBuf> {
    let folders = sources::Folders::current()?;
    sources::known_folders(sources::Os::current(), &folders)
        .into_iter()
        .map(|(_, dir)| dir)
        .find(|dir| dir.is_dir())
}

fn found(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn python() -> Option<&'static str> {
    let candidates: &[&'static str] = if cfg!(windows) {
        &["py", "python"]
    } else {
        &["python3", "python"]
    };
    candidates.iter().copied().find(|p| found(p, "--version"))
}

/// How a version's resources are prepared.
pub enum Preparer {
    /// horizon-prepare, linked into the launcher: the version ships the same preparer sources.
    InProcess(horizon_prepare::Options),
    /// The version's own preparer: its horizon-prepare binary when its sources differ from the
    /// launcher's (a newer or older release), or the Python script of versions made before it.
    Process(Command),
}

/// The version's horizon-prepare binary, when it ships one.
fn preparer_binary(dir: &Path) -> Option<PathBuf> {
    let binary = dir.join(if cfg!(windows) {
        "horizon-prepare.exe"
    } else {
        "horizon-prepare"
    });
    binary.is_file().then_some(binary)
}

/// Downloads the pinned version from Mojang into `dir` when the signed-in account owns
/// Minecraft (asked again now), else says what signing in would do, as the last line of the
/// not-found error.
fn download_for_owner(
    pins: &sources::Pins,
    dir: &Path,
    fetch: sources::Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    let owner = (|| -> Result<Option<(String, bool)>, String> {
        let ep = account::Endpoints::live();
        let mut accounts = account::Accounts::load();
        let Some((a, secrets)) = account::session_for_launch(&ep, account::store(), &mut accounts)?
        else {
            return Ok(None);
        };
        Ok(Some((a.name, account::owns_minecraft(&ep, &secrets)?)))
    })();
    match owner {
        Ok(Some((name, true))) => {
            log(format!(
                "{name} owns Minecraft: downloading {v} from Mojang"
            ));
            let os = match sources::Os::current() {
                sources::Os::Linux => "linux",
                sources::Os::MacOs => "osx",
                sources::Os::Windows => "windows",
            };
            mojang::download(pins, dir, os, 8, fetch, log).map_err(|e| {
                format!("\n  • Horizon tried to download {v} from Mojang for {name}, but: {e}")
            })
        }
        Ok(Some((name, false))) => Err(format!(
            "\n  • {name}, the signed-in account, doesn't own Minecraft: Java Edition, so Horizon \
             can't download it."
        )),
        Ok(None) => Err(format!(
            "\n  • Or sign in with the Microsoft account that owns Minecraft: Horizon then \
             downloads {v} from Mojang itself."
        )),
        Err(e) => Err(format!(
            "\n  • Downloading {v} from Mojang needs your Microsoft account, which could not be \
             checked: {e}"
        )),
    }
}

/// Checks what the preparation needs and picks how to run it. PLAY.txt describes the same
/// steps: the player's own launcher install, the server jar from that version's own json, and
/// Java (and Python, for versions before horizon-prepare). Also returns the Minecraft folder
/// to prepare from: `minecraft`, or the files another launcher keeps, linked into the official
/// launcher's layout (sources.rs).
pub fn preparer(
    tag: &str,
    minecraft: &Path,
    log: &dyn Fn(String),
) -> Result<(Preparer, PathBuf), String> {
    let version = game_version(tag);
    let dir = version_dir(tag);
    let reused = horizon_prepare::artifacts::reuse_imported(&dir, &|line| log(line.to_string()))?;
    let minecraft = if reused {
        minecraft.to_path_buf()
    } else {
        let pins = read_json(&dir.join("research-lock.json"))
            .and_then(|lock| sources::Pins::read(&lock))
            .ok_or("The version's research-lock.json doesn't pin the Minecraft files")?;
        let own = data_dir().join("minecraft");
        let mut known = vec![("Horizon's own download", own.clone())];
        known.extend(
            sources::Folders::current()
                .map(|f| sources::known_folders(sources::Os::current(), &f))
                .unwrap_or_default(),
        );
        let fetch = |url: &str| -> Result<Vec<u8>, String> {
            let mut body = Vec::new();
            get(url)?
                .into_reader()
                .read_to_end(&mut body)
                .map_err(|e| e.to_string())?;
            Ok(body)
        };
        let linked = data_dir().join("linked-minecraft");
        match sources::resolve(minecraft, &pins, &known, &linked, &fetch, log)? {
            sources::Resolved::Folder(dir) => dir,
            sources::Resolved::NotFound(checked) => {
                let not_found = sources::not_found(&version, &checked, minecraft);
                download_for_owner(&pins, &own, &fetch, log).map_err(|e| not_found + &e)?;
                own
            }
        }
    };
    let manifest = minecraft
        .join("versions")
        .join(&version)
        .join(format!("{version}.json"));
    let binary = preparer_binary(&dir);
    let python = if binary.is_some() {
        // The JDK the version pins, found as horizon-prepare finds it, so a missing one is
        // reported before the server jar is downloaded.
        let major = read_json(&dir.join("research-lock.json"))
            .and_then(|lock| lock["minecraft"]["java_major"].as_i64())
            .ok_or("The version's research-lock.json has no java_major")?;
        horizon_prepare::java::find(major)?;
        None
    } else {
        let python = python()
            .ok_or("This version prepares with Python: Python 3.10 or newer is needed. Install it, or pick a newer version.")?;
        if !found("java", "-version") {
            return Err(
                "Java 25 is needed (a JDK, e.g. Eclipse Temurin 25). Install it and try again."
                    .into(),
            );
        }
        Some(python)
    };
    let jar = data_dir()
        .join("server-jars")
        .join(format!("{version}.jar"));
    if !reused && !jar.exists() {
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let url = json["downloads"]["server"]["url"]
            .as_str()
            .ok_or_else(|| format!("{} has no server download", manifest.display()))?;
        log(format!("Downloading the {version} server jar from Mojang"));
        fs::create_dir_all(jar.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut body = Vec::new();
        get(url)?
            .into_reader()
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        let partial = jar.with_extension("partial");
        fs::write(&partial, body)
            .and_then(|_| fs::rename(&partial, &jar))
            .map_err(|e| e.to_string())?;
    }
    if let Some(binary) = binary {
        if horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE)).as_deref()
            == Some(horizon_prepare::SOURCES_SHA256)
        {
            let options = horizon_prepare::Options {
                root: dir,
                import: !reused,
                minecraft_dir: Some(minecraft.clone()),
                server_jar: (!reused).then_some(jar),
            };
            return Ok((Preparer::InProcess(options), minecraft));
        }
        let mut cmd = Command::new(binary);
        cmd.current_dir(&dir).arg("--root").arg(&dir);
        if !reused {
            cmd.args(["--prepare", "--server-jar"]).arg(&jar);
        }
        cmd.arg("--minecraft-dir").arg(&minecraft);
        return Ok((Preparer::Process(cmd), minecraft));
    }
    let mut cmd = Command::new(python.unwrap());
    cmd.current_dir(&dir)
        .args(["-u", "tools/prepare_native_client.py"]);
    if !reused {
        cmd.args(["--prepare", "--server-jar"]).arg(&jar);
    }
    cmd.arg("--minecraft-dir").arg(&minecraft);
    Ok((Preparer::Process(cmd), minecraft))
}

/// Copies the pinned version's sounds from the player's own launcher install into the version's
/// resources (`horizon-prepare sounds`), each line to `log`. Sound is optional, so a launcher
/// install without them is a line in the log, never a failed preparation. The version's own
/// preparer does it when it is not the launcher's and knows how; versions from before it read
/// the sounds where the launcher's preparer puts them.
pub fn sounds(tag: &str, minecraft: &Path, log: &dyn Fn(String)) {
    let dir = version_dir(tag);
    let own = horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE)).as_deref()
        == Some(horizon_prepare::SOURCES_SHA256);
    let binary = preparer_binary(&dir).filter(|binary| {
        !own && Command::new(binary)
            .arg("--help")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("horizon-prepare sounds"))
    });
    log("Copying the sounds from your Minecraft install".into());
    let result = match binary {
        Some(binary) => Command::new(binary)
            .current_dir(&dir)
            .arg("sounds")
            .arg("--root")
            .arg(&dir)
            .arg("--minecraft-dir")
            .arg(minecraft)
            .output()
            .map_err(|e| e.to_string())
            .and_then(|o| {
                let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
                for line in text(&o.stdout).lines() {
                    log(line.to_string());
                }
                match o.status.success() {
                    true => Ok(()),
                    false => Err(text(&o.stderr).trim_start_matches("error: ").to_string()),
                }
            }),
        None => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            horizon_prepare::sounds(&dir, Some(minecraft), &|line| log(line.to_string()))
        }))
        .unwrap_or_else(|_| Err("The sound copy stopped unexpectedly".into()))
        .map(|_| ()),
    };
    if let Err(e) = result {
        log(format!("No sound (the game will be silent): {e}"));
    }
}

/// Runs `Preparer::InProcess` on this thread, each progress line to `log`. A panic is an
/// error like any other, so the window never waits on a preparation that died.
pub fn prepare_in_process(
    options: &horizon_prepare::Options,
    log: &dyn Fn(String),
) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        horizon_prepare::prepare(options, &|line| log(line.to_string()))
    }))
    .unwrap_or_else(|_| Err("The preparation stopped unexpectedly".into()))
    .map(|_| ())
}

/// Where the game keeps saves and options.txt: `HORIZON_HOME`, else `horizon` in the platform's
/// data folder (~/.local/share/horizon on Linux, as the release folders' horizon.sh uses). It is
/// never inside a version folder, so installing, updating or removing a version keeps the saves.
pub fn game_home() -> PathBuf {
    if let Some(home) = std::env::var_os("HORIZON_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("horizon")
}

// ----- Logs and crash reports ---------------------------------------------------------------------

/// The game's logs (`latest.log` and the day archives), as it writes them in its home.
pub fn logs_dir() -> PathBuf {
    game_home().join("logs")
}

/// Where the game writes `crash-<date>_<time>-client.txt` when it crashes.
pub fn crash_reports_dir() -> PathBuf {
    game_home().join("crash-reports")
}

/// The crash reports in `dir`, oldest first (their names sort by time).
fn crash_reports_in(dir: &Path) -> Vec<PathBuf> {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    reports
}

/// The newest crash report, if any.
pub fn newest_crash_report() -> Option<PathBuf> {
    crash_reports_in(&crash_reports_dir()).pop()
}

/// The newest crash report written at or after `since` (one from the run that started then).
pub fn crash_report_since(since: std::time::SystemTime) -> Option<PathBuf> {
    crash_report_in_since(&crash_reports_dir(), since)
}

fn crash_report_in_since(dir: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_reports_in(dir).into_iter().rev().find(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since)
    })
}

/// The error a crash report describes, on one line: the lines between its description and the
/// stack (`panicked at src/game.rs:10:5: boom`).
pub fn crash_summary(report: &str) -> Option<String> {
    let mut lines = report
        .lines()
        .skip_while(|l| !l.starts_with("Description:"));
    lines.next()?;
    let error: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('\t'))
        .map(str::trim)
        .collect();
    (!error.is_empty()).then(|| error.join(" "))
}

/// The line the game prints when it has drawn its first frame.
pub const GAME_FIRST_FRAME: &str = "Horizon: first frame";

/// Whether a game's output ends in a crash while its Vulkan renderer started: an adapter line
/// (`Horizon renderer: <GPU> (Vulkan, <type>)`) and no first frame. A native crash in a graphics
/// driver leaves no crash report, only those lines (#746).
pub fn vulkan_start_crashed<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> bool {
    let mut vulkan = false;
    for line in lines {
        let line = line.as_ref();
        if line.starts_with(GAME_FIRST_FRAME) {
            return false;
        }
        if let Some(adapter) = line.strip_prefix("Horizon renderer: ") {
            // The adapter line, not one of the start's steps: "<GPU> (<backend>, <type>)".
            if adapter.ends_with(')') && adapter.contains(" (") {
                vulkan = adapter.contains(" (Vulkan, ");
            }
        }
    }
    vulkan
}

/// The game's graphics settings file in its home (the game's `graphics.rs`).
const GRAPHICS_FILE: &str = "horizon-graphics.txt";

/// Sets `graphicsApi:dx12` in `home`'s graphics settings when it is unset or `auto`, keeping
/// every other line. True when it changed; a player's own `vulkan` or `dx12` stays.
pub fn switch_to_dx12(home: &Path) -> Result<bool, String> {
    let path = home.join(GRAPHICS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    for line in text.lines() {
        match line.split_once(':') {
            Some((key, value)) if key.trim() == "graphicsApi" => {
                if value.trim() != "auto" {
                    return Ok(false);
                }
                found = true;
                lines.push("graphicsApi:dx12".to_owned());
            }
            _ => lines.push(line.to_owned()),
        }
    }
    if !found {
        lines.push("graphicsApi:dx12".to_owned());
    }
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// The last `lines` lines of `file`, or why it cannot be read.
fn tail_of(file: &Path, lines: usize) -> Result<String, String> {
    let bytes = fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..].join("\n") + "\n")
}

/// `logs`: the end of the game's latest.log, or with `crash` its newest crash report. The path
/// read comes first.
pub fn read_logs(lines: usize, crash: bool) -> Result<(PathBuf, String), String> {
    if crash {
        let path = newest_crash_report()
            .ok_or_else(|| format!("No crash report in {}.", crash_reports_dir().display()))?;
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok((path, text));
    }
    let path = logs_dir().join("latest.log");
    let text = tail_of(&path, lines.max(1))?;
    Ok((path, text))
}

/// Which drawing the game's menus use. Classic is vanilla's own screens, which have no way to
/// switch back, so the launcher is where the player returns to Horizon's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuStyle {
    Horizon,
    Classic,
}

impl MenuStyle {
    pub fn name(self) -> &'static str {
        match self {
            MenuStyle::Horizon => "horizon",
            MenuStyle::Classic => "classic",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "horizon" => Some(MenuStyle::Horizon),
            "classic" => Some(MenuStyle::Classic),
            _ => None,
        }
    }

    /// The style `horizon.txt` holds: Horizon unless it has the line `menuStyle:classic`, as the
    /// game reads it (crates/horizon-client/src/ui_theme.rs).
    fn from_file(text: &str) -> Self {
        if text.lines().any(|l| l.trim() == "menuStyle:classic") {
            MenuStyle::Classic
        } else {
            MenuStyle::Horizon
        }
    }

    /// `text` with its `menuStyle:` line set to this style and every other line kept.
    fn into_file(self, text: &str) -> String {
        let mut out: Vec<String> = text
            .lines()
            .filter(|l| !l.trim().starts_with("menuStyle:"))
            .map(String::from)
            .collect();
        out.insert(0, format!("menuStyle:{}", self.name()));
        out.join("\n") + "\n"
    }
}

/// The menu style saved in the game home's `horizon.txt` (Horizon when there is none).
pub fn menu_style() -> MenuStyle {
    MenuStyle::from_file(&fs::read_to_string(game_home().join("horizon.txt")).unwrap_or_default())
}

pub fn set_menu_style(style: MenuStyle) -> Result<(), String> {
    let home = game_home();
    fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let file = home.join("horizon.txt");
    let text = style.into_file(&fs::read_to_string(&file).unwrap_or_default());
    fs::write(file, text).map_err(|e| e.to_string())
}

/// Splits the player's extra game arguments (Settings' one line) into words, the way a shell
/// would: spaces separate them, single or double quotes group a word with spaces in it, and a
/// backslash escapes the next character (outside quotes, or inside double quotes).
pub fn split_args(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    // Inside double quotes only `"` and `\` are escaped, as a shell does.
                    match chars.next() {
                        Some(n @ ('"' | '\\')) => word.push(n),
                        Some(n) => {
                            word.push('\\');
                            word.push(n);
                        }
                        None => word.push('\\'),
                    }
                } else {
                    word.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    if let Some(n) = chars.next() {
                        word.push(n);
                        started = true;
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                _ => {
                    word.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The extra game arguments the player set in Settings, split into words.
pub fn game_args() -> Vec<String> {
    split_args(&Settings::load().game_args)
}

/// Appends the player's extra game arguments, so they add to the launcher's own or override them.
pub fn append_game_args(cmd: &mut Command) {
    cmd.args(game_args());
}

pub fn client_command(tag: &str) -> Command {
    let home = game_home();
    let _ = fs::create_dir_all(&home);
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(home).arg("--resources").arg(resources(tag));
    // PipeWire remembers volume and mute per application name. The game's own name is
    // "PipeWire ALSA [horizon-client]", shared with development and test runs, where
    // muting it once left every later game silent: played from here it is "Horizon".
    if std::env::var_os("PIPEWIRE_PROPS").is_none() {
        cmd.env(
            "PIPEWIRE_PROPS",
            "{ application.name = \"Horizon\" media.role = \"Game\" }",
        );
    }
    cmd
}

pub fn server_command(tag: &str, folder: &Path) -> Command {
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(folder)
        .arg("--resources")
        .arg(resources(tag))
        .args(["--dedicated", "--nogui"]);
    cmd
}

pub fn eula_agreed(folder: &Path) -> bool {
    fs::read_to_string(folder.join("eula.txt"))
        .is_ok_and(|s| s.lines().any(|l| l.trim() == "eula=true"))
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
        return installed
            .iter()
            .find(|t| *t == tag)
            .cloned()
            .ok_or_else(|| format!("{tag} is not installed."));
    }
    let selected = Settings::load().selected.filter(|s| installed.contains(s));
    selected
        .or_else(|| installed.first().cloned())
        .ok_or_else(|| "No version is installed yet.".into())
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
/// official launcher's default. It may be absent when imported files can be reused.
pub fn minecraft_dir(requested: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(requested
        .or(Settings::load().minecraft_dir)
        .or_else(default_minecraft_dir)
        .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None)))
}

/// The server folder: the one given, the one chosen in the launcher, or `server/` in the data folder.
pub fn server_dir(requested: Option<PathBuf>) -> PathBuf {
    requested
        .or(Settings::load().server_dir)
        .unwrap_or_else(|| data_dir().join("server"))
}

/// Whether `tag` is prepared, with the run-time reports it needs completed when another copy has them.
pub fn need_prepared(tag: &str) -> Result<(), String> {
    if is_prepared(tag) {
        complete_reports(&versions_dir(), tag);
        Ok(())
    } else {
        Err(format!(
            "{tag} is not prepared yet. Prepare it first (it needs your own Minecraft {}).",
            game_version(tag)
        ))
    }
}

/// The client's command, once its resources are prepared.
pub fn client(tag: &str) -> Result<Command, String> {
    need_prepared(tag)?;
    let mut cmd = client_command(tag);
    // A sign-in that can't be renewed (no network, expired) still lets the game start, offline.
    if let Err(e) = crate::account::attach_session(&mut cmd, &game_home()) {
        eprintln!("Playing offline: {e}");
    }
    append_game_args(&mut cmd);
    Ok(cmd)
}

/// Start the game straight into a friend's world: `--server friend:CODE`.
pub fn join_friend(cmd: &mut Command, code: &str) -> Result<(), String> {
    horizon_friends::invite::parse_detailed(code).map_err(|e| e.to_string())?;
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    cmd.arg("--server")
        .arg(format!("{}{code}", horizon_friends::ADDRESS_PREFIX));
    Ok(())
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
        Some(tag) => releases
            .into_iter()
            .find(|r| r.tag == tag)
            .ok_or_else(|| format!("No Horizon release is tagged {tag}.")),
        None => releases
            .into_iter()
            .next()
            .ok_or_else(|| "No release is published yet.".into()),
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
        log.iter()
            .skip(log.len().saturating_sub(lines))
            .cloned()
            .collect()
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
    let newer = installed
        .iter()
        .all(|t| compare_versions(&newest.tag, t).is_gt());
    newer.then(|| newest.clone())
}

/// The newest launcher release with an archive for this platform, if newer than this launcher
/// ([`own_tag`]: a launcher from before 0.2 takes any `launcher@` release).
pub fn launcher_update(releases: &[Release]) -> Option<Release> {
    launcher_update_from(releases, &own_tag())
}

fn launcher_update_from(releases: &[Release], own: &str) -> Option<Release> {
    releases
        .iter()
        .find(|r| r.asset.is_some())
        .filter(|r| compare_versions(&r.tag, own).is_gt())
        .cloned()
}

/// Load the game's release list and both update kinds from the same GitHub response.
/// Publish the game releases before any fallback lookups for older launcher tags, so those
/// lookups cannot hold up the release pages or opening animation. A list error reaches both.
pub fn releases_and_updates(
    on_releases: impl FnOnce(Result<Vec<Release>, String>),
) -> Result<Updates, String> {
    releases_and_updates_with(
        Settings::load().beta_builds,
        release_list,
        installed,
        launcher_releases_in,
        on_releases,
    )
}

fn releases_and_updates_with(
    beta: bool,
    load: impl FnOnce() -> Result<serde_json::Value, String>,
    installed: impl FnOnce() -> Vec<String>,
    launchers: impl FnOnce(&serde_json::Value) -> Vec<Release>,
    on_releases: impl FnOnce(Result<Vec<Release>, String>),
) -> Result<Updates, String> {
    let list = match load() {
        Ok(list) => list,
        Err(e) => {
            on_releases(Err(e.clone()));
            return Err(e);
        }
    };
    let releases = parse_releases(&list, ASSET, is_game_tag);
    let horizon = horizon_update(&channel(&releases, beta), &installed());
    on_releases(Ok(releases));
    Ok(Updates {
        horizon,
        launcher: launcher_update(&launchers(&list)),
    })
}

/// A fresh release-list request for both kinds of update (plus older-launcher fallback if needed).
pub fn check_updates() -> Result<Updates, String> {
    releases_and_updates(|_| {})
}

/// Whether a version installed as an update becomes the selected one: only when the player was
/// on the newest installed version before (or had none).
pub fn select_after_update(selected: Option<&str>, installed_before: &[String]) -> bool {
    match selected {
        None => true,
        Some(s) => installed_before.first().is_none_or(|newest| newest == s),
    }
}

/// Installs `tag` (or with "latest" the newest release of the player's channel), then updates the selection as
/// `select_after_update` says when `as_update`, or always selects it otherwise.
pub fn install_tag(tag: &str, as_update: bool, progress: impl Fn(f32)) -> Result<String, String> {
    let releases = releases()?;
    let release = if tag == "latest" {
        let beta = Settings::load().beta_builds;
        releases
            .iter()
            .find(|r| r.asset.is_some() && (beta || !r.beta))
            .ok_or("No release has a build for this platform yet.")?
    } else {
        releases
            .iter()
            .find(|r| r.tag == tag)
            .ok_or_else(|| format!("No Horizon release is tagged {tag}."))?
    };
    if release.asset.is_none() {
        return Err(format!("{} has no build for this platform.", release.tag));
    }
    let before = installed();
    install(release, progress)?;
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

/// The version in `horizon-launcher --version`'s output (`horizon-launcher 0.1.5`).
fn parse_version_output(out: &str) -> Option<&str> {
    out.trim()
        .strip_prefix("horizon-launcher ")
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// The version of the launcher binary now at `path`, which is not this process's once another
/// update (the command line, another window) has replaced the file.
fn installed_launcher_version(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    parse_version_output(&String::from_utf8_lossy(&out.stdout)).map(String::from)
}

/// Puts `new` at `exe`, the path this launcher started from. The running process keeps its
/// file (unlinked) and works on. Unix only: `self_replace` asks the system where the running
/// binary is, and Linux answers "<path> (deleted)" once the file has been replaced, so a second
/// update in the same run failed with "No such file or directory". Windows keeps a running
/// binary locked, so there `self_replace` moves it aside first.
#[cfg(unix)]
fn replace_binary(new: &Path, exe: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "the launcher has no folder")
    })?;
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let temp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let mode = fs::metadata(exe).map_or(0o755, |m| m.permissions().mode() | 0o111);
    let result = fs::copy(new, &temp)
        .and_then(|_| fs::set_permissions(&temp, fs::Permissions::from_mode(mode)))
        .and_then(|_| fs::rename(&temp, exe));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(not(unix))]
fn replace_binary(new: &Path, _exe: &Path) -> std::io::Result<()> {
    self_replace::self_replace(new)
}

/// The app bundle the launcher runs from (`…/Horizon.app` for `…/Horizon.app/Contents/MacOS/
/// horizon-launcher`), or None for a bare binary, as launchers before the .app shipped.
pub fn app_bundle(exe: &Path) -> Option<&Path> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then_some(app)
}

/// Finds the first `*.app` folder under `dir`.
fn find_app(dir: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.extension().is_some_and(|e| e == "app") {
                return Some(path);
            }
            if let Some(found) = find_app(&path) {
                return Some(found);
            }
        }
    }
    None
}

/// Whether the bundle's signature (Developer ID or ad hoc) is whole: every file it seals is there
/// and unchanged.
#[cfg(target_os = "macos")]
fn signature_holds(app: &Path) -> bool {
    Command::new("codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Puts the bundle `new` in place of `app`: copied beside it first (`ditto` keeps its links,
/// modes and signature) and checked, then the old one is moved aside and the new one renamed in,
/// so `app` is never half written, and checked again where it lands (the old one comes back if it
/// doesn't hold). The running launcher keeps its open files and works on.
#[cfg(target_os = "macos")]
fn replace_bundle(new: &Path, app: &Path) -> Result<(), String> {
    let dir = app.parent().ok_or("The app has no folder.")?;
    let name = app.file_name().unwrap_or_default().to_string_lossy();
    let staged = dir.join(format!(".{name}.update-{}", std::process::id()));
    let old = dir.join(format!(".{name}.old-{}", std::process::id()));
    let _ = fs::remove_dir_all(&staged);
    let copied = Command::new("ditto")
        .arg(new)
        .arg(&staged)
        .status()
        .map_err(|e| format!("ditto: {e}"))?;
    if !copied.success() {
        let _ = fs::remove_dir_all(&staged);
        return Err(format!(
            "Could not copy the new app next to {}.",
            app.display()
        ));
    }
    if !signature_holds(&staged) {
        let _ = fs::remove_dir_all(&staged);
        return Err(
            "The downloaded app's signature doesn't hold; the launcher stays as it is.".into(),
        );
    }
    fs::rename(app, &old).map_err(|e| {
        let _ = fs::remove_dir_all(&staged);
        format!("Could not move {} aside: {e}", app.display())
    })?;
    if let Err(e) = fs::rename(&staged, app) {
        let _ = fs::rename(&old, app);
        let _ = fs::remove_dir_all(&staged);
        return Err(format!(
            "Could not put the new app at {}: {e}",
            app.display()
        ));
    }
    if !signature_holds(app) {
        let failed = dir.join(format!(".{name}.failed-{}", std::process::id()));
        if fs::rename(app, &failed).is_ok() && fs::rename(&old, app).is_ok() {
            let _ = fs::remove_dir_all(&failed);
        }
        return Err(format!(
            "The new app's signature doesn't hold at {}; the launcher stays as it was.",
            app.display()
        ));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn replace_bundle(_new: &Path, _app: &Path) -> Result<(), String> {
    Err("App bundles are a macOS thing.".into())
}

/// Downloads a launcher release and puts its binary in place of this one. The running process
/// keeps working; `restart` starts the new one. When the file there already is that release or
/// a newer one (updated from the command line or another window), there is nothing to do.
pub fn self_update(release: &Release, progress: impl Fn(f32)) -> Result<(), String> {
    let (url, size) = release
        .asset
        .clone()
        .ok_or("This launcher release has no build for this platform.")?;
    let target = launcher_version(&release.tag).unwrap_or("0");
    if installed_launcher_version(exe_path())
        .is_some_and(|v| v != VERSION && compare_versions(&v, target).is_ge())
    {
        progress(1.0);
        return Ok(());
    }
    let sha256 = signed_digest(&release.tag, LAUNCHER_ASSET.unwrap_or_default())?;
    let work = data_dir().join(".launcher-update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let archive = work.join("download");
    download(&url, size, &sha256, &archive, &progress)?;
    let unpacked = work.join("unpacked");
    unpack(&archive, &url, &unpacked)?;
    // A launcher inside Horizon.app replaces the whole bundle (its Info.plist, icon and
    // signature belong with the binary); a bare binary, as older launchers are, only the binary,
    // which the macOS archive also holds (Horizon.app/Contents/MacOS/horizon-launcher).
    if let Some(app) = app_bundle(exe_path()) {
        if let Some(new) = find_app(&unpacked) {
            replace_bundle(&new, app)?;
            let _ = fs::remove_dir_all(&work);
            progress(1.0);
            return Ok(());
        }
    }
    let binary =
        find_file(&unpacked, LAUNCHER_BINARY).ok_or("The download has no launcher binary.")?;
    replace_binary(&binary, exe_path()).map_err(|e| {
        format!(
            "Could not replace the launcher at {}: {e}",
            exe_path().display()
        )
    })?;
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

/// Starts the launcher again (the updated binary) with the same arguments in place of this one:
/// on Unix the process becomes the new binary (exec), on Windows the new one starts and this one
/// exits. Returns only when the new binary could not start.
pub fn restart() -> String {
    let mut cmd = Command::new(exe_path());
    cmd.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    let error = {
        use std::os::unix::process::CommandExt;
        cmd.exec()
    };
    #[cfg(not(unix))]
    let error = match cmd.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => e,
    };
    format!(
        "Could not start {}: {error}. Close the launcher and open it again.",
        exe_path().display()
    )
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
            Event::End(TagEnd::Paragraph) => {
                out.push_str(if lists.is_empty() { "\n\n" } else { "\n" })
            }
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
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| l.len() > 24 && !l.starts_with("---"))?;
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
    Links(Links),
    Progress(String, f32),
    Installed(String, Result<(), String>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
    Updates(Result<Updates, String>),
    LauncherProgress(f32),
    LauncherUpdated(Result<(), String>),
    /// `reuse_prepared_everywhere` prepared these versions.
    Reused(Vec<(String, String)>),
    /// Whether secrets are kept in the OS keychain (else only until the launcher closes).
    Keychain(bool),
    /// The selected account's session file for the game about to start (none = offline), or
    /// why it could not be renewed.
    Session(Result<Option<PathBuf>, String>),
    /// A modpack step's line (`modpacks`).
    PackLine(String),
    /// A modpack step ended: the pack as it now is.
    PackDone(Result<Box<crate::modpacks::Imported>, String>),
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
    let out = child
        .stdout
        .take()
        .map(|o| Box::new(o) as Box<dyn Read + Send>);
    let err = child
        .stderr
        .take()
        .map(|o| Box::new(o) as Box<dyn Read + Send>);
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
    /// Unset means on: only a player who turned it off has `false` here. 0.1.1 wrote its
    /// `auto_install` (off by default) for everyone, so this is a new key and that one is ignored.
    #[serde(default)]
    pub install_new_versions: Option<bool>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// Whether new versions include betas (releases not promoted to stable yet). Off by default.
    #[serde(default)]
    pub beta_builds: bool,
}

impl Settings {
    pub fn load() -> Self {
        Self::parse(&fs::read_to_string(data_dir().join("settings.json")).unwrap_or_default())
    }
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// Whether new Horizon versions are installed automatically.
    pub fn auto_install(&self) -> bool {
        self.install_new_versions.unwrap_or(true)
    }
    pub fn save(&self) {
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(
            data_dir().join("settings.json"),
            serde_json::to_string_pretty(self).unwrap(),
        );
    }
}

#[cfg(test)]
#[path = "backend/release_loading_tests.rs"]
mod release_loading_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn each_mac_picks_the_build_for_its_processor() {
        assert_eq!(
            platform_assets("macos", "aarch64"),
            Some((
                "horizon-macos-arm64.zip",
                "horizon-launcher-macos-arm64.zip"
            ))
        );
        assert_eq!(
            platform_assets("macos", "x86_64"),
            Some(("horizon-macos-x64.zip", "horizon-launcher-macos-x64.zip"))
        );
        assert_eq!(
            platform_assets("windows", "x86_64").map(|(game, _)| game),
            Some("horizon-windows-x64.zip")
        );
        assert_eq!(platform_assets("linux", "aarch64"), None);
        assert_eq!(platform_assets("macos", "x86"), None);
        assert_eq!(
            ASSET,
            platform_assets(std::env::consts::OS, std::env::consts::ARCH).map(|(game, _)| game)
        );
    }

    /// Every asset a launcher looks for is one the release workflows build, sign and publish.
    #[test]
    fn platform_assets_are_published() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let read = |path: &str| fs::read_to_string(repo.join(path)).unwrap();
        let release = read(".github/workflows/release.yml");
        let launcher = read(".github/workflows/launcher.yml");
        for (os, arch, game, launcher_asset) in PLATFORMS {
            assert!(
                release.contains(&format!("asset: {game}")),
                "release.yml builds no {game}"
            );
            // Signed (SHA256SUMS), then published.
            assert_eq!(
                release.matches(&format!("dist/{game}")).count(),
                2,
                "release.yml must sign and publish {game} ({os} {arch})"
            );
            assert!(
                launcher.contains(&format!("asset: {launcher_asset}")),
                "launcher.yml builds no {launcher_asset}"
            );
        }
    }

    #[test]
    fn the_game_plays_under_its_own_audio_stream_name() {
        if std::env::var_os("PIPEWIRE_PROPS").is_some() {
            return;
        }
        let cmd = client_command("v2026.10.13");
        let props = cmd
            .get_envs()
            .find(|(k, _)| *k == "PIPEWIRE_PROPS")
            .and_then(|(_, v)| v);
        assert!(props.is_some_and(|v| v
            .to_string_lossy()
            .contains("application.name = \"Horizon\"")));
    }

    #[test]
    fn a_crash_after_the_vulkan_adapter_line_and_before_the_first_frame_is_a_start_crash() {
        let start = [
            "Horizon: resources loaded in 11.3s; 35723 block states, 1658 items, 67 biomes",
            "Horizon renderer: Intel(R) HD Graphics 530 (Vulkan, IntegratedGpu)",
        ];
        assert!(vulkan_start_crashed(start));
        // A step line after the adapter keeps it a start crash.
        let step = [start[1], "Horizon renderer: creating the pipelines"];
        assert!(vulkan_start_crashed(step));
        // A crash after the first frame is the game's, not the renderer start's.
        let later = [start[1], "Horizon: first frame after 812 ms"];
        assert!(!vulkan_start_crashed(later));
        // Already on DirectX 12, or no renderer yet: nothing to switch.
        let dx12 = ["Horizon renderer: Intel(R) HD Graphics 530 (Dx12, IntegratedGpu)"];
        assert!(!vulkan_start_crashed(dx12));
        assert!(!vulkan_start_crashed(&start[..1]));
        // Vulkan failed to open and DirectX 12 took over: its line is the last adapter.
        assert!(!vulkan_start_crashed([start[1], dx12[0]]));
    }

    #[test]
    fn a_renderer_start_crash_switches_auto_to_dx12_once() {
        let home = std::env::temp_dir().join(format!("horizon-dx12-switch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        // No settings file yet: one with the API alone.
        assert_eq!(switch_to_dx12(&home), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "graphicsApi:dx12\n"
        );
        // Already DirectX 12, or the player's own Vulkan: left alone.
        assert_eq!(switch_to_dx12(&home), Ok(false));
        fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:vulkan\n").unwrap();
        assert_eq!(switch_to_dx12(&home), Ok(false));
        // Auto becomes DirectX 12 and the other settings stay.
        fs::write(
            home.join(GRAPHICS_FILE),
            "sky:false\ngraphicsApi:auto\nfog:true\n",
        )
        .unwrap();
        assert_eq!(switch_to_dx12(&home), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "sky:false\ngraphicsApi:dx12\nfog:true\n"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn extra_game_arguments_split_like_a_shell() {
        assert!(split_args("").is_empty());
        assert!(split_args("   ").is_empty());
        assert_eq!(split_args("--server-thread"), ["--server-thread"]);
        assert_eq!(
            split_args("--render-distance 32 --server-thread"),
            ["--render-distance", "32", "--server-thread"]
        );
        // Quotes keep a word with spaces together; either kind works.
        assert_eq!(
            split_args("--name 'Two Words' --world \"My World\""),
            ["--name", "Two Words", "--world", "My World"]
        );
        // An empty quoted word is still a word.
        assert_eq!(split_args("--name \"\""), ["--name", ""]);
        // A backslash escapes a space outside quotes, and " and \ inside double quotes.
        assert_eq!(split_args("--name My\\ World"), ["--name", "My World"]);
        assert_eq!(split_args("a\\tb c"), ["atb", "c"]);
        assert_eq!(split_args("\"a\\\"b\" 'c\\d'"), ["a\"b", "c\\d"]);
    }

    #[test]
    fn the_game_arguments_setting_round_trips_and_reaches_the_command() {
        let settings = Settings::parse(r#"{"game_args":"--server-thread"}"#);
        assert_eq!(settings.game_args, "--server-thread");
        // Unset in an older settings.json: no arguments, and nothing is written for them.
        let old = Settings::parse(r#"{"installed":true}"#);
        assert_eq!(old.game_args, "");
        assert!(!serde_json::to_string(&old).unwrap().contains("game_args"));

        let mut cmd = Command::new("true");
        cmd.args(split_args("--server-thread --render-distance 40"));
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--server-thread", "--render-distance", "40"]);
    }

    #[test]
    fn tags_never_mix() {
        assert!(is_game_tag("v2026.10.13"));
        assert!(!is_game_tag("launcher-v0.2.0"));
        assert!(!is_game_tag("vnext"));
        assert_eq!(launcher_version("launcher-v0.2.0"), Some("0.2.0"));
        assert_eq!(launcher_version("launcher@0.2.0"), Some("0.2.0"));
        assert!(!is_game_tag("launcher@0.2.0"));
        assert!(is_game_tag("v0.2.0"));
        assert!(is_launcher_tag("launcher@0.2.0"));
        assert_eq!(launcher_version("v2026.10.13"), None);
        assert_eq!(launcher_version("launcher-vx"), None);
        let list = serde_json::json!([
            {"tag_name": "v2026.10.12", "assets": [{"name": "horizon-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
            {"tag_name": "launcher-v9.0.0", "assets": [{"name": "horizon-launcher-linux-x86_64.tar.gz", "browser_download_url": "u", "size": 1}]},
            {"tag_name": "v2026.10.13", "assets": []},
            {"tag_name": "launcher-v0.0.1", "draft": true, "assets": []},
        ]);
        let game: Vec<_> = parse_releases(&list, ASSET, is_game_tag)
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(game, ["v2026.10.13", "v2026.10.12"]);
        let launcher: Vec<_> = parse_releases(&list, LAUNCHER_ASSET, is_launcher_tag)
            .into_iter()
            .map(|r| r.tag)
            .collect();
        assert_eq!(launcher, ["launcher-v9.0.0"]);
    }

    /// Rolling a release back (`gh release edit <tag> --draft`, docs/release-flow.md) makes the
    /// previous one the newest again: it is what `install latest` and the update check offer.
    #[test]
    fn withdrawn_release_is_never_offered() {
        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v2026.10.15.1", "draft": true, "assets": [{"name": asset, "browser_download_url": "bad", "size": 1}]},
            {"tag_name": "v2026.10.15", "assets": [{"name": asset, "browser_download_url": "good", "size": 1}]},
            {"tag_name": "v2026.10.14", "assets": [{"name": asset, "browser_download_url": "old", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        assert!(releases.iter().all(|r| r.tag != "v2026.10.15.1"));
        assert_eq!(
            latest(&releases).map(|r| r.tag.as_str()),
            Some("v2026.10.15")
        );
        let offered = |installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            horizon_update(&releases, &installed).map(|r| r.tag)
        };
        assert_eq!(offered(&["v2026.10.14"]), Some("v2026.10.15".into()));
        assert_eq!(offered(&[]), Some("v2026.10.15".into()));
        // Who installed the withdrawn one is not told to "update" to an older version.
        assert_eq!(offered(&["v2026.10.15.1", "v2026.10.14"]), None);
    }

    /// Betas (GitHub pre-releases) install by themselves only with "Beta builds" on; the stable
    /// channel waits for promote.yml (docs/release-flow.md, "Channels").
    #[test]
    fn updates_follow_the_stable_channel_unless_betas_are_on() {
        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v2026.10.19.02", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "b", "size": 1}]},
            {"tag_name": "v2026.10.19.01", "assets": [{"name": asset, "browser_download_url": "s", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        assert_eq!(
            releases.iter().map(|r| r.beta).collect::<Vec<_>>(),
            [true, false]
        );
        let offered = |beta: bool, installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|t| t.to_string()).collect();
            horizon_update(&channel(&releases, beta), &installed).map(|r| r.tag)
        };
        assert_eq!(offered(false, &[]), Some("v2026.10.19.01".into()));
        assert_eq!(
            offered(true, &["v2026.10.19.01"]),
            Some("v2026.10.19.02".into())
        );
        assert_eq!(offered(false, &["v2026.10.19.01"]), None);
        // A beta installed earlier is never "updated" to an older stable build.
        assert_eq!(offered(false, &["v2026.10.19.02"]), None);
        assert!(!Settings::parse("{}").beta_builds);
        assert!(Settings::parse(r#"{"beta_builds": true}"#).beta_builds);
    }

    #[test]
    fn the_version_is_the_one_the_release_workflow_injects() {
        assert_eq!(version_or(Some("0.4.2")), "0.4.2");
        assert_eq!(version_or(Some("")), env!("CARGO_PKG_VERSION"));
        assert_eq!(version_or(None), env!("CARGO_PKG_VERSION"));
        assert_eq!(
            VERSION,
            option_env!("HORIZON_LAUNCHER_VERSION")
                .filter(|v| !v.is_empty())
                .unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        // Self-update compares releases with that version: this launcher's own release is no update.
        let r = |tag: String| Release {
            tag,
            name: String::new(),
            body: String::new(),
            date: String::new(),
            asset: Some(("u".into(), 1)),
            beta: false,
        };
        assert!(launcher_update(&[r(own_tag())]).is_none());
        assert_eq!(launcher_version(&own_tag()), Some(VERSION));
        assert_eq!(
            parse_version_output(&format!("horizon-launcher {VERSION}\n")),
            Some(VERSION)
        );
    }

    #[test]
    fn a_launcher_knows_which_line_it_was_released_on() {
        // launcher.yml gives the tag; before it did, a release had only its `launcher-v` version.
        assert_eq!(
            own_tag_of(Some("launcher@0.2.3"), Some("0.2.3")),
            "launcher@0.2.3"
        );
        assert_eq!(
            own_tag_of(Some("launcher-v0.17.0"), Some("0.17.0")),
            "launcher-v0.17.0"
        );
        assert_eq!(own_tag_of(None, Some("0.16.0")), "launcher-v0.16.0");
        assert_eq!(own_tag_of(Some(""), Some("0.16.0")), "launcher-v0.16.0");
        // A build from source is on the 0.2 line.
        assert_eq!(
            own_tag_of(None, None),
            format!("launcher@{}", env!("CARGO_PKG_VERSION"))
        );
    }

    /// The update path to 0.2 (docs/versioning.md): launchers from before it take the last
    /// `launcher-v` release, which takes every `launcher@` one.
    #[test]
    fn launchers_from_before_0_2_update_to_it() {
        let r = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            asset: Some(("u".into(), 1)),
            beta: false,
        };
        let mut list = vec![
            r("launcher-v0.16.0"),
            r("launcher@0.2.0"),
            r("launcher-v0.17.0"),
            r("launcher@0.2.1"),
        ];
        list.sort_by(|a, b| compare_versions(&b.tag, &a.tag));
        let tags: Vec<_> = list.iter().map(|r| r.tag.as_str()).collect();
        assert_eq!(
            tags,
            [
                "launcher@0.2.1",
                "launcher@0.2.0",
                "launcher-v0.17.0",
                "launcher-v0.16.0"
            ]
        );
        let offered = |own: &str| launcher_update_from(&list, own).map(|r| r.tag);
        assert_eq!(offered("launcher-v0.16.0"), Some("launcher@0.2.1".into()));
        assert_eq!(offered("launcher-v0.17.0"), Some("launcher@0.2.1".into()));
        assert_eq!(offered("launcher@0.2.0"), Some("launcher@0.2.1".into()));
        assert_eq!(offered("launcher@0.2.1"), None);
        assert_eq!(offered("launcher@0.3.0-rc.1"), None);
        // The old launchers' own check only sees `launcher-v` tags: 0.17.0 is their way over.
        let old_scheme: Vec<_> = list
            .iter()
            .filter(|r| r.tag.starts_with(OLD_LAUNCHER_TAG_PREFIX))
            .cloned()
            .collect();
        assert_eq!(
            launcher_update_from(&old_scheme, "launcher-v0.16.0").map(|r| r.tag),
            Some("launcher-v0.17.0".into())
        );
    }

    #[test]
    fn game_0_2_follows_every_date_release() {
        assert_eq!(compare_versions("v0.2.0", "v2026.10.19.15"), Greater);
        assert_eq!(compare_versions("v0.2.0", "v2026.10.21"), Greater);
        assert_eq!(compare_versions("v2099.12.31.9", "v0.2.0"), Less);
        assert_eq!(compare_versions("v0.2.1", "v0.2.0"), Greater);
        assert_eq!(compare_versions("v0.2.10", "v0.2.9"), Greater);
        assert_eq!(compare_versions("v0.3.0", "v0.2.57"), Greater);
        assert_eq!(compare_versions("v0.2.0", "v0.2.0"), Equal);
        // Pre-releases come before their release and after the one before.
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0"), Less);
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.2.9"), Greater);
        assert_eq!(compare_versions("v0.3.0-beta.11", "v0.3.0-beta.2"), Greater);
        assert_eq!(compare_versions("v0.3.0-rc.1", "v0.3.0-beta.9"), Greater);
        assert_eq!(compare_versions("v0.3.0-beta.1", "v0.3.0-beta"), Greater);
        assert_eq!(compare_versions("v0.3.0-alpha", "v0.3.0-1"), Greater);
        assert_eq!(compare_versions("v0.2.1-beta.1", "v2026.10.21"), Greater);
        // Launcher and game tags of 0.2 compare alike; old launcher tags stay below.
        assert_eq!(
            compare_versions("launcher@0.2.0", "launcher-v0.16.0"),
            Greater
        );
        assert_eq!(compare_versions("launcher@0.2.0", "v0.2.0"), Equal);

        let asset = ASSET.unwrap_or("horizon-linux-x86_64.tar.gz");
        let list = serde_json::json!([
            {"tag_name": "v0.2.0", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "b", "size": 1}]},
            {"tag_name": "v2026.10.21", "prerelease": true, "assets": [{"name": asset, "browser_download_url": "d", "size": 1}]},
            {"tag_name": "v2026.10.19.15", "assets": [{"name": asset, "browser_download_url": "s", "size": 1}]},
        ]);
        let releases = parse_releases(&list, Some(asset), is_game_tag);
        let installed = ["v2026.10.19.15".to_string(), "v2026.10.12".to_string()];
        let offered = |beta| horizon_update(&channel(&releases, beta), &installed).map(|r| r.tag);
        // A beta 0.2.0 reaches launchers with Beta builds on; stable ones wait for its promotion.
        assert_eq!(offered(true), Some("v0.2.0".into()));
        assert_eq!(offered(false), None);
        let mut sorted = vec![
            "v2026.10.19.15".to_string(),
            "v0.2.1".into(),
            "v0.2.0".into(),
            "v2026.10.21".into(),
        ];
        sorted.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(
            sorted,
            ["v0.2.1", "v0.2.0", "v2026.10.21", "v2026.10.19.15"]
        );
    }

    #[test]
    fn the_fallback_looks_for_0_2_launchers_first() {
        let asset = LAUNCHER_ASSET.unwrap_or("unsupported");
        let release = |tag: &str| serde_json::json!({"tag_name": tag, "assets": [{"name": asset, "browser_download_url": "u", "size": 1}]});
        let asked = std::cell::RefCell::new(Vec::new());
        let lookup = |with_new: bool| {
            asked.borrow_mut().clear();
            launcher_releases_in_with(&serde_json::json!([]), |url| {
                asked
                    .borrow_mut()
                    .push(url.rsplit('/').next().unwrap_or("").to_string());
                if url.ends_with("matching-refs/tags/launcher@") {
                    Ok(serde_json::json!(if with_new {
                        vec![serde_json::json!({"ref": "refs/tags/launcher@0.2.0"})]
                    } else {
                        Vec::new()
                    }))
                } else if url.ends_with("matching-refs/tags/launcher-v") {
                    Ok(serde_json::json!([{"ref": "refs/tags/launcher-v0.17.0"}]))
                } else {
                    Ok(release(url.rsplit('/').next().unwrap_or("")))
                }
            })
            .into_iter()
            .map(|r| r.tag)
            .collect::<Vec<_>>()
        };
        assert_eq!(lookup(true), ["launcher@0.2.0"]);
        assert_eq!(*asked.borrow(), ["launcher@", "launcher@0.2.0"]);
        // Before the first `launcher@` release, the `launcher-v` ones.
        assert_eq!(lookup(false), ["launcher-v0.17.0"]);
        assert_eq!(
            *asked.borrow(),
            ["launcher@", "launcher-v", "launcher-v0.17.0"]
        );
    }

    #[test]
    fn launcher_tags_from_matching_refs_newest_first() {
        let refs = serde_json::json!([
            {"ref": "refs/tags/launcher-v0.1.9"},
            {"ref": "refs/tags/launcher-v0.1.10"},
            {"ref": "refs/tags/launcher-vnext"},
            {"ref": "refs/tags/v2026.10.15"},
            {"ref": "refs/tags/launcher-v0.2.0"},
            {"ref": "refs/tags/launcher@0.2.0"},
        ]);
        assert_eq!(
            newest_launcher_tags(&refs),
            [
                "launcher@0.2.0",
                "launcher-v0.2.0",
                "launcher-v0.1.10",
                "launcher-v0.1.9"
            ]
        );
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
        let r = |tag: &str| Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            asset: Some(("u".into(), 1)),
            beta: false,
        };
        let releases = [r("v2026.10.13"), r("v2026.10.12")];
        assert_eq!(
            horizon_update(&releases, &["v2026.10.12".into()]).map(|r| r.tag),
            Some("v2026.10.13".into())
        );
        assert!(horizon_update(&releases, &["v2026.10.13".into()]).is_none());
        assert!(launcher_update(&[r("launcher-v0.0.1")]).is_none());
        assert!(launcher_update(&[r("launcher@99.0.0")]).is_some());
        assert!(launcher_update_from(&[r("launcher-v99.0.0")], "launcher-v0.16.0").is_some());
        let installed = ["v2026.10.12".to_string(), "v2026.10.1".to_string()];
        assert!(select_after_update(Some("v2026.10.12"), &installed));
        assert!(!select_after_update(Some("v2026.10.1"), &installed));
        assert!(select_after_update(None, &installed));
    }

    #[test]
    fn new_versions_install_automatically_unless_turned_off() {
        // No settings file, an old one that never mentioned it, and 0.1.1's default `false`.
        assert!(Settings::parse("").auto_install());
        assert!(Settings::parse("{}").auto_install());
        assert!(
            Settings::parse(r#"{"selected": "v2026.10.13", "auto_install": false}"#).auto_install()
        );
        assert!(Settings::parse(r#"{"install_new_versions": true}"#).auto_install());
        assert!(!Settings::parse(r#"{"install_new_versions": false}"#).auto_install());
        // Saving after the player turned it off keeps it off.
        let off = Settings {
            install_new_versions: Some(false),
            ..Settings::default()
        };
        assert!(!Settings::parse(&serde_json::to_string(&off).unwrap()).auto_install());
        // The selection rule is unchanged: the new version is selected only from the newest.
        let installed = ["v2026.10.12".to_string(), "v2026.10.1".to_string()];
        assert!(select_after_update(Some("v2026.10.12"), &installed));
        assert!(!select_after_update(Some("v2026.10.1"), &installed));
    }

    #[test]
    fn links_fall_back_to_the_built_in_ones() {
        let defaults = Links::default();
        assert_eq!(defaults.discord, "https://discord.gg/HcTeMZxe5E");
        assert_eq!(defaults.sponsor, "https://github.com/sponsors/ludovic111");
        assert_eq!(defaults.website, "https://playhorizon.dev");
        // The file on main says the same thing as the defaults.
        let file = Links::parse(
            r#"{"discord": "https://discord.gg/HcTeMZxe5E", "sponsor": "https://github.com/sponsors/ludovic111", "website": "https://playhorizon.dev"}"#,
        );
        assert_eq!(file, defaults);
        // Unreadable or missing, offline included: the defaults.
        assert_eq!(Links::parse(""), defaults);
        assert_eq!(Links::parse("<html>404</html>"), defaults);
        assert_eq!(Links::parse("[1]"), defaults);
        assert_eq!(Links::parse(r#"{"discord": 5}"#), defaults);
        // A key that is missing keeps its default; a new address replaces it; empty hides it.
        assert_eq!(
            Links::parse(r#"{"discord": "https://x.y/z"}"#),
            Links {
                discord: "https://x.y/z".into(),
                ..defaults.clone()
            }
        );
        assert_eq!(
            Links::parse(r#"{"discord": "", "sponsor": " "}"#),
            Links {
                discord: String::new(),
                sponsor: String::new(),
                ..defaults.clone()
            }
        );
        // A key this launcher does not know is ignored.
        assert_eq!(Links::parse(r#"{"mastodon": "https://x.y/z"}"#), defaults);
    }

    #[test]
    fn menu_style_file_matches_the_games() {
        assert_eq!(MenuStyle::from_file(""), MenuStyle::Horizon);
        assert_eq!(
            MenuStyle::from_file("menuStyle:horizon\n"),
            MenuStyle::Horizon
        );
        assert_eq!(
            MenuStyle::from_file("menuStyle:classic\n"),
            MenuStyle::Classic
        );
        assert_eq!(
            MenuStyle::from_file("other:1\nmenuStyle:classic\n"),
            MenuStyle::Classic
        );
        assert_eq!(MenuStyle::Classic.into_file(""), "menuStyle:classic\n");
        assert_eq!(
            MenuStyle::Horizon.into_file("menuStyle:classic\nother:1\n"),
            "menuStyle:horizon\nother:1\n"
        );
        assert_eq!(MenuStyle::parse("classic"), Some(MenuStyle::Classic));
        assert_eq!(MenuStyle::parse("vanilla"), None);
    }

    #[test]
    fn a_new_version_reuses_resources_prepared_by_a_compatible_one() {
        let versions =
            std::env::temp_dir().join(format!("horizon-launcher-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&versions);
        let oracle = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java";
        let camera = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCameraOracle.java";
        let collision =
            "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java";
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, minecraft: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": minecraft, "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "tools/prepare_native_client.py", "schema 7");
            write(tag, oracle, "heightmap");
            write(tag, collision, "collision");
            write(tag, camera, tag); // Not an exporter: it may change freely.
        };
        let prepared = |tag: &str, minecraft: &str| {
            let manifest =
                serde_json::json!({"schema": 7, "collision_exporters_sha256": {collision: "x"}});
            write(
                tag,
                &format!(".local/horizon-client/{minecraft}/{RESOURCES_MANIFEST}"),
                &manifest.to_string(),
            );
            write(
                tag,
                &format!(".local/horizon-client/{minecraft}/worldgen/biomes.json"),
                "{}",
            );
        };
        let is_prepared = |tag: &str| {
            resources_at(&versions.join(tag))
                .join(RESOURCES_MANIFEST)
                .exists()
        };

        release("v2026.10.13", "26.3");
        prepared("v2026.10.13", "26.3");
        release("v2026.10.12", "26.2");
        prepared("v2026.10.12", "26.2");
        release("v2026.10.13.1", "26.3");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.13.1"),
            Some("v2026.10.13".into())
        );
        assert!(is_prepared("v2026.10.13.1"));
        assert!(resources_at(&versions.join("v2026.10.13.1"))
            .join("worldgen/biomes.json")
            .exists());
        let left: Vec<_> = fs::read_dir(versions.join("v2026.10.13.1/.local/horizon-client"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["26.3"]);
        // Already prepared: nothing to do.
        assert_eq!(reuse_prepared(&versions, "v2026.10.13.1"), None);

        // Another Minecraft version, a changed exporter or a changed prepare script: prepare again.
        release("v2026.10.14", "26.4");
        assert_eq!(reuse_prepared(&versions, "v2026.10.14"), None);
        release("v2026.10.15", "26.3");
        write("v2026.10.15", collision, "collision, fixed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.15"), None);
        release("v2026.10.16", "26.3");
        write("v2026.10.16", "tools/prepare_native_client.py", "schema 8");
        assert_eq!(reuse_prepared(&versions, "v2026.10.16"), None);
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));

        // A version an older launcher installed without preparing it is caught up by the sweep
        // that starting the window, `versions` and `update` run; the incompatible ones stay as
        // they are, and a second sweep has nothing to do.
        release("v2026.10.17", "26.3");
        assert_eq!(
            reuse_prepared_in(&versions),
            [("v2026.10.17".to_string(), "v2026.10.13.1".to_string())]
        );
        assert!(is_prepared("v2026.10.17"));
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));
        assert!(reuse_prepared_in(&versions).is_empty());

        // Versions with horizon-prepare: its sources are the preparer, so the same sources
        // reuse each other's resources, and a change or a version without them prepares again.
        let crate_file = "crates/horizon-prepare/src/lib.rs";
        release("v2026.10.18", "26.3");
        write("v2026.10.18", crate_file, "rust preparer");
        assert_eq!(reuse_prepared(&versions, "v2026.10.18"), None);
        prepared("v2026.10.18", "26.3");
        release("v2026.10.19", "26.3");
        write("v2026.10.19", crate_file, "rust preparer");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.19"),
            Some("v2026.10.18".into())
        );
        release("v2026.10.20", "26.3");
        write("v2026.10.20", crate_file, "rust preparer, changed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.20"), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_versions_get_the_run_time_reports_without_preparing_again() {
        let versions = std::env::temp_dir().join(format!(
            "horizon-launcher-reports-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, server: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": server}});
            write(tag, "research-lock.json", &lock.to_string());
            write(
                tag,
                &format!(".local/horizon-client/26.3/{RESOURCES_MANIFEST}"),
                "{}",
            );
        };
        let resources = |tag: &str| resources_at(&versions.join(tag));
        let datagen = |tag: &str, name: &str| {
            write(tag, &format!(".local/datagen/out/reports/{name}"), tag);
        };

        // Prepared before the reports shipped with the resources: its own scratch output.
        release("v2026.10.13", "s");
        datagen("v2026.10.13", "registries.json");
        datagen("v2026.10.13", "commands.json");
        // Resources copied from it: the reports come from the version that ran the generator.
        release("v2026.10.14", "s");
        // Another server jar: the reports of the others would be wrong, and it has none.
        release("v2026.10.15", "other");
        assert!(reuse_prepared_in(&versions).is_empty());
        for tag in ["v2026.10.13", "v2026.10.14"] {
            for name in RUNTIME_REPORTS {
                assert_eq!(
                    fs::read_to_string(resources(tag).join(name)).unwrap(),
                    "v2026.10.13"
                );
            }
        }
        assert!(!resources("v2026.10.15").join("registries.json").exists());
        assert!(!complete_reports(&versions, "v2026.10.15"));
        // Once there they stay as they are, even when the scratch output is gone.
        fs::remove_dir_all(versions.join("v2026.10.13/.local/datagen")).unwrap();
        assert!(complete_reports(&versions, "v2026.10.13"));
        let left: Vec<_> = fs::read_dir(resources("v2026.10.14"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.contains("partial"))
            .collect();
        assert!(left.is_empty());
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn finds_the_app_bundle_around_the_binary() {
        let exe = Path::new("/Applications/Horizon.app/Contents/MacOS/horizon-launcher");
        assert_eq!(
            app_bundle(exe),
            Some(Path::new("/Applications/Horizon.app"))
        );
        assert_eq!(app_bundle(Path::new("/Users/me/horizon-launcher")), None);
        assert_eq!(
            app_bundle(Path::new("/x/Contents/MacOS/horizon-launcher")),
            None
        );
        let work = std::env::temp_dir().join(format!("horizon-find-app-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        fs::create_dir_all(work.join("dist/Horizon.app/Contents/MacOS")).unwrap();
        assert_eq!(find_app(&work), Some(work.join("dist/Horizon.app")));
        let _ = fs::remove_dir_all(&work);
    }

    /// The whole update of an app bundle, as the release ships it: signed, zipped with `ditto`,
    /// unpacked by the launcher, swapped in place of the installed one, still valid.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_app_bundle_updates_in_place() {
        use std::os::unix::fs::PermissionsExt;
        let work = std::env::temp_dir().join(format!("horizon-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&work);
        let make = |dir: &Path, version: &str| {
            let app = dir.join("Horizon.app");
            fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            fs::create_dir_all(app.join("Contents/Resources")).unwrap();
            let bin = app.join("Contents/MacOS/horizon-launcher");
            fs::copy("/usr/bin/true", &bin).unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(app.join("Contents/Resources/version"), version).unwrap();
            fs::write(
                app.join("Contents/Info.plist"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\
                 <key>CFBundleExecutable</key><string>horizon-launcher</string>\
                 <key>CFBundleIdentifier</key><string>dev.horizon.test</string>\
                 <key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n",
            )
            .unwrap();
            let signed = Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&app)
                .status()
                .unwrap();
            assert!(signed.success());
            app
        };
        let installed = make(&work.join("Applications"), "1.0.0");
        let release = make(&work.join("release"), "2.0.0");
        let zip = work.join("release.zip");
        let zipped = Command::new("ditto")
            .args(["-c", "-k", "--norsrc", "--noextattr", "--keepParent"])
            .arg(&release)
            .arg(&zip)
            .status()
            .unwrap();
        assert!(zipped.success());
        unpack(&zip, "release.zip", &work.join("unpacked")).unwrap();
        let new = find_app(&work.join("unpacked")).unwrap();
        replace_bundle(&new, &installed).unwrap();
        let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
        assert_eq!(version, "2.0.0");
        let bin = installed.join("Contents/MacOS/horizon-launcher");
        assert!(
            Command::new(&bin).status().unwrap().success(),
            "still executable"
        );
        let valid = Command::new("codesign")
            .args(["--verify", "--strict"])
            .arg(&installed)
            .status()
            .unwrap();
        assert!(valid.success(), "the updated bundle's signature holds");
        let left: Vec<_> = fs::read_dir(work.join("Applications"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            left,
            ["Horizon.app"],
            "nothing staged or moved aside is left"
        );

        // A bundle changed after it was signed is refused, and the installed one stays.
        let tampered = make(&work.join("tampered"), "3.0.0");
        fs::write(tampered.join("Contents/Resources/version"), "3.0.1").unwrap();
        assert!(replace_bundle(&tampered, &installed).is_err());
        let version = fs::read_to_string(installed.join("Contents/Resources/version")).unwrap();
        assert_eq!(version, "2.0.0");
        let left = fs::read_dir(work.join("Applications")).unwrap().count();
        assert_eq!(left, 1, "nothing staged is left");
        let _ = fs::remove_dir_all(&work);
    }

    #[test]
    fn launcher_versions_read_from_the_binary() {
        assert_eq!(
            parse_version_output("horizon-launcher 0.1.5\n"),
            Some("0.1.5")
        );
        assert_eq!(parse_version_output("something else"), None);
        assert_eq!(parse_version_output(""), None);
        // This test binary is not a launcher: no version, so an update goes ahead.
        assert_eq!(
            installed_launcher_version(&std::env::current_exe().unwrap()),
            None
        );
        assert_eq!(
            installed_launcher_version(Path::new("/nonexistent/horizon-launcher")),
            None
        );
    }

    /// The launcher's file was replaced while it ran (an update from the command line, or an
    /// earlier one): the next replacement still lands at the path it started from.
    #[cfg(unix)]
    #[test]
    fn a_replaced_binary_is_replaced_again_at_its_path() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("horizon-launcher-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("horizon-launcher");
        fs::write(&exe, "old").unwrap();
        // Another update swaps the file under the running launcher.
        fs::write(dir.join("other"), "other update").unwrap();
        fs::rename(dir.join("other"), &exe).unwrap();
        fs::write(dir.join("new"), "new").unwrap();
        replace_binary(&dir.join("new"), &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        // The file there keeps its mode, made executable (the swapped-in one was 0644).
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // Gone altogether: it is put back, executable.
        fs::remove_file(&exe).unwrap();
        replace_binary(&dir.join("new"), &exe).unwrap();
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(
            fs::metadata(&exe).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // No temporary file is left beside it.
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["horizon-launcher", "new"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_reports_are_found_and_summarised() {
        let dir =
            std::env::temp_dir().join(format!("horizon-launcher-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let start = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
        assert!(crash_report_in_since(&dir, start).is_none());
        fs::create_dir_all(&dir).unwrap();
        let report = "---- Horizon Crash Report ----\n// Details below.\n\nTime: 2026-10-14 09:05:07\nDescription: Unexpected error\n\npanicked at src/game.rs:10:5:\nboom\n\t0: horizon_client::game::frame\n";
        fs::write(dir.join("crash-2026-10-14_09.05.07-client.txt"), report).unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        assert_eq!(
            crash_report_in_since(&dir, start).unwrap(),
            dir.join("crash-2026-10-14_09.05.07-client.txt")
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(crash_report_in_since(&dir, later).is_none());
        assert_eq!(
            crash_summary(report).as_deref(),
            Some("panicked at src/game.rs:10:5: boom")
        );
        assert_eq!(crash_summary("no report"), None);
        let log = dir.join("latest.log");
        fs::write(&log, "a\nb\nc\n").unwrap();
        assert_eq!(tail_of(&log, 2).unwrap(), "b\nc\n");
        assert_eq!(tail_of(&log, 100).unwrap(), "a\nb\nc\n");
        assert!(tail_of(&dir.join("missing.log"), 5).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn plain_notes() {
        let text = markdown_to_text(
            "# Title\n\nSome *words* and [a link](https://x.y).\n\n- one\n- two\n",
        );
        assert_eq!(
            text,
            "Title\n\nSome words and a link (https://x.y).\n\n- one\n- two\n"
        );
    }
}
