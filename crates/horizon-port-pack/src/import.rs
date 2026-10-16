//! Step 1, import: a Modrinth `.mrpack`, a CurseForge zip, or an instance
//! folder (Prism / MultiMC, or a plain `mods/` folder) to `pack.json`.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Pack {
    pub name: String,
    /// `mrpack`, `curseforge` or `instance`.
    pub format: String,
    pub minecraft: String,
    /// `neoforge`, `forge`, `fabric` or `quilt`.
    pub loader: String,
    pub loader_version: String,
    pub files: Vec<PackFile>,
    /// Override files by top folder (`config`, `kubejs`, ...).
    pub overrides: BTreeMap<String, usize>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PackFile {
    /// Path in the instance (`mods/create-1.21.1-6.0.4.jar`).
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
    /// `required`, `optional` or `unsupported`; empty when not stated.
    pub client: String,
    pub server: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub downloads: Vec<String>,
    /// CurseForge project and file ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<(u64, u64)>,
    /// The jar on disk (instances only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jar: Option<PathBuf>,
}

impl PackFile {
    /// `mods`, `resourcepacks`, `shaderpacks`, `datapacks`...
    pub fn kind(&self) -> &str {
        self.path.split('/').next().unwrap_or("")
    }
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

const LOADERS: [(&str, &str); 4] = [
    ("neoforge", "neoforge"),
    ("forge", "forge"),
    ("fabric-loader", "fabric"),
    ("quilt-loader", "quilt"),
];

pub fn import(path: &Path) -> Result<Pack, String> {
    if path.is_dir() {
        return instance(path);
    }
    let file = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(index) = read_json(&mut zip, "modrinth.index.json")? {
        return Ok(mrpack(&index, &names(&zip)));
    }
    if let Some(manifest) = read_json(&mut zip, "manifest.json")? {
        return Ok(curseforge(&manifest, &names(&zip)));
    }
    Err(format!(
        "{}: neither modrinth.index.json nor manifest.json",
        path.display()
    ))
}

fn names(zip: &zip::ZipArchive<fs::File>) -> Vec<String> {
    zip.file_names().map(str::to_owned).collect()
}

fn read_json(zip: &mut zip::ZipArchive<fs::File>, name: &str) -> Result<Option<Value>, String> {
    let Ok(mut entry) = zip.by_name(name) else {
        return Ok(None);
    };
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| format!("{name}: {e}"))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{name}: {e}"))
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

/// Override files by top folder, below any of `roots`.
fn overrides(names: &[String], roots: &[&str]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for name in names.iter().filter(|n| !n.ends_with('/')) {
        for root in roots {
            if let Some(rest) = name.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
                let top = rest.split('/').next().unwrap_or(rest);
                *out.entry(top.to_owned()).or_default() += 1;
            }
        }
    }
    out
}

pub fn mrpack(index: &Value, names: &[String]) -> Pack {
    let deps = &index["dependencies"];
    let (loader, loader_version) = LOADERS
        .iter()
        .find_map(|(key, name)| Some((name.to_string(), deps.get(*key)?.as_str()?.to_owned())))
        .unwrap_or_default();
    let files = index["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| PackFile {
            path: str_of(f, "path"),
            sha1: f["hashes"]["sha1"].as_str().map(str::to_owned),
            client: str_of(&f["env"], "client"),
            server: str_of(&f["env"], "server"),
            downloads: f["downloads"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d.as_str().map(str::to_owned))
                .collect(),
            ..PackFile::default()
        })
        .collect();
    Pack {
        name: str_of(index, "name"),
        format: "mrpack".into(),
        minecraft: str_of(deps, "minecraft"),
        loader,
        loader_version,
        files,
        overrides: overrides(
            names,
            &["overrides", "client-overrides", "server-overrides"],
        ),
    }
}

pub fn curseforge(manifest: &Value, names: &[String]) -> Pack {
    let primary = manifest["minecraft"]["modLoaders"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|l| l["primary"].as_bool().unwrap_or(true))
        .map(|l| str_of(l, "id"))
        .unwrap_or_default();
    let (loader, loader_version) = primary.split_once('-').unwrap_or((&primary, ""));
    let files = manifest["files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| {
            let (p, id) = (f["projectID"].as_u64(), f["fileID"].as_u64());
            PackFile {
                path: format!("mods/curseforge-{}-{}", p.unwrap_or(0), id.unwrap_or(0)),
                // CurseForge states no side; its API needs a key, so names
                // come from the user's installed instance (import that).
                curseforge: p.zip(id),
                ..PackFile::default()
            }
        })
        .collect();
    let root = manifest["overrides"].as_str().unwrap_or("overrides");
    Pack {
        name: str_of(manifest, "name"),
        format: "curseforge".into(),
        minecraft: str_of(&manifest["minecraft"], "version"),
        loader: loader.to_owned(),
        loader_version: loader_version.to_owned(),
        files,
        overrides: overrides(names, &[root]),
    }
}

/// A Prism / MultiMC instance (`mmc-pack.json`), a game folder, or a
/// folder of jars: the jars on disk, hashed.
fn instance(dir: &Path) -> Result<Pack, String> {
    let mut pack = Pack {
        name: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        format: "instance".into(),
        ..Pack::default()
    };
    if let Ok(text) = fs::read_to_string(dir.join("mmc-pack.json")) {
        let mmc: Value = serde_json::from_str(&text).map_err(|e| format!("mmc-pack.json: {e}"))?;
        for c in mmc["components"].as_array().into_iter().flatten() {
            let version = str_of(c, "version");
            match c["uid"].as_str().unwrap_or("") {
                "net.minecraft" => pack.minecraft = version,
                "net.neoforged" => {
                    (pack.loader, pack.loader_version) = ("neoforge".into(), version)
                }
                "net.minecraftforge" => {
                    (pack.loader, pack.loader_version) = ("forge".into(), version)
                }
                "net.fabricmc.fabric-loader" => {
                    (pack.loader, pack.loader_version) = ("fabric".into(), version)
                }
                "org.quiltmc.quilt-loader" => {
                    (pack.loader, pack.loader_version) = ("quilt".into(), version)
                }
                _ => {}
            }
        }
    }
    let game = [".minecraft", "minecraft", "."]
        .iter()
        .map(|g| dir.join(g))
        .find(|g| g.join("mods").is_dir())
        .or_else(|| dir.read_dir().is_ok().then(|| dir.to_path_buf()))
        .ok_or_else(|| format!("{}: no mods folder", dir.display()))?;
    let mods = if game.join("mods").is_dir() {
        game.join("mods")
    } else {
        game.clone()
    };
    let mut jars: Vec<PathBuf> = fs::read_dir(&mods)
        .map_err(|e| format!("{}: {e}", mods.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jar"))
        .collect();
    jars.sort();
    for jar in jars {
        let bytes = fs::read(&jar).map_err(|e| format!("{}: {e}", jar.display()))?;
        pack.files.push(PackFile {
            path: format!(
                "mods/{}",
                jar.file_name().unwrap_or_default().to_string_lossy()
            ),
            sha1: Some(sha1_hex(&bytes)),
            jar: Some(jar),
            ..PackFile::default()
        });
    }
    for top in ["config", "kubejs", "defaultconfigs", "datapacks"] {
        let n = count_files(&game.join(top));
        if n > 0 {
            pack.overrides.insert(top.into(), n);
        }
    }
    Ok(pack)
}

fn count_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let p = e.path();
            if p.is_dir() { count_files(&p) } else { 1 }
        })
        .sum()
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest;
    sha1::Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_an_mrpack_index() {
        let index = json!({"name": "Small", "dependencies": {"minecraft": "1.21.1", "neoforge": "21.1.233"},
            "files": [{"path": "mods/create.jar", "hashes": {"sha1": "ab"}, "env": {"client": "required", "server": "required"},
                "downloads": ["https://cdn.modrinth.com/create.jar"]},
                {"path": "resourcepacks/look.zip", "hashes": {"sha1": "cd"}}]});
        let names = [
            "overrides/config/a.toml",
            "overrides/config/b/c.toml",
            "client-overrides/kubejs/x.js",
            "overrides/",
        ]
        .map(String::from);
        let p = mrpack(&index, &names);
        assert_eq!(
            (
                p.minecraft.as_str(),
                p.loader.as_str(),
                p.loader_version.as_str()
            ),
            ("1.21.1", "neoforge", "21.1.233")
        );
        assert_eq!(p.files.len(), 2);
        assert_eq!(
            (p.files[0].kind(), p.files[0].file_name()),
            ("mods", "create.jar")
        );
        assert_eq!(p.files[0].sha1.as_deref(), Some("ab"));
        assert_eq!(p.files[1].kind(), "resourcepacks");
        assert_eq!(
            p.overrides,
            BTreeMap::from([("config".into(), 2), ("kubejs".into(), 1)])
        );
    }

    #[test]
    fn reads_a_curseforge_manifest() {
        let manifest = json!({"name": "CF", "overrides": "overrides",
            "minecraft": {"version": "1.21.1", "modLoaders": [{"id": "neoforge-21.1.259", "primary": true}]},
            "files": [{"projectID": 238222, "fileID": 5101366, "required": true}]});
        let p = curseforge(&manifest, &["overrides/kubejs/server_scripts/a.js".into()]);
        assert_eq!(
            (p.loader.as_str(), p.loader_version.as_str()),
            ("neoforge", "21.1.259")
        );
        assert_eq!(p.files[0].curseforge, Some((238222, 5101366)));
        assert_eq!(p.overrides["kubejs"], 1);
    }
}
