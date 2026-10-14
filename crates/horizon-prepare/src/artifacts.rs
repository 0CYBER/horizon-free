//! The player's own game files: the pinned version JSON, client jar and libraries from their
//! official launcher install, and the server jar they downloaded, each checked against
//! research-lock.json before it is copied to `.local/minecraft/<version>` (tools/lab.py's
//! `prepare`, which the Python preparer ran first), then checked again on every run
//! (tools/test_rust_parity.py's `verify_artifacts`).

use crate::pyjson::{self, Value};
use crate::{digest, read_json, relative_display, write_json, Log};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

const LAUNCHER_HINT: &str =
    "Horizon does not download the game: install Minecraft Java Edition with the official \
launcher, play {version} once, then rerun. If the launcher uses another game directory, \
pass --minecraft-dir or set HORIZON_MINECRAFT_DIR.";
const SERVER_HINT: &str = "Horizon does not download the game: download the {version} server jar yourself (minecraft.net/download/server \
while {version} is the current release, otherwise the downloads.server.url in your launcher's \
versions/{version}/{version}.json), then pass --server-jar PATH or set HORIZON_SERVER_JAR.";

fn hint(text: &str, version: &str) -> String {
    text.replace("{version}", version)
}

/// research-lock.json and the folder the pinned version's files go to.
pub struct Lock {
    pub json: Value,
    pub version: String,
    pub java_major: i64,
    pub directory: PathBuf,
}

impl Lock {
    pub fn read(root: &Path) -> Result<Lock, String> {
        let json = read_json(&root.join("research-lock.json"))?;
        let game = json
            .get("minecraft")
            .ok_or("research-lock.json has no minecraft")?;
        let version = game
            .get("version")
            .and_then(Value::as_str)
            .ok_or("research-lock.json has no minecraft version")?
            .to_string();
        if version.is_empty()
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || version == "."
            || version == ".."
        {
            return Err("Unsafe version identifier in lock file".into());
        }
        let java_major = game
            .get("java_major")
            .and_then(Value::as_i64)
            .ok_or("research-lock.json has no java_major")?;
        let directory = root.join(".local/minecraft").join(&version);
        Ok(Lock {
            json,
            version,
            java_major,
            directory,
        })
    }

    fn pin(&self, key: &str) -> Result<&str, String> {
        self.json
            .get("minecraft")
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("research-lock.json has no minecraft {key}"))
    }

    /// Mojang republishes a version JSON for asset-index changes; every known one is accepted.
    fn manifest_accepted(&self, checksum: &str) -> bool {
        let game = self.json.get("minecraft");
        self.pin("manifest_sha1").is_ok_and(|pin| pin == checksum)
            || game
                .and_then(|g| g.get("accepted_manifest_sha1"))
                .and_then(Value::as_array)
                .is_some_and(|list| list.iter().any(|v| v.as_str() == Some(checksum)))
    }
}

fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Python's `Path.expanduser` for the forms players type: `~` and `~/...`.
fn expand_user(path: &Path) -> PathBuf {
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first == "~" => home().join(components.as_path()),
        _ => path.to_path_buf(),
    }
}

/// The official launcher's game directory, which holds the player's own copy of the game.
pub fn minecraft_dir(chosen: Option<&Path>) -> PathBuf {
    if let Some(dir) = chosen.filter(|d| !d.as_os_str().is_empty()) {
        return expand_user(dir);
    }
    if let Some(dir) = std::env::var_os("HORIZON_MINECRAFT_DIR").filter(|d| !d.is_empty()) {
        return expand_user(Path::new(&dir));
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Application Support/minecraft")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join(".minecraft")
    } else {
        home().join(".minecraft")
    }
}

/// Copy one file from the player's install, checked like a download; an existing verified
/// copy is kept.
fn import_copy(
    source: &Path,
    destination: &Path,
    checksum: &str,
    algorithm: &str,
    hint: &str,
) -> Result<PathBuf, String> {
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    if destination.exists() && digest(destination, algorithm)? == checksum {
        return Ok(destination.to_path_buf());
    }
    if !source.is_file() {
        return Err(format!("Missing {}. {hint}", source.display())
            .trim()
            .to_string());
    }
    let actual = digest(source, algorithm)?;
    if actual != checksum {
        return Err(format!(
            "{} is not the pinned file: expected {algorithm} {checksum}, got {actual}",
            source.display()
        ));
    }
    let temporary = destination.with_file_name(format!(
        ".{}.partial-{}",
        destination.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let copied = fs::copy(source, &temporary).and_then(|_| fs::rename(&temporary, destination));
    let _ = fs::remove_file(&temporary);
    copied.map_err(|e| format!("{}: {e}", destination.display()))?;
    Ok(destination.to_path_buf())
}

/// The pinned version JSON from the launcher's versions/ directory. The launcher's copy wins
/// over the cached one so the manifest matches the asset index installed alongside it.
fn import_manifest(lock: &Lock, minecraft: &Path) -> Result<Value, String> {
    let version = &lock.version;
    let destination = lock.directory.join("manifest.json");
    let source = minecraft
        .join("versions")
        .join(version)
        .join(format!("{version}.json"));
    if source.is_file() {
        let manifest = read_json(&source)?;
        let downloads = manifest.get("downloads");
        let sha1 = |side: &str| {
            downloads
                .and_then(|d| d.get(side))
                .and_then(|d| d.get("sha1"))
                .and_then(Value::as_str)
        };
        if manifest.get("id").and_then(Value::as_str) != Some(version)
            || sha1("client") != Some(lock.pin("client_sha1")?)
            || sha1("server") != Some(lock.pin("server_sha1")?)
        {
            return Err(format!(
                "{} does not describe the pinned {version} release",
                source.display()
            ));
        }
        let checksum = digest(&source, "sha1")?;
        if !lock.manifest_accepted(&checksum) {
            return Err(format!(
                "{} is a {version} version file Horizon does not know yet (sha1 {checksum}); \
                 Mojang may have republished it. Update Horizon, or report it if this is the latest release.",
                source.display()
            ));
        }
        if !(destination.exists() && digest(&destination, "sha1")? == checksum) {
            fs::create_dir_all(&lock.directory).map_err(|e| e.to_string())?;
            fs::copy(&source, &destination).map_err(|e| e.to_string())?;
        }
    } else if !(destination.exists() && lock.manifest_accepted(&digest(&destination, "sha1")?)) {
        return Err(format!(
            "Missing {}. {}",
            source.display(),
            hint(LAUNCHER_HINT, version)
        ));
    }
    let manifest = read_json(&destination)?;
    let java = manifest
        .get("javaVersion")
        .and_then(|j| j.get("majorVersion"))
        .and_then(Value::as_i64);
    if java != Some(lock.java_major) {
        return Err("Manifest Java version does not match lock file".into());
    }
    Ok(manifest)
}

fn import_server(lock: &Lock, server_jar: Option<&Path>) -> Result<PathBuf, String> {
    let version = &lock.version;
    let destination = lock.directory.join("jars/server.jar");
    let checksum = lock.pin("server_sha1")?;
    let hint = hint(SERVER_HINT, version);
    let source = server_jar
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HORIZON_SERVER_JAR").map(PathBuf::from));
    let Some(source) = source else {
        if destination.exists() && digest(&destination, "sha1")? == checksum {
            return Ok(destination);
        }
        return Err(format!("No {version} server jar. {hint}"));
    };
    import_copy(&expand_user(&source), &destination, checksum, "sha1", &hint)
}

/// The launcher's `os.name` for this machine.
fn rule_system() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// This machine's architecture and its aliases, as Python's `platform.machine()` names them.
fn rule_architectures() -> Vec<&'static str> {
    match std::env::consts::ARCH {
        "x86_64" => vec!["x86_64", "amd64", "x64"],
        "aarch64" => vec!["arm64", "aarch64"],
        "x86" => vec!["i686"],
        other => vec![other],
    }
}

/// The kernel release Python's `platform.release()` reports (only old version JSONs ask).
fn os_release() -> String {
    #[cfg(unix)]
    {
        if let Ok(out) = std::process::Command::new("uname").arg("-r").output() {
            return String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }
    String::new()
}

/// The official launcher's rules: the last matching rule decides, no rules means allowed.
fn library_allowed(library: &Value) -> Result<bool, String> {
    let system = rule_system();
    let architectures = rule_architectures();
    let rules = library
        .get("rules")
        .and_then(Value::as_array)
        .unwrap_or(&[]);
    let mut allowed = rules.is_empty();
    for rule in rules {
        let empty = Value::Object(Vec::new());
        let os = rule.get("os").unwrap_or(&empty);
        if os.get("name").and_then(Value::as_str).unwrap_or(system) != system {
            continue;
        }
        if let Some(arch) = os.get("arch") {
            let pattern = regex::Regex::new(&format!("^(?:{})$", arch.as_str().unwrap_or("")))
                .map_err(|e| e.to_string())?;
            if !architectures.iter().any(|name| pattern.is_match(name)) {
                continue;
            }
        }
        if let Some(version) = os.get("version") {
            let pattern =
                regex::Regex::new(version.as_str().unwrap_or("")).map_err(|e| e.to_string())?;
            if !pattern.is_match(&os_release()) {
                continue;
            }
        }
        if rule.get("features").is_some_and(|f| match f {
            Value::Object(o) => !o.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Null | Value::Bool(false) => false,
            Value::Str(s) => !s.is_empty(),
            _ => true,
        }) {
            continue;
        }
        allowed = rule.get("action").and_then(Value::as_str) == Some("allow");
    }
    Ok(allowed)
}

fn classifier_ends_with(classifier: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        classifier == *name
            || classifier
                .strip_suffix(name)
                .is_some_and(|rest| rest.ends_with('-'))
    })
}

/// Modern manifests list x64 and ARM natives with OS-only rules; keep this machine's. A plain
/// natives-<os> jar is the x64 build when the manifest also lists an ARM variant of it.
fn native_library_matches(library: &Value, names: &[&str]) -> bool {
    let arm = rule_architectures().contains(&"arm64");
    let name = library.get("name").and_then(Value::as_str).unwrap_or("");
    let classifier = name.split(':').nth(3).unwrap_or("");
    if classifier_ends_with(classifier, &["arm64", "aarch64", "aarch_64"]) {
        return arm;
    }
    if classifier_ends_with(classifier, &["x86_64", "x64"]) {
        return !arm;
    }
    if arm && classifier.starts_with("natives-") {
        return !["arm64", "aarch64"]
            .iter()
            .any(|suffix| names.contains(&format!("{name}-{suffix}").as_str()));
    }
    true
}

/// The client libraries the official launcher installs on this OS and architecture.
fn client_library_artifacts(manifest: &Value) -> Result<Vec<&Value>, String> {
    let libraries = manifest
        .get("libraries")
        .and_then(Value::as_array)
        .ok_or("The version JSON has no libraries")?;
    let names: Vec<&str> = libraries
        .iter()
        .filter_map(|l| l.get("name").and_then(Value::as_str))
        .collect();
    let mut artifacts = Vec::new();
    for library in libraries {
        let Some(artifact) = library.get("downloads").and_then(|d| d.get("artifact")) else {
            continue;
        };
        if library_allowed(library)? && native_library_matches(library, &names) {
            artifacts.push(artifact);
        }
    }
    Ok(artifacts)
}

/// A relative path with only normal components and forward slashes, from a manifest or bundle.
fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.contains('\\') || path.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if parts.contains(&"..") || (cfg!(windows) && parts.first().is_some_and(|p| p.contains(':'))) {
        return None;
    }
    Some(parts.iter().collect())
}

fn artifact_fields(artifact: &Value) -> Result<(&str, &str), String> {
    let path = artifact.get("path").and_then(Value::as_str);
    let sha1 = artifact.get("sha1").and_then(Value::as_str);
    path.zip(sha1)
        .ok_or_else(|| "A library in the version JSON has no path or sha1".to_string())
}

/// `(sha256, identifier, path)` lines of a server bundle's versions.list or libraries.list.
fn bundle_entries(
    archive: &mut zip::ZipArchive<fs::File>,
    name: &str,
) -> Result<Vec<(String, String, PathBuf, String)>, String> {
    let text = crate::zip_text(archive, name)?;
    let mut entries = Vec::new();
    for line in crate::split_lines(&text) {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        let [checksum, identifier, relative] = fields[..] else {
            return Err(format!("Malformed line in {name}"));
        };
        let path =
            safe_relative(relative).ok_or_else(|| format!("Unsafe bundled path: {relative}"))?;
        // PurePosixPath's text: empty and `.` parts dropped.
        let posix = relative
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect::<Vec<_>>()
            .join("/");
        entries.push((checksum.to_string(), identifier.to_string(), path, posix));
    }
    Ok(entries)
}

fn extract_verified(
    archive: &mut zip::ZipArchive<fs::File>,
    member: &str,
    destination: &Path,
    checksum: &str,
) -> Result<(), String> {
    let data = crate::zip_bytes(archive, member)?;
    if crate::sha256_hex(&data) != checksum {
        return Err(format!("Bundled artifact checksum mismatch: {member}"));
    }
    fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::write(destination, data).map_err(|e| format!("{}: {e}", destination.display()))
}

/// Modern server downloads are launchers: the game jar and its libraries are inside.
fn unpack_server(
    server: PathBuf,
    directory: &Path,
    version: &str,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let mut archive = crate::open_zip(&server)?;
    if archive.index_for_name("META-INF/versions.list").is_none() {
        return Ok((server, Vec::new()));
    }
    let versions = bundle_entries(&mut archive, "META-INF/versions.list")?;
    if versions.len() != 1 || versions[0].1 != version {
        return Err("Expected one bundled server matching the pinned version".into());
    }
    let (checksum, _, _, relative) = &versions[0];
    let game = directory.join("jars/server-game.jar");
    extract_verified(
        &mut archive,
        &format!("META-INF/versions/{relative}"),
        &game,
        checksum,
    )?;
    let mut libraries = Vec::new();
    for (checksum, _, path, relative) in bundle_entries(&mut archive, "META-INF/libraries.list")? {
        let destination = directory.join("libraries/server").join(path);
        extract_verified(
            &mut archive,
            &format!("META-INF/libraries/{relative}"),
            &destination,
            checksum.as_str(),
        )?;
        libraries.push(destination);
    }
    Ok((game, libraries))
}

/// Imports the player's own pinned game files into `.local/minecraft/<version>` and writes the
/// class inventories and prepared.json (tools/lab.py `prepare`). The decompiler lab.py also
/// fetches is a development tool, not something preparing needs; `tools/lab.py prepare` gets it.
pub fn import(
    root: &Path,
    lock: &Lock,
    minecraft: Option<&Path>,
    server_jar: Option<&Path>,
    log: Log,
) -> Result<(), String> {
    let minecraft = minecraft_dir(minecraft);
    let version = &lock.version;
    let manifest = import_manifest(lock, &minecraft)?;
    let client = import_copy(
        &minecraft
            .join("versions")
            .join(version)
            .join(format!("{version}.jar")),
        &lock.directory.join("jars/client.jar"),
        lock.pin("client_sha1")?,
        "sha1",
        &hint(LAUNCHER_HINT, version),
    )?;
    let downloads = manifest.get("downloads");
    if downloads.and_then(|d| d.get("client_mappings")).is_some()
        || downloads.and_then(|d| d.get("server_mappings")).is_some()
    {
        return Err(
            "This workflow targets unobfuscated releases. Older releases need a remapping stage."
                .into(),
        );
    }
    let id = manifest
        .get("id")
        .and_then(Value::as_str)
        .ok_or("The version JSON has no id")?;
    log(&format!(
        "Imported client {id} from {}",
        minecraft.display()
    ));
    let server = import_server(lock, server_jar)?;
    let (server, server_libraries) = unpack_server(server, &lock.directory, id)?;
    let mut client_libraries = Vec::new();
    for artifact in client_library_artifacts(&manifest)? {
        let (path, sha1) = artifact_fields(artifact)?;
        let relative = safe_relative(path).ok_or("Unsafe library path")?;
        client_libraries.push(import_copy(
            &minecraft.join("libraries").join(&relative),
            &lock.directory.join("libraries/client").join(&relative),
            sha1,
            "sha1",
            &hint(LAUNCHER_HINT, version),
        )?);
    }
    for (side, jar) in [("client", &client), ("server", &server)] {
        let data = crate::inventory::inventory(jar)?;
        let count = |key| data.get(key).cloned().unwrap_or(Value::Null);
        write_json(
            &lock
                .directory
                .join(format!("reports/{side}-inventory.json")),
            &data,
        )?;
        log(&format!(
            "{side}: {} classes, {} methods",
            pyjson::dumps(&count("class_count"), &pyjson::DEFAULT),
            pyjson::dumps(&count("method_count"), &pyjson::DEFAULT)
        ));
    }
    let paths = |list: &[PathBuf]| -> Result<Value, String> {
        Ok(Value::Array(
            list.iter()
                .map(|p| relative_display(p, root).map(Value::Str))
                .collect::<Result<_, _>>()?,
        ))
    };
    write_json(
        &lock.directory.join("prepared.json"),
        &Value::Object(vec![
            ("version".into(), Value::str(id)),
            (
                "jars".into(),
                Value::Object(vec![
                    (
                        "client".into(),
                        Value::Str(relative_display(&client, root)?),
                    ),
                    (
                        "server".into(),
                        Value::Str(relative_display(&server, root)?),
                    ),
                ]),
            ),
            (
                "libraries".into(),
                Value::Object(vec![
                    ("client".into(), paths(&client_libraries)?),
                    ("server".into(), paths(&server_libraries)?),
                ]),
            ),
        ]),
    )?;
    log(&format!("Artifacts: {}", lock.directory.display()));
    Ok(())
}

/// The checked jars (`client`, `server` = the bundle's inner game jar) and the server
/// libraries, authenticated against the lock and the bundle's own checksums on every run.
pub struct Artifacts {
    pub manifest: Value,
    pub jars: HashMap<&'static str, PathBuf>,
    pub libraries: Vec<PathBuf>,
}

pub fn verify(lock: &Lock) -> Result<Artifacts, String> {
    let manifest_path = lock.directory.join("manifest.json");
    if !manifest_path.exists() {
        return Err(format!(
            "{} is missing: prepare with --prepare first",
            manifest_path.display()
        ));
    }
    if !lock.manifest_accepted(&digest(&manifest_path, "sha1")?) {
        return Err("Manifest checksum changed; prepare again with --prepare".into());
    }
    let manifest = read_json(&manifest_path)?;
    let id = manifest.get("id").and_then(Value::as_str).unwrap_or("");
    if id != lock.version {
        return Err("Manifest version differs from the lock".into());
    }
    let mut jars = HashMap::new();
    for side in ["client", "server"] {
        let jar = lock.directory.join(format!("jars/{side}.jar"));
        let pinned = manifest
            .get("downloads")
            .and_then(|d| d.get(side))
            .and_then(|d| d.get("sha1"))
            .and_then(Value::as_str);
        if !jar.exists() || Some(digest(&jar, "sha1")?.as_str()) != pinned {
            return Err(format!(
                "{side} artifact checksum changed; prepare again with --prepare"
            ));
        }
        jars.insert(side, jar);
    }
    let mut archive = crate::open_zip(&jars["server"])?;
    let versions = bundle_entries(&mut archive, "META-INF/versions.list")?;
    if versions.len() != 1 || versions[0].1 != id {
        return Err("Unexpected server bundle".into());
    }
    let game = lock.directory.join("jars/server-game.jar");
    if !game.exists() || digest(&game, "sha256")? != versions[0].0 {
        return Err("Inner server artifact checksum changed".into());
    }
    jars.insert("server", game);
    let mut libraries = Vec::new();
    for (checksum, _, path, relative) in bundle_entries(&mut archive, "META-INF/libraries.list")? {
        let library = lock.directory.join("libraries/server").join(path);
        if !library.exists() || digest(&library, "sha256")? != checksum {
            return Err(format!("Dependency checksum changed: {relative}"));
        }
        libraries.push(library);
    }
    Ok(Artifacts {
        manifest,
        jars,
        libraries,
    })
}

/// The client libraries, checked against the version JSON (tools/test_model_baking_parity.py's
/// `client_libraries`); the client-side exporters compile against them.
pub fn client_libraries(lock: &Lock, manifest: &Value) -> Result<Vec<PathBuf>, String> {
    let mut libraries = Vec::new();
    for artifact in client_library_artifacts(manifest)? {
        let (path, sha1) = artifact_fields(artifact)?;
        let relative = safe_relative(path).ok_or("Unsafe client library path")?;
        let library = lock.directory.join("libraries/client").join(&relative);
        if !library.exists() || digest(&library, "sha1")? != sha1 {
            return Err(format!("Client dependency checksum changed: {path}"));
        }
        libraries.push(library);
    }
    Ok(libraries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natives_follow_the_architecture() {
        let names = ["a:b:1:natives-macos", "a:b:1:natives-macos-arm64"];
        let lib = |name: &str| pyjson::loads(&format!("{{\"name\":\"{name}\"}}")).unwrap();
        let arm = rule_architectures().contains(&"arm64");
        assert_eq!(
            native_library_matches(&lib("a:b:1:natives-macos-arm64"), &names),
            arm
        );
        assert_eq!(
            native_library_matches(&lib("x:y:1:linux-x86_64"), &names),
            !arm
        );
        assert_eq!(
            native_library_matches(&lib("x:y:1:linux-aarch_64"), &names),
            arm
        );
        assert_eq!(
            native_library_matches(&lib("a:b:1:natives-macos"), &names),
            !arm
        );
        assert!(native_library_matches(&lib("a:b:1"), &names));
    }

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(safe_relative("a/../b").is_none());
        assert!(safe_relative("/a").is_none());
        assert!(safe_relative("a\\b").is_none());
        assert_eq!(
            safe_relative("a/b.jar").unwrap(),
            Path::new("a").join("b.jar")
        );
    }
}
