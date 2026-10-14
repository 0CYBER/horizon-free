//! The pinned version's sounds (sounds.json and the .ogg files) from the player's own official
//! launcher install, each object checked against the asset index's SHA-1, copied into
//! `.local/horizon-client/<version>/assets` where the game reads them (tools/fetch_sounds.py and
//! the tools/lab.py functions it called). Nothing is downloaded.

use crate::artifacts::{self, Lock};
use crate::pyjson::Value;
use crate::{digest, read_json, Log};
use std::fs;
use std::path::{Path, PathBuf};

/// The launcher's asset directory and the pinned version's verified asset index
/// (tools/lab.py `launcher_assets`).
fn launcher_assets(manifest: &Value, minecraft: &Path) -> Result<(PathBuf, PathBuf), String> {
    let assets = minecraft.join("assets");
    let info = manifest
        .get("assetIndex")
        .ok_or("The version JSON has no assetIndex")?;
    let field = |key: &str| {
        info.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("The version JSON's assetIndex has no {key}"))
    };
    let (id, sha1) = (field("id")?, field("sha1")?);
    if artifacts::safe_relative(id).is_none_or(|p| p.components().count() != 1) {
        return Err(format!("Unsafe asset index id {id:?}"));
    }
    let index = assets.join("indexes").join(format!("{id}.json"));
    if !index.is_file() {
        let version = manifest.get("id").and_then(Value::as_str).unwrap_or("");
        return Err(format!(
            "Missing {}. {}",
            index.display(),
            artifacts::launcher_hint(version)
        ));
    }
    if digest(&index, "sha1")? != sha1 {
        return Err(format!("{} is not the pinned asset index", index.display()));
    }
    Ok((assets, index))
}

/// Where the launcher's asset store keeps the object with this SHA-1.
fn object_path(assets: &Path, checksum: &str) -> Result<PathBuf, String> {
    if checksum.len() != 40 || !checksum.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("Bad asset hash {checksum:?} in the asset index"));
    }
    Ok(assets.join("objects").join(&checksum[..2]).join(checksum))
}

/// One verified object from the launcher's asset store (tools/lab.py `launcher_object`).
fn launcher_object(assets: &Path, checksum: &str) -> Result<PathBuf, String> {
    let path = object_path(assets, checksum)?;
    if !path.is_file() {
        return Err(format!(
            "Missing asset object {}. Play the pinned version once in the official launcher.",
            path.display()
        ));
    }
    if digest(&path, "sha1")? != checksum {
        return Err(format!(
            "Corrupt asset object {}; let the launcher repair it",
            path.display()
        ));
    }
    Ok(path)
}

/// Whether the game plays this asset: sounds.json and everything under sounds/.
fn is_sound(name: &str) -> bool {
    name == "minecraft/sounds.json" || name.starts_with("minecraft/sounds/")
}

/// Copies the pinned version's asset index and sounds from the official launcher's game
/// directory (`minecraft`, else HORIZON_MINECRAFT_DIR, else the launcher's own) into `root`'s
/// `.local/horizon-client/<version>`; returns that folder. Files already there with the right
/// SHA-1 are kept.
pub fn sounds(root: &Path, minecraft: Option<&Path>, log: Log) -> Result<PathBuf, String> {
    let lock = Lock::read(root)?;
    let minecraft = artifacts::minecraft_dir(minecraft);
    let manifest = artifacts::import_manifest(&lock, &minecraft)?;
    let (store, index) = launcher_assets(&manifest, &minecraft)?;
    let id = manifest
        .get("id")
        .and_then(Value::as_str)
        .ok_or("The version JSON has no id")?;
    let target = root.join(".local/horizon-client").join(id);
    fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    fs::copy(&index, target.join("asset-index.json"))
        .map_err(|e| format!("{}: {e}", target.join("asset-index.json").display()))?;
    let objects = read_json(&index)?;
    let objects = objects
        .get("objects")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{} has no objects", index.display()))?;
    // A repeated name keeps its first place and its last value, as a Python dict does.
    let mut wanted: Vec<(&str, &Value)> = Vec::new();
    for (name, info) in objects.iter().filter(|(name, _)| is_sound(name)) {
        match wanted.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = info,
            None => wanted.push((name, info)),
        }
    }
    let mut total: u64 = 0;
    for (name, info) in &wanted {
        let hash = info
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{name} has no hash in the asset index"))?;
        let size = info
            .get("size")
            .and_then(Value::as_i64)
            .and_then(|s| u64::try_from(s).ok())
            .ok_or_else(|| format!("{name} has no size in the asset index"))?;
        let relative = artifacts::safe_relative(name)
            .ok_or_else(|| format!("Unsafe asset name {name:?} in the asset index"))?;
        let source = launcher_object(&store, hash)?;
        artifacts::import_copy(
            &source,
            &target.join("assets").join(relative),
            hash,
            "sha1",
            "",
        )?;
        total += size;
    }
    log(&format!(
        "Sound assets ready: {} files, {:.1} MB",
        wanted.len(),
        total as f64 / 1e6
    ));
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha1::Digest;

    fn sha1_hex(data: &[u8]) -> String {
        crate::hex(&sha1::Sha1::digest(data))
    }

    /// A Horizon folder and an official launcher directory holding version `9.9` with
    /// sounds.json, two sounds and a language file in its asset store.
    struct Fixture {
        dir: PathBuf,
        root: PathBuf,
        minecraft: PathBuf,
        objects: Vec<(&'static str, Vec<u8>)>,
    }

    impl Fixture {
        fn new(name: &str) -> Fixture {
            let dir = std::env::temp_dir().join(format!(
                "horizon-prepare-sounds-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            let root = dir.join("horizon");
            let minecraft = dir.join("minecraft");
            let objects: Vec<(&'static str, Vec<u8>)> = vec![
                ("minecraft/sounds.json", b"{\"a\": {}}".to_vec()),
                ("minecraft/sounds/ambient/cave1.ogg", b"OggS cave".to_vec()),
                ("minecraft/sounds/dig/stone1.ogg", b"OggS stone".to_vec()),
                ("minecraft/lang/fr_fr.json", b"{}".to_vec()),
            ];
            let mut index = String::from("{\"objects\": {");
            for (i, (name, data)) in objects.iter().enumerate() {
                let hash = sha1_hex(data);
                let path = minecraft
                    .join("assets/objects")
                    .join(&hash[..2])
                    .join(&hash);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, data).unwrap();
                let comma = if i > 0 { ", " } else { "" };
                index += &format!(
                    "{comma}\"{name}\": {{\"hash\": \"{hash}\", \"size\": {}}}",
                    data.len()
                );
            }
            index += "}}";
            let indexes = minecraft.join("assets/indexes");
            fs::create_dir_all(&indexes).unwrap();
            fs::write(indexes.join("7.json"), &index).unwrap();
            let manifest = format!(
                "{{\"id\": \"9.9\", \"assetIndex\": {{\"id\": \"7\", \"sha1\": \"{}\"}}, \
                 \"downloads\": {{\"client\": {{\"sha1\": \"c\"}}, \"server\": {{\"sha1\": \"s\"}}}}, \
                 \"javaVersion\": {{\"majorVersion\": 25}}}}",
                sha1_hex(index.as_bytes())
            );
            let versions = minecraft.join("versions/9.9");
            fs::create_dir_all(&versions).unwrap();
            fs::write(versions.join("9.9.json"), &manifest).unwrap();
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join("research-lock.json"),
                format!(
                    "{{\"minecraft\": {{\"version\": \"9.9\", \"java_major\": 25, \
                     \"client_sha1\": \"c\", \"server_sha1\": \"s\", \"manifest_sha1\": \"{}\"}}}}",
                    sha1_hex(manifest.as_bytes())
                ),
            )
            .unwrap();
            Fixture {
                dir,
                root,
                minecraft,
                objects,
            }
        }

        fn run(&self) -> Result<PathBuf, String> {
            sounds(&self.root, Some(&self.minecraft), &|_| {})
        }

        fn object(&self, i: usize) -> PathBuf {
            let hash = sha1_hex(&self.objects[i].1);
            self.minecraft
                .join("assets/objects")
                .join(&hash[..2])
                .join(hash)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn sounds_land_where_the_game_reads_them() {
        let f = Fixture::new("layout");
        let lines = std::cell::RefCell::new(Vec::new());
        let target = sounds(&f.root, Some(&f.minecraft), &|l| {
            lines.borrow_mut().push(l.to_string())
        })
        .unwrap();
        assert_eq!(target, f.root.join(".local/horizon-client/9.9"));
        for (name, data) in &f.objects[..3] {
            assert_eq!(&fs::read(target.join("assets").join(name)).unwrap(), data);
        }
        assert!(!target.join("assets/minecraft/lang").exists());
        assert_eq!(
            fs::read(target.join("asset-index.json")).unwrap(),
            fs::read(f.minecraft.join("assets/indexes/7.json")).unwrap()
        );
        assert!(f.root.join(".local/minecraft/9.9/manifest.json").is_file());
        assert_eq!(*lines.borrow(), ["Sound assets ready: 3 files, 0.0 MB"]);
        // Again: the verified copies are kept.
        f.run().unwrap();
    }

    #[test]
    fn every_object_is_checked() {
        let f = Fixture::new("corrupt");
        fs::write(f.object(1), b"not the sound").unwrap();
        let error = f.run().unwrap_err();
        assert!(error.starts_with("Corrupt asset object"), "{error}");
        assert!(!f
            .root
            .join(".local/horizon-client/9.9/assets/minecraft/sounds/ambient/cave1.ogg")
            .exists());
        fs::remove_file(f.object(1)).unwrap();
        assert!(f.run().unwrap_err().starts_with("Missing asset object"));
    }

    #[test]
    fn the_index_must_be_the_pinned_one() {
        let f = Fixture::new("index");
        let index = f.minecraft.join("assets/indexes/7.json");
        fs::write(&index, "{\"objects\": {}}").unwrap();
        assert!(f
            .run()
            .unwrap_err()
            .ends_with("is not the pinned asset index"));
        fs::remove_file(&index).unwrap();
        let error = f.run().unwrap_err();
        assert!(
            error.starts_with("Missing ") && error.contains("play 9.9 once"),
            "{error}"
        );
    }

    #[test]
    fn object_paths_come_from_real_hashes() {
        let store = Path::new("assets");
        let hash = "abfaa525f923f807df8b4e4d29c1b5e3a104adbe";
        assert_eq!(
            object_path(store, hash).unwrap(),
            store.join("objects/ab").join(hash)
        );
        assert!(object_path(store, "../../etc/passwd").is_err());
        assert!(is_sound("minecraft/sounds/x.ogg") && is_sound("minecraft/sounds.json"));
        assert!(!is_sound("minecraft/soundsx.json") && !is_sound("realms/sounds/x.ogg"));
    }
}
