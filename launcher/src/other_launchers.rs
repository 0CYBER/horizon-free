//! Instances other launchers keep, found without changing anything there, and copied into new
//! Minecraft: Java Edition instances. Built from the formats' public descriptions only:
//!
//! - Prism Launcher and MultiMC (the same instance format: an instance folder copied from one
//!   works in the other): `instances/<id>/` holds `instance.cfg` (INI, its `name`),
//!   `mmc-pack.json` (`components`, each a `uid` and a `version`: `net.minecraft` is the game,
//!   the others its loader) and the game folder, `minecraft/` or `.minecraft/`.
//!   <https://prismlauncher.org/wiki/getting-started/migrating-multimc/>,
//!   <https://prismlauncher.org/wiki/getting-started/data-location/>,
//!   <https://prismlauncher.org/wiki/overview/troubleshooting/>
//! - The official launcher: `launcher_profiles.json`'s `profiles`, each a `name`, a `type`
//!   (`custom`, `latest-release`, `latest-snapshot`), the version it plays (`lastVersionId`, an
//!   id of `versions/<id>/<id>.json`) and its game folder (`gameDir`, else the launcher's own).
//!   <https://minecraft.wiki/w/Launcher_profiles.json>
use crate::instances::{self, Instance};
use crate::sources::{self, Folders, Os};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const OFFICIAL: &str = "the official launcher";

/// A mod loader and its version, as the other launcher names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loader {
    /// `fabric`, `quilt`, `neoforge` or `forge`.
    pub name: &'static str,
    pub version: String,
}

/// An instance found in another launcher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub launcher: &'static str,
    pub name: String,
    /// The Minecraft version: an id (`1.21.1`), `latest` or `latest-snapshot`.
    pub minecraft: String,
    pub loader: Option<Loader>,
    /// Its game folder: saves, mods, packs, options.
    pub game: PathBuf,
    /// The game folder is the launcher's own (the official launcher's default): only what an
    /// instance holds is copied, not the launcher's versions, libraries and assets.
    pub shared: bool,
}

/// What a game folder shared with a launcher gives a new instance.
const SHARED: [&str; 9] = [
    "saves",
    "mods",
    "config",
    "resourcepacks",
    "shaderpacks",
    "screenshots",
    "options.txt",
    "servers.dat",
    "optionsshaders.txt",
];
/// What is never copied from an instance's own game folder.
const SKIPPED: [&str; 2] = ["logs", "crash-reports"];

fn loader_of_uid(uid: &str) -> Option<&'static str> {
    match uid {
        "net.fabricmc.fabric-loader" => Some("fabric"),
        "org.quiltmc.quilt-loader" => Some("quilt"),
        "net.neoforged" => Some("neoforge"),
        "net.minecraftforge" => Some("forge"),
        _ => None,
    }
}

/// `key=value` of an INI file's lines, outside any section or in `[General]`.
fn ini_value(text: &str, key: &str) -> Option<String> {
    let mut section = "";
    for line in text.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s;
            continue;
        }
        if !matches!(section, "" | "General") {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// One Prism or MultiMC instance folder.
pub fn prism_instance(dir: &Path, launcher: &'static str) -> Option<Found> {
    let pack: Value =
        serde_json::from_str(&fs::read_to_string(dir.join("mmc-pack.json")).ok()?).ok()?;
    let components = pack["components"].as_array()?;
    let version = |uid: &str| {
        components
            .iter()
            .find(|c| c["uid"] == uid)
            .and_then(|c| c["version"].as_str())
            .map(String::from)
    };
    let minecraft = version("net.minecraft")?;
    let loader = components.iter().find_map(|c| {
        Some(Loader {
            name: loader_of_uid(c["uid"].as_str()?)?,
            version: c["version"].as_str()?.to_string(),
        })
    });
    let game = ["minecraft", ".minecraft"]
        .into_iter()
        .map(|g| dir.join(g))
        .find(|g| g.is_dir())
        .unwrap_or_else(|| dir.join("minecraft"));
    let folder = dir.file_name()?.to_string_lossy().into_owned();
    let name = fs::read_to_string(dir.join("instance.cfg"))
        .ok()
        .and_then(|t| ini_value(&t, "name"))
        .filter(|n| !n.is_empty())
        .unwrap_or(folder);
    Some(Found {
        launcher,
        name,
        minecraft,
        loader,
        game,
        shared: false,
    })
}

/// Every instance in a Prism or MultiMC folder (its root, or its `instances/` itself).
pub fn prism_instances(root: &Path, launcher: &'static str) -> Vec<Found> {
    let instances = if root.join("instances").is_dir() {
        root.join("instances")
    } else {
        root.to_path_buf()
    };
    let mut found: Vec<Found> = fs::read_dir(&instances)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| prism_instance(&e.path(), launcher))
        .collect();
    found.sort_by_key(|f| f.name.to_lowercase());
    found
}

/// The game version and loader a version id of the official launcher plays: a loader's
/// profile names the game's version in `inheritsFrom`, and the loader in its id
/// (`fabric-loader-0.16.9-1.21.1`, `quilt-loader-0.27.1-1.21.1`, `neoforge-21.1.77`,
/// `1.20.1-forge-47.3.0`).
fn official_version(root: &Path, id: &str) -> (String, Option<Loader>) {
    let file = root.join("versions").join(id).join(format!("{id}.json"));
    let parent = fs::read_to_string(file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["inheritsFrom"].as_str().map(String::from));
    let Some(minecraft) = parent else {
        return (id.to_string(), None);
    };
    let after = |prefix: &str| {
        id.strip_prefix(prefix)
            .map(|rest| rest.strip_suffix(&format!("-{minecraft}")).unwrap_or(rest))
            .map(String::from)
    };
    let loader = after("fabric-loader-")
        .map(|v| ("fabric", v))
        .or_else(|| after("quilt-loader-").map(|v| ("quilt", v)))
        .or_else(|| after("neoforge-").map(|v| ("neoforge", v)))
        .or_else(|| {
            id.split_once("-forge-")
                .map(|(_, v)| ("forge", v.to_string()))
        })
        .map(|(name, version)| Loader { name, version });
    (minecraft, loader)
}

/// The official launcher's profiles in its folder `root`.
pub fn official_profiles(root: &Path) -> Vec<Found> {
    let Some(text) = fs::read_to_string(root.join("launcher_profiles.json")).ok() else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let mut found: Vec<Found> = json["profiles"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, p)| {
            let kind = p["type"].as_str().unwrap_or("custom");
            let id = p["lastVersionId"].as_str().unwrap_or_default();
            let (minecraft, loader) = match kind {
                "latest-release" => ("latest".to_string(), None),
                "latest-snapshot" => ("latest-snapshot".to_string(), None),
                _ if id.is_empty() => return None,
                _ => official_version(root, id),
            };
            let name = p["name"].as_str().unwrap_or_default().trim();
            let name = match (name, kind) {
                ("", "latest-release") => "Latest release".to_string(),
                ("", "latest-snapshot") => "Latest snapshot".to_string(),
                ("", _) => id.to_string(),
                (n, _) => n.to_string(),
            };
            let game = p["gameDir"]
                .as_str()
                .filter(|d| !d.is_empty())
                .map(PathBuf::from);
            Some(Found {
                launcher: OFFICIAL,
                name,
                minecraft,
                loader,
                shared: game.as_ref().is_none_or(|g| g == root),
                game: game.unwrap_or_else(|| root.to_path_buf()),
            })
        })
        .collect();
    found.sort_by_key(|f| f.name.to_lowercase());
    found
}

/// What one folder holds: an official launcher's profiles, a Prism or MultiMC root (or its
/// `instances/`), or a single instance folder.
pub fn in_folder(dir: &Path, launcher: Option<&'static str>) -> Vec<Found> {
    if dir.join("launcher_profiles.json").is_file() {
        return official_profiles(dir);
    }
    let named = |names: &[&str]| {
        dir.ancestors()
            .filter_map(Path::file_name)
            .any(|n| names.iter().any(|m| n == *m))
    };
    let launcher = launcher.unwrap_or(if named(&["multimc", "MultiMC"]) {
        "MultiMC"
    } else if named(&["PrismLauncher"]) {
        "Prism Launcher"
    } else {
        "Prism Launcher or MultiMC"
    });
    match prism_instance(dir, launcher) {
        Some(one) => vec![one],
        None => prism_instances(dir, launcher),
    }
}

/// Every instance the launchers on this computer keep in their default folders.
pub fn scan(os: Os, folders: &Folders) -> Vec<Found> {
    let mut found = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for (launcher, dir) in sources::known_folders(os, folders) {
        if seen.contains(&dir) || !dir.is_dir() {
            continue;
        }
        seen.push(dir.clone());
        match launcher {
            OFFICIAL => found.extend(official_profiles(&dir)),
            "Prism Launcher" | "MultiMC" => found.extend(prism_instances(&dir, launcher)),
            _ => {}
        }
    }
    found
}

/// Copies `found`'s game folder into `to`: all of it from an instance's own folder but its
/// logs, and only what an instance holds from a folder shared with the launcher. Nothing in
/// the other launcher's folder changes.
pub fn copy_game(found: &Found, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    let Ok(entries) = fs::read_dir(&found.game) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let wanted = if found.shared {
            SHARED.contains(&name.as_ref())
        } else {
            !SKIPPED.contains(&name.as_ref())
        };
        if !wanted {
            continue;
        }
        if entry.file_type()?.is_dir() {
            crate::backend::copy_dir(&entry.path(), &to.join(name.as_ref()))?;
        } else {
            fs::copy(entry.path(), to.join(name.as_ref()))?;
        }
    }
    Ok(())
}

/// A new Java instance named `name` playing what `found` plays, with a copy of its game folder.
pub fn import(found: &Found, name: &str) -> Result<Instance, String> {
    let versions = crate::minecraft_versions::load()?;
    let version = versions.find(&found.minecraft)?.id.clone();
    let instance = instances::create_java(name, &version)?;
    if let Err(e) = copy_game(found, &instance.home) {
        let _ = instances::delete(&instance.id);
        return Err(format!("Could not copy {}: {e}", found.game.display()));
    }
    Ok(instance)
}

#[cfg(test)]
mod tests;
