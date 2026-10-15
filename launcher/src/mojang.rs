//! The player's own download of the pinned version from Mojang, for a signed-in account that
//! owns Minecraft: Java Edition (#434), as other launchers get it: the version JSON, the client
//! jar, this system's libraries and the assets (the sounds), each checked against its sha1,
//! in the official launcher's layout in the launcher's data folder. sources.rs then finds them
//! there like any launcher's. Nothing of Mojang's is part of Horizon.

use crate::sources::{fetch_checked, fetch_manifest, sha1_file, write_file, Fetch, Pins};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;

const OBJECTS: &str = "https://resources.download.minecraft.net";

/// One file to download.
struct File {
    url: String,
    sha1: String,
    path: PathBuf,
}

/// Downloads the pinned version into `dir`, `jobs` files at a time; files already there and
/// whole are kept. An asset that fails is a line in the log (the game is then partly silent),
/// anything else an error.
pub fn download(
    pins: &Pins,
    dir: &Path,
    os: &str,
    jobs: usize,
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let v = &pins.version;
    let versions = dir.join("versions").join(v);
    let manifest = versions.join(format!("{v}.json"));
    fetch_manifest(pins, &manifest, fetch, log)?;
    let manifest = read_json(&manifest)?;
    let client = &manifest["downloads"]["client"];
    let mut game = vec![File {
        url: text(client, "url")?,
        sha1: pins.client_sha1.clone(),
        path: versions.join(format!("{v}.jar")),
    }];
    let libraries = manifest["libraries"]
        .as_array()
        .ok_or("The version file has no libraries")?;
    for library in libraries.iter().filter(|l| allowed(l, os)) {
        let artifact = &library["downloads"]["artifact"];
        if artifact.is_null() {
            continue;
        }
        game.push(File {
            url: text(artifact, "url")?,
            sha1: text(artifact, "sha1")?,
            path: dir
                .join("libraries")
                .join(relative(&text(artifact, "path")?)?),
        });
    }
    log(format!(
        "Downloading Minecraft {v} from Mojang: the game and {} libraries",
        game.len() - 1
    ));
    fetch_all(&game, jobs, fetch, log)?;
    let index = &manifest["assetIndex"];
    let index = File {
        url: text(index, "url")?,
        sha1: text(index, "sha1")?,
        path: dir
            .join("assets/indexes")
            .join(relative(&format!("{}.json", text(index, "id")?))?),
    };
    let assets = fetch_all(std::slice::from_ref(&index), 1, fetch, log)
        .and_then(|_| objects(&read_json(&index.path)?, dir))
        .and_then(|objects| {
            log(format!("Downloading the sounds ({} files)", objects.len()));
            fetch_all(&objects, jobs, fetch, log)
        });
    if let Err(e) = assets {
        log(format!("The sounds could not all be downloaded: {e}"));
    }
    Ok(())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v[key]
        .as_str()
        .map(String::from)
        .ok_or_else(|| format!("The version file has no {key} in {v}"))
}

/// A path from the version file, refused if it could leave the folder.
fn relative(path: &str) -> Result<PathBuf, String> {
    let safe = !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    safe.then(|| PathBuf::from(path))
        .ok_or_else(|| format!("Unsafe path {path:?} in the version file"))
}

/// Whether a library's rules let this system (`osx`, `windows`, `linux`) use it: the last
/// rule that matches decides.
fn allowed(library: &Value, os: &str) -> bool {
    let Some(rules) = library["rules"].as_array() else {
        return true;
    };
    rules
        .iter()
        .rfind(|rule| rule["os"]["name"].as_str().is_none_or(|name| name == os))
        .is_some_and(|rule| rule["action"] == "allow")
}

/// The asset index's objects, once each.
fn objects(index: &Value, dir: &Path) -> Result<Vec<File>, String> {
    let objects = index["objects"]
        .as_object()
        .ok_or("The asset index has no objects")?;
    let mut hashes: Vec<&str> = objects
        .values()
        .filter_map(|o| o["hash"].as_str())
        .collect();
    hashes.sort_unstable();
    hashes.dedup();
    hashes
        .into_iter()
        .map(|hash| {
            if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("Bad asset hash {hash:?} in the asset index"));
            }
            let path = format!("{}/{hash}", &hash[..2]);
            Ok(File {
                url: format!("{OBJECTS}/{path}"),
                sha1: hash.to_string(),
                path: dir.join("assets/objects").join(path),
            })
        })
        .collect()
}

/// Downloads the files not already there, `jobs` at a time, with a line every few seconds;
/// stops at the first error.
fn fetch_all(
    files: &[File],
    jobs: usize,
    fetch: Fetch,
    log: &dyn Fn(String),
) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failed = Mutex::new(None);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs.clamp(1, files.len().max(1)))
            .map(|_| {
                scope.spawn(|| {
                    while let Some(file) = files.get(next.fetch_add(1, Relaxed)) {
                        if failed.lock().unwrap().is_some() {
                            break;
                        }
                        if sha1_file(&file.path).as_ref() != Some(&file.sha1) {
                            let got = fetch_checked(fetch, &file.url, &file.sha1)
                                .map_err(|e| format!("{} {e}", file.url))
                                .and_then(|body| write_file(&file.path, &body));
                            if let Err(e) = got {
                                failed.lock().unwrap().get_or_insert(e);
                            }
                        }
                        done.fetch_add(1, Relaxed);
                    }
                })
            })
            .collect();
        let mut waited = 0;
        while !workers.iter().all(|w| w.is_finished()) {
            std::thread::sleep(Duration::from_millis(100));
            waited += 1;
            if waited % 50 == 0 {
                log(format!("{} of {} files", done.load(Relaxed), files.len()));
            }
        }
    });
    failed.into_inner().unwrap().map_or(Ok(()), Err)
}

#[cfg(test)]
#[path = "mojang_tests.rs"]
mod tests;
