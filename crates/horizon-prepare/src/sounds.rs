//! The pinned version's sounds, title panorama, Unifont glyphs and built-in resource packs
//! from the player's own official launcher install, each object checked against the asset index's SHA-1, copied into
//! `.local/horizon-client/<version>/assets` where the game reads them (tools/fetch_sounds.py and
//! the tools/lab.py functions it called). Nothing is downloaded.

use crate::artifacts::{self, Lock};
use crate::pyjson::Value;
use crate::{digest, read_json, Log};
use std::fs;
use std::path::{Path, PathBuf};

/// The launcher's asset directory and the pinned version's asset index (tools/lab.py
/// `launcher_assets`). Mojang republishes a version's index under the same id for translation
/// fixes, and a launcher keeps only the last one it downloaded (#1076): an index the lock does
/// not know is still used, with a line in the log, since every object copied from it is
/// checked against its own SHA-1.
fn launcher_assets(
    lock: &Lock,
    manifest: &Value,
    minecraft: &Path,
    log: Log,
) -> Result<(PathBuf, PathBuf), String> {
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
    let checksum = digest(&index, "sha1")?;
    if checksum != sha1 && !lock.asset_index_accepted(&checksum) {
        log(&format!(
            "{} is an asset index Horizon does not know yet (sha1 {checksum}); Mojang may have \
             republished it. Using it: every file is still checked against its own SHA-1.",
            index.display()
        ));
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
            "Missing asset object {}. Play the pinned version once in your launcher.",
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
    copy_assets(
        root,
        minecraft,
        log,
        is_sound,
        "Sound",
        &["minecraft/sounds.json"],
    )
}

/// Replaces the client JAR's 1x1 title placeholders with the full panorama from the
/// launcher's asset store. Also usable on already-prepared resources without Java.
pub fn panorama(root: &Path, minecraft: Option<&Path>, log: Log) -> Result<PathBuf, String> {
    copy_assets(root, minecraft, log, is_panorama, "Panorama", &PANORAMA)
}

/// Replaces the client JAR's empty `font/include/unifont.json` with the asset index's, and
/// adds the GNU Unifont glyphs it names (`font/unifont*.zip`): the default font's fallback for
/// every character the bitmap fonts lack (#1076). Also usable on prepared resources without Java.
pub fn fonts(root: &Path, minecraft: Option<&Path>, log: Log) -> Result<PathBuf, String> {
    copy_assets(root, minecraft, log, is_font, "Font", &UNIFONT)
}

/// Full preparation can run from cached JARs without an official launcher install.
pub(crate) fn optional_assets(root: &Path, minecraft: Option<&Path>, log: Log) {
    if let Err(error) = panorama(root, minecraft, log) {
        log(&format!(
            "Title panorama not imported: {error}; run horizon-prepare panorama later"
        ));
    }
    if let Err(error) = fonts(root, minecraft, log) {
        log(&format!(
            "Unifont glyphs not imported: {error}; run horizon-prepare fonts later"
        ));
    }
    if let Err(error) = packs(root, minecraft, log) {
        log(&format!(
            "Programmer Art and High Contrast not imported: {error}; run horizon-prepare packs later"
        ));
    }
}

const PANORAMA: [&str; 7] = [
    "minecraft/textures/gui/title/background/panorama_0.png",
    "minecraft/textures/gui/title/background/panorama_1.png",
    "minecraft/textures/gui/title/background/panorama_2.png",
    "minecraft/textures/gui/title/background/panorama_3.png",
    "minecraft/textures/gui/title/background/panorama_4.png",
    "minecraft/textures/gui/title/background/panorama_5.png",
    "minecraft/textures/gui/title/background/panorama_overlay.png",
];

fn is_panorama(name: &str) -> bool {
    PANORAMA.contains(&name)
}

const UNIFONT: [&str; 2] = [
    "minecraft/font/include/unifont.json",
    "minecraft/font/unifont.zip",
];

/// Adds the built-in Programmer Art and High Contrast packs, which only the asset index holds
/// (`BuiltInPackSource.populatePackList` lists `minecraft:resourcepacks` of the vanilla pack).
/// Packs made to sit on Programmer Art, like Golden Days, show the default textures without it.
/// Also usable on prepared resources without Java.
pub fn packs(root: &Path, minecraft: Option<&Path>, log: Log) -> Result<PathBuf, String> {
    copy_assets(
        root,
        minecraft,
        log,
        is_built_in_pack,
        "Built-in pack",
        &BUILT_IN_PACKS,
    )
}

/// The built-in packs `packs` copies, under the prepared resources' `assets`.
pub const BUILT_IN_PACKS: [&str; 2] = [
    "minecraft/resourcepacks/programmer_art.zip",
    "minecraft/resourcepacks/high_contrast.zip",
];

fn is_built_in_pack(name: &str) -> bool {
    name.starts_with("minecraft/resourcepacks/")
}

/// The fonts only the asset index holds (the JAR has the bitmap fonts).
fn is_font(name: &str) -> bool {
    name.starts_with("minecraft/font/")
}

fn copy_assets(
    root: &Path,
    minecraft: Option<&Path>,
    log: Log,
    wanted_asset: fn(&str) -> bool,
    label: &str,
    required: &[&str],
) -> Result<PathBuf, String> {
    let lock = Lock::read(root)?;
    let minecraft = artifacts::minecraft_dir(minecraft);
    let manifest = artifacts::import_manifest(&lock, &minecraft)?;
    let (store, index) = launcher_assets(&lock, &manifest, &minecraft, log)?;
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
    for (name, info) in objects.iter().filter(|(name, _)| wanted_asset(name)) {
        match wanted.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = info,
            None => wanted.push((name, info)),
        }
    }
    for name in required {
        if !wanted.iter().any(|(n, _)| n == name) {
            return Err(format!("{name} is missing from the asset index"));
        }
    }
    let mut total: u64 = 0;
    let mut copies = Vec::with_capacity(wanted.len());
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
        copies.push((source, target.join("assets").join(relative), hash));
        total += size;
    }
    // Check the entire set before replacing any placeholders: a missing late face
    // must not leave a mixture of real textures and gray faces in either entry point.
    for (source, destination, hash) in copies {
        artifacts::import_copy(&source, &destination, hash, "sha1", "")?;
    }
    log(&format!(
        "{label} assets ready: {} files, {:.1} MB",
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

    /// Where the fixture's built-in packs and panorama objects start.
    const FIRST_PACK: usize = 6;
    const FIRST_FACE: usize = 8;

    /// A Horizon folder and an official launcher directory holding version `9.9` with
    /// sounds, a language file, Unifont, the built-in packs and the full title panorama in its
    /// asset store.
    struct Fixture {
        dir: PathBuf,
        root: PathBuf,
        minecraft: PathBuf,
        objects: Vec<(&'static str, Vec<u8>)>,
    }

    impl Fixture {
        fn new(name: &str) -> Fixture {
            Self::with_panorama(name, &PANORAMA)
        }

        fn with_panorama(name: &str, panorama: &[&'static str]) -> Fixture {
            let dir = std::env::temp_dir().join(format!(
                "horizon-prepare-sounds-{name}-{}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            let root = dir.join("horizon");
            let minecraft = dir.join("minecraft");
            let mut objects: Vec<(&'static str, Vec<u8>)> = vec![
                ("minecraft/sounds.json", b"{\"a\": {}}".to_vec()),
                ("minecraft/sounds/ambient/cave1.ogg", b"OggS cave".to_vec()),
                ("minecraft/sounds/dig/stone1.ogg", b"OggS stone".to_vec()),
                ("minecraft/lang/fr_fr.json", b"{}".to_vec()),
                (
                    UNIFONT[0],
                    b"{\"providers\": [{\"type\": \"unihex\"}]}".to_vec(),
                ),
                (UNIFONT[1], b"PK unifont".to_vec()),
                (BUILT_IN_PACKS[0], b"PK programmer art".to_vec()),
                (BUILT_IN_PACKS[1], b"PK high contrast".to_vec()),
            ];
            for &name in panorama {
                // Distinct object bytes let the test catch swapped faces too.
                objects.push((name, name.as_bytes().to_vec()));
            }
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
        assert!(!target.join("assets/minecraft/textures").exists());
        assert_eq!(
            fs::read(target.join("asset-index.json")).unwrap(),
            fs::read(f.minecraft.join("assets/indexes/7.json")).unwrap()
        );
        assert!(f.root.join(".local/minecraft/9.9/manifest.json").is_file());
        assert_eq!(*lines.borrow(), ["Sound assets ready: 3 files, 0.0 MB"]);
        // Again: the verified copies are kept.
        f.run().unwrap();
    }

    /// #1076: the launcher holds an index Mojang republished under the same id, so its SHA-1
    /// is not the one in the version JSON. A known one is used quietly, an unknown one with a
    /// line in the log; the sounds come through either way.
    #[test]
    fn republished_asset_indexes_still_give_sounds() {
        let f = Fixture::new("republished");
        let index = f.minecraft.join("assets/indexes/7.json");
        let republished = format!("{}\n", fs::read_to_string(&index).unwrap());
        fs::write(&index, &republished).unwrap();
        let checksum = sha1_hex(republished.as_bytes());
        let lock = f.root.join("research-lock.json");
        let pinned = fs::read_to_string(&lock).unwrap();
        let sound = f
            .root
            .join(".local/horizon-client/9.9/assets/minecraft/sounds/dig/stone1.ogg");
        for known in [true, false] {
            let accepted = if known { checksum.as_str() } else { "0" };
            fs::write(
                &lock,
                pinned.replace(
                    "\"manifest_sha1\"",
                    &format!("\"accepted_asset_index_sha1\": [\"{accepted}\"], \"manifest_sha1\""),
                ),
            )
            .unwrap();
            let _ = fs::remove_file(&sound);
            let lines = std::cell::RefCell::new(Vec::new());
            sounds(&f.root, Some(&f.minecraft), &|l| {
                lines.borrow_mut().push(l.to_string())
            })
            .unwrap();
            assert_eq!(fs::read(&sound).unwrap(), b"OggS stone");
            let lines = lines.into_inner();
            assert_eq!(lines.last().unwrap(), "Sound assets ready: 3 files, 0.0 MB");
            assert_eq!(lines.len(), if known { 1 } else { 2 }, "{lines:?}");
            if !known {
                assert!(
                    lines[0].contains(&format!("does not know yet (sha1 {checksum})")),
                    "{}",
                    lines[0]
                );
            }
        }
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
    fn panorama_replaces_jar_placeholders_and_repairs_repeated_preparations() {
        let f = Fixture::new("panorama");
        let target = f.root.join(".local/horizon-client/9.9");
        for _ in 0..2 {
            // Full preparation extracts the JAR before importing the asset store.
            for (name, _) in &f.objects[FIRST_FACE..] {
                let path = target.join("assets").join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"1x1 gray placeholder").unwrap();
            }
            assert_eq!(
                panorama(&f.root, Some(&f.minecraft), &|_| {}).unwrap(),
                target
            );
            for (name, data) in &f.objects[FIRST_FACE..] {
                assert_eq!(&fs::read(target.join("assets").join(name)).unwrap(), data);
            }
        }
        assert!(!target.join("assets/minecraft/sounds.json").exists());
        assert!(!target.join("assets/minecraft/lang").exists());
    }

    #[test]
    fn panorama_objects_are_verified_before_replacing_placeholders() {
        let f = Fixture::new("panorama-corrupt");
        fs::write(f.object(FIRST_FACE), b"corrupt panorama").unwrap();
        let path = f
            .root
            .join(".local/horizon-client/9.9/assets")
            .join(f.objects[FIRST_FACE].0);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"placeholder").unwrap();
        assert!(panorama(&f.root, Some(&f.minecraft), &|_| {})
            .unwrap_err()
            .starts_with("Corrupt asset object"));
        assert_eq!(fs::read(path).unwrap(), b"placeholder");
        fs::remove_file(f.object(FIRST_FACE)).unwrap();
        assert!(panorama(&f.root, Some(&f.minecraft), &|_| {})
            .unwrap_err()
            .starts_with("Missing asset object"));
    }

    #[test]
    fn panorama_requires_every_face_and_the_overlay_in_the_index() {
        let f = Fixture::with_panorama("panorama-incomplete", &PANORAMA[..6]);
        let error = panorama(&f.root, Some(&f.minecraft), &|_| {}).unwrap_err();
        assert_eq!(
            error,
            format!("{} is missing from the asset index", PANORAMA[6])
        );
        assert!(!f.root.join(".local/horizon-client/9.9/assets").exists());
    }

    #[test]
    fn full_preparation_keeps_placeholders_when_panorama_is_unavailable() {
        for missing in ["store", "install", "last-object"] {
            let f = Fixture::new(&format!("optional-panorama-{missing}"));
            // The JARs and manifest were imported earlier; the launcher need not remain.
            artifacts::import_manifest(&Lock::read(&f.root).unwrap(), &f.minecraft).unwrap();
            let target = f.root.join(".local/horizon-client/9.9/assets");
            for name in PANORAMA {
                let path = target.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, b"jar placeholder").unwrap();
            }
            match missing {
                "store" => fs::remove_dir_all(f.minecraft.join("assets")).unwrap(),
                "install" => fs::remove_dir_all(&f.minecraft).unwrap(),
                _ => fs::remove_file(f.object(f.objects.len() - 1)).unwrap(),
            }
            // The repair subcommand remains strict, even for a missing final object.
            let error = panorama(&f.root, Some(&f.minecraft), &|_| {}).unwrap_err();
            let lines = std::cell::RefCell::new(Vec::new());
            // This is the optional import step used by full preparation after extraction.
            optional_assets(&f.root, Some(&f.minecraft), &|line| {
                lines.borrow_mut().push(line.to_string())
            });
            let lines = lines.into_inner();
            assert_eq!(
                lines[0],
                format!("Title panorama not imported: {error}; run horizon-prepare panorama later")
            );
            // The fonts and packs come on their own when only a panorama face is missing.
            match missing {
                "last-object" => assert_eq!(
                    lines[1..],
                    [
                        "Font assets ready: 2 files, 0.0 MB",
                        "Built-in pack assets ready: 2 files, 0.0 MB"
                    ]
                ),
                _ => {
                    assert!(lines[1].starts_with("Unifont glyphs not imported: "));
                    assert!(
                        lines[2].starts_with("Programmer Art and High Contrast not imported: ")
                            && lines[2].ends_with("; run horizon-prepare packs later"),
                        "{lines:?}"
                    );
                }
            }
            assert_eq!(lines.len(), 3);
            for name in PANORAMA {
                assert_eq!(fs::read(target.join(name)).unwrap(), b"jar placeholder");
            }
        }
    }

    /// #1076: the JAR's `font/include/unifont.json` is empty; the asset index has the real
    /// one and the glyph archives it names.
    #[test]
    fn fonts_replace_the_jar_include_and_add_unifont() {
        let f = Fixture::new("fonts");
        let target = f.root.join(".local/horizon-client/9.9");
        let include = target.join("assets").join(UNIFONT[0]);
        fs::create_dir_all(include.parent().unwrap()).unwrap();
        fs::write(&include, b"{\"providers\": []}").unwrap();
        let lines = std::cell::RefCell::new(Vec::new());
        let done = fonts(&f.root, Some(&f.minecraft), &|l| {
            lines.borrow_mut().push(l.to_string())
        });
        assert_eq!(done.unwrap(), target);
        for (name, data) in &f.objects[4..FIRST_PACK] {
            assert_eq!(&fs::read(target.join("assets").join(name)).unwrap(), data);
        }
        assert_eq!(*lines.borrow(), ["Font assets ready: 2 files, 0.0 MB"]);
        assert!(!target.join("assets/minecraft/sounds.json").exists());
        assert!(!target.join("assets/minecraft/textures").exists());
    }

    /// Golden Days is made to sit on Programmer Art, which the client JAR lacks: the asset
    /// index has it, where `BuiltInPackSource` lists `minecraft:resourcepacks`.
    #[test]
    fn packs_add_programmer_art_and_high_contrast_where_the_pack_list_finds_them() {
        let f = Fixture::new("packs");
        let target = f.root.join(".local/horizon-client/9.9");
        let lines = std::cell::RefCell::new(Vec::new());
        let done = packs(&f.root, Some(&f.minecraft), &|l| {
            lines.borrow_mut().push(l.to_string())
        });
        assert_eq!(done.unwrap(), target);
        for (name, data) in &f.objects[FIRST_PACK..FIRST_FACE] {
            assert_eq!(&fs::read(target.join("assets").join(name)).unwrap(), data);
        }
        assert!(target
            .join("assets/minecraft/resourcepacks/programmer_art.zip")
            .is_file());
        assert_eq!(
            *lines.borrow(),
            ["Built-in pack assets ready: 2 files, 0.0 MB"]
        );
        assert!(!target.join("assets/minecraft/sounds.json").exists());
        assert!(!target.join("assets/minecraft/font").exists());
    }

    #[test]
    fn the_index_must_hold_the_sounds() {
        let f = Fixture::new("index");
        let index = f.minecraft.join("assets/indexes/7.json");
        fs::write(&index, "{\"objects\": {}}").unwrap();
        assert_eq!(
            f.run().unwrap_err(),
            "minecraft/sounds.json is missing from the asset index"
        );
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
