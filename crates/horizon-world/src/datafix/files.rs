//! Opening an older world as 26.3 does: the file fixes of `FileFixerUpper`
//! (`net.minecraft.util.filefix`, from DataVersion 4772 on) that move the
//! files into the 26.x layout, then level.dat brought to the current version,
//! and the upgrades vanilla runs as it reads each file ([`upgrade_world`]
//! does them all at once, as a converter).
use super::dynamic::{as_bool, as_int, as_str, int_list, remove, set, string};
use super::json::{self, Json};
use super::progress::{UpgradeProgress, UpgradeType};
use super::schema::Ty;
use super::world_upgrader::worker_threads;
use super::{CURRENT, update};
use crate::compression::Compression;
use crate::region::{ChunkRecord, RegionFile, RegionWriter};
use horizon_core::position::ChunkPos;
use horizon_core::{
    nbt,
    nbt::{Compound, JavaString, Limits, NamedTag, Tag},
};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The name `SharedConstants.getCurrentVersion().name()` writes in level.dat.
pub const VERSION_NAME: &str = horizon_core::version::VANILLA_VERSION;

const FILE_FIXER_INTRODUCTION: i32 = 4772;
/// The versions of the file fixes, in order.
const FILE_FIXES: [i32; 4] = [1493, 4772, 4773, 4899];

/// `CanceledFileFixException`: the player pressed Cancel.
fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "the world upgrade was canceled")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// `NbtIo.readCompressed`: the root compound of a gzip NBT file, if it exists.
pub fn read_dat(path: &Path) -> io::Result<Option<Tag>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let limits = Limits::saved();
    let raw = Compression::Gzip.decode(&bytes, limits.max_bytes)?;
    let (root, _) = nbt::read_named(&raw, limits).map_err(|e| invalid(e.to_string()))?;
    Ok(Some(root.tag))
}

/// `SavedDataStorage.readTagFromDisk`: a file without the gzip header is plain NBT.
pub fn read_saved_dat(path: &Path) -> io::Result<Option<Tag>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let limits = Limits::saved();
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
        Compression::Gzip.decode(&bytes, limits.max_bytes)?
    } else {
        bytes
    };
    let (root, _) = nbt::read_named(&raw, limits).map_err(|e| invalid(e.to_string()))?;
    Ok(Some(root.tag))
}

/// `NbtIo.writeCompressed`, creating the parent folders, through a
/// temporary file renamed over `path`.
pub fn write_dat(path: &Path, tag: &Tag) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let root = NamedTag {
        name: JavaString::default(),
        tag: tag.clone(),
    };
    let raw = nbt::write_named(&root, Limits::saved()).map_err(|e| invalid(e.to_string()))?;
    let bytes = Compression::Gzip.encode(&raw)?;
    // A sibling written and synced, then renamed over the file: a stop or a
    // full disk leaves the old file whole, never an emptied one.
    // A linked file is written where the link points, as `fs::write` did.
    // A dangling link is replaced by the file, at the link's path.
    let resolved;
    let path = match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => match fs::canonicalize(path) {
            Ok(target) => {
                resolved = target;
                &resolved
            }
            Err(_) => path,
        },
        _ => path,
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = path.with_file_name(format!("{name}.horizon-new"));
    // A stale temporary from a stopped write is ours to replace; anything
    // else with its name (a link, a folder) makes `create_new` fail.
    if fs::symlink_metadata(&temporary).is_ok_and(|meta| meta.file_type().is_file()) {
        fs::remove_file(&temporary)?;
    }
    let written = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::rename(&temporary, path));
    if let Err(e) = written {
        if fs::symlink_metadata(&temporary).is_ok_and(|meta| meta.file_type().is_file()) {
            let _ = fs::remove_file(&temporary);
        }
        return Err(e);
    }
    crate::fs_sync::sync_dir(path.parent().unwrap_or(Path::new(".")))
}

/// `NbtUtils.getDataVersion`.
pub fn data_version(tag: &Tag, default: i32) -> i32 {
    match tag.get("DataVersion") {
        Some(Tag::Int(v)) => *v,
        _ => default,
    }
}

/// `FileFixerUpper.requiresFileFixing`.
pub fn requires_file_fixing(level_version: i32) -> bool {
    let from = if level_version < FILE_FIXER_INTRODUCTION {
        0
    } else {
        level_version
    };
    from < *FILE_FIXES.last().unwrap()
}

// ------------------------------------------------------------------ file operations

fn resolve(base: &Path, relative: &str) -> PathBuf {
    base.join(relative.trim_start_matches('/'))
}

/// `FileFixUtil.moveFile`: nothing when the source is missing, an error when
/// the target exists.
fn move_file(base: &Path, from: &str, to: &str) -> io::Result<()> {
    let from = resolve(base, from);
    if !from.exists() {
        return Ok(());
    }
    let to = resolve(base, to);
    if to.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "cannot move {} to {}: target already exists",
                from.display(),
                to.display()
            ),
        ));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(from, to)
}

/// `FileFixUtil.deleteFileOrEmptyDirectory`.
fn delete(base: &Path, relative: &str) -> io::Result<()> {
    let path = resolve(base, relative);
    if path.is_dir() {
        let entries: Vec<_> = fs::read_dir(&path)?.collect::<Result<_, _>>()?;
        if entries.len() == 1 && entries[0].file_name() == ".DS_Store" {
            let _ = fs::remove_file(entries[0].path());
        }
        if fs::read_dir(&path)?.next().is_some() {
            return Ok(());
        }
        fs::remove_dir(&path)
    } else if path.exists() {
        fs::remove_file(&path)
    } else {
        Ok(())
    }
}

pub(super) fn subdirectories(path: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(path)
        .map(|r| {
            r.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// `FileRelation.DIMENSIONS`.
fn dimensions(base: &Path) -> Vec<PathBuf> {
    let root = base.join("dimensions");
    let defaults = || {
        ["overworld", "the_nether", "the_end"]
            .map(|d| base.join("dimensions/minecraft").join(d))
            .to_vec()
    };
    if !root.exists() {
        return defaults();
    }
    let found: Vec<PathBuf> = subdirectories(&root)
        .iter()
        .flat_map(|ns| subdirectories(ns))
        .collect();
    if found.is_empty() { defaults() } else { found }
}

// ------------------------------------------------------------------ file accessors

/// `LevelDat.read`: level.dat's `Data` brought to `version`.
fn read_level(base: &Path, version: i32) -> io::Result<Option<Tag>> {
    Ok(read_dat(&base.join("level.dat"))?.map(|root| {
        let mut data = root
            .get("Data")
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Compound::new()));
        let from = data_version(&data, -1);
        update(Ty::Level, &mut data, from, version);
        data
    }))
}
fn write_level(base: &Path, mut data: Tag, version: i32) -> io::Result<()> {
    set(&mut data, "DataVersion", Tag::Int(version));
    write_dat(&base.join("level.dat"), &Tag::compound([("Data", data)]))
}
/// `SavedDataNbt.write`.
fn write_saved_data(path: &Path, data: Tag, version: i32) -> io::Result<()> {
    write_dat(
        path,
        &Tag::compound([("data", data), ("DataVersion", Tag::Int(version))]),
    )
}
/// `SavedDataNbt.read`: the `data` of a saved data file brought to `version`.
fn read_saved_data(path: &Path, ty: Ty, version: i32) -> io::Result<Option<Tag>> {
    Ok(read_dat(path)?.map(|mut tag| {
        let from = data_version(&tag, -1);
        update(ty, &mut tag, from, version);
        tag.get("data")
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Compound::new()))
    }))
}

// ------------------------------------------------------------------ the file fixes

/// 4772: `ResourcePackLocationFileFix`, `DimensionStorageFileFix`,
/// `PlayerStorageFileFix`, `LevelDatToSavedDataFileFix`, `RemoveObsoleteFilesFileFix`.
fn fix_4772(base: &Path) -> io::Result<()> {
    move_file(base, "resources.zip", "resourcepacks/resources.zip")?;

    for dim in dimensions(base) {
        let data = dim.join("data");
        move_file(&data, "chunks.dat", "minecraft/chunk_tickets.dat")?;
        move_file(&data, "raids.dat", "minecraft/raids.dat")?;
        move_file(&data, "world_border.dat", "minecraft/world_border.dat")?;
    }
    for (from, to) in [
        ("data", "dimensions/minecraft/overworld/data/minecraft"),
        (
            "DIM-1/data",
            "dimensions/minecraft/the_nether/data/minecraft",
        ),
        ("DIM1/data", "dimensions/minecraft/the_end/data/minecraft"),
    ] {
        for (a, b) in [
            ("chunks.dat", "chunk_tickets.dat"),
            ("raids.dat", "raids.dat"),
            ("raids_end.dat", "raids.dat"),
            ("world_border.dat", "world_border.dat"),
        ] {
            move_file(base, &format!("{from}/{a}"), &format!("{to}/{b}"))?;
        }
    }
    delete(base, "DIM1/data")?;
    delete(base, "DIM-1/data")?;
    let data = base.join("data");
    for (a, b) in [
        ("scoreboard.dat", "minecraft/scoreboard.dat"),
        ("stopwatches.dat", "minecraft/stopwatches.dat"),
    ] {
        move_file(&data, a, b)?;
    }
    regex_move(&data, |name| {
        let ns = name
            .strip_prefix("command_storage_")?
            .strip_suffix(".dat")?;
        (!ns.is_empty()
            && ns
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-')))
        .then(|| format!("{ns}/command_storage.dat"))
    })?;
    move_file(&data, "idcounts.dat", "minecraft/maps/last_id.dat")?;
    regex_move(&data, |name| {
        let id = name.strip_prefix("map_")?.strip_suffix(".dat")?;
        (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
            .then(|| format!("minecraft/maps/{id}.dat"))
    })?;
    move_file(
        &data,
        "random_sequences.dat",
        "minecraft/random_sequences.dat",
    )?;
    for (from, to) in [
        ("", "dimensions/minecraft/overworld"),
        ("DIM-1", "dimensions/minecraft/the_nether"),
        ("DIM1", "dimensions/minecraft/the_end"),
    ] {
        for f in ["region", "entities", "poi"] {
            move_file(base, &format!("{from}/{f}"), &format!("{to}/{f}"))?;
        }
    }
    delete(base, "DIM-1")?;
    delete(base, "DIM1")?;

    move_file(base, "advancements", "players/advancements")?;
    move_file(base, "playerdata", "players/data")?;
    move_file(base, "stats", "players/stats")?;

    level_dat_to_saved_data(base)?;

    for f in [
        "data/villages.dat",
        "data/villages_end.dat",
        "data/villages_nether.dat",
        "data/advancements",
        "data/functions",
    ] {
        delete(base, f)?;
    }
    for s in [
        "Monument",
        "Stronghold",
        "Mineshaft",
        "Temple",
        "Mansion",
        "Village",
        "Fortress",
        "EndCity",
    ] {
        delete(base, &format!("data/{s}.dat"))?;
        delete(base, &format!("data/{s}_index.dat"))?;
    }
    Ok(())
}

/// `RegexMove` over the files of a folder.
fn regex_move(folder: &Path, rename: impl Fn(&str) -> Option<String>) -> io::Result<()> {
    if !folder.is_dir() {
        return Ok(());
    }
    let mut names: Vec<String> = fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        if let Some(new) = rename(&name) {
            move_file(folder, &name, &new)?;
        }
    }
    Ok(())
}

fn level_dat_to_saved_data(base: &Path) -> io::Result<()> {
    const V: i32 = 4772;
    let Some(mut content) = read_level(base, V)? else {
        return Ok(());
    };
    let data = base.join("data/minecraft");
    let dim_data = |dim: &str, file: &str| {
        base.join(format!("dimensions/minecraft/{dim}/data/minecraft/{file}"))
    };
    let extract = |content: &mut Tag, key: &str, path: PathBuf| -> io::Result<()> {
        if let Some(tag) = remove(content, key) {
            write_saved_data(&path, tag, V)?;
        }
        Ok(())
    };
    extract(
        &mut content,
        "dragon_fight",
        dim_data("the_end", "ender_dragon_fight.dat"),
    )?;
    if let Some(mut player) = remove(&mut content, "Player") {
        let from = data_version(&player, -1);
        update(Ty::Player, &mut player, from, V);
        let uuid = match player.get("UUID") {
            Some(u) => u.clone(),
            None => {
                set(&mut player, "DataVersion", Tag::Int(V));
                write_dat(
                    &base.join("players/data/00000000-0000-0000-0000-000000000000.dat"),
                    &player,
                )?;
                int_list(&[0, 0, 0, 0])
            }
        };
        set(&mut content, "singleplayer_uuid", uuid);
    }
    extract(
        &mut content,
        "wandering_trader_migration_data",
        data.join("wandering_trader.dat"),
    )?;
    extract(
        &mut content,
        "CustomBossEvents",
        data.join("custom_boss_events.dat"),
    )?;
    extract(&mut content, "weather_data", data.join("weather.dat"))?;
    extract(
        &mut content,
        "scheduled_events",
        data.join("scheduled_events.dat"),
    )?;
    if let Some(border) = content.get("world_border").cloned() {
        for (dim, divider) in [("overworld", 1.0), ("the_nether", 8.0), ("the_end", 1.0)] {
            let mut b = border.clone();
            for k in ["center_x", "center_z"] {
                if let Some(v) = b.get(k) {
                    let v = super::dynamic::as_double(Some(v), 0.0) / divider;
                    set(&mut b, k, Tag::Double(v.to_bits()));
                }
            }
            write_saved_data(&dim_data(dim, "world_border.dat"), b, V)?;
        }
    }
    remove(&mut content, "world_border");
    extract(&mut content, "game_rules", data.join("game_rules.dat"))?;
    if let Some(mut settings) = remove(&mut content, "world_gen_settings") {
        let from = data_version(&content, -1);
        update(Ty::WorldGenSettings, &mut settings, from, V);
        write_saved_data(&data.join("world_gen_settings.dat"), settings, V)?;
    }
    extract(&mut content, "world_clocks", data.join("world_clocks.dat"))?;
    write_level(base, content, V)
}

/// 1493: `LegacyStructureFileFix`. The structures of `data/*.dat` (1.12) become the
/// `Structures` of the chunks they reach, in the region folders of the old layout.
pub(crate) fn fix_1493(base: &Path) -> io::Result<()> {
    use std::collections::{BTreeMap, HashMap};
    const LEGACY_TO_CURRENT: [(&str, &str); 4] = [
        ("Iglu", "Igloo"),
        ("TeDP", "Desert_Pyramid"),
        ("TeJP", "Jungle_Pyramid"),
        ("TeSH", "Swamp_Hut"),
    ];
    #[derive(Default)]
    struct LegacyData {
        starts: HashMap<String, Tag>,
        indexes: BTreeMap<String, Vec<i64>>,
    }
    let pack = |x: i32, z: i32| i64::from(x) & 0xFFFF_FFFF | (i64::from(z) & 0xFFFF_FFFF) << 32;
    let Some(level) = read_level(base, 1493)? else {
        return Ok(());
    };
    let overworld_generator = match as_str(level.get("generatorName")).as_deref() {
        Some("flat") => "minecraft:flat",
        Some("debug_all_block_states") => "minecraft:debug",
        _ => "minecraft:noise",
    };
    let dimensions: [(&[&str], &str, &str, &str); 3] = [
        (
            &["Monument", "Stronghold", "Mineshaft", "Temple", "Mansion"],
            "",
            "minecraft:overworld",
            overworld_generator,
        ),
        (
            &["Fortress"],
            "DIM-1",
            "minecraft:the_nether",
            "minecraft:noise",
        ),
        (&["EndCity"], "DIM1", "minecraft:the_end", "minecraft:noise"),
    ];
    for (ids, folder, dimension, generator) in dimensions {
        let mut structures: BTreeMap<(i32, i32), LegacyData> = BTreeMap::new();
        for id in ids {
            let path = base.join("data").join(format!("{id}.dat"));
            let Some(data) = read_saved_data(&path, Ty::SavedStructureFeatureIndices, 1493)? else {
                continue;
            };
            let Some(Tag::Compound(features)) = data.get("Features") else {
                continue;
            };
            let values: Vec<(String, Tag)> = features
                .iter()
                .map(|(k, v)| (k.to_string_lossy(), v.clone()))
                .collect();
            for (_, mut value) in super::items::java_hash_order(values, |(k, _)| k.clone()) {
                let chunk_x = as_int(value.get("ChunkX"), 0);
                let chunk_z = as_int(value.get("ChunkZ"), 0);
                let children = super::dynamic::stream(value.get("Children"));
                if let Some(first) = children.first()
                    && let Some(current) = as_str(first.get("id"))
                        .and_then(|i| LEGACY_TO_CURRENT.iter().find(|(from, _)| *from == i))
                {
                    set(&mut value, "id", string(current.1));
                }
                let Some(feature_id) = as_str(value.get("id")) else {
                    continue;
                };
                structures
                    .entry((chunk_x, chunk_z))
                    .or_default()
                    .starts
                    .insert(feature_id.clone(), value);
                for nx in chunk_x - 8..=chunk_x + 8 {
                    for nz in chunk_z - 8..=chunk_z + 8 {
                        structures
                            .entry((nx, nz))
                            .or_default()
                            .indexes
                            .entry(feature_id.clone())
                            .or_default()
                            .push(pack(chunk_x, chunk_z));
                    }
                }
            }
        }
        if structures.is_empty() {
            continue;
        }
        let region = if folder.is_empty() {
            base.join("region")
        } else {
            base.join(folder).join("region")
        };
        upgrade_regions(&region, &Progress::none(), None, |tag| {
            let version = as_int(tag.get("DataVersion"), -1);
            let x = as_int(tag.get("Level").and_then(|l| l.get("xPos")), 0);
            let z = as_int(tag.get("Level").and_then(|l| l.get("zPos")), 0);
            let Some(legacy) = structures.get(&(x, z)) else {
                return;
            };
            let _ = version;
            super::update_chunk_to(tag, dimension, Some(generator), 1493);
            let mut level = tag
                .get("Level")
                .cloned()
                .unwrap_or_else(super::dynamic::empty_map);
            let mut structure = level
                .get("Structures")
                .cloned()
                .filter(|s| matches!(s, Tag::Compound(_)))
                .unwrap_or_else(super::dynamic::empty_map);
            let mut starts = structure
                .get("Starts")
                .cloned()
                .filter(|s| matches!(s, Tag::Compound(_)))
                .unwrap_or_else(super::dynamic::empty_map);
            let mut references = structure
                .get("References")
                .cloned()
                .filter(|s| matches!(s, Tag::Compound(_)))
                .unwrap_or_else(super::dynamic::empty_map);
            for (id, value) in &legacy.starts {
                set(&mut starts, id, value.clone());
            }
            for (id, indexes) in &legacy.indexes {
                set(&mut references, id, Tag::LongArray(indexes.clone()));
            }
            set(&mut structure, "Starts", starts);
            set(&mut structure, "References", references);
            set(&mut level, "Structures", structure);
            set(tag, "Level", level);
        })?;
    }
    Ok(())
}

/// 4773: `GeneratedStructuresRenameFileFix`.
fn fix_4773(base: &Path) -> io::Result<()> {
    for ns in subdirectories(&base.join("generated")) {
        move_file(&ns, "structures", "structure")?;
    }
    Ok(())
}

/// 4899: `ReenableSpectatorsGenerateChunksInHardcoreWorldsFileFix`.
fn fix_4899(base: &Path) -> io::Result<()> {
    const V: i32 = 4899;
    let Some(level) = read_level(base, V)? else {
        return Ok(());
    };
    let hardcore = level
        .get("difficulty_settings")
        .is_some_and(|d| as_bool(d.get("hardcore"), false));
    if level.get("singleplayer_uuid").is_some() && hardcore {
        let path = base.join("data/minecraft/game_rules.dat");
        if let Some(mut rules) = read_saved_data(&path, Ty::SavedGameRules, V)? {
            if rules.get("minecraft:spectators_generate_chunks").is_some() {
                set(
                    &mut rules,
                    "minecraft:spectators_generate_chunks",
                    Tag::Byte(1),
                );
            }
            write_saved_data(&path, rules, V)?;
        }
    }
    Ok(())
}

/// `FileFixerUpper.addVersionsToLevelData`.
fn add_versions(data: &mut Tag) {
    set(data, "DataVersion", Tag::Int(CURRENT));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    set(data, "LastPlayed", Tag::Long(now));
    set(
        data,
        "Version",
        Tag::compound([
            ("Name", string(VERSION_NAME)),
            ("Id", Tag::Int(CURRENT)),
            ("Snapshot", Tag::Byte(0)),
            ("Series", string("main")),
        ]),
    );
}

/// `FileFixerUpper.fix`: the file fixes a world saved at an older version
/// needs, then its level.dat at the current version. Returns the DataVersion
/// the world had.
pub fn fix_world_files(base: &Path) -> io::Result<i32> {
    let (loaded, pending) = fix_world_files_deferred(base, &UpgradeProgress::new())?;
    if let Some(data) = pending {
        write_dat(&base.join("level.dat"), &Tag::compound([("Data", data)]))?;
    }
    Ok(loaded)
}

/// [`fix_world_files`] without the final level.dat write: returns the loaded
/// DataVersion and the level data at the current version still to be written
/// (`None` when the world is already current). A caller that converts the
/// world's data afterwards writes it last, so a failed upgrade keeps the old
/// DataVersion and is retried on the next open.
fn fix_world_files_deferred(
    base: &Path,
    progress: &UpgradeProgress,
) -> io::Result<(i32, Option<Tag>)> {
    let root = read_dat(&base.join("level.dat"))?.ok_or_else(|| invalid("missing level.dat"))?;
    let mut data = root
        .get("Data")
        .cloned()
        .unwrap_or_else(|| Tag::Compound(Compound::new()));
    let loaded = data_version(&data, -1);
    if loaded >= CURRENT {
        return Ok((loaded, None));
    }
    let mut from = loaded;
    if requires_file_fixing(loaded) {
        let start = if loaded < FILE_FIXER_INTRODUCTION {
            0
        } else {
            loaded
        };
        // `FileFixerUpper.applyFileFixers`: the type, the count (a file fix is
        // one operation here), then each fixer in turn.
        let applicable: Vec<i32> = FILE_FIXES
            .into_iter()
            .filter(|&version| version > start && version <= CURRENT)
            .collect();
        progress.set_file_fixing();
        progress.set_type(UpgradeType::Files);
        progress.add_total_file_fix_operations(applicable.len() as i32);
        progress.set_applicable_fixer_amount(applicable.len() as i32);
        for (done, version) in applicable.into_iter().enumerate() {
            // `FileFix.runFixOperations` stops on a cancel, which vanilla rolls
            // back. These fixes move files in place, so only the one that
            // would be the first leaves the world as it was.
            if done == 0 && progress.is_canceled() {
                return Err(canceled());
            }
            progress.increment_running_file_fixer();
            match version {
                1493 => fix_1493(base)?,
                4772 => fix_4772(base)?,
                4773 => fix_4773(base)?,
                _ => fix_4899(base)?,
            }
            progress.increment_finished_operations();
        }
        // `writeUpdatedLevelData`, then the world swapped in and read again.
        let root =
            read_dat(&base.join("level.dat"))?.ok_or_else(|| invalid("missing level.dat"))?;
        data = root
            .get("Data")
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Compound::new()));
        from = data_version(&data, -1);
        let _ = fs::remove_file(base.join("level.dat_old"));
        let _ = fs::remove_file(base.join("session.lock"));
    }
    update(Ty::Level, &mut data, from, CURRENT);
    add_versions(&mut data);
    Ok((loaded, Some(data)))
}

/// The world's generator settings as 26.3 reads them once the file fixes have run, without
/// running them: for a world older than 4772 they come out of level.dat the way
/// `LevelDatToSavedDataFileFix` moves them (`world_gen_settings.dat` is not there yet), for any
/// other from the saved data file, brought to the current version. `None` when there are none.
pub fn current_world_gen_settings(base: &Path) -> io::Result<Option<Tag>> {
    const V: i32 = 4772;
    let Some(root) = read_dat(&base.join("level.dat"))? else {
        return Ok(None);
    };
    let loaded = root.get("Data").map_or(-1, |data| data_version(data, -1));
    if loaded < V {
        let Some(content) = read_level(base, V)? else {
            return Ok(None);
        };
        let Some(mut settings) = content.get("world_gen_settings").cloned() else {
            return Ok(None);
        };
        let from = data_version(&content, -1);
        update(Ty::WorldGenSettings, &mut settings, from, V);
        update(Ty::WorldGenSettings, &mut settings, V, CURRENT);
        return Ok(Some(settings));
    }
    read_saved_data(
        &base.join("data/minecraft/world_gen_settings.dat"),
        Ty::SavedWorldGenSettings,
        CURRENT,
    )
}

// ------------------------------------------------------------------ the read-time upgrades

/// Chunk generator types per dimension from `world_gen_settings.dat`, the
/// `generator` of the context the chunk fixes read.
pub fn generators(base: &Path) -> io::Result<Vec<(String, String)>> {
    let Some(tag) = read_dat(&base.join("data/minecraft/world_gen_settings.dat"))? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    if let Some(Tag::Compound(dims)) = tag.get("data").and_then(|d| d.get("dimensions")) {
        for (id, dim) in dims {
            if let Some(ty) = dim.get("generator").and_then(|g| as_str(g.get("type"))) {
                out.push((id.to_string_lossy(), ty));
            }
        }
    }
    Ok(out)
}

fn files_with_suffix(folder: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(folder)
        .map(|r| {
            r.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file() && p.to_string_lossy().ends_with(suffix))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Rewrite every chunk of the region files of a folder with `fix`, the files
/// spread over [`worker_threads`] workers. Each file is read, fixed and
/// written whole by one worker, so its bytes do not depend on the thread count.
/// `target` is the DataVersion `fix` brings a chunk to, `None` for a fix that
/// keeps it.
fn upgrade_regions(
    folder: &Path,
    progress: &Progress,
    target: Option<i32>,
    fix: impl Fn(&mut Tag) + Sync,
) -> io::Result<()> {
    upgrade_regions_with_threads(folder, progress, target, fix, worker_threads())
}

/// [`upgrade_regions`] on `threads` workers. A cancel or an error stops every
/// worker before its next file; the error returned is the first file's in
/// the folder's order.
fn upgrade_regions_with_threads(
    folder: &Path,
    progress: &Progress,
    target: Option<i32>,
    fix: impl Fn(&mut Tag) + Sync,
    threads: usize,
) -> io::Result<()> {
    let files = files_with_suffix(folder, ".mca");
    if files.is_empty() {
        return Ok(());
    }
    let staging = folder.join(".datafix");
    match fs::symlink_metadata(&staging) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "{} is a link or a file, not a folder: the world upgrade leaves it alone",
                    staging.display()
                ),
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&staging)?,
        Err(e) => return Err(e),
    }
    clear_staging(folder, &staging, target)?;
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failed: Mutex<Option<(usize, io::Error)>> = Mutex::new(None);
    let work = || {
        while !stop.load(Ordering::Relaxed) {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(path) = files.get(index) else {
                break;
            };
            match progress
                .check()
                .and_then(|()| upgrade_region_file(path, &staging, &fix))
            {
                Ok(()) => progress.progress.increment_finished_operations(),
                Err(e) => {
                    stop.store(true, Ordering::Relaxed);
                    let mut failed = failed.lock().unwrap_or_else(|e| e.into_inner());
                    if failed.as_ref().is_none_or(|(first, _)| index < *first) {
                        *failed = Some((index, e));
                    }
                }
            }
        }
    };
    std::thread::scope(|scope| {
        // This thread is worker 0; a helper that fails to start leaves its
        // share to the others.
        for i in 1..threads.clamp(1, files.len()) {
            let _ = std::thread::Builder::new()
                .name(format!("World Upgrader #{i}"))
                .spawn_scoped(scope, work);
        }
        work();
    });
    if let Some((_, e)) = failed.into_inner().unwrap_or_else(|e| e.into_inner()) {
        let _ = fs::remove_dir(&staging);
        return Err(e);
    }
    // The regions' renames are on the disk before the world is stamped.
    crate::fs_sync::sync_dir(folder)?;
    fs::remove_dir(&staging).map_err(|e| {
        let entry = fs::read_dir(&staging)
            .ok()
            .and_then(|mut entries| entries.next())
            .and_then(Result::ok);
        match entry {
            Some(entry) => io::Error::new(
                e.kind(),
                format!(
                    "cannot remove {}: it holds {}, which the world upgrade did not write",
                    staging.display(),
                    entry.file_name().to_string_lossy()
                ),
            ),
            None => e,
        }
    })
}

/// `<prefix><x>.<z><suffix>`, both numbers written as the upgrader writes
/// them.
fn coordinates(name: &str, prefix: &str, suffix: &str) -> Option<(i32, i32)> {
    let (x, z) = name
        .strip_prefix(prefix)?
        .strip_suffix(suffix)?
        .split_once('.')?;
    let number = |s: &str| s.parse::<i32>().ok().filter(|v| v.to_string() == s);
    Some((number(x)?, number(z)?))
}

/// `RegionWriter`'s temporary file, `.horizon-<process>-<n>.tmp`.
fn is_writer_temporary(name: &str) -> bool {
    let number = |s: &str| s.parse::<u64>().is_ok_and(|v| v.to_string() == s);
    name.strip_prefix(".horizon-")
        .and_then(|rest| rest.strip_suffix(".tmp"))
        .and_then(|rest| rest.split_once('-'))
        .is_some_and(|(process, n)| number(process) && number(n))
}

/// Whether nothing, or a plain file, has this name (a link is neither).
fn absent_or_file(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_file()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(e),
    }
}

/// Clear what a stopped upgrade left in `staging`, touching only regular
/// files with the names the upgrader writes; anything else stays.
///
/// - A region file or a writer temporary is a copy of files still in
///   `folder`: removed.
/// - A `c.X.Z.mcc` that a staged region file refers to belongs to a region
///   that was not moved out: removed with it.
/// - Any other `c.X.Z.mcc` goes beside the region only where that provably
///   loses nothing (see [`may_install`]); otherwise it is kept in
///   `.datafix-leftover`, which nothing reads, and the retry rebuilds the
///   chunk from the live files.
fn clear_staging(folder: &Path, staging: &Path, target: Option<i32>) -> io::Result<()> {
    let mut regions = Vec::new();
    let mut externals = Vec::new();
    let mut copies = Vec::new();
    for entry in fs::read_dir(staging)? {
        let path = entry?.path();
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(region) = coordinates(name, "r.", ".mca") {
            regions.push(region);
            copies.push(path);
        } else if let Some((x, z)) = coordinates(name, "c.", ".mcc") {
            externals.push((ChunkPos::new(x, z), path));
        } else if is_writer_temporary(name) {
            copies.push(path);
        }
    }
    for (chunk, path) in externals {
        let region = chunk.region();
        if regions.contains(&(region.x, region.z)) {
            let staged = staging.join(format!("r.{}.{}.mca", region.x, region.z));
            if reads_external(&staged, chunk, staging) {
                fs::remove_file(&path)?;
            } else {
                set_aside(folder, &path)?;
            }
            continue;
        }
        let live = folder.join(path.file_name().unwrap_or_default());
        if may_install(folder, staging, chunk, &path, &live, target)? {
            // Its data on the disk before it replaces anything.
            File::open(&path)?.sync_all()?;
            fs::rename(&path, &live)?;
        } else {
            set_aside(folder, &path)?;
        }
    }
    for path in copies {
        fs::remove_file(path)?;
    }
    crate::fs_sync::sync_dir(folder)
}

/// Whether the region file `region` marks `chunk` as external and has its
/// `c.X.Z.mcc` in `directory`.
fn reads_external(region: &Path, chunk: ChunkPos, directory: &Path) -> bool {
    RegionFile::open(region)
        .and_then(|mut file| file.read_external_from(chunk, directory))
        .is_ok_and(|record| record.is_some())
}

/// Whether the staged `c.X.Z.mcc` of `chunk` may go to `live`: the live
/// region's stub for the chunk is external and reads it whole, and either
/// - nothing is at `live`: installing overwrites nothing; the file is at or
///   below `target` (an older one is upgraded by the retry), or
/// - a plain file is at `live`, the live region was already replaced by the
///   upgrader (it holds an inline chunk at `target`, which only a
///   replacement writes), the staged file is at `target`, and the live file
///   was modified no later than it: the staged file is the one the region
///   was written for. A later live file was saved since (an older game
///   played the world), so it stays.
///
/// A live `.mcc` beside a region not yet replaced may be newer than the
/// staged copy (the world played or restored in between, or a fixed build
/// at the same target), so it is never replaced; that case, and every
/// unclear one, is set aside.
fn may_install(
    folder: &Path,
    staging: &Path,
    chunk: ChunkPos,
    staged: &Path,
    live: &Path,
    target: Option<i32>,
) -> io::Result<bool> {
    let at = chunk.region();
    let Ok(mut region) = RegionFile::open(folder.join(format!("r.{}.{}.mca", at.x, at.z))) else {
        return Ok(false);
    };
    let Ok(Some(record)) = region.read_external_from(chunk, staging) else {
        return Ok(false);
    };
    let Ok(root) = record.nbt(Limits::saved()) else {
        return Ok(false);
    };
    let version = data_version(&root.tag, -1);
    match fs::symlink_metadata(live) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Ok(target.is_none_or(|target| version <= target))
        }
        Err(e) => Err(e),
        Ok(meta) if !meta.file_type().is_file() => Ok(false),
        Ok(meta) => {
            let not_newer = match (meta.modified(), fs::metadata(staged)?.modified()) {
                (Ok(live), Ok(staged)) => live <= staged,
                _ => false,
            };
            Ok(not_newer
                && target.is_some_and(|target| {
                    version == target && holds_inline_chunk_at(&mut region, target)
                }))
        }
    }
}

/// Whether the region holds an inline chunk at DataVersion `target`.
fn holds_inline_chunk_at(region: &mut RegionFile, target: i32) -> bool {
    let positions: Vec<_> = region.chunks().map(|(p, _)| p).collect();
    positions.into_iter().any(|pos| {
        matches!(region.is_external(pos), Ok(false))
            && matches!(
                region.read(pos),
                Ok(Some(record)) if record
                    .nbt(Limits::saved())
                    .is_ok_and(|root| data_version(&root.tag, -1) == target)
            )
    })
}

/// `path` moved into `folder/.datafix-leftover` under a free name.
fn set_aside(folder: &Path, path: &Path) -> io::Result<()> {
    let leftover = folder.join(".datafix-leftover");
    match fs::symlink_metadata(&leftover) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is not a folder", leftover.display()),
            ));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => fs::create_dir(&leftover)?,
        Err(e) => return Err(e),
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut kept = leftover.join(&*name);
    let mut n = 1;
    while fs::symlink_metadata(&kept).is_ok() {
        kept = leftover.join(format!("{name}.{n}"));
        n += 1;
    }
    fs::rename(path, &kept)?;
    log::warn!(
        "World upgrade: {} does not match its region, kept as {}",
        path.display(),
        kept.display()
    );
    Ok(())
}

/// One file of [`upgrade_regions`]: every chunk fixed, then the file written
/// anew in `staging` and moved over the old one, its oversized chunks'
/// `c.X.Z.mcc` files first. A file without chunks (most of a large world's
/// are empty) has nothing to fix and is left as it is.
fn upgrade_region_file(path: &Path, staging: &Path, fix: &impl Fn(&mut Tag)) -> io::Result<()> {
    let limits = Limits::saved();
    let mut region = RegionFile::open(path)?;
    let positions: Vec<_> = region.chunks().map(|(p, _)| p).collect();
    if positions.is_empty() {
        return Ok(());
    }
    let mut records = Vec::with_capacity(positions.len());
    for pos in positions {
        let Some(record) = region.read(pos)? else {
            continue;
        };
        let mut root = record.nbt(limits)?;
        fix(&mut root.tag);
        records.push((
            pos,
            ChunkRecord::from_nbt(&root, record.compression, record.timestamp, limits)?,
        ));
    }
    drop(region);
    let temporary = staging.join(path.file_name().unwrap_or_default());
    // `clear_staging` removed a plain file here. A link is unlinked (its
    // target is not touched); anything else is reported.
    let in_the_way = match fs::symlink_metadata(&temporary) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::remove_file(&temporary).or_else(|_| fs::remove_dir(&temporary))?;
            false
        }
        Ok(_) => true,
        Err(_) => false,
    };
    if in_the_way {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "{} is in the way of the world upgrade: remove it",
                temporary.display()
            ),
        ));
    }
    let mut writer = RegionWriter::create(&temporary)?;
    for (pos, record) in &records {
        writer.write(*pos, record)?;
    }
    let externals = writer.external_files().to_vec();
    writer.finish()?;
    // The `.mcc` files go before the region that points to them: a stop in
    // between leaves the old region, which reads its own chunks or these
    // upgraded copies (the compression is kept), never a stub whose file is
    // missing.
    let folder = path.parent().unwrap_or(Path::new("."));
    for external in externals {
        let live = folder.join(external.file_name().unwrap_or_default());
        if !absent_or_file(&live)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} is not a plain file", live.display()),
            ));
        }
        fs::rename(&external, &live)?;
    }
    crate::fs_sync::sync_dir(folder)?;
    fs::rename(&temporary, path)
}

/// The progress a data upgrade reports to and whether it may stop early: a
/// cancel is honoured only while the world's files are still unmoved, since
/// the DataVersion is stamped last and a retry would run the file fixes again.
struct Progress<'a> {
    progress: &'a UpgradeProgress,
    cancelable: bool,
}
impl Progress<'static> {
    /// A converter run with nobody watching.
    fn none() -> Self {
        static NONE: std::sync::LazyLock<UpgradeProgress> =
            std::sync::LazyLock::new(UpgradeProgress::new);
        Progress {
            progress: &NONE,
            cancelable: false,
        }
    }
}
impl Progress<'_> {
    fn check(&self) -> io::Result<()> {
        if self.cancelable && self.progress.is_canceled() {
            Err(canceled())
        } else {
            Ok(())
        }
    }
}

/// `SavedDataStorage.readTagFromDisk` over every known file of a data folder.
fn saved_data_files(data: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !data.is_dir() {
        return Ok(files);
    }
    let mut stack = vec![data.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)?.filter_map(Result::ok) {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().ends_with(".dat") {
                files.push(p);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn upgrade_saved_data(data: &Path, progress: &Progress) -> io::Result<()> {
    for file in saved_data_files(data)? {
        progress.check()?;
        let relative = file
            .strip_prefix(data)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        if let Some(ty) = super::saved_data_type(&relative)
            && let Some(mut tag) = read_saved_dat(&file)?
        {
            super::update_saved_data(ty, &mut tag);
            set(&mut tag, "DataVersion", Tag::Int(CURRENT));
            write_dat(&file, &tag)?;
        }
        progress.progress.increment_finished_operations();
    }
    Ok(())
}

/// A JSON file of the player folders as NBT for the fixes (`JsonOps` and
/// `NbtOps` see the same structure), booleans kept apart.
fn json_to_tag(j: &Json) -> Tag {
    match j {
        Json::Bool(b) => Tag::compound([("\u{0}bool", Tag::Byte(i8::from(*b)))]),
        Json::Arr(items) => Tag::List(items.iter().map(json_to_tag).collect()),
        Json::Obj(entries) => Tag::Compound(
            entries
                .iter()
                .map(|(k, v)| (JavaString(k.clone()), json_to_tag(v)))
                .collect(),
        ),
        other => json::to_nbt(other).unwrap_or(Tag::End),
    }
}
fn tag_to_json(t: &Tag) -> Json {
    match t {
        Tag::Compound(c) if c.len() == 1 && c.contains_key(&JavaString::from("\u{0}bool")) => {
            Json::Bool(as_int(t.get("\u{0}bool"), 0) != 0)
        }
        Tag::List(items) => Json::Arr(items.iter().map(tag_to_json).collect()),
        Tag::Compound(c) => Json::Obj(
            c.iter()
                .map(|(k, v)| (k.0.clone(), tag_to_json(v)))
                .collect(),
        ),
        other => json::from_nbt(other),
    }
}

fn upgrade_json(path: &Path, ty: Ty) -> io::Result<()> {
    upgrade_json_to(path, ty, CURRENT)
}

fn upgrade_json_to(path: &Path, ty: Ty, to: i32) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    let units: Vec<u16> = text.encode_utf16().collect();
    let parsed = json::parse_lenient(&units)
        .ok_or_else(|| invalid(format!("bad JSON in {}", path.display())))?;
    let mut tag = json_to_tag(&parsed);
    let from = super::dynamic::as_int(tag.get("DataVersion"), 1343);
    if ty == Ty::Advancements {
        // `PlayerAdvancements`: the version field is not part of the data.
        remove(&mut tag, "DataVersion");
    }
    update(ty, &mut tag, from, to);
    set(&mut tag, "DataVersion", Tag::Int(to));
    let out = json::to_stable_string(&tag_to_json(&tag));
    fs::write(path, String::from_utf16_lossy(&out))
}

/// Everything vanilla upgrades as it reads a world, done at once: every chunk,
/// entity chunk and POI chunk, saved data, player, advancements and
/// statistics file. Run after [`fix_world_files`].
pub fn upgrade_world_data(base: &Path) -> io::Result<()> {
    upgrade_world_data_with(base, &UpgradeProgress::new(), false)
}

/// [`upgrade_world_data`] reporting to `progress` (type `Regions`, one
/// operation per file) and stopping on its cancel when `cancelable`.
pub fn upgrade_world_data_with(
    base: &Path,
    progress: &UpgradeProgress,
    cancelable: bool,
) -> io::Result<()> {
    let generators = generators(base)?;
    let dimensions: Vec<(String, PathBuf)> = subdirectories(&base.join("dimensions"))
        .into_iter()
        .flat_map(|ns| {
            subdirectories(&ns).into_iter().map(move |dim| {
                let id = format!(
                    "{}:{}",
                    ns.file_name().unwrap_or_default().to_string_lossy(),
                    dim.file_name().unwrap_or_default().to_string_lossy()
                );
                (id, dim)
            })
        })
        .collect();
    let players = files_with_suffix(&base.join("players/data"), ".dat");
    let advancements = files_with_suffix(&base.join("players/advancements"), ".json");
    let stats = files_with_suffix(&base.join("players/stats"), ".json");
    // Counting, as `FileFixerUpper.countFiles` does before the work.
    progress.set_type(UpgradeType::Regions);
    let mut total = players.len() + advancements.len() + stats.len();
    total += saved_data_files(&base.join("data"))?.len();
    for (_, dim) in &dimensions {
        for folder in ["region", "entities", "poi"] {
            total += files_with_suffix(&dim.join(folder), ".mca").len();
        }
        total += saved_data_files(&dim.join("data"))?.len();
    }
    progress.add_total_file_fix_operations(total as i32);
    let run = Progress {
        progress,
        cancelable,
    };
    for (id, dim) in &dimensions {
        let generator = generators
            .iter()
            .find(|(d, _)| d == id)
            .map(|(_, g)| g.clone());
        upgrade_regions(&dim.join("region"), &run, Some(CURRENT), |tag| {
            super::update_chunk(tag, id, generator.as_deref())
        })?;
        upgrade_regions(&dim.join("entities"), &run, Some(CURRENT), |tag| {
            super::update_region_tag(Ty::EntityChunk, tag)
        })?;
        upgrade_regions(&dim.join("poi"), &run, Some(CURRENT), |tag| {
            super::update_region_tag(Ty::PoiChunk, tag)
        })?;
        upgrade_saved_data(&dim.join("data"), &run)?;
    }
    upgrade_saved_data(&base.join("data"), &run)?;
    for file in players {
        run.check()?;
        if let Some(mut tag) = read_dat(&file)? {
            super::update_player(&mut tag);
            set(&mut tag, "DataVersion", Tag::Int(CURRENT));
            write_dat(&file, &tag)?;
        }
        progress.increment_finished_operations();
    }
    for file in advancements {
        run.check()?;
        upgrade_json(&file, Ty::Advancements)?;
        progress.increment_finished_operations();
    }
    for file in stats {
        run.check()?;
        upgrade_json(&file, Ty::Stats)?;
        progress.increment_finished_operations();
    }
    Ok(())
}

/// A world in the layout of before 1.16 (dimension folders in the world folder,
/// `playerdata`, `stats`, `advancements`) brought to DataVersion `target` and
/// no further, with the file fixes of that range but not the ones that move the
/// files into the 26.x layout: the harness checks the fixes of a range of
/// versions on their own (the vanilla oracle does the same given a target).
pub fn upgrade_band(base: &Path, target: i32) -> io::Result<()> {
    if target >= 1493 {
        fix_1493(base)?;
    }
    let level = base.join("level.dat");
    if let Some(mut root) = read_dat(&level)? {
        let mut data = root
            .get("Data")
            .cloned()
            .unwrap_or_else(|| Tag::Compound(Compound::new()));
        let from = data_version(&data, -1);
        update(Ty::Level, &mut data, from, target);
        set(&mut data, "DataVersion", Tag::Int(target));
        set(&mut root, "Data", data);
        write_dat(&level, &root)?;
    }
    for (folder, id) in [
        ("", "minecraft:overworld"),
        ("DIM-1", "minecraft:the_nether"),
        ("DIM1", "minecraft:the_end"),
    ] {
        let dim = base.join(folder);
        upgrade_regions(
            &dim.join("region"),
            &Progress::none(),
            Some(target),
            |tag| super::update_chunk_to(tag, id, None, target),
        )?;
        upgrade_regions(
            &dim.join("entities"),
            &Progress::none(),
            Some(target),
            |tag| super::update_region_tag_to(Ty::EntityChunk, tag, target),
        )?;
        upgrade_regions(&dim.join("poi"), &Progress::none(), Some(target), |tag| {
            super::update_region_tag_to(Ty::PoiChunk, tag, target)
        })?;
    }
    for file in files_with_suffix(&base.join("playerdata"), ".dat") {
        if let Some(mut tag) = read_dat(&file)? {
            super::update_player_to(&mut tag, target);
            write_dat(&file, &tag)?;
        }
    }
    for (folder, ty) in [("advancements", Ty::Advancements), ("stats", Ty::Stats)] {
        for file in files_with_suffix(&base.join(folder), ".json") {
            upgrade_json_to(&file, ty, target)?;
        }
    }
    // The saved data of the old layout.
    for file in files_with_suffix(&base.join("data"), ".dat") {
        let name = file
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ty = match name.as_str() {
            "scoreboard" => Ty::SavedScoreboard,
            n if n
                .strip_prefix("map_")
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())) =>
            {
                Ty::SavedMapData
            }
            _ => continue,
        };
        if let Some(mut tag) = read_dat(&file)? {
            let from = data_version(&tag, 1343);
            update(ty, &mut tag, from, target);
            set(&mut tag, "DataVersion", Tag::Int(target));
            write_dat(&file, &tag)?;
        }
    }
    Ok(())
}

/// Bring a whole world saved by 1.20 to 26.2 to the 26.3 format in place.
pub fn upgrade_world(base: &Path) -> io::Result<i32> {
    upgrade_world_with(base, &UpgradeProgress::new())
}

/// [`upgrade_world`] reporting to `progress`, which also carries the player's
/// cancel (`FileFixerProgressScreen`'s button).
pub fn upgrade_world_with(base: &Path, progress: &UpgradeProgress) -> io::Result<i32> {
    let (version, pending) = fix_world_files_deferred(base, progress)?;
    upgrade_world_data_with(base, progress, !progress.is_file_fixing())?;
    // Stamp the current DataVersion only once everything converted, so a failed
    // upgrade is retried on the next open.
    if let Some(data) = pending {
        write_dat(&base.join("level.dat"), &Tag::compound([("Data", data)]))?;
    }
    Ok(version)
}

#[cfg(test)]
mod upgrade_tests {
    use super::*;

    fn level_version(base: &Path) -> i32 {
        let root = read_dat(&base.join("level.dat")).unwrap().unwrap();
        data_version(root.get("Data").unwrap(), -1)
    }

    /// `FileFixerProgressScreen` reads the upgrade's counts: the data stage is
    /// type `Regions` with one operation per file, and a world that needs no
    /// file fixing does not open the screen.
    #[test]
    fn upgrade_reports_one_operation_per_file() {
        let base = std::env::temp_dir().join(format!("horizon-progress-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let level = Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(4899))]))]);
        write_dat(&base.join("level.dat"), &level).unwrap();
        let player = Tag::compound([("DataVersion", Tag::Int(4899))]);
        write_dat(&base.join("players/data/a.dat"), &player).unwrap();
        write_dat(&base.join("players/data/b.dat"), &player).unwrap();

        let progress = UpgradeProgress::new();
        upgrade_world_with(&base, &progress).unwrap();
        assert!(!progress.is_file_fixing());
        assert_eq!(progress.upgrade_type(), Some(UpgradeType::Regions));
        assert_eq!(progress.total_file_fix_stats().total_operations(), 2);
        assert_eq!(progress.total_file_fix_stats().finished_operations(), 2);
        assert_eq!(progress.total_file_fix_stats().progress(), 1.0);
        let _ = fs::remove_dir_all(&base);
    }

    /// Cancel stops the data stage of a world with no file fixing before the
    /// next file, and the DataVersion stays old so the next open retries.
    #[test]
    fn cancel_stops_the_data_upgrade_and_keeps_the_version() {
        let base = std::env::temp_dir().join(format!("horizon-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let level = Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(4899))]))]);
        write_dat(&base.join("level.dat"), &level).unwrap();
        write_dat(
            &base.join("players/data/a.dat"),
            &Tag::compound([("DataVersion", Tag::Int(4899))]),
        )
        .unwrap();

        let progress = UpgradeProgress::new();
        progress.set_canceled();
        let error = upgrade_world_with(&base, &progress).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(progress.total_file_fix_stats().finished_operations(), 0);
        assert_eq!(level_version(&base), 4899);
        let _ = fs::remove_dir_all(&base);
    }

    /// A failed data upgrade must keep the old DataVersion so the next open
    /// retries, and a later successful run stamps the current one.
    #[test]
    fn failed_upgrade_keeps_old_data_version() {
        let base = std::env::temp_dir().join(format!("horizon-upgrade-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let old = 4899;
        let level = Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(old))]))]);
        write_dat(&base.join("level.dat"), &level).unwrap();
        let region = base.join("dimensions/minecraft/overworld/region");
        fs::create_dir_all(&region).unwrap();
        // Not a valid region file name, so opening it fails mid-upgrade.
        fs::write(region.join("broken.mca"), b"x").unwrap();

        assert!(upgrade_world(&base).is_err());
        assert_eq!(level_version(&base), old);

        fs::remove_file(region.join("broken.mca")).unwrap();
        assert_eq!(upgrade_world(&base).unwrap(), old);
        assert_eq!(level_version(&base), CURRENT);
        let _ = fs::remove_dir_all(&base);
    }

    fn old_chunk(x: i32, z: i32) -> Tag {
        Tag::compound([
            ("DataVersion", Tag::Int(4000)),
            ("xPos", Tag::Int(x)),
            ("zPos", Tag::Int(z)),
            ("Status", Tag::String("minecraft:full".into())),
        ])
    }

    /// `files` region files of `chunks` old chunks each.
    fn old_region_folder(dir: &Path, files: i32, chunks: i32) {
        fs::create_dir_all(dir).unwrap();
        for rx in 0..files {
            let mut region =
                RegionFile::open_writable(dir.join(format!("r.{rx}.0.mca")), true).unwrap();
            for i in 0..chunks {
                let pos = horizon_core::position::ChunkPos::new(rx * 32 + i, i % 3);
                let root = NamedTag {
                    name: "".into(),
                    tag: old_chunk(pos.x, pos.z),
                };
                let record =
                    ChunkRecord::from_nbt(&root, Compression::Zlib, 7, Limits::default()).unwrap();
                region.write(pos, &record).unwrap();
            }
        }
    }

    /// Every region file's chunk versions, in file then chunk order.
    fn chunk_versions(dir: &Path) -> Vec<Vec<i32>> {
        files_with_suffix(dir, ".mca")
            .iter()
            .map(|file| {
                let mut region = RegionFile::open(file).unwrap();
                let positions: Vec<_> = region.chunks().map(|(p, _)| p).collect();
                positions
                    .into_iter()
                    .map(|pos| {
                        let record = region.read(pos).unwrap().unwrap();
                        data_version(&record.nbt(Limits::default()).unwrap().tag, -1)
                    })
                    .collect()
            })
            .collect()
    }

    fn fix_overworld_chunk(tag: &mut Tag) {
        crate::datafix::update_chunk(tag, "minecraft:overworld", Some("minecraft:noise"));
    }

    /// Region files upgraded on 4 workers come out byte for byte as on one,
    /// every chunk at the current version.
    #[test]
    fn parallel_region_upgrade_matches_one_thread() {
        let base = std::env::temp_dir().join(format!("horizon-par-regions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let mut outputs = Vec::new();
        for threads in [1, 4] {
            let dir = base.join(format!("threads-{threads}"));
            old_region_folder(&dir, 9, 20);
            // An empty file, as the game leaves for regions it never saved into.
            fs::write(dir.join("r.-1.5.mca"), []).unwrap();
            let progress = UpgradeProgress::new();
            let run = Progress {
                progress: &progress,
                cancelable: true,
            };
            upgrade_regions_with_threads(&dir, &run, Some(CURRENT), fix_overworld_chunk, threads)
                .unwrap();
            assert_eq!(progress.total_file_fix_stats().finished_operations(), 10);
            assert_eq!(
                fs::read(dir.join("r.-1.5.mca")).unwrap(),
                b"",
                "the empty file is left alone"
            );
            fs::remove_file(dir.join("r.-1.5.mca")).unwrap();
            assert_eq!(chunk_versions(&dir), vec![vec![CURRENT; 20]; 9]);
            assert!(!dir.join(".datafix").exists());
            let bytes: Vec<Vec<u8>> = files_with_suffix(&dir, ".mca")
                .iter()
                .map(|f| fs::read(f).unwrap())
                .collect();
            outputs.push(bytes);
        }
        assert!(outputs[0] == outputs[1], "the files differ");
        let _ = fs::remove_dir_all(&base);
    }

    /// A cancel during the region stage stops every worker after the file in
    /// hand: at most one file each is converted, the rest stay old.
    #[test]
    fn cancel_stops_every_region_worker_after_its_file() {
        let dir = std::env::temp_dir().join(format!("horizon-par-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        old_region_folder(&dir, 9, 20);
        let progress = UpgradeProgress::new();
        let run = Progress {
            progress: &progress,
            cancelable: true,
        };
        let error = upgrade_regions_with_threads(
            &dir,
            &run,
            Some(CURRENT),
            |tag: &mut Tag| {
                progress.set_canceled();
                fix_overworld_chunk(tag);
            },
            4,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        let versions = chunk_versions(&dir);
        let converted = versions.iter().filter(|f| **f == [CURRENT; 20]).count();
        let untouched = versions.iter().filter(|f| **f == [4000; 20]).count();
        assert!((1..=4).contains(&converted), "{converted} files converted");
        assert_eq!(converted + untouched, 9);
        assert_eq!(
            progress.total_file_fix_stats().finished_operations() as usize,
            converted
        );
        assert!(!dir.join(".datafix").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A fix that only marks the chunk upgraded, so the tests below see the
    /// moves and limits and not the chunk fixes.
    fn stamp(tag: &mut Tag) {
        set(tag, "DataVersion", Tag::Int(CURRENT));
    }

    fn upgrade(dir: &Path) -> io::Result<()> {
        upgrade_regions_with_threads(dir, &Progress::none(), Some(CURRENT), stamp, 1)
    }

    /// The chunk at `pos` at DataVersion `version`, carrying `payload`.
    fn chunk_record(
        pos: ChunkPos,
        version: i32,
        payload: Tag,
        compression: Compression,
    ) -> ChunkRecord {
        let mut tag = old_chunk(pos.x, pos.z);
        set(&mut tag, "DataVersion", Tag::Int(version));
        set(&mut tag, "Payload", payload);
        let root = NamedTag {
            name: "".into(),
            tag,
        };
        ChunkRecord::from_nbt(&root, compression, 7, Limits::saved()).unwrap()
    }

    /// That chunk written into `r.0.0.mca` in `dir`, created if missing.
    fn region_with_chunk(
        dir: &Path,
        pos: ChunkPos,
        version: i32,
        payload: Tag,
        compression: Compression,
    ) {
        fs::create_dir_all(dir).unwrap();
        let mut region = RegionFile::open_writable(dir.join("r.0.0.mca"), true).unwrap();
        region
            .write(pos, &chunk_record(pos, version, payload, compression))
            .unwrap();
    }

    fn read_chunk(dir: &Path, pos: ChunkPos) -> Tag {
        let mut region = RegionFile::open(dir.join("r.0.0.mca")).unwrap();
        let record = region.read(pos).unwrap().expect("the chunk reads");
        record.nbt(Limits::saved()).unwrap().tag
    }

    /// Stored uncompressed, a chunk with this payload takes more than 255
    /// sectors: its own `c.X.Z.mcc`.
    fn big(fill: u8) -> Tag {
        Tag::ByteArray(vec![fill; 1_100_000])
    }

    const EXTERNAL: &str = "c.3.4.mcc";

    /// `dir` with `r.0.0.mca` holding one oversized chunk at (3, 4) at
    /// `version` (its `c.3.4.mcc` beside it), then `stage` run on it.
    fn oversized_world(name: &str, version: i32, stage: impl FnOnce(&Path, &Path)) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-mcc-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        region_with_chunk(
            &dir,
            ChunkPos::new(3, 4),
            version,
            big(7),
            Compression::None,
        );
        assert!(dir.join(EXTERNAL).is_file(), "the fixture is external");
        let staging = dir.join(".datafix");
        fs::create_dir_all(&staging).unwrap();
        stage(&dir, &staging);
        dir
    }

    /// The upgrade succeeded: staging gone, nothing kept aside, the `.mcc`
    /// beside its region, the chunk upgraded with its content.
    fn assert_upgraded_beside(dir: &Path) {
        assert!(!dir.join(".datafix").exists(), "staging is gone");
        assert!(
            !dir.join(".datafix-leftover").exists(),
            "nothing kept aside"
        );
        assert!(dir.join(EXTERNAL).is_file(), "the .mcc is beside");
        let chunk = read_chunk(dir, ChunkPos::new(3, 4));
        assert_eq!(data_version(&chunk, -1), CURRENT);
        assert_eq!(chunk.get("Payload"), Some(&big(7)));
    }

    /// A chunk too large for its region file lives in its own `c.X.Z.mcc`
    /// beside it: the upgraded one replaces the old and staging goes.
    #[test]
    fn an_oversized_chunk_moves_out_with_its_region() {
        let dir = oversized_world("first", 4000, |_, _| {});
        upgrade(&dir).unwrap();
        assert_upgraded_beside(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A stopped run's copies in staging (its region file and the `.mcc` it
    /// names), the old files in place: the retry gets past them.
    #[test]
    fn a_stopped_runs_copies_do_not_block_the_retry() {
        let dir = oversized_world("copies", 4000, |_, staging| {
            // Stopped after `finish`: the staged region and the `.mcc` it
            // names, and a writer temporary of an earlier stop.
            let mut writer = RegionWriter::create(staging.join("r.0.0.mca")).unwrap();
            let pos = ChunkPos::new(3, 4);
            writer
                .write(pos, &chunk_record(pos, CURRENT, big(7), Compression::None))
                .unwrap();
            writer.finish().unwrap();
            fs::write(staging.join(".horizon-1-2.tmp"), b"partial").unwrap();
        });
        upgrade(&dir).unwrap();
        assert_upgraded_beside(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The upgrader before this fix moved an upgraded region out and left its
    /// chunk's `.mcc` in staging: it moves out beside the region.
    #[test]
    fn a_stranded_mcc_moves_out() {
        let dir = oversized_world("stranded", CURRENT, |dir, staging| {
            fs::rename(dir.join(EXTERNAL), staging.join(EXTERNAL)).unwrap();
        });
        upgrade(&dir).unwrap();
        assert_upgraded_beside(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// As above, while an older `.mcc` of the same name stayed beside the
    /// replaced region: the staged one is the right one.
    #[test]
    fn a_stranded_mcc_replaces_a_stale_one() {
        let dir = oversized_world("stale", CURRENT, |dir, staging| {
            fs::rename(dir.join(EXTERNAL), staging.join(EXTERNAL)).unwrap();
            let stale = chunk_record(ChunkPos::new(3, 4), 4000, big(6), Compression::None);
            fs::write(dir.join(EXTERNAL), &stale.compressed).unwrap();
            modified_at(&dir.join(EXTERNAL), 1_000);
            modified_at(&staging.join(EXTERNAL), 2_000);
            // The upgrader replaced the region: its inline chunks are at the target.
            region_with_chunk(
                dir,
                ChunkPos::new(1, 1),
                CURRENT,
                Tag::Byte(1),
                Compression::Zlib,
            );
        });
        upgrade(&dir).unwrap();
        assert_upgraded_beside(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// `path`'s modification time set to `seconds` after the epoch.
    fn modified_at(path: &Path, seconds: u64) {
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    /// As above, but the live `.mcc` was saved after the staged one (an
    /// older game played the world in between): it stays, and the staged one
    /// is kept aside.
    #[test]
    fn a_stranded_mcc_does_not_replace_a_newer_live_one() {
        let staged = upgraded_mcc_of(big(7));
        let newer = chunk_record(ChunkPos::new(3, 4), 4000, big(6), Compression::None).compressed;
        let dir = oversized_world("newer", CURRENT, |dir, staging| {
            fs::rename(dir.join(EXTERNAL), staging.join(EXTERNAL)).unwrap();
            fs::write(dir.join(EXTERNAL), &newer).unwrap();
            modified_at(&staging.join(EXTERNAL), 1_000);
            modified_at(&dir.join(EXTERNAL), 2_000);
            region_with_chunk(
                dir,
                ChunkPos::new(1, 1),
                CURRENT,
                Tag::Byte(1),
                Compression::Zlib,
            );
        });
        upgrade(&dir).unwrap();
        assert_eq!(
            fs::read(dir.join(".datafix-leftover").join(EXTERNAL)).unwrap(),
            staged
        );
        let chunk = read_chunk(&dir, ChunkPos::new(3, 4));
        assert_eq!(
            chunk.get("Payload"),
            Some(&big(6)),
            "the live file's content"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// `staged` left as `c.3.4.mcc` in staging beside a region whose chunk is
    /// stored as `compression` at an old version: the file is kept aside
    /// whole, not deleted, and the world still upgrades.
    fn assert_kept_aside(name: &str, compression: Compression, staged: &[u8]) {
        let dir = std::env::temp_dir().join(format!("horizon-aside-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pos = ChunkPos::new(3, 4);
        region_with_chunk(&dir, pos, 4000, big(7), compression);
        let staging = dir.join(".datafix");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join(EXTERNAL), staged).unwrap();

        upgrade(&dir).unwrap();
        assert!(!staging.exists());
        assert_eq!(
            fs::read(dir.join(".datafix-leftover").join(EXTERNAL)).unwrap(),
            staged,
            "kept aside whole"
        );
        let chunk = read_chunk(&dir, pos);
        assert_eq!(data_version(&chunk, -1), CURRENT);
        assert_eq!(chunk.get("Payload"), Some(&big(7)));
        let _ = fs::remove_dir_all(&dir);
    }

    fn upgraded_mcc_of(payload: Tag) -> Vec<u8> {
        chunk_record(ChunkPos::new(3, 4), CURRENT, payload, Compression::None).compressed
    }

    fn upgraded_mcc(compression: Compression) -> Vec<u8> {
        chunk_record(ChunkPos::new(3, 4), CURRENT, big(7), compression).compressed
    }

    /// The live stub is inline: a staged `.mcc` is not its file.
    #[test]
    fn a_staged_mcc_beside_an_inline_stub_is_kept_aside() {
        assert_kept_aside(
            "inline",
            Compression::Zlib,
            &upgraded_mcc(Compression::None),
        );
    }

    /// A run killed while writing it left half a `.mcc`.
    #[test]
    fn a_truncated_staged_mcc_is_kept_aside() {
        let upgraded = upgraded_mcc(Compression::None);
        assert_kept_aside(
            "truncated",
            Compression::None,
            &upgraded[..upgraded.len() / 2],
        );
    }

    /// The staged `.mcc` is in another compression than the stub names.
    #[test]
    fn a_staged_mcc_in_another_compression_is_kept_aside() {
        assert_kept_aside(
            "compression",
            Compression::None,
            &upgraded_mcc(Compression::Zlib),
        );
    }

    /// A staged `.mcc` at the target version beside a region the upgrader
    /// has not replaced, whose own `.mcc` is there: the live one may be newer
    /// (played or restored in between), so it stays and the staged one is
    /// kept aside.
    #[test]
    fn a_staged_mcc_beside_an_unreplaced_region_is_kept_aside() {
        let staged = upgraded_mcc_of(big(6));
        let dir = oversized_world("unreplaced", 4000, |dir, staging| {
            fs::write(staging.join(EXTERNAL), &staged).unwrap();
            // Inline chunks only at the old version: not replaced.
            region_with_chunk(
                dir,
                ChunkPos::new(1, 1),
                4000,
                Tag::Byte(1),
                Compression::Zlib,
            );
        });
        upgrade(&dir).unwrap();
        assert_eq!(
            fs::read(dir.join(".datafix-leftover").join(EXTERNAL)).unwrap(),
            staged
        );
        let chunk = read_chunk(&dir, ChunkPos::new(3, 4));
        assert_eq!(data_version(&chunk, -1), CURRENT);
        assert_eq!(
            chunk.get("Payload"),
            Some(&big(7)),
            "rebuilt from the live file"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A stranded `.mcc` at an older version (written by another build), its
    /// region's stub without a live file: installing overwrites nothing, and
    /// the retry upgrades it instead of the chunk being lost.
    #[test]
    fn an_older_stranded_mcc_without_a_live_one_is_installed() {
        let dir = oversized_world("older", 4000, |dir, staging| {
            fs::rename(dir.join(EXTERNAL), staging.join(EXTERNAL)).unwrap();
        });
        upgrade(&dir).unwrap();
        assert_upgraded_beside(&dir);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A stranded `.mcc` above the target version was not written for this
    /// upgrade: kept aside even where nothing would be overwritten.
    #[test]
    fn a_stranded_mcc_above_the_target_is_kept_aside() {
        let above = chunk_record(ChunkPos::new(3, 4), CURRENT + 1, big(6), Compression::None);
        let dir = oversized_world("above", 4000, |dir, staging| {
            fs::remove_file(dir.join(EXTERNAL)).unwrap();
            fs::write(staging.join(EXTERNAL), &above.compressed).unwrap();
        });
        upgrade(&dir).unwrap();
        assert_eq!(
            fs::read(dir.join(".datafix-leftover").join(EXTERNAL)).unwrap(),
            above.compressed
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A run stopped between its `.mcc` moves: one upgraded `.mcc` is beside
    /// the old region, the other still in staging with the staged region.
    #[test]
    fn a_stop_between_mcc_moves_is_finished_by_the_retry() {
        let dir = std::env::temp_dir().join(format!("horizon-mcc-between-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (a, b) = (ChunkPos::new(3, 4), ChunkPos::new(5, 6));
        for pos in [a, b] {
            region_with_chunk(&dir, pos, 4000, big(7), Compression::None);
        }
        let staging = dir.join(".datafix");
        let mut writer = RegionWriter::create(staging.join("r.0.0.mca")).unwrap();
        for pos in [a, b] {
            writer
                .write(pos, &chunk_record(pos, CURRENT, big(7), Compression::None))
                .unwrap();
        }
        writer.finish().unwrap();
        fs::rename(staging.join("c.3.4.mcc"), dir.join("c.3.4.mcc")).unwrap();

        upgrade(&dir).unwrap();
        assert!(!staging.exists());
        assert!(!dir.join(".datafix-leftover").exists());
        for pos in [a, b] {
            let chunk = read_chunk(&dir, pos);
            assert_eq!(data_version(&chunk, -1), CURRENT, "{pos:?}");
            assert_eq!(chunk.get("Payload"), Some(&big(7)), "{pos:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A staged region file that does not refer to a staged `.mcc` does not
    /// make it a copy: it is kept aside, not deleted.
    #[test]
    fn a_staged_mcc_its_staged_region_does_not_name_is_kept_aside() {
        let staged = upgraded_mcc_of(big(6));
        let dir = oversized_world("unnamed", 4000, |_, staging| {
            let mut writer = RegionWriter::create(staging.join("r.0.0.mca")).unwrap();
            let pos = ChunkPos::new(1, 1);
            writer
                .write(
                    pos,
                    &chunk_record(pos, CURRENT, Tag::Byte(1), Compression::Zlib),
                )
                .unwrap();
            writer.finish().unwrap();
            fs::write(staging.join(EXTERNAL), &staged).unwrap();
        });
        upgrade(&dir).unwrap();
        assert_eq!(
            fs::read(dir.join(".datafix-leftover").join(EXTERNAL)).unwrap(),
            staged
        );
        let chunk = read_chunk(&dir, ChunkPos::new(3, 4));
        assert_eq!(chunk.get("Payload"), Some(&big(7)));
        let _ = fs::remove_dir_all(&dir);
    }

    /// A file in staging the upgrader does not write stays, and the error
    /// that the folder cannot go names it.
    #[test]
    fn a_foreign_file_in_staging_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("horizon-mcc-foreign-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pos = ChunkPos::new(1, 2);
        region_with_chunk(&dir, pos, 4000, Tag::Byte(1), Compression::Zlib);
        let staging = dir.join(".datafix");
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("notes.txt"), b"mine").unwrap();

        let error = upgrade(&dir).unwrap_err();
        assert!(error.to_string().contains("notes.txt"), "{error}");
        assert_eq!(fs::read(staging.join("notes.txt")).unwrap(), b"mine");
        assert_eq!(data_version(&read_chunk(&dir, pos), -1), CURRENT);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A staging folder that is a link is refused, and what it points to is
    /// not touched.
    #[cfg(unix)]
    #[test]
    fn a_linked_staging_folder_is_refused() {
        let base = std::env::temp_dir().join(format!("horizon-mcc-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let dir = base.join("region");
        let elsewhere = base.join("elsewhere");
        let pos = ChunkPos::new(1, 2);
        region_with_chunk(&dir, pos, 4000, Tag::Byte(1), Compression::Zlib);
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("r.0.0.mca"), b"keep").unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join(".datafix")).unwrap();

        assert!(upgrade(&dir).is_err());
        assert_eq!(fs::read(elsewhere.join("r.0.0.mca")).unwrap(), b"keep");
        assert_eq!(data_version(&read_chunk(&dir, pos), -1), 4000);
        let _ = fs::remove_dir_all(&base);
    }

    /// A `.dat` write that fails leaves the previous file whole: here its
    /// temporary file cannot be created.
    #[test]
    fn a_failed_dat_write_keeps_the_old_file() {
        let dir = std::env::temp_dir().join(format!("horizon-dat-replace-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("level.dat");
        let old = Tag::compound([("DataVersion", Tag::Int(4000))]);
        write_dat(&file, &old).unwrap();
        fs::create_dir(dir.join("level.dat.horizon-new")).unwrap();

        let new = Tag::compound([("DataVersion", Tag::Int(CURRENT))]);
        assert!(write_dat(&file, &new).is_err());
        assert_eq!(read_dat(&file).unwrap(), Some(old));
        let _ = fs::remove_dir_all(&dir);
    }

    /// A temporary left by a stopped `.dat` write is replaced.
    #[test]
    fn a_stale_dat_temporary_is_replaced() {
        let dir = std::env::temp_dir().join(format!("horizon-dat-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("level.dat");
        write_dat(&file, &Tag::compound([("DataVersion", Tag::Int(4000))])).unwrap();
        fs::write(dir.join("level.dat.horizon-new"), b"stopped").unwrap();

        let new = Tag::compound([("DataVersion", Tag::Int(CURRENT))]);
        write_dat(&file, &new).unwrap();
        assert_eq!(read_dat(&file).unwrap(), Some(new));
        assert!(!dir.join("level.dat.horizon-new").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// A link at the temporary's name is not written through: the write
    /// fails, the old file and the link's target stay as they were.
    #[cfg(unix)]
    #[test]
    fn a_linked_dat_temporary_is_refused() {
        let dir = std::env::temp_dir().join(format!("horizon-dat-link-tmp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("level.dat");
        let old = Tag::compound([("DataVersion", Tag::Int(4000))]);
        write_dat(&file, &old).unwrap();
        fs::write(dir.join("elsewhere"), b"keep").unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("level.dat.horizon-new"))
            .unwrap();

        assert!(write_dat(&file, &Tag::compound([("DataVersion", Tag::Int(CURRENT))])).is_err());
        assert_eq!(read_dat(&file).unwrap(), Some(old));
        assert_eq!(fs::read(dir.join("elsewhere")).unwrap(), b"keep");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A linked `.dat` is written where the link points, as before.
    #[cfg(unix)]
    #[test]
    fn a_linked_dat_is_written_through() {
        let dir = std::env::temp_dir().join(format!("horizon-dat-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("real")).unwrap();
        let real = dir.join("real/level.dat");
        write_dat(&real, &Tag::compound([("DataVersion", Tag::Int(4000))])).unwrap();
        let link = dir.join("level.dat");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let new = Tag::compound([("DataVersion", Tag::Int(CURRENT))]);
        write_dat(&link, &new).unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read_dat(&real).unwrap(), Some(new));
        let _ = fs::remove_dir_all(&dir);
    }

    /// The upgrader reads chunks and `.dat` files as the game loads them: past
    /// the default node cap, within the saved-data limits.
    #[test]
    fn files_past_the_default_node_cap_upgrade() {
        let base = std::env::temp_dir().join(format!("horizon-big-nbt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let nodes = Limits::default().max_nodes + 1;
        let payload = || Tag::List(vec![Tag::Byte(1); nodes]);
        let whole =
            |tag: &Tag| matches!(tag.get("Payload"), Some(Tag::List(l)) if l.len() == nodes);

        let region = base.join("region");
        let pos = ChunkPos::new(1, 2);
        region_with_chunk(&region, pos, 4000, payload(), Compression::Zlib);
        upgrade(&region).unwrap();
        let chunk = read_chunk(&region, pos);
        assert_eq!(data_version(&chunk, -1), CURRENT);
        assert!(whole(&chunk));
        drop(chunk);

        let file = base.join("data/minecraft/big.dat");
        write_dat(&file, &Tag::compound([("Payload", payload())])).unwrap();
        assert!(whole(&read_dat(&file).unwrap().unwrap()));
        assert!(whole(&read_saved_dat(&file).unwrap().unwrap()));
        let _ = fs::remove_dir_all(&base);
    }
}
