//! World persistence in the vanilla 26.3 layout, readable by both Horizon and
//! the vanilla game:
//! - `level.dat` (`Data`: name, game type, time, spawn, difficulty, version)
//!   plus the saved-data files `data/minecraft/{weather,world_clocks,
//!   world_gen_settings,game_rules}.dat`;
//! - the player in `players/data/<uuid>.dat` (`singleplayer_uuid`);
//! - per dimension `dimensions/minecraft/<name>/region/r.X.Z.mca` (chunks:
//!   paletted sections with light, heightmaps, block entities, scheduled
//!   block/fluid ticks) and `entities/r.X.Z.mca` (entities, `entity_nbt`).
//!
//! Data Horizon does not simulate (structure references, unknown block
//! entities and entities, player fields such as the ender chest or effects,
//! game rules) is kept from the loaded files and written back unchanged.
//! Worlds saved by older Horizon builds (`region/`, `DIM-1/`, `Player` in
//! `level.dat`) are migrated on open.
use crate::mod_chunk_tickets::{ModTicket, ModTickets, TicketOwner};
mod backpressure;
use backpressure::compress_records;
pub mod biome_regions;
pub mod block_entity_encode;
mod block_entity_order;
mod far_regions;
pub mod level_mods;
mod respawn;
mod section_nbt;
mod writes;
use writes::{Outstanding, Writer};

use crate::{
    Result,
    beehive::BeehiveExt,
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, ColumnLayout, Light, Section, block_strategy},
    mobs::{spawner::SpawnerExt, trial_spawner::TrialSpawnerExt},
    streaming::ChunkGenerator,
};
use horizon_core::{
    paletted::{PalettedContainer, SavedPaletteBudget},
    position::{BlockPos, ChunkPos},
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use horizon_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound};
use horizon_world::worldgen::blending::BlendingData;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
};
use {
    horizon_core::{
        nbt,
        nbt::{Compound, JavaString, Limits, NamedTag, Tag},
    },
    horizon_world::{
        DATA_VERSION,
        chunk::{BlockState, block_entity_pos},
        chunk_status::ChunkStatus,
        compression::Compression,
        heightmap::{Heightmap, HeightmapChunk, HeightmapState, HeightmapType},
        region::{ChunkRecord, RegionFile},
        registry::{BlockRegistry, BlockStateId},
        worldgen::terrain::TerrainBlockStates,
    },
};

pub const VERSION_NAME: &str = horizon_core::VANILLA_VERSION;

#[cfg(test)]
mod compression_bench;
mod compression_pool;
#[cfg(test)]
mod compression_tests;
mod gateway;
#[cfg(test)]
mod loot_tests;

/// Snapshot live block entities and custom names for a save batch without
/// mutating the world. Each map is visited once, retaining its iteration order
/// within each chunk (including repeated chunk coordinates in the input).
/// An empty batch skips both maps; one chunk uses the direct filter path.
pub fn chunks_with_block_entities<'a, N: std::hash::BuildHasher>(
    chunks: impl IntoIterator<Item = &'a Arc<Chunk>>,
    entities: &HashMap<(i32, i32, i32), crate::containers::BlockEntity>,
    names: &HashMap<(i32, i32, i32), Tag, N>,
) -> Vec<Chunk> {
    let chunks: Vec<_> = chunks.into_iter().collect();
    if chunks.is_empty() {
        return Vec::new();
    }
    if chunks.len() == 1 {
        let mut c = (**chunks[0]).clone();
        c.block_entities = entities
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        c.block_names = names
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, n)| (*p, n.clone()))
            .collect();
        return vec![c];
    }
    // Index only requested chunks, borrowing their entries so unsaved entities
    // are never cloned. Clone one chunk at a time for inventory cache locality.
    let mut grouped = crate::fast_hash::FxHashMap::<_, (Vec<_>, Vec<_>)>::default();
    grouped.reserve(chunks.len());
    for c in &chunks {
        grouped.entry((c.x, c.z)).or_default();
    }
    for (p, e) in entities {
        if let Some((entries, _)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, e));
        }
    }
    for (p, n) in names {
        if let Some((_, entries)) = grouped.get_mut(&(p.0 >> 4, p.2 >> 4)) {
            entries.push((p, n));
        }
    }
    chunks
        .into_iter()
        .map(|chunk| {
            let mut c = (**chunk).clone();
            let (entities, names) = &grouped[&(c.x, c.z)];
            c.block_entities = entities.iter().map(|(p, e)| (**p, (*e).clone())).collect();
            c.block_names = names.iter().map(|(p, n)| (**p, (*n).clone())).collect();
            c
        })
        .collect()
}

use crate::block_entity_nbt::component as nbt_component;
use horizon_client_core::nbt_value::string;
pub use horizon_client_core::nbt_value::{float, number, text};
/// Insert or replace a key of a compound tag.
pub(crate) fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.into(), value);
    }
}
fn compound_or_empty(tag: Option<Tag>) -> Tag {
    match tag {
        Some(t @ Tag::Compound(_)) => t,
        _ => Tag::Compound(Compound::new()),
    }
}

/// `dimensions/<namespace>/<path>` inside a world folder. An id that is no safe folder name
/// (`test:../minecraft/overworld`, an absolute path) never leaves the world: it gets a folder
/// of its own under `dimensions/%invalid/`, named by its bytes, which no valid id can reach.
pub fn dimension_dir(world: &Path, dimension: &str) -> PathBuf {
    let id = if dimension.contains(':') {
        dimension.to_owned()
    } else {
        format!("minecraft:{dimension}")
    };
    let dimensions = world.join("dimensions");
    match id.split_once(':') {
        Some((namespace, path))
            if horizon_core::level_id::valid_level_namespace(namespace)
                && horizon_core::level_id::valid_level_path(path) =>
        {
            dimensions.join(namespace).join(path)
        }
        _ => {
            let hex: String = id.bytes().map(|b| format!("{b:02x}")).collect();
            dimensions.join("%invalid").join(hex)
        }
    }
}
/// `DirectoryLock`: `session.lock` holds "\u{2603}" (UTF-16BE) under an exclusive
/// file lock for as long as the world is open, so two instances cannot share it.
pub struct SessionLock(fs::File);
impl Drop for SessionLock {
    /// `DirectoryLock.close`: release the lock, then close the file. Closing
    /// alone keeps the lock while any copy of the descriptor lives on, such as
    /// the one a process spawned on another thread holds until its `exec`.
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
impl SessionLock {
    pub fn acquire(world: &Path) -> Result<Self> {
        use std::io::Write;
        fs::create_dir_all(world)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(world.join("session.lock"))?;
        file.try_lock()
            .map_err(|_| format!("{} is already open in another instance", world.display()))?;
        file.set_len(0)?;
        file.write_all(&[0x26, 0x03])?;
        file.flush()?;
        Ok(Self(file))
    }
    /// `DirectoryLock.isLocked`: another copy holds the world's `session.lock`.
    /// A world without the file is not locked; one whose file may not be opened
    /// is.
    pub fn is_locked(world: &Path) -> bool {
        let file = match fs::OpenOptions::new()
            .write(true)
            .open(world.join("session.lock"))
        {
            Ok(file) => file,
            Err(e) => return e.kind() == std::io::ErrorKind::PermissionDenied,
        };
        match file.try_lock() {
            Err(fs::TryLockError::WouldBlock) => true,
            Err(_) => false,
            // Take the probe's own lock back at once, as `SessionLock` does on drop.
            Ok(()) => {
                let _ = file.unlock();
                false
            }
        }
    }
}
/// Bring a world last saved by an older game version to the current format, as
/// 26.3 does when it opens one: the file fixes and level.dat first
/// (`FileFixerUpper`), then the DataFixers upgrades of every chunk, entity chunk,
/// POI chunk, player, saved data, advancements and statistics file. Vanilla runs
/// the latter as it reads each file; Horizon runs them all on opening, which reads
/// the same data, and region records still older in a world whose level.dat is
/// current are upgraded as they are read (`upgrade_record`).
pub fn upgrade_older_world(world: &Path) -> Result<()> {
    upgrade_older_world_with(
        world,
        &horizon_world::datafix::progress::UpgradeProgress::new(),
    )
}
/// [`upgrade_older_world`] reporting to `progress`, which `FileFixerProgressScreen` reads.
pub fn upgrade_older_world_with(
    world: &Path,
    progress: &horizon_world::datafix::progress::UpgradeProgress,
) -> Result<()> {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return Ok(());
    };
    let version = root.get("Data").map_or(i64::from(DATA_VERSION), |d| {
        number(d.get("DataVersion")).unwrap_or(-1) // saves of 1.8 and before have none
    });
    if version >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    if version < i64::from(horizon_world::datafix::OLDEST_SUPPORTED) {
        return Err(format!(
            "{} was saved with DataVersion {version}; worlds older than DataVersion {} \
             cannot be upgraded yet (tools/upgrade_world.py runs vanilla's own upgrader)",
            world.display(),
            horizon_world::datafix::OLDEST_SUPPORTED
        )
        .into());
    }
    horizon_world::datafix::files::upgrade_world_with(world, progress)
        .map_err(|e| format!("upgrading {}: {e}", world.display()))?;
    Ok(())
}
/// Whether opening the world runs file fixes, which vanilla asks for a backup
/// first (`BackupStatus.FILE_FIXING_REQUIRED`). Saves of 1.8 and before have no
/// DataVersion, read as -1.
pub fn needs_file_fixes(world: &Path) -> bool {
    let Some(root) = read_dat(&world.join("level.dat")) else {
        return false;
    };
    let Some(data) = root.get("Data") else {
        return false;
    };
    let version = number(data.get("DataVersion")).unwrap_or(-1);
    i32::try_from(version).is_ok_and(horizon_world::datafix::files::requires_file_fixing)
}
/// `LevelVersion.parse`: the saved game version's name, this build's when the `Version`
/// compound has no `Name`, and empty without a `Version` (`getWorldVersionName` then
/// shows `selectWorld.versionUnknown`).
fn saved_version_name(version: Option<&Tag>) -> String {
    match version {
        Some(v) => text(v.get("Name")).unwrap_or_else(|| VERSION_NAME.to_string()),
        None => String::new(),
    }
}
/// `LevelSummary.isCompatible` (`DataVersion.isCompatible`: the saved series must be
/// the current `main` one). None when the world opens; otherwise the saved game
/// version's name, empty when it has none (`getWorldVersionName`).
pub fn incompatible_version(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let series = text(version.and_then(|v| v.get("Series"))).unwrap_or_else(|| "main".into());
    if series == "main" {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelSummary.backupStatus` DOWNGRADE: the saved game version's name (empty when it
/// has none) when the world's `Version.Id` is newer than this build's `DataVersion`
/// (`LevelVersion.parse`: no `Version` reads as 0, no `Id` as the current one).
pub fn downgraded_from(world: &Path) -> Option<String> {
    let root = read_dat(&world.join("level.dat"))?;
    let version = root.get("Data")?.get("Version");
    let id = version.map_or(0, |v| {
        number(v.get("Id")).unwrap_or_else(|| i64::from(DATA_VERSION))
    });
    if id <= i64::from(DATA_VERSION) {
        return None;
    }
    Some(saved_version_name(version))
}
/// `LevelStorageAccess.checkForLowDiskSpace`: under 64 MiB usable.
pub const LOW_DISK_SPACE: u64 = 67_108_864;

/// The `PlayerData::inventory` slots of `EquipmentSlot.BODY` and `SADDLE`.
/// Vanilla saves them in the `equipment` compound with the armor and off hand.
pub const BODY_SLOT: u8 = 151;
pub const SADDLE_SLOT: u8 = 152;

/// The `EquipmentSlot` ordinals a player saves in `equipment`, with the
/// `PlayerData::inventory` slot each is held in: armor 100 (feet) to 103
/// (head), the off hand 150, body 151, saddle 152. The main hand is the
/// selected inventory slot and never in `equipment`.
const PLAYER_EQUIPMENT_SLOTS: [(usize, u8); 7] = [
    (horizon_items::equipment::FEET, 100),
    (horizon_items::equipment::LEGS, 101),
    (horizon_items::equipment::CHEST, 102),
    (horizon_items::equipment::HEAD, 103),
    (horizon_items::equipment::OFFHAND, 150),
    (horizon_items::equipment::BODY, BODY_SLOT),
    (horizon_items::equipment::SADDLE, SADDLE_SLOT),
];

/// The `EquipmentSlot` ordinal a saved slot number stands for, for the
/// slots a player holder keeps in its `Equipment`.
pub fn equipment_slot(slot: u8) -> Option<usize> {
    match slot {
        BODY_SLOT => Some(horizon_items::equipment::BODY),
        SADDLE_SLOT => Some(horizon_items::equipment::SADDLE),
        _ => None,
    }
}
/// The saved slot number of a held `EquipmentSlot` ordinal (the inverse).
pub fn saved_slot(equipment_slot: usize) -> Option<u8> {
    match equipment_slot {
        horizon_items::equipment::BODY => Some(BODY_SLOT),
        horizon_items::equipment::SADDLE => Some(SADDLE_SLOT),
        _ => None,
    }
}
/// `LevelStorageAccess.estimateDiskSpace`: the usable bytes of the disk holding `dir`
/// (`FileStore.getUsableSpace`), `u64::MAX` when it cannot be asked.
pub fn estimate_disk_space(dir: &Path) -> u64 {
    fs4::available_space(dir).unwrap_or(u64::MAX)
}
/// `LevelStorageAccess.checkForLowDiskSpace`.
pub fn low_disk_space(dir: &Path) -> bool {
    estimate_disk_space(dir) < LOW_DISK_SPACE
}
/// `WorldOptions.isOldCustomizedWorld`: the generator settings keep `legacy_custom_options`.
pub fn is_old_customized_world(world: &Path) -> bool {
    world_gen_settings(world).is_some_and(|s| text(s.get("legacy_custom_options")).is_some())
}
/// A dimension's `LevelStem` field read as a resource location (an inline definition is not one).
fn stem_id(tag: Option<&Tag>) -> Option<String> {
    let id = text(tag)?;
    Some(if id.contains(':') {
        id
    } else {
        format!("minecraft:{id}")
    })
}
/// `WorldDimensions.isVanillaLike`: whether a saved dimension is the stable Overworld, Nether or
/// End (`isStableOverworld`, `isStableNether`, `isStableEnd`); any other key never is.
fn dimension_is_vanilla_like(key: &str, dimension: &Tag) -> bool {
    let kind = stem_id(dimension.get("type"));
    let generator = dimension.get("generator");
    let source = generator.and_then(|g| g.get("biome_source"));
    let source_kind = source.and_then(|s| stem_id(s.get("type")));
    // MultiNoiseBiomeSource.stable(preset): a reference to the preset, not its parameters.
    let multi_noise_preset = |preset: &str| {
        source_kind.as_deref() == Some("minecraft:multi_noise")
            && stem_id(source.and_then(|s| s.get("preset"))).as_deref() == Some(preset)
    };
    // NoiseBasedChunkGenerator.stable(settings): the generator uses the settings by key.
    let noise_settings = |settings: &str| {
        generator.is_some_and(|g| stem_id(g.get("type")).as_deref() == Some("minecraft:noise"))
            && stem_id(generator.and_then(|g| g.get("settings"))).as_deref() == Some(settings)
    };
    match key {
        "minecraft:overworld" => {
            matches!(
                kind.as_deref(),
                Some("minecraft:overworld" | "minecraft:overworld_caves")
            ) && (source_kind.as_deref() != Some("minecraft:multi_noise")
                || multi_noise_preset("minecraft:overworld"))
        }
        "minecraft:the_nether" => {
            kind.as_deref() == Some("minecraft:the_nether")
                && noise_settings("minecraft:nether")
                && multi_noise_preset("minecraft:nether")
        }
        "minecraft:the_end" => {
            kind.as_deref() == Some("minecraft:the_end")
                && noise_settings("minecraft:end")
                && source_kind.as_deref() == Some("minecraft:the_end")
        }
        _ => false,
    }
}
/// A data pack's JSON as the NBT `dimension_is_vanilla_like` reads (what a codec gets from
/// either one is the same data).
pub(crate) fn json_tag(value: &serde_json::Value) -> Tag {
    use serde_json::Value;
    match value {
        Value::Null => Tag::compound(std::iter::empty::<(&str, Tag)>()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => n.as_i64().map_or_else(
            || Tag::Double(n.as_f64().unwrap_or_default().to_bits()),
            Tag::Long,
        ),
        Value::String(s) => Tag::String(s.as_str().into()),
        Value::Array(items) => Tag::List(items.iter().map(json_tag).collect()),
        Value::Object(map) => Tag::compound(map.iter().map(|(k, v)| (k.as_str(), json_tag(v)))),
    }
}
/// The level stems the world's enabled data packs define, as `WorldOpenFlows.loadWorldStem`'s
/// `datapackDimensions`.
fn datapack_dimensions(world: &Path) -> Vec<(String, Tag)> {
    let packs = crate::datapack::discover(world);
    let enabled_features = Level::read(world)
        .map(|l| l.enabled_features)
        .unwrap_or_default();
    let enabled = crate::datapack::enabled_ids(world, &packs, &enabled_features);
    let enabled: Vec<&crate::datapack::DataPack> = enabled
        .iter()
        .filter_map(|id| packs.iter().find(|p| &p.id == id))
        .collect();
    crate::datapack::dimension_stems(&enabled)
        .into_iter()
        .map(|(id, json)| (id, json_tag(&json)))
        .collect()
}
/// `data.worldGenSettingsLifecycle() != Lifecycle.stable()` (`WorldDimensions.bake`): the known
/// dimensions (the saved ones and the enabled data packs', a pack's replacing the saved one of
/// its id) lack one of the three builtin ones, or have one that is not the vanilla one.
pub fn world_gen_unstable(world: &Path) -> bool {
    let saved = world_gen_settings(world)
        .as_ref()
        .and_then(|s| s.get("dimensions"))
        .cloned();
    let stems = datapack_dimensions(world);
    if saved.is_none() && stems.is_empty() {
        return false;
    }
    let mut dimensions: std::collections::BTreeMap<String, Tag> = match saved {
        Some(Tag::Compound(saved)) => saved
            .into_iter()
            .map(|(key, dimension)| (key.to_string_lossy(), dimension))
            .collect(),
        _ => std::collections::BTreeMap::new(),
    };
    // `baseDimensions.getOptional(key).or(saved)`.
    dimensions.extend(stems);
    let builtin = [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ];
    builtin.iter().any(|key| !dimensions.contains_key(*key))
        || dimensions
            .iter()
            .any(|(key, dimension)| !dimension_is_vanilla_like(key, dimension))
}
/// Move a world saved by an older Horizon build (pre-26.3 layout with a
/// DataVersion that keeps vanilla from upgrading it) into the 26.3 layout.
pub fn migrate_legacy(world: &Path) -> Result<()> {
    for (old, dimension) in [
        ("", "minecraft:overworld"),
        ("DIM-1", "minecraft:the_nether"),
        ("DIM1", "minecraft:the_end"),
    ] {
        let from = if old.is_empty() {
            world.to_owned()
        } else {
            world.join(old)
        };
        let to = dimension_dir(world, dimension);
        for folder in ["region", "entities", "poi"] {
            let (src, dst) = (from.join(folder), to.join(folder));
            if src.is_dir() && !dst.exists() {
                fs::create_dir_all(&to)?;
                fs::rename(&src, &dst)?;
            }
        }
        if !old.is_empty() && from.is_dir() && fs::read_dir(&from)?.next().is_none() {
            let _ = fs::remove_dir(&from);
        }
    }
    Ok(())
}

/// Registry lookups shared by chunk encoding and decoding.
pub struct SaveTables {
    /// Block registry IDs (`BlockId`, scheduled tick types) in registry order.
    block_names: Vec<String>,
    block_ids: HashMap<String, u16>,
    /// Default state of each state's block, to write vanilla's short palette form.
    default_state: Vec<bool>,
    block_entity: Vec<bool>,
    heightmaps: crate::assets::Properties,
    air: u32,
    /// The POI type and its max tickets of each state (`PoiTypes`).
    poi: Vec<Option<(&'static str, i32)>>,
}
impl SaveTables {
    pub fn new(assets: &crate::assets::Assets) -> Self {
        let registry = assets.registry;
        let mut block_names: Vec<String> = Vec::new();
        let mut default_state = Vec::with_capacity(registry.len());
        for id in 0..registry.len() {
            let state = registry
                .state(BlockStateId(id as u32))
                .expect("registry state");
            if block_names.last() != Some(&state.name) {
                block_names.push(state.name.clone());
            }
            default_state
                .push(registry.default_state(&state.name) == Some(BlockStateId(id as u32)));
        }
        let block_ids = block_names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as u16))
            .collect();
        Self {
            block_names,
            block_ids,
            default_state,
            block_entity: Vec::new(),
            heightmaps: assets.properties.clone(),
            air: registry.default_state("minecraft:air").map_or(0, |s| s.0),
            poi: (0..registry.len())
                .map(|id| {
                    registry
                        .state(BlockStateId(id as u32))
                        .and_then(horizon_world::poi::poi_type)
                })
                .collect(),
        }
    }
    /// Tables with only the heightmaps' state facts, for tests.
    #[cfg(test)]
    pub(crate) fn for_heightmaps(heightmaps: crate::assets::Properties) -> Self {
        Self {
            block_names: Vec::new(),
            block_ids: HashMap::new(),
            default_state: Vec::new(),
            block_entity: Vec::new(),
            heightmaps,
            air: 0,
            poi: Vec::new(),
        }
    }
    /// Which states carry a block entity (`BlockFacts`), to drop entries for
    /// unknown block entities whose block is gone.
    pub fn with_block_entities(mut self, facts: &crate::blocks::BlockFacts) -> Self {
        self.block_entity = facts.states.iter().map(|s| s.block_entity).collect();
        self
    }
}

pub use crate::chunk::{ChunkExtras, PendingContainerLoot, SavedProto};

fn tick_list(ticks: &[SavedTick], name: impl Fn(u32) -> Option<String>) -> Tag {
    Tag::List(
        ticks
            .iter()
            .filter_map(|t| {
                Some(Tag::compound([
                    ("i", string(&name(t.kind.0)?)),
                    ("x", Tag::Int(t.pos.x)),
                    ("y", Tag::Int(t.pos.y)),
                    ("z", Tag::Int(t.pos.z)),
                    ("t", Tag::Int(t.delay)),
                    ("p", Tag::Int(t.priority as i32)),
                ]))
            })
            .collect(),
    )
}
fn read_ticks(
    tag: Option<&Tag>,
    chunk: (i32, i32),
    id: impl Fn(&str) -> Option<u32>,
) -> Vec<SavedTick> {
    let Some(Tag::List(list)) = tag else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|t| {
            let kind = id(&text(t.get("i"))?)?;
            let pos = BlockPos::new(
                number(t.get("x"))? as i32,
                number(t.get("y"))? as i32,
                number(t.get("z"))? as i32,
            );
            // SavedTick.filterTickListForChunk
            if (pos.x >> 4, pos.z >> 4) != chunk {
                return None;
            }
            Some(SavedTick {
                kind: TickTypeId(kind),
                pos,
                delay: number(t.get("t")).unwrap_or(0) as i32,
                priority: TickPriority::by_value(number(t.get("p")).unwrap_or(0) as i32),
            })
        })
        .collect()
}
fn fluid_name(kind: u32) -> Option<String> {
    use horizon_world::fluid::FluidKind;
    match FluidKind::from_tick_id(kind)? {
        FluidKind::Empty => None,
        fluid => Some(fluid.name().to_owned()),
    }
}
fn fluid_id(name: &str) -> Option<u32> {
    use horizon_world::fluid::FluidKind;
    FluidKind::from_name(name)
        .filter(|kind| *kind != FluidKind::Empty)
        .map(FluidKind::tick_id)
}

pub use horizon_items::item_nbt::{
    decode_items, encode_item, encode_items, item_from_nbt, item_to_nbt,
};
/// Whether a mod block entity is temporarily out of the world for its tick.
pub fn block_entity_placeholder(entity: &crate::containers::BlockEntity) -> bool {
    matches!(entity, crate::containers::BlockEntity::Mod(m) if m.id().is_empty())
}

/// The block entity type ID for the block it sits in (a chest-like container
/// is a chest, trapped chest or barrel).
pub fn block_entity_id<'a>(e: &'a crate::containers::BlockEntity, block: &'a str) -> &'a str {
    let block = crate::block_entity_types::factory_block(block);
    match (e, block) {
        (
            crate::containers::BlockEntity::Chest(_),
            "minecraft:trapped_chest"
            | "minecraft:barrel"
            | "minecraft:dropper"
            | "minecraft:ender_chest",
        ) => block,
        (crate::containers::BlockEntity::Chest(_), b) if b.ends_with("shulker_box") => {
            "minecraft:shulker_box"
        }
        _ => e.id(),
    }
}
/// `block_entity_nbt::overlay`, then a container that now holds items loses its
/// loot table (vanilla clears it when the loot is generated into the container).
fn merge_block_entity(original: &Tag, loaded: &Tag, current: &Tag) -> Tag {
    let mut merged = crate::block_entity_nbt::overlay(original, loaded, current);
    let has_items = matches!(current.get("Items"), Some(Tag::List(items)) if !items.is_empty());
    if has_items && let Tag::Compound(map) = &mut merged {
        map.remove(&"LootTable".into());
        map.remove(&"LootTableSeed".into());
    }
    merged
}
/// `BlockDataAccessor.getData` (`BlockEntity.saveWithFullMetadata`): the tag
/// `data get block` shows and `data merge block` changes, without the
/// chunk-only `keepPacked`, and with the fields Horizon keeps outside its
/// model: a container's `lock` (`LockCode`), from the tag it was loaded or set
/// with.
pub fn block_entity_data(
    p: (i32, i32, i32),
    e: &crate::containers::BlockEntity,
    block: &str,
    lock: Option<&Tag>,
) -> Tag {
    let mut tag = block_entity_encode::encode(p, e, block);
    if let Tag::Compound(map) = &mut tag {
        map.remove(&"keepPacked".into());
        if let Some(lock) = lock {
            map.insert("lock".into(), lock.clone());
        }
    }
    tag
}
/// `BannerBlockEntity.loadAdditional` (and the `components` a block entity keeps).
pub fn decode_banner(tag: &Tag) -> crate::containers::Banner {
    crate::containers::Banner {
        layers: crate::banner::layers_from_nbt(tag.get("patterns")),
        // `parseCustomNameSafe`: the component codec keeps a name's style.
        name: tag.get("CustomName").and_then(nbt_component),
        components: match tag.get("components") {
            Some(Tag::Compound(c)) => c.clone(),
            _ => Compound::new(),
        },
    }
}
/// `TheEndGatewayBlockEntity.loadAdditional`.
pub fn decode_end_gateway(tag: &Tag) -> crate::containers::EndGateway {
    crate::containers::EndGateway {
        age: number(tag.get("Age")).unwrap_or(0),
        exit_portal: tag
            .get("exit_portal")
            .and_then(crate::block_entity_nbt::gateway_exit),
        exact_teleport: crate::entity_nbt::flag(tag, "ExactTeleport"),
    }
}
/// A mod's block entity: its compound without `id` and the position, and
/// its `Items` as slots (`ContainerHelper.loadAllItems`; the game sizes them
/// to the block's container when it uses them). `Items` that are not an item
/// list saved exactly so stay the mod's own.
fn decode_mod_block_entity(id: String, tag: &Tag) -> Option<crate::containers::ModBlockEntity> {
    let Tag::Compound(map) = tag else {
        return None;
    };
    let mut nbt = map.clone();
    for key in ["id", "x", "y", "z"] {
        nbt.remove(&key.into());
    }
    // `Slot` is a byte: 256 slots at most.
    let mut items = decode_items(nbt.get(&"Items".into()), 256);
    if nbt.get(&"Items".into()) == Some(&encode_items(&items)) {
        nbt.remove(&"Items".into());
        while items.last().is_some_and(Option::is_none) {
            items.pop();
        }
    } else {
        items.clear();
    }
    Some(crate::containers::ModBlockEntity::new(
        id,
        crate::containers::ModBlockEntityFields {
            nbt,
            items,
            data: Vec::new(),
        },
    ))
}
/// Decode an entity at a known block, refusing undeclared IDs and invalid native membership.
/// Canonical membership wins; blocks absent from the table use vanilla membership.
pub fn decode_block_entity_for_block(
    tag: &Tag,
    block: &str,
) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    let decoded = decode_block_entity(tag)?;
    // Several vanilla types share a decoded representation (for example barrel
    // and chest). Membership belongs to the saved type, not that representation.
    let saved_id = text(tag.get("id"))?;
    let valid = match &decoded.1 {
        crate::containers::BlockEntity::Mod(m) => {
            crate::block_entity_types::valid(block, m.id()) == Some(true)
        }
        _ => crate::block_entity_types::valid(block, &saved_id)
            .unwrap_or_else(|| decoded.1.fits(block)),
    };
    valid.then_some(decoded)
}

pub fn decode_block_entity(tag: &Tag) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    use crate::containers::BlockEntity;
    let int = |k: &str| tag.get(k).and_then(crate::nbt_path::int_value); // getIntOr
    let id = text(tag.get("id"))?;
    let pos = (int("x")?, int("y")?, int("z")?);
    if id.contains(':') && !id.starts_with("minecraft:") {
        return Some((
            pos,
            BlockEntity::Mod(Box::new(decode_mod_block_entity(id, tag)?)),
        ));
    }
    if id == "minecraft:end_gateway" {
        return Some((pos, BlockEntity::EndGateway(decode_end_gateway(tag))));
    }
    if id == "minecraft:jukebox" {
        let record = tag.get("RecordItem").and_then(item_from_nbt);
        let clock = number(tag.get("ticks_since_song_started"));
        // `JukeboxBlockEntity.loadAdditional` -> `setSongWithoutPlaying`: only a
        // saved clock brings a song back, and it comes back without playing.
        let clocked = record.is_some() && clock.is_some();
        return Some((
            pos,
            BlockEntity::Jukebox(crate::containers::Jukebox {
                items: [record],
                ticks: clock.unwrap_or(0),
                song_clock_present: clocked,
                playing: clocked,
                loaded_clock: clocked,
            }),
        ));
    }
    if id == "minecraft:creaking_heart" {
        let heart = crate::containers::CreakingHeart {
            creaking: uuid_from_tag(tag.get("creaking")).map(|uuid| (None, uuid)),
            ..Default::default()
        };
        return Some((pos, BlockEntity::CreakingHeart(Box::new(heart))));
    }
    if id == "minecraft:lectern" {
        // `LecternBlockEntity.loadAdditional`: the page is `Mth.clamp(Page, 0,
        // getPageCount(book) - 1)`, so a stored page before the book's first
        // page or past its last comes back as the page the block holds. No
        // book, or a book with no pages, counts 0 pages: the maximum is then
        // -1, and `Mth.clamp` (`Math.min(Math.max(value, min), max)`) answers
        // it, -1.
        let book = tag.get("Book").and_then(item_from_nbt);
        let count = book.as_ref().map_or(0, crate::menu::book_pages) as i32;
        let lectern = crate::containers::Lectern {
            book,
            page: int("Page").unwrap_or(0).max(0).min(count - 1),
        };
        return Some((pos, BlockEntity::Lectern(lectern)));
    }
    if id == "minecraft:banner" {
        return Some((pos, BlockEntity::Banner(decode_banner(tag))));
    }
    if id == "minecraft:beacon" {
        let beacon = crate::containers::Beacon {
            // BeaconBlockEntity.loadAdditional never reads `Levels`: updateBase sets them.
            primary: text(tag.get("primary_effect")),
            secondary: text(tag.get("secondary_effect")),
            last_check_y: i32::MIN / 2,
            ..Default::default()
        };
        return Some((pos, BlockEntity::Beacon(Box::new(beacon))));
    }
    // A skull's block entity id is not its block's name (every head block).
    if id == "minecraft:skull" {
        return Some((
            pos,
            BlockEntity::Skull(crate::containers::Skull::new(
                tag.get("profile").cloned(),
                text(tag.get("note_block_sound")),
                tag.get("custom_name")
                    .and_then(crate::block_entity_nbt::component),
            )),
        ));
    }
    // A spawner's block entity id is not its block's name.
    let block = match id.as_str() {
        "minecraft:mob_spawner" => "minecraft:spawner",
        other => other,
    };
    let mut entity = BlockEntity::for_block(block)?;
    match &mut entity {
        BlockEntity::Chest(items) => {
            // Randomizable containers reset their inventory, then load Items only
            // when tryLoadLootTable did not read a valid loot-table key.
            let has_table = text(tag.get("LootTable"))
                .and_then(|id| horizon_client_core::identifier::normalize_id(&id))
                .is_some();
            if !has_table {
                *items = decode_items(tag.get("Items"), items.len());
            }
        }
        BlockEntity::Furnace(f) => {
            let items = decode_items(tag.get("Items"), 3);
            f.items = [items[0].clone(), items[1].clone(), items[2].clone()];
            f.lit_remaining = int("lit_time_remaining").unwrap_or(0);
            f.lit_total = int("lit_total_time").unwrap_or(0);
            f.cooking_timer = int("cooking_time_spent").unwrap_or(0);
            f.cooking_total = int("cooking_total_time").unwrap_or(0);
            if let Some(speed) = float(tag.get("speed_multiplier")) {
                f.speed = speed as f32;
            }
            // A save without it owes nothing: the field is newer than some worlds.
            if let Some(Tag::Compound(used)) = tag.get("RecipesUsed") {
                for (id, count) in used {
                    let count = match count {
                        Tag::Int(n) => *n,
                        _ => continue,
                    };
                    // Saves from before the key was namespaced hold both forms of an id.
                    *f.recipes_used
                        .entry(crate::containers::recipe_used_key(&id.to_string_lossy()))
                        .or_insert(0) += count.max(0) as u32;
                }
            }
        }
        BlockEntity::Brewing(b) => {
            let items = decode_items(tag.get("Items"), 5);
            b.items = [
                items[0].clone(),
                items[1].clone(),
                items[2].clone(),
                items[3].clone(),
                items[4].clone(),
            ];
            b.brew_time = int("BrewTime").unwrap_or(0);
            b.total_brew_time = int("total_brew_time").unwrap_or(400);
            if b.brew_time > 0 {
                b.ingredient = b.items[3].as_ref().map(|s| s.item.clone());
            }
            b.fuel = int("Fuel").unwrap_or(0);
            b.total_fuel = int("total_fuel").unwrap_or(20);
            b.speed = float(tag.get("speed_multiplier")).map_or(1.0, |v| v as f32);
        }
        BlockEntity::Brushable(b) => {
            b.loot_table = tag.get("LootTable").cloned();
            b.table_seed_tag = tag.get("LootTableSeed").cloned();
            b.item = if b.loot_table.is_none() {
                tag.get("item").and_then(item_from_nbt)
            } else {
                None
            };
            b.hit_direction = int("hit_direction").filter(|v| (0..=5).contains(v));
        }
        BlockEntity::Sign(sign) => sign.decode(tag),
        BlockEntity::Sculk(s) => s.load(tag),
        BlockEntity::Shelf(shelf) => {
            for (slot, item) in decode_items(tag.get("Items"), 3).into_iter().enumerate() {
                shelf.items[slot] = item;
            }
            shelf.align_to_bottom = int("align_items_to_bottom").unwrap_or(0) != 0;
        }
        // ChiseledBookShelfBlockEntity.loadAdditional.
        BlockEntity::ChiseledBookshelf(b) => {
            for (slot, item) in decode_items(tag.get("Items"), 6).into_iter().enumerate() {
                b.items[slot] = item;
            }
            b.last_slot = int("last_interacted_slot").unwrap_or(-1);
        }
        // CampfireBlockEntity.loadAdditional: missing time arrays stay zeros.
        BlockEntity::Campfire(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 4).into_iter().enumerate() {
                c.items[slot] = item;
            }
            let ints = |key: &str| match tag.get(key) {
                Some(Tag::IntArray(v)) => std::array::from_fn(|i| v.get(i).copied().unwrap_or(0)),
                _ => [0; 4],
            };
            c.progress = ints("CookingTimes");
            c.total = ints("CookingTotalTimes");
        }
        BlockEntity::Crafter(c) => {
            for (slot, item) in decode_items(tag.get("Items"), 9).into_iter().enumerate() {
                c.items[slot] = item;
            }
            if let Some(Tag::IntArray(slots)) = tag.get("disabled_slots") {
                for &slot in slots {
                    if (0..9).contains(&slot) && c.items[slot as usize].is_none() {
                        c.disabled[slot as usize] = true;
                    }
                }
            }
            c.triggered = int("triggered").unwrap_or(0);
            c.state_triggered = None;
            c.crafting_ticks = int("crafting_ticks_remaining").unwrap_or(0);
        }
        // PotentSulfurBlockEntity.loadAdditional.
        BlockEntity::PotentSulfur(sulfur) => {
            if let Some(countdown) = int("countdown") {
                sulfur.waiting_countdown = countdown;
            }
        }
        BlockEntity::Jukebox(_)
        | BlockEntity::Beacon(_)
        | BlockEntity::Lectern(_)
        | BlockEntity::Banner(_)
        | BlockEntity::CreakingHeart(_)
        | BlockEntity::EndGateway(_)
        | BlockEntity::Skull(_)
        | BlockEntity::DaylightDetector
        | BlockEntity::Bell
        | BlockEntity::EndPortal
        | BlockEntity::EnchantingTable
        | BlockEntity::Mod(_) => {}
        // ConduitBlockEntity.loadAdditional: EntityReference.read through
        // UUIDUtil.CODEC, an int array of four.
        BlockEntity::Conduit(target) => {
            *target = match tag.get("Target") {
                Some(Tag::IntArray(v)) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
                _ => None,
            };
        }
        // `BlockEntity.loadAdditional`: the components the block entity keeps.
        BlockEntity::CopperGolemStatue(components) => {
            *components = match tag.get("components") {
                Some(Tag::Compound(c)) => c.clone(),
                _ => Compound::new(),
            };
        }
        BlockEntity::Spawner(s) => **s = crate::mobs::spawner::Spawner::load(tag),
        BlockEntity::TrialSpawner(s) => **s = crate::mobs::trial_spawner::TrialSpawner::load(tag),
        BlockEntity::Vault(v) => **v = crate::vault::Vault::load(tag),
        BlockEntity::Beehive(h) => **h = crate::beehive::Beehive::from_nbt(tag),
        BlockEntity::CommandBlock(c) => **c = crate::command_block::CommandBlock::load(tag),
        BlockEntity::StructureBlock(b) => **b = crate::structure_block::StructureBlock::load(tag),
        BlockEntity::TestBlock(b) => **b = crate::test_block::TestBlock::load(tag),
        BlockEntity::TestInstanceBlock(b) => b.load(tag),
        // The state is not known here (Game::decode_block_entity_at applies it).
        BlockEntity::JigsawBlock(b) => {
            **b = crate::jigsaw_block::JigsawBlock::load(
                tag,
                crate::jigsaw_block::JointType::Rollable,
            )
        }
    }
    Some((pos, entity))
}

mod villager_poi;
pub use villager_poi::encode_poi;
/// A record's `free_tickets` from the POI file the chunk had, while the same
/// POI is still there: a bed or job site a villager claimed stays claimed.
fn keep_claims(saved: &mut NamedTag, old: &NamedTag) {
    let key = |r: &Tag| match (r.get("pos"), text(r.get("type"))) {
        (Some(Tag::IntArray(pos)), Some(kind)) => Some((pos.clone(), kind)),
        _ => None,
    };
    let mut claims = HashMap::new();
    if let Some(Tag::Compound(sections)) = old.tag.get("Sections") {
        for section in sections.values() {
            if let Some(Tag::List(records)) = section.get("Records") {
                for r in records {
                    if let Some(k) = key(r) {
                        claims.insert(k, number(r.get("free_tickets")).unwrap_or(0) as i32);
                    }
                }
            }
        }
    }
    let Tag::Compound(root) = &mut saved.tag else {
        return;
    };
    let Some(Tag::Compound(sections)) = root.get_mut(&"Sections".into()) else {
        return;
    };
    for section in sections.values_mut() {
        let Tag::Compound(section) = section else {
            continue;
        };
        let Some(Tag::List(records)) = section.get_mut(&"Records".into()) else {
            continue;
        };
        for r in records {
            let Some(&tickets) = key(r).and_then(|k| claims.get(&k)) else {
                continue;
            };
            if let Tag::Compound(r) = r {
                if tickets == 0 {
                    r.remove(&"free_tickets".into());
                } else {
                    r.insert("free_tickets".into(), Tag::Int(tickets));
                }
            }
        }
    }
}
fn poi_root(sections: Compound) -> NamedTag {
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("Sections", Tag::Compound(sections)),
        ]),
    }
}

pub use crate::chunk::LevelShape;

/// A column's sections as `Heightmap.primeHeightmaps` reads them.
struct HeightmapView<'a> {
    sections: &'a [Arc<Section>],
    /// The minimum Y of `sections[0]`.
    column_min_y: i32,
    tables: &'a SaveTables,
    min_y: i32,
    height: i32,
}
impl HeightmapChunk for HeightmapView<'_> {
    type State = crate::assets::StateFacts;
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn highest_section_y(&self) -> i32 {
        let top = self
            .sections
            .iter()
            .rposition(|s| !s.is_uniform(self.tables.air))
            .unwrap_or(0);
        self.column_min_y + top as i32 * 16
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State {
        // Chunk.get: no block outside the sections.
        let state = usize::try_from((y - self.column_min_y) >> 4)
            .ok()
            .and_then(|s| self.sections.get(s))
            .map_or(0, |s| s.get(crate::chunk::index(x, y, z)));
        self.tables.heightmaps.heightmap_state(BlockStateId(state))
    }
}
/// Heightmaps kept after world generation (`ChunkStatus.FULL.heightmapsAfter`).
fn heightmaps(chunk: &Chunk, tables: &SaveTables, shape: LevelShape) -> Tag {
    let kinds = [
        HeightmapType::WorldSurface,
        HeightmapType::OceanFloor,
        HeightmapType::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves,
    ];
    let mut maps: Vec<Heightmap> = kinds
        .iter()
        .filter_map(|k| Heightmap::new(*k, shape.min_y, shape.height).ok())
        .collect();
    let view = HeightmapView {
        sections: &chunk.sections,
        column_min_y: chunk.min_y(),
        tables,
        min_y: shape.min_y,
        height: shape.height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Tag::Compound(Compound::new());
    }
    Tag::compound(maps.iter().map(|m| {
        (
            m.kind().name(),
            Tag::LongArray(m.raw_data().iter().map(|&w| w as i64).collect()),
        )
    }))
}
/// The client heightmaps of a column's sections (from `column_min_y`), in a
/// level from `min_y`, `height` blocks high.
pub fn client_heightmaps_in(
    sections: &[Arc<Section>],
    column_min_y: i32,
    tables: &SaveTables,
    min_y: i32,
    height: i32,
) -> Vec<(i32, Vec<i64>)> {
    let kinds = [
        (1, HeightmapType::WorldSurface),
        (4, HeightmapType::MotionBlocking),
        (5, HeightmapType::MotionBlockingNoLeaves),
    ];
    let mut maps: Vec<(i32, Heightmap)> = kinds
        .iter()
        .filter_map(|(id, k)| Heightmap::new(*k, min_y, height).ok().map(|m| (*id, m)))
        .collect();
    let view = HeightmapView {
        sections,
        column_min_y,
        tables,
        min_y,
        height,
    };
    let mut refs: Vec<&mut Heightmap> = maps.iter_mut().map(|(_, m)| m).collect();
    if Heightmap::prime(&view, &mut refs).is_err() {
        return Vec::new();
    }
    maps.iter()
        .map(|(id, m)| (*id, m.raw_data().iter().map(|&w| w as i64).collect()))
        .collect()
}
/// Vanilla light layer (nibbles, index y<<8|z<<4|x, low nibble first).
fn nibbles(light: &Light, sky: bool) -> Vec<u8> {
    let mut out = vec![0u8; 2048];
    for (i, byte) in out.iter_mut().enumerate() {
        let value = |j: usize| {
            let v = light.get(j);
            if sky { v >> 4 } else { v & 15 }
        };
        *byte = value(2 * i) | value(2 * i + 1) << 4;
    }
    out
}

pub fn encode(chunk: &Chunk, registry: &BlockRegistry, biomes: &Biomes) -> NamedTag {
    encode_with(chunk, registry, biomes, None, LevelShape::OVERWORLD)
}
/// A save's lookup of an old chunk's calculated blending data.
pub type BlendingLookup = Box<dyn Fn(i32, i32) -> Option<Arc<BlendingData>> + Send + Sync>;
/// [`encode_with`] for a save of `SaveDir`: an old chunk whose blending data
/// the generation calculated saves it (`BlendingData.pack`), when the record
/// it was loaded from has the same section range; otherwise `blending_data`
/// stays as it was loaded.
fn encode_saved(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    blending: Option<&BlendingData>,
    palette_cache: Option<&Arc<SavedPaletteBudget>>,
) -> NamedTag {
    let mut root = encode_with_palette_cache(chunk, registry, biomes, tables, shape, palette_cache);
    if let Some(data) = blending {
        put_calculated_blending(&mut root, data);
    }
    root
}
fn put_calculated_blending(root: &mut NamedTag, data: &BlendingData) {
    let Tag::Compound(map) = &mut root.tag else {
        return;
    };
    let Some(saved) = map.get(&"blending_data".into()) else {
        return;
    };
    let (min_section, max_section, heights) = data.pack();
    if saved.get("min_section") != Some(&Tag::Int(min_section))
        || saved.get("max_section") != Some(&Tag::Int(max_section))
    {
        return;
    }
    let mut fields = vec![
        ("min_section", Tag::Int(min_section)),
        ("max_section", Tag::Int(max_section)),
    ];
    if let Some(heights) = heights {
        fields.push((
            "heights",
            Tag::List(heights.iter().map(|h| Tag::Float(h.to_bits())).collect()),
        ));
    }
    map.insert("blending_data".into(), Tag::compound(fields));
}
/// The chunk NBT vanilla's `SerializableChunkData.write` produces for a full
/// chunk in a level of `shape`.
pub fn encode_with(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
) -> NamedTag {
    encode_with_palette_cache(chunk, registry, biomes, tables, shape, None)
}

fn encode_with_palette_cache(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
    palette_cache: Option<&Arc<SavedPaletteBudget>>,
) -> NamedTag {
    let sky_light = shape.sky_light;
    let mut sections = Vec::new();
    // Sky layers below the highest section with light other than 15 are
    // written (even all-zero ones): vanilla derives missing layers from the
    // layer above them.
    let top_sky = chunk
        .sections
        .iter()
        .rposition(|s| s.light.any_not(0xF0, 0xF0));
    for (si, section) in chunk.sections.iter().enumerate() {
        let y = (chunk.min_y() >> 4) + si as i32;
        if !shape.has_section(y) {
            continue;
        }
        let (block_states, biomes_tag) =
            section_nbt::palettes(section, registry, biomes, tables, palette_cache);
        let mut entries = vec![
            ("Y", Tag::Byte(y as i8)),
            ("block_states", block_states),
            ("biomes", biomes_tag),
        ];
        if chunk.lit {
            let block_lit = section.light.any(15);
            if block_lit {
                entries.push(("BlockLight", Tag::ByteArray(nibbles(&section.light, false))));
            }
            if sky_light && top_sky.is_some_and(|top| si <= top + 1) {
                entries.push(("SkyLight", Tag::ByteArray(nibbles(&section.light, true))));
            }
        }
        sections.push(Tag::compound(entries));
    }
    let extras = chunk.saved.as_deref();
    let block_name = |p: &(i32, i32, i32)| -> String {
        registry
            .state(chunk.block(p.0 & 15, p.1, p.2 & 15))
            .map_or_else(String::new, |s| s.name.clone())
    };
    let mut block_entities: Vec<Tag> = chunk
        .block_entities
        .iter()
        // A mod block entity a native mod is ticking right now
        // (`tick_mod_block_entity`): its data is out of the world.
        .filter(|(_, e)| !block_entity_placeholder(e))
        .map(|(p, e)| {
            let block = block_name(p);
            let mut current = block_entity_encode::encode(*p, e, &block);
            if let Some((_, name)) = chunk.block_names.iter().find(|(q, _)| q == p)
                && let Tag::Compound(map) = &mut current
            {
                map.insert("CustomName".into(), name.clone());
            }
            let source = extras
                .and_then(|x| x.block_entity_sources.iter().find(|(q, _)| q == p))
                .map(|(_, tag)| tag)
                .filter(|tag| {
                    crate::block_entity_nbt::id(tag) == crate::block_entity_nbt::id(&current)
                });
            let merged = match source {
                Some(original) => {
                    let loaded = decode_block_entity(original).map_or_else(
                        || current.clone(),
                        |(q, e)| {
                            let mut loaded = block_entity_encode::encode(q, &e, &block);
                            if !matches!(
                                e,
                                crate::containers::BlockEntity::Banner(_)
                                    | crate::containers::BlockEntity::Sign(_)
                                    | crate::containers::BlockEntity::Mod(_)
                            ) && let Some(name) = original
                                .get("CustomName")
                                .and_then(crate::block_entity_nbt::component)
                                && let Tag::Compound(map) = &mut loaded
                            {
                                // BaseContainerBlockEntity.loadAdditional: the
                                // external name is also part of the loaded model.
                                // Its removal must win over the source overlay.
                                map.insert("CustomName".into(), name);
                            }
                            loaded
                        },
                    );
                    merge_block_entity(original, &loaded, &current)
                }
                None => current,
            };
            let mut saved = crate::block_entity_nbt::canonical(&merged).unwrap_or(merged);
            if let Some(limit) = extras
                .and_then(|x| x.block_entity_sources.iter().find(|(q, _)| q == p))
                .and_then(|(_, tag)| tag.get("horizon:stack_limit"))
                && let Tag::Compound(map) = &mut saved
            {
                map.insert("horizon:stack_limit".into(), limit.clone());
            }
            if let Some((_, attachments)) =
                extras.and_then(|e| e.block_entity_attachments.iter().find(|(pos, _)| pos == p))
                && let Tag::Compound(map) = &mut saved
                && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
            {
                let data = attachments.to_nbt();
                if data.is_empty() {
                    map.remove(&ATTACHMENTS_KEY.into());
                } else {
                    map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
                }
            }
            if matches!(e, crate::containers::BlockEntity::Chest(_))
                && let Tag::Compound(map) = &mut saved
            {
                map.remove(&"LootTable".into());
                map.remove(&"LootTableSeed".into());
                if let Some(pending) = extras.and_then(|x| x.pending_loot.get(p)) {
                    pending.write_tags(map);
                    if e.slots().iter().any(Option::is_some) {
                        map.insert("Items".into(), encode_items(e.slots()));
                    }
                }
            }
            saved
        })
        .collect();
    if let Some(extras) = extras {
        for raw in &extras.block_entities {
            let pos = (
                number(raw.get("x")).unwrap_or(0) as i32,
                number(raw.get("y")).unwrap_or(0) as i32,
                number(raw.get("z")).unwrap_or(0) as i32,
            );
            let state = chunk.get(pos.0 & 15, pos.1, pos.2 & 15) as usize;
            let still_there =
                tables.is_none_or(|t| t.block_entity.get(state).copied().unwrap_or(true));
            if still_there && !chunk.block_entities.iter().any(|(p, _)| *p == pos) {
                block_entities
                    .push(crate::block_entity_nbt::canonical(raw).unwrap_or_else(|| raw.clone()));
            }
        }
    }
    let mut root = extras
        .map(|e| Tag::Compound(e.root.clone()))
        .unwrap_or_else(|| Tag::Compound(Compound::new()));
    for (key, value) in [
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("xPos", Tag::Int(chunk.x)),
        ("zPos", Tag::Int(chunk.z)),
        ("yPos", Tag::Int(shape.min_section())),
        ("Status", string("minecraft:full")),
        ("LastUpdate", Tag::Long(extras.map_or(0, |e| e.last_update))),
        ("isLightOn", Tag::Byte(i8::from(chunk.lit))),
        ("sections", Tag::List(sections)),
        ("block_entities", Tag::List(block_entities)),
    ] {
        put(&mut root, key, value);
    }
    if let Some(Tag::Compound(map)) = Some(&mut root) {
        if let Some(extras) = extras
            && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
        {
            let attachments = extras.attachments.to_nbt();
            if attachments.is_empty() {
                map.remove(&ATTACHMENTS_KEY.into());
            } else {
                map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(attachments));
            }
        }
        map.entry("InhabitedTime".into()).or_insert(Tag::Long(0));
        map.entry("structures".into()).or_insert_with(|| {
            Tag::compound([
                ("References", Tag::Compound(Compound::new())),
                ("starts", Tag::Compound(Compound::new())),
            ])
        });
        if !chunk.structure_refs.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            let mut refs: std::collections::BTreeMap<&str, Vec<i64>> = Default::default();
            for (id, (x, z)) in &chunk.structure_refs {
                refs.entry(id)
                    .or_default()
                    .push(i64::from(*x as u32) | i64::from(*z) << 32);
            }
            let refs: Compound = refs
                .into_iter()
                .map(|(id, v)| (id.into(), Tag::LongArray(v)))
                .collect();
            s.insert("References".into(), Tag::Compound(refs));
        }
        let starts: Compound = chunk
            .structure_starts
            .iter()
            .filter_map(|start| match start.save() {
                Some(tag) => Some((start.name.as_str().into(), tag)),
                None => {
                    log::warn!(
                        "Horizon: structure start {} not saved: one of its pieces cannot be",
                        start.name
                    );
                    None
                }
            })
            .collect();
        if !starts.is_empty()
            && let Some(Tag::Compound(s)) = map.get_mut(&"structures".into())
        {
            s.insert("starts".into(), Tag::Compound(starts));
        }
        map.entry("PostProcessing".into())
            .or_insert_with(|| Tag::List(vec![Tag::List(Vec::new()); shape.section_count()]));
    }
    if let Some(t) = tables {
        put(&mut root, "Heightmaps", heightmaps(chunk, t, shape));
        let names = |k: u32| t.block_names.get(k as usize).cloned();
        let (blocks, fluids) = extras.map_or((&[][..], &[][..]), |e| {
            (&e.block_ticks[..], &e.fluid_ticks[..])
        });
        put(&mut root, "block_ticks", tick_list(blocks, names));
        put(&mut root, "fluid_ticks", tick_list(fluids, fluid_name));
    }
    NamedTag {
        name: "".into(),
        tag: root,
    }
}
/// Root keys `encode_with` regenerates; everything else is kept as loaded.
const GENERATED_KEYS: [&str; 14] = [
    "DataVersion",
    "xPos",
    "zPos",
    "yPos",
    "Status",
    "LastUpdate",
    "isLightOn",
    "sections",
    "block_entities",
    "Heightmaps",
    "block_ticks",
    "fluid_ticks",
    "entities",
    "CarvingMask",
];
pub fn decode(root: &NamedTag, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
    decode_with(root, registry, biomes, None, ColumnLayout::VANILLA)
}
/// Only full chunks load here; partially generated ones (vanilla's world
/// border of proto chunks) resume generation through `decode_proto`.
pub fn decode_with(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    if text(root.tag.get("Status")).is_some_and(|s| s != "minecraft:full" && s != "full") {
        return None;
    }
    decode_column(root, registry, biomes, tables, layout)
}
/// A full chunk with the light saved with it (`isLightOn`), as far terrain
/// reads another client's cache of a server; `None` without its light.
pub fn decode_lit(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    layout: ColumnLayout,
    sky_light: bool,
) -> Option<Chunk> {
    let mut chunk = decode_with(root, registry, biomes, None, layout)?;
    load_light(&root.tag, &mut chunk, sky_light);
    chunk.light_loaded.then_some(chunk)
}
/// `decode_with` whatever the chunk's status, into a chunk of `layout`
/// (sections outside it are left out).
fn decode_column(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<Chunk> {
    let tag = &root.tag;
    let Tag::Int(x) = tag.get("xPos")? else {
        return None;
    };
    let Tag::Int(z) = tag.get("zPos")? else {
        return None;
    };
    let mut chunk = Chunk::with_layout(*x, *z, layout);
    // structures.References: structure id -> packed start chunk positions.
    if let Some(Tag::Compound(refs)) = tag.get("structures").and_then(|s| s.get("References")) {
        for (id, v) in refs {
            if let Tag::LongArray(starts) = v {
                let id: Arc<str> = Arc::from(id.to_string_lossy());
                for &p in starts {
                    chunk
                        .structure_refs
                        .push((id.clone(), (p as i32, (p >> 32) as i32)));
                }
            }
        }
    }
    let air = registry.default_state("minecraft:air")?.0;
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return Some(chunk);
    };
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(si) = layout.section_index(y as i32 * 16) else {
            continue;
        };
        if let Some(states) = section.get("block_states")
            && let Some(Tag::List(palette)) = states.get("palette")
        {
            let palette: Vec<u32> = palette
                .iter()
                .map(|p| registry.saved_state_id(p).map_or(air, |id| id.0))
                .collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            let data = match states.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            if let Ok(blocks) =
                PalettedContainer::from_packed(block_strategy(), palette, data.as_deref())
            {
                target.blocks = blocks;
            }
        }
        if let Some(b) = section.get("biomes")
            && let Some(Tag::List(palette)) = b.get("palette")
        {
            let ids: Vec<u16> = palette
                .iter()
                .map(|p| match p {
                    Tag::String(s) => biomes.saved_id(&s.to_string_lossy()),
                    _ => 0,
                })
                .collect();
            let data = match b.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            let palette: Vec<u32> = ids.iter().map(|&id| u32::from(id)).collect();
            let target = Arc::make_mut(&mut chunk.sections[si]);
            if let Ok(container) =
                PalettedContainer::from_packed(BIOME_STRATEGY, palette.clone(), data.as_deref())
            {
                target.biomes = container;
            } else if let Some(&first) = palette.first() {
                target.biomes = PalettedContainer::new(BIOME_STRATEGY, first);
            }
        }
    }
    let mut extras = ChunkExtras {
        last_update: number(tag.get("LastUpdate")).unwrap_or(0),
        ..ChunkExtras::default()
    };
    if let Some(Tag::List(entities)) = tag.get("block_entities") {
        for e in entities {
            if let Some(at) = block_entity_order::saved_at(e, [*x, *z]) {
                extras.block_entity_order.push(at);
            }
            let int = |k: &str| number(e.get(k)).map(|v| v as i32);
            let decoded = if let (Some(x), Some(y), Some(z)) = (int("x"), int("y"), int("z")) {
                let state = chunk.block(x & 15, y, z & 15);
                registry
                    .state(state)
                    .and_then(|block| decode_block_entity_for_block(e, &block.name))
                    .map(|(_, be)| (block_entity_pos((x, y, z), [chunk.x, chunk.z]), be))
            } else {
                None
            };
            match decoded {
                Some(be) => {
                    if let Ok(Some(data)) = attachment_compound(e.get(ATTACHMENTS_KEY)) {
                        extras.block_entity_attachments.push((
                            be.0,
                            horizon_mod_runtime::attachments::Attachments::from_nbt(data.clone()),
                        ));
                    }
                    // A banner keeps its name in its model (`decode_banner`).
                    let name = match &be.1 {
                        crate::containers::BlockEntity::Banner(_) => None,
                        crate::containers::BlockEntity::Sign(_)
                        | crate::containers::BlockEntity::Mod(_) => {
                            text(e.get("CustomName")).map(|name| string(&name))
                        }
                        _ => e.get("CustomName").and_then(nbt_component),
                    };
                    if let Some(name) = name {
                        chunk.block_names.push((be.0, name));
                    }
                    let randomizable = matches!(be.1, crate::containers::BlockEntity::Chest(_));
                    if randomizable && let Some(pending) = PendingContainerLoot::from_tag(e) {
                        extras.pending_loot.insert(be.0, pending);
                    }
                    let mut source = e.clone();
                    if randomizable && let Tag::Compound(map) = &mut source {
                        map.remove(&"LootTable".into());
                        map.remove(&"LootTableSeed".into());
                    }
                    extras.block_entity_sources.push((be.0, source));
                    chunk.block_entities.push(be);
                }
                None => extras.block_entities.push(e.clone()),
            }
        }
    }
    if let Tag::Compound(map) = tag {
        if let Ok(Some(data)) = attachment_compound(map.get(&ATTACHMENTS_KEY.into())) {
            extras.attachments =
                horizon_mod_runtime::attachments::Attachments::from_nbt(data.clone());
        }
        extras.root = map
            .iter()
            .filter(|(k, _)| !GENERATED_KEYS.contains(&k.to_string_lossy().as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }
    if let Some(t) = tables {
        extras.block_ticks = read_ticks(tag.get("block_ticks"), (*x, *z), |n| {
            t.block_ids.get(n).map(|&i| u32::from(i))
        });
        extras.fluid_ticks = read_ticks(tag.get("fluid_ticks"), (*x, *z), fluid_id);
    }
    chunk.saved = Some(Arc::new(extras));
    Some(chunk)
}

pub fn encode_proto(
    proto: &SavedProto,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    shape: LevelShape,
) -> NamedTag {
    let mut root = encode_with(&proto.column, registry, biomes, tables, shape);
    let status = if proto.decorated {
        ChunkStatus::Features
    } else {
        ChunkStatus::Terrain
    };
    // ProtoChunk.postProcessing: one list per section, packed section offsets.
    let mut marks = vec![Vec::new(); shape.section_count()];
    for &[x, y, z] in &proto.column.postprocess {
        let section = usize::try_from((y - shape.min_y) >> 4).ok();
        if let Some(list) = section.and_then(|s| marks.get_mut(s)) {
            list.push(Tag::Short(
                ((x & 15) | (y & 15) << 4 | (z & 15) << 8) as i16,
            ));
        }
    }
    for (key, value) in [
        ("Status", string(status.name())),
        ("isLightOn", Tag::Byte(0)),
        ("block_entities", Tag::List(proto.block_entities.clone())),
        ("entities", Tag::List(proto.entities.clone())),
        (
            "PostProcessing",
            Tag::List(marks.into_iter().map(Tag::List).collect()),
        ),
    ] {
        put(&mut root.tag, key, value);
    }
    root
}
/// A proto chunk generation can resume: `terrain` (written to by a
/// neighbour's decoration) up to `spawn`. Earlier statuses hold nothing that
/// generating the chunk again doesn't reproduce.
pub fn decode_proto(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    layout: ColumnLayout,
) -> Option<SavedProto> {
    let status = ChunkStatus::parse(&text(root.tag.get("Status"))?).ok()?;
    if !(ChunkStatus::Terrain..ChunkStatus::Full).contains(&status) {
        return None;
    }
    let mut column = decode_column(root, registry, biomes, tables, layout)?;
    column.block_entities.clear();
    let base = number(root.tag.get("yPos")).map_or(column.min_y(), |y| y as i32 * 16);
    if let Some(Tag::List(sections)) = root.tag.get("PostProcessing") {
        for (si, marks) in sections.iter().enumerate() {
            let Tag::List(marks) = marks else {
                continue;
            };
            for mark in marks {
                if let Tag::Short(p) = mark {
                    let p = i32::from(*p);
                    column.postprocess.push([
                        column.x * 16 + (p & 15),
                        base + si as i32 * 16 + ((p >> 4) & 15),
                        column.z * 16 + ((p >> 8) & 15),
                    ]);
                }
            }
        }
    }
    let list = |key: &str| match root.tag.get(key) {
        Some(Tag::List(list)) => list.clone(),
        _ => Vec::new(),
    };
    Some(SavedProto {
        column,
        decorated: status >= ChunkStatus::Features,
        block_entities: list("block_entities"),
        entities: list("entities"),
    })
}

/// Read the stored `SkyLight`/`BlockLight` layers of a chunk saved with
/// `isLightOn` (`SerializableChunkData.read` + `ThreadedLevelLightEngine`),
/// so it needs no relighting. A missing sky layer repeats the bottom layer of
/// the section above it (15 above the highest stored one); a missing block
/// layer is dark.
pub fn load_light(tag: &Tag, chunk: &mut Chunk, sky_light: bool) {
    if tag.get("isLightOn") != Some(&Tag::Byte(1)) {
        return;
    }
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return;
    };
    let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); chunk.sections.len()];
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let Some(si) = chunk.section_index(y as i32 * 16) else {
            continue;
        };
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        layers[si] = (layer("SkyLight"), layer("BlockLight"));
    }
    // A saved layer is already the resident `Light::Sky`/`Light::Block`
    // layer: byte `i` holds block `2i` in its low nibble and `2i + 1` in its
    // high one. Both nibbles of each byte must match for a layer to be
    // uniform; only then is the whole section one value.
    let uniform_layer = |layer: &[u8]| {
        let first = layer[0];
        (first & 15 == first >> 4 && layer.iter().all(|&v| v == first)).then_some(first & 15)
    };
    let copy_layer = |layer: &[u8]| {
        let mut out = Box::new([0u8; 2048]);
        out.copy_from_slice(layer);
        out
    };
    let mut above = [if sky_light { 15 } else { 0 }; 256];
    for (si, (sky, block)) in layers.into_iter().enumerate().rev() {
        let sky = sky.filter(|_| sky_light);
        let sky_uniform = sky.and_then(uniform_layer);
        let sky_varies = match sky {
            Some(_) => sky_uniform.is_none(),
            None => above.iter().any(|&v| v != above[0]),
        };
        let block_uniform = block.and_then(uniform_layer);
        let block_varies = block.is_some() && block_uniform.is_none();
        let sky_first = sky.map_or(above[0], |layer| layer[0] & 15);
        let block_first = block.map_or(0, |layer| layer[0] & 15);
        let section = Arc::make_mut(&mut chunk.sections[si]);
        section.compact();
        // The saved layer decides the resident variant directly: expand
        // only when both nibbles vary (which keeps `Light::Full`).
        section.light = match (sky_varies, block_varies) {
            (false, false) => Light::Uniform(sky_first << 4 | block_first),
            (false, true) => Light::Block(copy_layer(block.expect("block layer")), sky_first),
            (true, false) => {
                let layer = match sky {
                    Some(layer) => copy_layer(layer),
                    // No stored sky: the same 256-byte bottom row over and
                    // over, which is exactly `Light::packed`'s layer here.
                    None => {
                        let mut layer = Box::new([0u8; 2048]);
                        for row in layer.as_chunks_mut::<128>().0 {
                            for (out, v) in
                                row.iter_mut().zip(above.as_chunks::<2>().0.iter().cycle())
                            {
                                *out = v[0] | v[1] << 4;
                            }
                        }
                        layer
                    }
                };
                Light::Sky(layer, block_first)
            }
            (true, true) => {
                let mut data = Box::new([0u8; 4096]);
                match (sky, block) {
                    (Some(sky), Some(block)) => {
                        for ((pair, &s), &b) in
                            data.as_chunks_mut::<2>().0.iter_mut().zip(sky).zip(block)
                        {
                            pair[0] = s << 4 | (b & 15);
                            pair[1] = (s & 0xf0) | (b >> 4);
                        }
                    }
                    (None, Some(block)) => {
                        for (i, (pair, &b)) in data
                            .as_chunks_mut::<2>()
                            .0
                            .iter_mut()
                            .zip(block)
                            .enumerate()
                        {
                            pair[0] = above[(i * 2) & 255] << 4 | (b & 15);
                            pair[1] = above[(i * 2 + 1) & 255] << 4 | (b >> 4);
                        }
                    }
                    _ => unreachable!("both light kinds vary"),
                }
                Light::Full(data)
            }
        };
        if let Some(sky) = sky {
            for (pair, &s) in above.as_chunks_mut::<2>().0.iter_mut().zip(sky) {
                pair[0] = s & 15;
                pair[1] = s >> 4;
            }
        }
    }
    chunk.lit = true;
    chunk.light_loaded = true;
}

type RegionCache = Mutex<RegionFiles>;
/// `RegionFileStorage.MAX_CACHE_SIZE`: the region files of a folder kept
/// open. Each holds a file descriptor: kept for every region ever visited,
/// a server whose players travel runs out of them (`Too many open files`).
const MAX_OPEN_REGIONS: usize = 256;
/// A folder's region files, and the ones known to be missing, by region,
/// with when each was last asked for.
#[derive(Default)]
struct RegionFiles {
    regions: HashMap<(i32, i32), (Option<Arc<Mutex<RegionFile>>>, u64)>,
    clock: u64,
    /// Each region's write stamp (`far_regions::writing`).
    written: HashMap<(i32, i32), u64>,
}
impl RegionFiles {
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
    /// The open region files.
    #[cfg(test)]
    fn open(&self) -> usize {
        self.regions.values().filter(|(r, _)| r.is_some()).count()
    }
    /// Close the least recently used regions until there is room for one
    /// more, as `RegionFileStorage.getRegionFile` does. A region a reader or
    /// the writer still holds stays: closing it under them would let a
    /// second handle read a header older than its sectors.
    fn make_room(&mut self) {
        while self.regions.len() >= MAX_OPEN_REGIONS {
            let oldest = self
                .regions
                .iter()
                .filter(|(_, (r, _))| r.as_ref().is_none_or(|r| Arc::strong_count(r) == 1))
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key);
            let Some(key) = oldest else {
                return;
            };
            self.regions.remove(&key);
        }
    }
}
/// The region file at `path` through the cache, which readers and the writer
/// share, so a read never sees a header older than the sectors it points to.
/// `create` makes a missing file (for a write); otherwise a missing file is
/// `None`, until a write creates it.
fn cached_region(
    cache: &RegionCache,
    key: (i32, i32),
    path: &Path,
    create: bool,
) -> Result<Option<Arc<Mutex<RegionFile>>>> {
    let mut cache = cache.lock().map_err(|_| "region cache poisoned")?;
    cache.clock += 1;
    let now = cache.clock;
    if let Some((cached, used)) = cache.regions.get_mut(&key)
        && (cached.is_some() || !create)
    {
        *used = now;
        return Ok(cached.clone());
    }
    cache.make_room();
    if create && let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // A file this process can't write (a read-only world) is still read.
    let region = match RegionFile::open_writable(path, create) {
        Err(e) if !create && e.kind() == std::io::ErrorKind::PermissionDenied => {
            RegionFile::open(path)
        }
        other => other,
    };
    let region = match region {
        Ok(r) => Some(Arc::new(Mutex::new(r))),
        Err(_) if !create => None,
        Err(e) => return Err(e.into()),
    };
    cache.regions.insert(key, (region.clone(), now));
    Ok(region)
}
/// Write `records` into the region files of `folder` in place, as vanilla's
/// `RegionFile.write` does: each chunk takes free sectors and the header
/// then points to it; the region's other chunks are not touched. `None`
/// clears a chunk. The chunks are encoded and compressed before their region
/// is locked for the writes (`compress_records`), and each region is flushed
/// to the disk once.
fn write_region_records(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    records: &[((i32, i32), &Record)],
) -> Result<()> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let compressed = compress_records(records, timestamp);
    let mut by_region: BTreeMap<(i32, i32), Vec<(ChunkPos, Option<ChunkRecord>)>> = BTreeMap::new();
    for (&((x, z), _), record) in records.iter().zip(compressed) {
        let record = record?;
        by_region
            .entry((x >> 5, z >> 5))
            .or_default()
            .push((ChunkPos::new(x, z), record));
    }
    for ((rx, rz), list) in by_region {
        let path = region_path(root, folder, rx, rz);
        // Clearing chunks of a region that doesn't exist changes nothing.
        let create = list.iter().any(|(_, r)| r.is_some());
        let Some(region) = cached_region(cache, (rx, rz), &path, create)? else {
            continue;
        };
        let mut region = region.lock().map_err(|_| "region file poisoned")?;
        let _writing = far_regions::writing(cache, (rx, rz));
        let mut changed = false;
        for (pos, record) in &list {
            match record {
                Some(record) => {
                    region.write(*pos, record)?;
                    changed = true;
                }
                None => changed |= region.clear(*pos, timestamp)?,
            }
        }
        if changed {
            region.sync()?;
        }
    }
    Ok(())
}

/// `job(i)` for every `i` below `count`, in order, shared between this thread
/// and up to `helpers - 1` scoped threads, each taking the next index as it
/// finishes one. A helper the system refuses to start (`start` false, or a
/// container at its limit of threads: EAGAIN) leaves its share to the
/// others: this thread does all of it at worst, and never panics for it, as
/// `thread::scope`'s own `spawn` would, which stopped the region writer.
#[cfg(test)]
fn share_out<R: Send>(
    count: usize,
    helpers: usize,
    job: impl Fn(usize) -> R + Sync,
    mut start: impl FnMut() -> bool,
) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let work = || {
        let mut out = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= count {
                return out;
            }
            out.push((i, job(i)));
        }
    };
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let threads: Vec<_> = (1..helpers)
            .filter(|_| start())
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(scope, work).ok())
            .collect();
        let mut done = work();
        for t in threads {
            done.extend(t.join().unwrap_or_else(|e| std::panic::resume_unwind(e)));
        }
        done
    });
    done.sort_unstable_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

fn region_path(root: &Path, folder: &str, rx: i32, rz: i32) -> PathBuf {
    root.join(format!("{folder}/r.{rx}.{rz}.mca"))
}

/// A record waiting for the writer thread.
#[derive(Clone)]
enum Record {
    Tag(Arc<NamedTag>),
    /// A chunk to encode on the writer thread.
    /// The old chunk's calculated blending data is taken when the save is
    /// requested ([`SaveDir::calculated_blending`]): a snapshot of that save.
    Chunk(Arc<Chunk>, Arc<Encoding>, Option<Arc<BlendingData>>),
    Proto(Arc<SavedProto>, Arc<Encoding>),
    /// A chunk's points of interest, made on the writer thread against the
    /// saved ones (`resolve_poi`).
    Poi(
        Arc<Chunk>,
        Arc<SaveTables>,
        Option<Arc<crate::mobs::villager::ResolvedProfessions>>,
    ),
    Delete,
}
/// What encoding a chunk needs, captured when it is queued.
struct Encoding {
    registry: &'static BlockRegistry,
    biomes: Arc<Biomes>,
    tables: Option<Arc<SaveTables>>,
    shape: LevelShape,
    section_palettes: Option<Arc<SavedPaletteBudget>>,
}
impl Record {
    fn tag(&self) -> Option<Arc<NamedTag>> {
        match self {
            Record::Tag(t) => Some(t.clone()),
            Record::Chunk(c, e, blending) => Some(Arc::new(encode_saved(
                c,
                e.registry,
                &e.biomes,
                e.tables.as_deref(),
                e.shape,
                blending.as_deref(),
                e.section_palettes.as_ref(),
            ))),
            Record::Proto(p, e) => Some(Arc::new(encode_proto(
                p,
                e.registry,
                &e.biomes,
                e.tables.as_deref(),
                e.shape,
            ))),
            Record::Poi(c, t, professions) => {
                encode_poi(c, t, professions.as_deref()).map(Arc::new)
            }
            Record::Delete => None,
        }
    }
}
/// A queued POI record as it is written: claims kept from the saved one, and
/// a chunk that lost its last POI gets an empty one, as vanilla trusts a
/// saved section. None when the chunk never had any.
fn resolve_poi(
    root: &Path,
    cache: &RegionCache,
    (x, z): (i32, i32),
    record: &Record,
) -> Option<Record> {
    let Record::Poi(chunk, tables, professions) = record else {
        return Some(record.clone());
    };
    let old = read_region_record(root, cache, "poi", x, z)
        .and_then(|old| {
            old.map(|mut old| {
                upgrade_record(Folder::Poi, &mut old, || None)?;
                Ok(old)
            })
            .transpose()
        })
        .unwrap_or_else(|e| {
            log::warn!("Horizon: could not read the poi record of chunk {x} {z}: {e}");
            None
        });
    let tag = match (encode_poi(chunk, tables, professions.as_deref()), old) {
        (Some(mut tag), Some(old)) => {
            keep_claims(&mut tag, &old);
            tag
        }
        (Some(tag), None) => tag,
        (None, Some(_)) => poi_root(Compound::new()),
        (None, None) => return None,
    };
    Some(Record::Tag(Arc::new(tag)))
}
fn read_region_record(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    x: i32,
    z: i32,
) -> Result<Option<NamedTag>> {
    let key = (x >> 5, z >> 5);
    let path = region_path(root, folder, key.0, key.1);
    let Some(region) = cached_region(cache, key, &path, false)? else {
        return Ok(None);
    };
    let record = region
        .lock()
        .map_err(|_| "region file poisoned")?
        .read(ChunkPos::new(x, z))?;
    Ok(match record {
        Some(r) => Some(r.nbt(Limits::saved())?),
        None => None,
    })
}
/// The `__context` the chunk fixes read: the dimension and its generator type
/// (`ChunkMap.getChunkDataFixContextTag`).
struct DatafixContext {
    dimension: String,
    generator: Option<String>,
}
impl DatafixContext {
    /// From a dimension folder made by [`dimension_dir`] and the world's
    /// `world_gen_settings.dat`; None for a folder outside a world's `dimensions/`.
    fn of(root: &Path) -> Option<Self> {
        let mut parts = Vec::new();
        let mut at = root;
        while at.file_name()? != "dimensions" {
            parts.push(at.file_name()?.to_str()?);
            at = at.parent()?;
        }
        let world = at.parent()?;
        let namespace = parts.pop()?;
        if parts.is_empty() {
            return None;
        }
        parts.reverse();
        let dimension = format!("{namespace}:{}", parts.join("/"));
        let generator = horizon_world::datafix::files::generators(world)
            .ok()
            .and_then(|g| g.into_iter().find(|(d, _)| *d == dimension))
            .map(|(_, g)| g);
        Some(Self {
            dimension,
            generator,
        })
    }
}
/// `SimpleRegionStorage.upgradeChunkTag`, as `ChunkMap.upgradeChunkTag` (chunks),
/// `EntityStorage` and `SectionStorage.PackedChunk.parse` (POI) call it on every
/// record they read: a world 26.3 has opened keeps the chunks nobody loaded since
/// an older version in that version's format, and `upgrade_older_world` only runs
/// for an older level.dat. A record without a DataVersion counts as -1 (POI: 1945).
/// `context` is needed only for an older chunk; without one it stays as saved.
fn upgrade_record<'a>(
    folder: Folder,
    record: &mut NamedTag,
    context: impl FnOnce() -> Option<&'a DatafixContext>,
) -> Result<()> {
    use horizon_world::datafix::{self, schema::Ty};
    let default = if folder == Folder::Poi { 1945 } else { -1 };
    if number(record.tag.get("DataVersion")).unwrap_or(default) >= i64::from(DATA_VERSION) {
        return Ok(());
    }
    let context = match folder {
        Folder::Region => match context() {
            Some(c) => Some(c),
            None => return Ok(()),
        },
        _ => None,
    };
    let tag = &mut record.tag;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (folder, context) {
        (Folder::Region, Some(c)) => {
            datafix::update_chunk(tag, &c.dimension, c.generator.as_deref());
        }
        (Folder::Entities, _) => datafix::update_region_tag(Ty::EntityChunk, tag),
        (Folder::Poi, _) => datafix::update_region_tag(Ty::PoiChunk, tag),
        (Folder::Region, None) => {}
    }))
    .map_err(|_| "upgrading an older record failed".into())
}
/// Chunk and entity records waiting for the writer thread, by (entities?,
/// x, z): reads see them before the region files do.
type PendingWrites = Mutex<HashMap<(Folder, i32, i32), (u64, Record)>>;
type WriteBatch = (Folder, Vec<((i32, i32), Record, u64)>);
/// The region folders of a dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Folder {
    Region,
    Entities,
    Poi,
}
impl Folder {
    fn of(name: &str) -> Self {
        match name {
            "entities" => Self::Entities,
            "poi" => Self::Poi,
            _ => Self::Region,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Region => "region",
            Self::Entities => "entities",
            Self::Poi => "poi",
        }
    }
}

/// One dimension folder (`dimensions/minecraft/<name>`). Region files are
/// read lazily by worker threads.
pub struct SaveDir {
    pub root: PathBuf,
    regions: Arc<RegionCache>,
    entity_regions: Arc<RegionCache>,
    poi_regions: Arc<RegionCache>,
    pending: Arc<PendingWrites>,
    sequence: AtomicU64,
    writer: Mutex<Option<Writer>>,
    outstanding: Arc<Outstanding>,
    /// Set by `Persistent` before the first load.
    pub tables: OnceLock<Arc<SaveTables>>,
    professions: std::sync::RwLock<Option<Arc<crate::mobs::villager::ResolvedProfessions>>>,
    trusted_fluid_load: bool,
    /// The dimension's skylight and height, from its folder name.
    pub shape: LevelShape,
    /// The region file headers kept for far readers (`far_record`).
    far_regions: far_regions::FarRegions,
    /// Saved chunks decoded for far terrain, shared by its sampling threads
    /// (`lod_surfaces_shared`), oldest first.
    lod_decoded: Mutex<LodDecoded>,
    /// What older chunks are upgraded with, found on the first one read.
    datafix_context: OnceLock<Option<DatafixContext>>,
    /// The Overworld's old-world blending cache, read when a save is requested
    /// (installed where `SavedOldWorld` is made for this same folder).
    blending_lookup: OnceLock<BlendingLookup>,
    /// Byte budget of the canonical section palettes its saves leave cached
    /// on resident sections. Its own until [`Self::share_section_palettes`]
    /// joins it to another level's folder of the same world.
    section_palettes: Arc<SavedPaletteBudget>,
}
fn intern_saved_fluids(root: &NamedTag) {
    if let Some(Tag::List(entities)) = root.tag.get("block_entities") {
        for entity in entities {
            if let Some(Tag::List(tanks)) = entity.get("fluids") {
                for tank in tanks {
                    if let Some(Tag::String(id)) = tank.get("Fluid") {
                        mod_api::__private::intern_unresolved_fluid(&id.to_string_lossy());
                    }
                }
            }
        }
    }
}

fn mod_ticket_to_nbt(ticket: &ModTicket, controller: &str) -> Tag {
    let owner = match ticket.owner {
        TicketOwner::Block(pos) => ("block", Tag::IntArray(pos.to_vec())),
        TicketOwner::Entity(uuid) => (
            "entity",
            Tag::IntArray((0..4).map(|i| (uuid >> (96 - 32 * i)) as i32).collect()),
        ),
    };
    Tag::compound([
        ("controller", Tag::String(controller.into())),
        ("owner", Tag::compound([owner])),
        (
            "chunk_pos",
            Tag::IntArray(vec![ticket.chunk.0, ticket.chunk.1]),
        ),
        (
            "natural_spawning",
            Tag::Byte(i8::from(ticket.natural_spawning)),
        ),
    ])
}
/// A saved ticket: the id of the controller that held it, and the ticket without its slot.
fn mod_ticket_from_nbt(tag: &Tag) -> Option<(String, ModTicket)> {
    let Some(Tag::String(controller)) = tag.get("controller") else {
        return None;
    };
    let owner = tag.get("owner")?;
    let owner = match (owner.get("block"), owner.get("entity")) {
        (Some(Tag::IntArray(p)), _) if p.len() == 3 => TicketOwner::Block([p[0], p[1], p[2]]),
        (_, Some(Tag::IntArray(u))) if u.len() == 4 => TicketOwner::Entity(
            u.iter()
                .fold(0u128, |acc, &n| acc << 32 | u128::from(n as u32)),
        ),
        _ => return None,
    };
    let Some(Tag::IntArray(pos)) = tag.get("chunk_pos") else {
        return None;
    };
    let controller = controller.to_string().ok()?;
    (pos.len() == 2).then(|| {
        let ticket = ModTicket {
            controller: 0,
            owner,
            chunk: (pos[0], pos[1]),
            // Tickets saved without the flag don't force natural spawning.
            natural_spawning: matches!(tag.get("natural_spawning"), Some(Tag::Byte(b)) if *b != 0),
        };
        (controller, ticket)
    })
}
fn is_forced_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:forced"))
}
fn is_portal_ticket(ticket: &Tag) -> bool {
    matches!(ticket.get("type"), Some(Tag::String(t)) if *t == JavaString::from("minecraft:portal"))
}
#[derive(Default)]
struct LodDecoded {
    chunks: crate::fast_hash::FxHashMap<(i32, i32), (std::time::Instant, LodSurfaces)>,
    order: std::collections::VecDeque<(i32, i32)>,
}
/// Saved chunks kept decoded for far terrain (each tens of kilobytes), and
/// for how long: edits saved later show after it.
const LOD_DECODED: usize = 1024;
const LOD_DECODED_TTL: std::time::Duration = std::time::Duration::from_secs(30);
impl SaveDir {
    /// This dimension's server-owned mod attachments: `data/horizon/attachments.dat`, Horizon's
    /// own namespace. A file that doesn't read is moved aside to `attachments.dat.corrupt`, with
    /// a warning, and the level opens without them (as vanilla's `SavedDataStorage` starts a
    /// saved data over when its file is broken), so the next save can't overwrite it.
    pub fn load_attachments(&self) -> horizon_mod_runtime::attachments::Attachments {
        let path = self.attachments_path();
        let data = match try_read_dat(&path) {
            Err(DatIssue::Missing) => return Default::default(),
            Ok(root) => match root.get("data") {
                Some(Tag::Compound(data)) => Ok(data.clone()),
                _ => Err("no data compound".to_owned()),
            },
            Err(DatIssue::Unreadable(why)) => Err(why),
        };
        data.map(horizon_mod_runtime::attachments::Attachments::from_nbt)
            .unwrap_or_else(|why| {
                let aside = path.with_extension("dat.corrupt");
                log::warn!(
                    "{}: {why}; mod attachments start over, the file is kept as {}",
                    path.display(),
                    aside.display()
                );
                let _ = fs::rename(&path, &aside);
                Default::default()
            })
    }

    /// Save this dimension's server-owned mod attachments; nothing while it has none and never had.
    pub fn save_attachments(
        &self,
        attachments: &horizon_mod_runtime::attachments::Attachments,
    ) -> Result<()> {
        let path = self.attachments_path();
        let data = attachments.to_nbt();
        if data.is_empty() && !path.exists() {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::Compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }

    fn attachments_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("horizon")
            .join("attachments.dat")
    }

    /// The saves of a level of `shape` at `root` (its dimension folder).
    pub fn new(root: PathBuf, shape: LevelShape) -> Self {
        Self {
            root,
            regions: Default::default(),
            entity_regions: Default::default(),
            poi_regions: Default::default(),
            pending: Default::default(),
            sequence: AtomicU64::new(0),
            writer: Mutex::new(None),
            outstanding: Default::default(),
            tables: OnceLock::new(),
            professions: Default::default(),
            trusted_fluid_load: false,
            shape,
            far_regions: Default::default(),
            lod_decoded: Default::default(),
            datafix_context: OnceLock::new(),
            blending_lookup: OnceLock::new(),
            section_palettes: section_nbt::new_budget(),
        }
    }
    /// Mark this as a server-owned world save, not a remote cache.
    pub fn enable_trusted_fluid_load(&mut self) {
        self.trusted_fluid_load = true;
    }
    /// Cache section palettes under `other`'s budget, so the levels of one
    /// world share one cap instead of one each. Set before this folder saves.
    pub fn share_section_palettes(&mut self, other: &SaveDir) {
        self.section_palettes = Arc::clone(&other.section_palettes);
    }
    /// Lets saves of this folder take an old chunk's calculated blending data
    /// (`lookup` must not calculate). Set once, before the folder saves.
    pub fn set_blending_lookup(&self, lookup: BlendingLookup) {
        let _ = self.blending_lookup.set(lookup);
    }
    /// The calculated blending data of the old chunk at `x, z`, if this folder
    /// has a lookup and its cache holds one.
    pub fn calculated_blending(&self, x: i32, z: i32) -> Option<Arc<BlendingData>> {
        self.blending_lookup.get().and_then(|lookup| lookup(x, z))
    }
    /// 26.3's TicketStorage: persistent tickets live in each dimension's
    /// data/minecraft/chunk_tickets.dat, with ChunkPos.CODEC's [x, z] int array.
    pub fn load_forced_chunks(&self) -> HashSet<(i32, i32)> {
        let Some(data) = read_saved_data(&self.root, "chunk_tickets") else {
            return HashSet::new();
        };
        let Some(Tag::List(tickets)) = data.get("tickets") else {
            return HashSet::new();
        };
        tickets
            .iter()
            .filter(|t| is_forced_ticket(t))
            .filter_map(|t| match t.get("chunk_pos")? {
                Tag::IntArray(pos) if pos.len() == 2 => Some((pos[0], pos[1])),
                _ => None,
            })
            .collect()
    }
    /// The saved `PORTAL` tickets, for `TicketStorage.activateAllDeactivatedTickets`
    /// (`Ticket.CODEC`: a non-negative level, `ticks_left` 0 when missing).
    pub fn load_portal_tickets(&self) -> Vec<crate::ticket_levels::PortalTicket> {
        let Some(Tag::List(tickets)) =
            read_saved_data(&self.root, "chunk_tickets").and_then(|d| d.get("tickets").cloned())
        else {
            return Vec::new();
        };
        tickets
            .iter()
            .filter(|t| is_portal_ticket(t))
            .filter_map(|t| {
                let chunk = match t.get("chunk_pos")? {
                    Tag::IntArray(pos) if pos.len() == 2 => (pos[0], pos[1]),
                    _ => return None,
                };
                let level = match t.get("level")? {
                    &Tag::Int(level) if level >= 0 => level,
                    _ => return None,
                };
                let ticks_left = match t.get("ticks_left") {
                    None => 0,
                    Some(&Tag::Long(n)) => n,
                    Some(&Tag::Int(n)) => n.into(),
                    Some(&Tag::Short(n)) => n.into(),
                    Some(&Tag::Byte(n)) => n.into(),
                    Some(_) => return None,
                };
                Some(crate::ticket_levels::PortalTicket {
                    chunk,
                    level,
                    ticks_left,
                })
            })
            .collect()
    }
    /// `TicketStorage.packTickets`: the forced chunks and the `PORTAL` tickets
    /// the level holds now, in place of the ones saved before.
    pub fn save_chunk_tickets(
        &self,
        forced: &HashSet<(i32, i32)>,
        portal: &[crate::ticket_levels::PortalTicket],
    ) -> Result<()> {
        if forced.is_empty()
            && portal.is_empty()
            && !saved_data_path(&self.root, "chunk_tickets").exists()
        {
            return Ok(());
        }
        let mut data = compound_or_empty(read_saved_data(&self.root, "chunk_tickets"));
        // Preserve persistent tickets Horizon does not simulate.
        let mut tickets = match data.get("tickets") {
            Some(Tag::List(tickets)) => tickets
                .iter()
                .filter(|t| !is_forced_ticket(t) && !is_portal_ticket(t))
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let mut chunks: Vec<_> = forced.iter().copied().collect();
        chunks.sort_unstable();
        tickets.extend(chunks.into_iter().map(|(x, z)| {
            Tag::compound([
                ("type", Tag::String("minecraft:forced".into())),
                ("level", Tag::Int(31)), // ChunkMap.FORCED_TICKET_LEVEL: ENTITY_TICKING.
                ("chunk_pos", Tag::IntArray(vec![x, z])),
            ])
        }));
        tickets.extend(portal.iter().map(|t| {
            let mut ticket = Tag::compound([
                ("type", Tag::String("minecraft:portal".into())),
                ("level", Tag::Int(t.level)),
                ("chunk_pos", Tag::IntArray(vec![t.chunk.0, t.chunk.1])),
            ]);
            // `optionalFieldOf("ticks_left", 0L)` leaves out its default.
            if t.ticks_left != 0 {
                put(&mut ticket, "ticks_left", Tag::Long(t.ticks_left));
            }
            ticket
        }));
        put(&mut data, "tickets", Tag::List(tickets));
        write_saved_data(&self.root, "chunk_tickets", data)
    }
    /// The chunk tickets of mods (`data/horizon/mod_chunk_tickets.dat`), kept
    /// out of vanilla's `chunk_tickets.dat` because vanilla decodes its ticket
    /// types through a registry. Each is saved under its controller's id;
    /// `slot_of` gives the slot of an id a loaded mod registered. A ticket of
    /// any other id (a mod that is gone, or a file from before controllers) is
    /// dropped with a warning, and one that doesn't read is skipped.
    pub fn load_mod_tickets(&self, slot_of: impl Fn(&str) -> Option<u32>) -> Vec<ModTicket> {
        let Some(root) = read_dat(&self.mod_tickets_path()) else {
            return Vec::new();
        };
        let Some(Tag::List(tickets)) = root.get("data").and_then(|data| data.get("tickets")) else {
            return Vec::new();
        };
        let mut dropped = std::collections::BTreeSet::new();
        let loaded = tickets
            .iter()
            .filter_map(|tag| {
                let Some((controller, ticket)) = mod_ticket_from_nbt(tag) else {
                    log::warn!("Dropping a saved mod chunk ticket that has no readable controller");
                    return None;
                };
                let Some(slot) = slot_of(&controller) else {
                    dropped.insert(controller);
                    return None;
                };
                Some(ModTicket {
                    controller: slot,
                    ..ticket
                })
            })
            .collect();
        for controller in dropped {
            log::warn!(
                "Dropping the saved chunk tickets of ticket controller {controller}: no loaded mod registers it"
            );
        }
        loaded
    }
    /// Save the tickets of mods under their controllers' ids (`id_of` gives the id of a slot);
    /// nothing while there are none and never were.
    pub fn save_mod_tickets(
        &self,
        tickets: &ModTickets,
        id_of: impl Fn(u32) -> Option<&'static str>,
    ) -> Result<()> {
        let path = self.mod_tickets_path();
        let list: Vec<Tag> = tickets
            .to_save()
            .filter_map(|t| Some(mod_ticket_to_nbt(t, id_of(t.controller)?)))
            .collect();
        if list.is_empty() && !path.exists() {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::compound([("tickets", Tag::List(list))])),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }
    fn mod_tickets_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("horizon")
            .join("mod_chunk_tickets.dat")
    }
    fn region_path(&self, folder: &str, rx: i32, rz: i32) -> PathBuf {
        region_path(&self.root, folder, rx, rz)
    }
    /// The chunks of a region file that an older version generated, as
    /// `IOWorker.createOldDataForRegion` finds them (bit `x + z * 32`): saved
    /// before DataVersion 4997, or with `blending_data`.
    pub fn old_chunk_bits(&self, rx: i32, rz: i32) -> [u64; 16] {
        let mut bits = [0u64; 16];
        let Ok(region) = RegionFile::open(self.region_path("region", rx, rz)) else {
            return bits;
        };
        let present: Vec<ChunkPos> = region.chunks().map(|(pos, _)| pos).collect();
        drop(region);
        for pos in present {
            // `IOWorker` reads the record as saved, before any upgrade.
            let Some(root) = self.read_record_raw("region", pos.x, pos.z) else {
                continue;
            };
            if record_is_old(&root) {
                let index = ((pos.x & 31) + (pos.z & 31) * 32) as usize;
                bits[index / 64] |= 1 << (index % 64);
            }
        }
        bits
    }
    /// Whether the saved chunk at `x, z` is old ([`Self::old_chunk_bits`]'s test), from its
    /// record alone: false for a chunk not saved.
    pub fn is_old_chunk(&self, x: i32, z: i32) -> bool {
        self.read_record_raw("region", x, z)
            .is_some_and(|root| record_is_old(&root))
    }
    /// A saved chunk brought to the current version, for the readers of its
    /// fields that `load` does not decode (`blending_data`).
    pub fn raw_chunk(&self, x: i32, z: i32) -> Option<NamedTag> {
        self.read_record("region", x, z)
    }
    /// The references a saved chunk keeps for its structure start `name`
    /// (`StructureStart.getReferences`, saved as `references`): 0 for a chunk
    /// not saved yet, a start it does not have or one without the field.
    pub fn start_references(&self, x: i32, z: i32, name: &str) -> i32 {
        let Some(root) = self.read_record("region", x, z) else {
            return 0;
        };
        match root
            .tag
            .get("structures")
            .and_then(|s| s.get("starts"))
            .and_then(|s| s.get(name))
            .and_then(|s| s.get("references"))
        {
            Some(Tag::Int(n)) => *n,
            _ => 0,
        }
    }
    /// A record as saved: the pending write first, then the region file.
    fn read_record_raw(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        self.try_read_record_raw(folder, x, z).ok().flatten()
    }
    /// `read_record_raw`, telling a record that isn't there (`Ok(None)`) from a read that failed
    /// (`Err`, already reported): the second may succeed on a later read.
    fn try_read_record_raw(
        &self,
        folder: &str,
        x: i32,
        z: i32,
    ) -> std::result::Result<Option<NamedTag>, ()> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| ())?
            .get(&(Folder::of(folder), x, z))
            .map(|(_, r)| r.clone());
        if let Some(record) = pending {
            return Ok(record.tag().map(|t| (*t).clone()));
        }
        let cache = match Folder::of(folder) {
            Folder::Region => &self.regions,
            Folder::Entities => &self.entity_regions,
            Folder::Poi => &self.poi_regions,
        };
        read_region_record(&self.root, cache, folder, x, z)
            .inspect(|record| {
                if self.trusted_fluid_load
                    && folder == "region"
                    && let Some(root) = record
                {
                    intern_saved_fluids(root);
                }
            })
            .map_err(|e| {
                crate::chunk_io::report_load_failure(&self.root, x, z, &e);
            })
    }
    /// A record brought to the current version (`upgrade_record`); `Err` when that failed.
    fn try_read_record(
        &self,
        folder: &str,
        x: i32,
        z: i32,
    ) -> std::result::Result<Option<NamedTag>, ()> {
        let Some(mut record) = self.try_read_record_raw(folder, x, z)? else {
            return Ok(None);
        };
        match self.upgrade_record(Folder::of(folder), &mut record) {
            Ok(()) => Ok(Some(record)),
            Err(e) => {
                crate::chunk_io::report_load_failure(&self.root, x, z, &e);
                Err(())
            }
        }
    }
    /// A record brought to the current version (`upgrade_record`).
    fn read_record(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        self.try_read_record(folder, x, z).ok().flatten()
    }
    fn upgrade_record(&self, folder: Folder, record: &mut NamedTag) -> Result<()> {
        upgrade_record(folder, record, || {
            self.datafix_context
                .get_or_init(|| DatafixContext::of(&self.root))
                .as_ref()
        })
    }
    pub fn load(&self, x: i32, z: i32, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
        self.try_load(x, z, registry, biomes).ok().flatten()
    }
    /// `load`, telling a chunk that isn't saved as a full chunk (`Ok(None)`) from a read that
    /// failed (`Err`): a later read may succeed.
    fn try_load(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> std::result::Result<Option<Chunk>, ()> {
        let Some(root) = self.try_read_record("region", x, z)? else {
            return Ok(None);
        };
        let tables = self.tables.get().map(|t| &**t);
        let Some(mut chunk) = decode_with(&root, registry, biomes, tables, self.shape.column())
        else {
            return Ok(None);
        };
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        chunk.entities = horizon_world::chunk::legacy_entities(&root.tag);
        Ok(Some(chunk))
    }
    /// A saved full chunk read for a player's far ring (`lan::far`), as `load` but past the gameplay
    /// region cache (`far_record`).
    pub fn load_far(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<Chunk> {
        let root = self.far_record(x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        let mut chunk = decode_with(&root, registry, biomes, tables, self.shape.column())?;
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        // Without its saved light (`isLightOn`) it would need the light engine: not one for the far ring.
        chunk.light_loaded.then_some(chunk)
    }
    /// Chunks queued for the writer thread and not yet in their region files.
    pub fn pending_writes(&self) -> usize {
        self.pending.lock().map_or(0, |p| p.len())
    }
    /// A proto chunk saved before it became final (`decode_proto`).
    pub fn load_proto(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<SavedProto> {
        let root = self.read_record("region", x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        decode_proto(&root, registry, biomes, tables, self.shape.column())
    }
    /// Queue proto chunks for the writer thread; loads see them meanwhile.
    pub fn save_protos(
        &self,
        protos: Vec<SavedProto>,
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<()> {
        biomes.ensure_saveable()?;
        let encoding = Arc::new(Encoding {
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
            section_palettes: None,
        });
        let records = protos
            .into_iter()
            .map(|p| {
                (
                    (p.column.x, p.column.z),
                    Record::Proto(Arc::new(p), encoding.clone()),
                )
            })
            .collect();
        self.queue(Folder::Region, records, false)
    }
    /// Entities saved for a chunk in `entities/` (an old chunk's own list: [`Self::load`]).
    pub fn load_entities(&self, x: i32, z: i32) -> Vec<Tag> {
        let mut out = Vec::new();
        if let Some(root) = self.read_record("entities", x, z)
            && let Some(Tag::List(list)) = root.tag.get("Entities")
        {
            out.extend(list.iter().cloned());
        }
        out
    }
    /// Write `chunks` into their region files and wait until they are on disk.
    pub fn save(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<usize> {
        biomes.ensure_saveable()?;
        let tables = self.tables.get().map(|t| &**t);
        let records = chunks
            .iter()
            .map(|c| {
                let blending = self.calculated_blending(c.x, c.z);
                (
                    (c.x, c.z),
                    Some(encode_saved(
                        c,
                        registry,
                        biomes,
                        tables,
                        self.shape,
                        blending.as_deref(),
                        Some(&self.section_palettes),
                    )),
                )
            })
            .collect();
        self.write_records("region", records, true)?;
        self.save_poi(chunks, true)?;
        Ok(chunks.len())
    }
    fn save_poi(&self, chunks: &[Arc<Chunk>], wait: bool) -> Result<()> {
        let Some(tables) = self.tables.get() else {
            return Ok(());
        };
        let professions = self
            .professions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let records = chunks
            .iter()
            .map(|c| {
                (
                    (c.x, c.z),
                    Record::Poi(c.clone(), tables.clone(), professions.clone()),
                )
            })
            .collect();
        self.queue(Folder::Poi, records, wait)
    }
    /// `save` on the writer thread: returns before the files are written;
    /// loads see the chunks meanwhile.
    pub fn save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<usize> {
        let encoding = self.encoding_for_save_later(chunks, registry, biomes)?;
        let records = chunks
            .iter()
            .map(|c| {
                let blending = self.calculated_blending(c.x, c.z);
                (
                    (c.x, c.z),
                    Record::Chunk(c.clone(), encoding.clone(), blending),
                )
            })
            .collect();
        self.queue(Folder::Region, records, false)?;
        self.save_poi(chunks, false)?;
        Ok(chunks.len())
    }
    /// Write the entity chunks (`EntityStorage`): an empty list deletes it.
    pub fn save_entities(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.write_entities(chunks, true)
    }
    /// `save_entities` on the writer thread.
    pub fn save_entities_later(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.write_entities(chunks, false)
    }
    fn write_entities(&self, chunks: Vec<((i32, i32), Vec<Tag>)>, wait: bool) -> Result<()> {
        let records = chunks
            .into_iter()
            .map(|((x, z), entities)| {
                let tag = (!entities.is_empty()).then(|| NamedTag {
                    name: "".into(),
                    tag: Tag::compound([
                        ("DataVersion", Tag::Int(DATA_VERSION)),
                        ("Position", Tag::IntArray(vec![x, z])),
                        ("Entities", Tag::List(entities)),
                    ]),
                });
                ((x, z), tag)
            })
            .collect();
        self.write_records("entities", records, wait)
    }
}

pub fn read_dat(path: &Path) -> Option<Tag> {
    try_read_dat(path).ok()
}
/// `read_dat`, with why the file did not read.
fn try_read_dat(path: &Path) -> std::result::Result<Tag, DatIssue> {
    let bytes = fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DatIssue::Missing,
        _ => DatIssue::Unreadable(e.to_string()),
    })?;
    let raw = match Compression::Gzip.decode(&bytes, Limits::saved().max_bytes) {
        Ok(raw) => raw,
        Err(_) => bytes,
    };
    crate::recipe_book::read_saved(&raw)
        .map(|(named, _)| named.tag)
        .map_err(|e| DatIssue::Unreadable(e.to_string()))
}
/// What `LevelStorageAccess.collectIssues` finds wrong with a level.dat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatIssue {
    /// No such file (`FileNotFoundException`).
    Missing,
    /// It exists but does not read as a level: why.
    Unreadable(String),
}
/// `LevelStorageAccess.collectIssues`: why level.dat (level.dat_old with
/// `old`) can't be read as the world's level data, `None` when it can.
pub fn level_dat_issue(dir: &Path, old: bool) -> Option<DatIssue> {
    let file = if old { "level.dat_old" } else { "level.dat" };
    match try_read_dat(&dir.join(file)) {
        Ok(root) if matches!(root.get("Data"), Some(Tag::Compound(_))) => None,
        Ok(_) => Some(DatIssue::Unreadable("Invalid root tag".into())),
        Err(issue) => Some(issue),
    }
}
/// `WorldOpenFlows.openWorldLoadLevelData`: what keeps `world`'s level data from reading, which
/// opens RecoverWorldDataScreen instead of the world. A folder with neither level.dat nor
/// level.dat_old is a world not made yet, not a broken one.
pub fn level_data_needs_recovery(world: &Path) -> Option<DatIssue> {
    match level_dat_issue(world, false)? {
        DatIssue::Missing if !world.join("level.dat_old").exists() => None,
        issue => Some(issue),
    }
}
/// `LevelStorageAccess.restoreLevelDataFromOld` (`Util.safeReplaceOrMoveFile`):
/// level.dat moves aside to `level.dat_corrupted_<time>` and level.dat_old
/// takes its place. Whether it worked. Vanilla passes `noRollback` and reports
/// success even when level.dat_old could not be moved in; here the broken
/// level.dat goes back and the failure is reported.
pub fn restore_level_data_from_old(dir: &Path) -> bool {
    let (current, latest) = (dir.join("level.dat"), dir.join("level.dat_old"));
    let backup = dir.join(format!("level.dat_corrupted_{}", backup_stamp()));
    if current.exists() && fs::rename(&current, &backup).is_err() {
        return false;
    }
    if fs::rename(&latest, &current).is_err() {
        if backup.exists() {
            let _ = fs::rename(&backup, &current);
        }
        return false;
    }
    true
}
/// `FileNameDateFormatter`: the time in the names of backed-up unreadable files.
fn backup_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string()
}
/// `LevelStorageAccess.getUnfixedDataTagWithFallback`, which the dedicated server's `Main` calls:
/// a level.dat that does not read falls back to level.dat_old, which is restored in its place
/// (`restoreLevelDataFromOld`). Whether it did; `false` leaves the world as it was, for the
/// caller to refuse (both files broken, or nothing to fall back to).
pub fn fall_back_to_level_dat_old(world: &Path) -> bool {
    level_dat_issue(world, false).is_some()
        && world.join("level.dat_old").exists()
        && level_dat_issue(world, true).is_none()
        && restore_level_data_from_old(world)
}
/// `LevelStorageSource.readLevelDataTagFixed`: level.dat, else level.dat_old.
fn read_level_dat(dir: &Path) -> Option<Tag> {
    read_dat(&dir.join("level.dat")).or_else(|| read_dat(&dir.join("level.dat_old")))
}
/// `restoreLevelDataFromOld`: an unreadable level.dat is kept as
/// `level.dat_corrupted_<time>` and level.dat_old takes its place, so the next
/// write does not bury either file.
fn restore_level_dat(dir: &Path) {
    let path = dir.join("level.dat");
    if read_dat(&path).is_some() {
        return;
    }
    if path.exists() {
        let _ = fs::rename(
            &path,
            dir.join(format!("level.dat_corrupted_{}", backup_stamp())),
        );
    }
    let old = dir.join("level.dat_old");
    if read_dat(&old).is_some() {
        let _ = fs::rename(old, path);
    }
}
/// `PlayerDataStorage.backup`: a player file that fails to load is copied to
/// `<uuid>_corrupted_<time>.dat` before it is written over.
fn back_up_unreadable_player(path: &Path) {
    if path.is_file()
        && read_dat(path).is_none()
        && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
    {
        let _ = fs::copy(
            path,
            path.with_file_name(format!("{stem}_corrupted_{}.dat", backup_stamp())),
        );
    }
}
/// Write a gzip NBT file the way vanilla does (`<name>_new`, keep `_old`).
pub fn write_dat(path: &Path, tag: Tag) -> Result<()> {
    write_dat_as(path, tag, false)
}

/// Validate the NBT that a reload will see before replacing either file.
pub fn write_dat_checked(
    path: &Path,
    tag: Tag,
    validate: impl FnOnce(&Tag, &Tag) -> bool,
) -> Result<bool> {
    let root = NamedTag {
        name: "".into(),
        tag,
    };
    let Ok(bytes) = nbt::write_named(&root, Limits::saved()) else {
        return Ok(false);
    };
    let Ok((saved, _)) = crate::recipe_book::read_saved(&bytes) else {
        return Ok(false);
    };
    if !validate(&root.tag, &saved.tag) {
        return Ok(false);
    }
    write_dat_bytes(path, &bytes, false)?;
    Ok(true)
}

/// [`write_dat`], synced to the disk: the new file before it is renamed into place, and the
/// folder after, so a power loss leaves the old file or the new one. A hand-over's commit
/// point (`player-confirm`) replies only once its files are.
pub fn write_dat_durable(path: &Path, tag: Tag) -> Result<()> {
    write_dat_as(path, tag, true)
}

/// `bytes` written to `path` through `<name>_new` and a rename, synced to the disk as
/// [`write_dat_durable`] syncs: never a half-written file in place.
pub fn write_durable(path: &Path, bytes: &[u8]) -> Result<()> {
    Ok(write_durable_io(path, bytes)?)
}

/// [`write_durable`], keeping the operating system's error (its kind and code).
pub fn write_durable_io(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = path.file_name().and_then(|n| n.to_str()).unwrap_or("data");
    let temporary = path.with_file_name(format!("{file}_new"));
    write_synced(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    sync_folder(path)
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The folder of `path` synced, so that a rename into it is on the disk (POSIX; Windows has
/// no folder handle to sync, and its rename is journaled by NTFS).
fn sync_folder(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        // A bare file name's parent is "": the working directory.
        let dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir
        };
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_dat_as(path: &Path, tag: Tag, durable: bool) -> Result<()> {
    let bytes = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag,
        },
        Limits::saved(),
    )
    .map_err(|e| e.to_string())?;
    write_dat_bytes(path, &bytes, durable)
}

fn write_dat_bytes(path: &Path, bytes: &[u8], durable: bool) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let compressed = Compression::Gzip.encode(bytes)?;
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.dat");
    let temporary = path.with_file_name(format!("{file}_new"));
    if durable {
        write_synced(&temporary, &compressed)?;
    } else {
        fs::write(&temporary, compressed)?;
    }
    if path.exists() {
        let old = match file.strip_suffix(".dat") {
            Some(stem) => path.with_file_name(format!("{stem}.dat_old")),
            None => path.with_file_name(format!("{file}_old")),
        };
        // Util.safeReplaceFile: no backup, no replacement.
        fs::rename(path, old).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    fs::rename(temporary, path)?;
    if durable {
        sync_folder(path)?;
    }
    Ok(())
}
mod saved_data;
use saved_data::{read_saved_data, saved_data_path, write_saved_data};
/// The world's generator settings: world_gen_settings.dat (26.3) or
/// level.dat's Data.WorldGenSettings.
pub fn world_gen_settings(world: &Path) -> Option<Tag> {
    // A world still to be file-fixed has them in level.dat in an older shape: they are read as
    // the fixes would leave them, so the questions on opening it are the same ones.
    if let Ok(Some(settings)) = horizon_world::datafix::files::current_world_gen_settings(world) {
        return Some(settings);
    }
    read_saved_data(world, "world_gen_settings").or_else(|| {
        read_dat(&world.join("level.dat"))?
            .get("Data")?
            .get("WorldGenSettings")
            .cloned()
    })
}
include!("save/preset_saved.rs");
pub use horizon_client_core::uuid::{uuid_from_text, uuid_text};
/// A random (version 4) UUID as vanilla's four-int form.
///
/// Every call gets its own: the generator is seeded once from the clock and
/// the process, then each call takes the next step of one shared counter. A
/// seed from the clock alone repeats within its resolution (a microsecond on
/// macOS): a pack of mobs generated together all had the same UUID, and a
/// vanilla client keeps only one entity per UUID (the rest never moved).
pub fn random_uuid() -> [i32; 4] {
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static NEXT: OnceLock<AtomicU64> = OnceLock::new();
    let next = NEXT.get_or_init(|| {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        AtomicU64::new(t ^ (std::process::id() as u64).rotate_left(32))
    });
    // SplitMix64: distinct steps give distinct outputs.
    let split = || {
        let mut z = next
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let (a, b) = (split(), split());
    let a = (a & !0xF000) | 0x4000;
    let b = (b & !(0xC000_0000_0000_0000)) | 0x8000_0000_0000_0000;
    [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
}
/// `Mth.createInsecureUUID`: a new entity's UUID is its own random's first two
/// draws (`Entity.uuid`).
pub fn insecure_uuid(random: &mut impl horizon_core::random::RandomSource) -> [i32; 4] {
    let most = random.next_long() & -61441 | 16384;
    let least = random.next_long() & 4_611_686_018_427_387_903 | i64::MIN;
    [
        (most >> 32) as i32,
        most as i32,
        (least >> 32) as i32,
        least as i32,
    ]
}
pub(crate) fn uuid_tag(u: [i32; 4]) -> Tag {
    Tag::IntArray(u.to_vec())
}
pub(crate) fn uuid_from_tag(tag: Option<&Tag>) -> Option<[i32; 4]> {
    match tag? {
        Tag::IntArray(v) if v.len() == 4 => Some([v[0], v[1], v[2], v[3]]),
        Tag::String(s) => uuid_from_text(&s.to_string_lossy()),
        _ => None,
    }
}
// `DoubleTag.valueOf` / `FloatTag.valueOf` store -0.0 as the shared +0.0
// `ZERO`, so a resting entity's `Motion` reads `0.0d`, never `-0.0d`.
pub(crate) fn doubles(values: &[f64]) -> Tag {
    Tag::List(
        values
            .iter()
            .map(|&v| Tag::Double(if v == 0.0 { 0 } else { v.to_bits() }))
            .collect(),
    )
}
pub(crate) fn floats(values: &[f32]) -> Tag {
    Tag::List(
        values
            .iter()
            .map(|&v| Tag::Float(if v == 0.0 { 0 } else { v.to_bits() }))
            .collect(),
    )
}
pub(crate) fn read_doubles<const N: usize>(tag: Option<&Tag>) -> Option<[f64; N]> {
    match tag? {
        Tag::List(l) if l.len() == N => {
            let mut out = [0.0; N];
            for (o, v) in out.iter_mut().zip(l) {
                *o = float(Some(v))?;
            }
            Some(out)
        }
        _ => None,
    }
}

/// The world presets of 26.3's `worldgen/world_preset` that the create
/// screen offers, in the order of `WorldPresetTags.NORMAL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldPreset {
    Normal,
    Flat,
    LargeBiomes,
    Amplified,
    SingleBiomeSurface,
    /// `minecraft:debug_all_block_states`: a grid of every block state
    /// (`DebugLevelSource`). The create screen offers it only with Alt held;
    /// a data pack, `level-type` or a saved world names it too.
    Debug,
}
impl WorldPreset {
    /// The presets, in the order the world-type button cycles through them.
    pub const ALL: [Self; 5] = [
        Self::Normal,
        Self::Flat,
        Self::LargeBiomes,
        Self::Amplified,
        Self::SingleBiomeSurface,
    ];
    /// The preset's registry id (`minecraft:normal`).
    pub const fn id(self) -> &'static str {
        match self {
            Self::Normal => "minecraft:normal",
            Self::Flat => "minecraft:flat",
            Self::LargeBiomes => "minecraft:large_biomes",
            Self::Amplified => "minecraft:amplified",
            Self::SingleBiomeSurface => "minecraft:single_biome_surface",
            Self::Debug => "minecraft:debug_all_block_states",
        }
    }
    /// The preset an id names, if it is one of them.
    pub fn of_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Debug])
            .find(|preset| preset.id() == id)
    }
    /// The language key the world-type button shows
    /// (`Identifier.toLanguageKey("generator")`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "generator.minecraft.normal",
            Self::Flat => "generator.minecraft.flat",
            Self::LargeBiomes => "generator.minecraft.large_biomes",
            Self::Amplified => "generator.minecraft.amplified",
            Self::SingleBiomeSurface => "generator.minecraft.single_biome_surface",
            Self::Debug => "generator.minecraft.debug_all_block_states",
        }
    }
    /// The preset the world-type button cycles to next.
    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
    /// The preset the world-type button cycles to previously.
    pub fn prev(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|preset| *preset == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
    /// The preset a saved overworld generator is: the noise settings name
    /// the preset, and the single-biome preset shares the overworld's noise
    /// settings, so its fixed biome source tells the two apart.
    pub fn of_generator(generator: &Tag) -> Self {
        if text(generator.get("type")).as_deref() == Some("minecraft:flat") {
            return Self::Flat;
        }
        if text(generator.get("type")).as_deref() == Some("minecraft:debug") {
            return Self::Debug;
        }
        let source = generator.get("biome_source");
        let fixed = text(source.and_then(|s| s.get("type"))).as_deref() == Some("minecraft:fixed");
        match text(generator.get("settings")).as_deref() {
            Some("minecraft:large_biomes") => Self::LargeBiomes,
            Some("minecraft:amplified") => Self::Amplified,
            Some("minecraft:overworld") if fixed => Self::SingleBiomeSurface,
            _ => Self::Normal,
        }
    }
}
/// World and player state stored in `level.dat`, the saved-data files and
/// the player file.
/// The world options CreateWorldScreen sets (`WorldOptions` and the world
/// preset) that `world_gen_settings` keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGen {
    /// The world preset the world is created with.
    pub preset: WorldPreset,
    pub structures: bool,
    pub bonus_chest: bool,
}
impl WorldGen {
    /// A saved world's choices, from its `world_gen_settings`.
    pub fn read(world: &Path) -> Option<Self> {
        let settings = world_gen_settings(world)?;
        let flag = |key: &str, default: bool| number(settings.get(key)).map_or(default, |v| v != 0);
        let generator = settings
            .get("dimensions")
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        Some(Self {
            preset: generator.map_or(WorldPreset::Normal, WorldPreset::of_generator),
            structures: flag("generate_structures", true),
            bonus_chest: flag("bonus_chest", false),
        })
    }
    /// The bundled world preset a saved world's overworld was generated
    /// with, from its generator: `minecraft:amplified` and
    /// `minecraft:large_biomes` by their noise settings, and
    /// `minecraft:single_biome_surface` by a fixed biome source. Anything
    /// else (and any world without settings) is `minecraft:normal`.
    pub fn preset(world: &Path) -> &'static str {
        let settings = world_gen_settings(world);
        let generator = settings
            .as_ref()
            .and_then(|s| s.get("dimensions"))
            .and_then(|d| d.get("minecraft:overworld"))
            .and_then(|o| o.get("generator"));
        let noise = text(generator.and_then(|g| g.get("settings")));
        let fixed_biome = generator
            .and_then(|g| g.get("biome_source"))
            .and_then(|b| text(b.get("type")))
            .is_some_and(|t| t == "minecraft:fixed");
        match noise.as_deref() {
            Some("minecraft:amplified") => "minecraft:amplified",
            Some("minecraft:large_biomes") => "minecraft:large_biomes",
            Some("minecraft:overworld") if fixed_biome => "minecraft:single_biome_surface",
            _ => "minecraft:normal",
        }
    }
}
impl Default for WorldGen {
    fn default() -> Self {
        Self {
            preset: WorldPreset::Normal,
            structures: true,
            bonus_chest: false,
        }
    }
}
/// Replaces the seed of an existing world's `world_gen_settings`, keeping its options and dimensions. A world with no
/// such file gets a default one with `seed`.
pub fn write_world_seed(dir: &Path, seed: i64) -> Result<()> {
    match read_saved_data(dir, "world_gen_settings") {
        Some(Tag::Compound(mut settings)) => {
            settings.insert("seed".into(), Tag::Long(seed));
            write_saved_data(dir, "world_gen_settings", Tag::Compound(settings))
        }
        _ => write_world_gen_settings(dir, seed, WorldGen::default()),
    }
}
/// Writes a new world's `world_gen_settings` (seed, options and dimensions).
pub fn write_world_gen_settings(dir: &Path, seed: i64, options: WorldGen) -> Result<()> {
    write_world_gen_settings_with(dir, seed, options, None, None, None)
}
include!("save/preset_generation.rs");
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub name: String,
    pub seed: i64,
    pub creative: bool,
    /// The game mode id (0 survival, 1 creative, 2 adventure, 3 spectator);
    /// `creative` wins when the two disagree.
    pub game_mode: i32,
    /// `previousPlayerGameType`: the mode the player left last (the F3+F4 switcher's
    /// return mode), when it has one.
    pub previous_game_mode: Option<i32>,
    pub time: i64,
    pub day_time: i64,
    /// The End's clock (`world_clocks.dat`'s `minecraft:the_end`); `None` in a world that has
    /// none yet, which starts it at the game time.
    pub end_clock: Option<i64>,
    /// The world spawn or a legacy bare Overworld spawn column. Personal
    /// points use `respawn_elsewhere` so their blocks are checked on death.
    pub spawn: Option<[f64; 3]>,
    /// A respawn point resolved through its block when the player respawns
    /// (a bed, a respawn anchor, a forced point in any dimension):
    /// dimension, block, yaw, pitch and `forced`.
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, f32, bool)>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub health: f32,
    /// LivingEntity `AbsorptionAmount`: the golden hearts left.
    pub absorption: f32,
    pub food: i32,
    pub saturation: f32,
    /// (slot, item, count, damage): 0..36 inventory, 100..103 armor (feet..head), 150 offhand,
    /// 151 body and 152 saddle (`BODY_SLOT`).
    pub inventory: Vec<(u8, String, u32, u32)>,
    /// Extra item components of `inventory` entries, by slot.
    pub inventory_components: HashMap<u8, Box<crate::item_data::StackData>>,
    /// Player.XpSeed: the enchanting table seed.
    pub xp_seed: i32,
    /// ServerPlayer.seenCredits: the end poem has been shown.
    pub seen_credits: bool,
    /// ServerPlayer.postEffects: the `post_effect` chains `/posteffect` set.
    pub post_effects: Vec<String>,
    /// Player.lastDeathLocation: the dimension and block of the last death.
    pub last_death: Option<(String, [i32; 3])>,
    /// Player.active_effects: (id, duration, amplifier, ambient,
    /// show_particles, show_icon).
    pub active_effects: Vec<crate::effects::SavedEffect>,
    /// Player.EnderItems: the ender chest inventory (27 slots).
    pub ender_items: Vec<Option<crate::items::ItemStack>>,
    /// The `recipeBook` compound: open/filter settings and unlocked recipes.
    pub recipe_book: crate::recipe_book::Settings,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    pub score: i32,
    pub weather: crate::world::Weather,
    /// Player.Dimension (minecraft:overworld / minecraft:the_nether).
    pub dimension: String,
    /// World spawn (`level.dat` `spawn`), distinct from a bed respawn point.
    pub world_spawn: Option<[i32; 3]>,
    /// The world spawn's yaw and pitch (`setworldspawn`'s rotation).
    pub world_spawn_angle: [f32; 2],
    /// The world spawn's dimension (`level.dat` `spawn.dimension`); the Overworld when unset.
    pub world_spawn_dimension: Option<String>,
    pub difficulty: String,
    pub hardcore: bool,
    /// `difficulty_settings.locked` (LevelData.isDifficultyLocked).
    pub difficulty_locked: bool,
    /// `allowCommands`: read from `level.dat` or chosen on the Create New
    /// World screen; unset is written as off.
    pub allow_commands: Option<bool>,
    /// `DataPackConfig` (Create World's Data Packs and Experiments): the
    /// enabled and disabled pack ids. Unset keeps vanilla's defaults.
    pub data_packs: Option<(Vec<String>, Vec<String>)>,
    /// `WorldDataConfiguration`'s `enabled_features`: the feature flag ids
    /// the world enables, `minecraft:vanilla` included. Empty is vanilla's
    /// default, which the level.dat omits.
    pub enabled_features: std::sync::Arc<[String]>,
    /// The code mods (`fml.LoadingModList` beside `Data`), then the content mods (`horizon.ContentModList`), the world was saved with; `None` without either list.
    pub saved_mods: Option<Vec<level_mods::ModRecord>>,
    /// Game rule values by name without namespace (`keep_inventory`, ...).
    pub game_rules: BTreeMap<String, i32>,
    /// Definitions used by this world session.
    pub rule_registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    pub player_uuid: Option<[i32; 4]>,
    pub selected_slot: i32,
    pub motion: [f64; 3],
    pub fall_distance: f64,
    pub on_ground: bool,
    /// `LivingEntity.addAdditionalSaveData`'s `FallFlying`: the player was
    /// gliding (shared flag 7), so the glide goes on after the load.
    pub fall_flying: bool,
    /// Current body's save projection. None before a live body supplies it;
    /// retained raw data stays intact until then.
    pub player_impulse_grace: Option<i32>,
    pub player_fall_particles: Option<bool>,
    pub food_exhaustion: f32,
    pub food_tick_timer: i32,
    /// Abilities.flying: whether creative/spectator flight is active.
    pub flying: bool,
    /// Abilities.flyingSpeed, when the file has one (vanilla's is 0.05).
    pub fly_speed: Option<f32>,
    /// The `abilities` compound of the player file (`Abilities.Packed`), as
    /// read and as a server player's stored abilities write it. When none,
    /// `player_tag` derives it from the game mode.
    pub abilities: Option<crate::lan::abilities::PlayerAbilities>,
    /// The vehicle the player rides, by UUID (`ServerPlayer.saveParentVehicle`'s
    /// "RootVehicle"), so the ride comes back on the next login.
    pub root_vehicle: Option<[i32; 4]>,
    /// The player compound as loaded, for fields Horizon does not model
    /// (ender chest, effects, abilities, recipe book, attributes, ...).
    pub player_extra: Option<Tag>,
    /// The player was read from the file level.dat names (`singleplayer_uuid`,
    /// or its `.dat_old`): `PlayerList.loadPlayerData` gives the world's owner
    /// that file, not their own.
    pub singleplayer_file: bool,
}

/// Whether the player file can hold a stack's saved compound, warning about the
/// stack it leaves out: the writer refuses a list or compound at
/// `Limits::saved().max_depth` (`Writer::payload`), and the player compound is
/// depth 0, so an `Inventory` or `EnderItems` element and an `equipment` value
/// sit at depth 2. [`nbt::within_writer_depth`] applies the writer's own rule,
/// including the synthetic layer it puts around a mixed-type list's elements.
fn stack_fits_the_file(kind: &str, tag: &Tag) -> bool {
    if nbt::within_writer_depth(tag, Limits::saved().max_depth - 2) {
        return true;
    }
    log::warn!(
        "Horizon: dropping {} from {kind} slot {} in the player save: its saved data nests past the NBT depth limit; the rest of the player is saved",
        text(tag.get("id")).unwrap_or_default(),
        number(tag.get("Slot")).unwrap_or(0),
    );
    false
}

/// A saved item list without the stacks the writer cannot hold. Vanilla writes
/// such a stack without complaint (`Inventory.save` writes each non-empty slot
/// through `ItemStackWithSlot.CODEC`, and `TagValueOutput.TypedListWrapper.add`
/// skips an element whose codec fails, reporting it to the `ProblemReporter`),
/// then `NbtAccounter.pushDepth` refuses the file on load and
/// `PlayerDataStorage.load` rolls the player back to `.dat_old`. Dropping the
/// one stack is what vanilla's list codec does, and the rest of the player
/// survives instead of being lost whole (#18384).
fn keep_writable(kind: &str, tag: Tag) -> Tag {
    match tag {
        Tag::List(elements) => Tag::List(
            elements
                .into_iter()
                .filter(|element| stack_fits_the_file(kind, element))
                .collect(),
        ),
        other => other,
    }
}

impl Level {
    /// `PrimaryLevelData.setDifficulty`: the level data holds the world's
    /// difficulty, so the game and the file follow each other and a world
    /// that was set to hard is hard when it is opened again.
    pub fn set_difficulty(&mut self, difficulty: u8) {
        self.difficulty = crate::title::DIFFICULTIES[difficulty as usize % 4].to_owned();
    }

    /// `PrimaryLevelData.getDifficulty`: the id of the level data's
    /// difficulty (peaceful 0 .. hard 3), normal when it is not one of them.
    pub fn difficulty_id(&self) -> u8 {
        crate::title::DIFFICULTIES
            .iter()
            .position(|d| *d == self.difficulty)
            .map_or(2, |i| i as u8)
    }
    /// Initialize the offline single-player identity before saving or dropping items.
    pub fn ensure_player_uuid(&mut self) -> [i32; 4] {
        *self.player_uuid.get_or_insert_with(|| {
            let u = horizon_protocol::crypto::offline_uuid("Player");
            [
                (u >> 96) as u32 as i32,
                (u >> 64) as u32 as i32,
                (u >> 32) as u32 as i32,
                u as u32 as i32,
            ]
        })
    }
    pub fn game_rule(&self, name: &str) -> i32 {
        self.game_rules
            .get(name)
            .copied()
            .or_else(|| {
                self.rule_registry
                    .rule(name)
                    .and_then(|rule| self.game_rules.get(&rule.name).copied())
            })
            .or_else(|| {
                self.rule_registry
                    .rule(name)
                    .map(|rule| match rule.default {
                        crate::gamerules::Value::Bool(value) => i32::from(value),
                        crate::gamerules::Value::Int(value) => value,
                    })
            })
            .unwrap_or(0)
    }
    /// The saved game mode id.
    pub fn mode(&self) -> i32 {
        match (self.creative, self.game_mode) {
            (true, _) => 1,
            (false, 1) => 0,
            (false, m) => m.clamp(0, 3),
        }
    }
    pub fn read(dir: &Path) -> Option<Self> {
        Self::read_root(dir, read_level_dat(dir)?, Default::default())
    }
    /// Load a world using its session rule definitions.
    pub fn read_with_registry(
        dir: &Path,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Option<Self> {
        Self::read_root(dir, read_level_dat(dir)?, registry)
    }
    /// `read` from level.dat alone, without the level.dat_old fallback: what
    /// `LevelStorageSource.readLevelSummary` reads for the world list.
    pub fn read_summary(dir: &Path) -> Option<Self> {
        Self::read_root(dir, read_dat(&dir.join("level.dat"))?, Default::default())
    }
    fn read_root(
        dir: &Path,
        root: Tag,
        registry: std::sync::Arc<crate::gamerules::RuleRegistry>,
    ) -> Option<Self> {
        let data = root.get("Data")?;
        let n = |t: Option<&Tag>| number(t).unwrap_or(0);
        let mut level = Self {
            rule_registry: registry,
            name: text(data.get("LevelName")).unwrap_or_else(|| "World".into()),
            creative: n(data.get("GameType")) == 1,
            game_mode: n(data.get("GameType")) as i32,
            time: n(data.get("Time")),
            health: 20.0,
            food: 20,
            saturation: 5.0,
            dimension: "minecraft:overworld".into(),
            difficulty: "normal".into(),
            ..Self::default()
        };
        // 26.3: spawn {pos: [I; x, y, z], dimension, yaw, pitch}; older: SpawnX/Y/Z.
        level.read_world_spawn(data);
        if let Some(d) = data.get("difficulty_settings") {
            level.difficulty = text(d.get("difficulty")).unwrap_or_else(|| "normal".into());
            level.hardcore = n(d.get("hardcore")) != 0;
            level.difficulty_locked = n(d.get("locked")) != 0;
        }
        level.allow_commands = Some(n(data.get("allowCommands")) != 0);
        // WorldDataConfiguration.MAP_CODEC's `enabled_features`, flat in the
        // Data compound and omitted when the flags are vanilla's default.
        if let Some(Tag::List(flags)) = data.get("enabled_features") {
            level.enabled_features = flags.iter().filter_map(|f| text(Some(f))).collect();
        }
        level.saved_mods = level_mods::saved(&root);
        if let Some(packs) = data.get("DataPacks") {
            let ids = |key| match packs.get(key) {
                Some(Tag::List(ids)) => ids.iter().filter_map(|id| text(Some(id))).collect(),
                _ => Vec::new(),
            };
            level.data_packs = Some((ids("Enabled"), ids("Disabled")));
        }
        let registry = level.rule_registry.clone();
        let mut rules: BTreeMap<String, i32> = registry
            .available_rules(false)
            .map(|rule| {
                (
                    rule.name.to_owned(),
                    match rule.default {
                        crate::gamerules::Value::Bool(value) => i32::from(value),
                        crate::gamerules::Value::Int(value) => value,
                    },
                )
            })
            .collect();
        let saved_rules = read_saved_data(dir, "game_rules");
        if let Some(Tag::Compound(map)) = &saved_rules {
            for (k, v) in map {
                let name = k.to_string_lossy();
                let name = if name.contains(':') {
                    name
                } else {
                    format!("minecraft:{name}")
                };
                if let Some(v) = number(Some(v)) {
                    rules.insert(name, v as i32);
                }
            }
        }
        // GameRules(enabledFeatures, map) removes disabled experimental rules
        // and supplies their defaults when enabled but absent from saved data.
        let registry = level.rule_registry.clone();
        let minecart_rule = registry.rule("max_minecart_speed").expect("vanilla rule");
        if level
            .enabled_features
            .iter()
            .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS)
        {
            let crate::gamerules::Value::Int(default) = minecart_rule.default else {
                unreachable!()
            };
            let value = rules
                .entry(minecart_rule.name.to_owned())
                .or_insert(default);
            if !(minecart_rule.min..=minecart_rule.max).contains(value) {
                *value = default;
            }
        } else {
            rules.remove(&minecart_rule.name);
        }
        level.game_rules = rules;
        // Seed: world_gen_settings.dat (26.3) or Data.WorldGenSettings.
        level.seed = read_saved_data(dir, "world_gen_settings")
            .and_then(|w| number(w.get("seed")))
            .or_else(|| number(data.get("WorldGenSettings").and_then(|w| w.get("seed"))))
            .unwrap_or(0);
        // Day time: the overworld clock (26.3) or Data.DayTime.
        level.day_time = read_saved_data(dir, "world_clocks")
            .and_then(|c| {
                number(
                    c.get("minecraft:overworld")
                        .and_then(|o| o.get("total_ticks")),
                )
            })
            .or_else(|| number(data.get("DayTime")))
            .unwrap_or(level.time);
        level.end_clock = read_saved_data(dir, "world_clocks").and_then(|c| {
            number(
                c.get("minecraft:the_end")
                    .and_then(|o| o.get("total_ticks")),
            )
        });
        let weather = read_saved_data(dir, "weather");
        let w = |new: &str, old: &str| {
            number(weather.as_ref().and_then(|w| w.get(new)))
                .or_else(|| number(data.get(old)))
                .unwrap_or(0)
        };
        let raining = w("raining", "raining") != 0;
        let thundering = w("thundering", "thundering") != 0;
        level.weather = crate::world::Weather {
            clear_time: w("clear_weather_time", "clearWeatherTime") as i32,
            rain_time: w("rain_time", "rainTime") as i32,
            thunder_time: w("thunder_time", "thunderTime") as i32,
            raining,
            thundering,
            rain_level: if raining { (1.0, 1.0) } else { (0.0, 0.0) },
            thunder_level: if thundering { (1.0, 1.0) } else { (0.0, 0.0) },
        };
        // The player: players/data/<singleplayer_uuid>.dat, or the legacy Data.Player. With no
        // `singleplayer_uuid` (a world from a server, a Realms download) the owner loads their
        // own file, not another player's (`PlayerList.loadPlayerData`): `load_owner` does.
        let uuid = uuid_from_tag(data.get("singleplayer_uuid"));
        let player_dir = dir.join("players").join("data");
        // PlayerDataStorage.load: the player's file, else its .dat_old.
        let owner = uuid.and_then(|u| {
            let name = uuid_text(u);
            read_dat(&player_dir.join(format!("{name}.dat")))
                .or_else(|| read_dat(&player_dir.join(format!("{name}.dat_old"))))
        });
        level.singleplayer_file = owner.is_some();
        let player = owner.or_else(|| data.get("Player").cloned());
        level.player_uuid = uuid_from_tag(player.as_ref().and_then(|p| p.get("UUID"))).or(uuid);
        let spawn = level
            .world_spawn
            .map(|s| [f64::from(s[0]), f64::from(s[1]), f64::from(s[2])]);
        level.spawn = spawn;
        if let Some(p) = &player {
            level.read_player(p);
        }
        level.player_extra = player;
        Some(level)
    }
    pub fn read_player(&mut self, p: &Tag) {
        self.player_impulse_grace = None;
        self.player_fall_particles = None;
        let n = |k: &str| number(p.get(k));
        let f = |k: &str| float(p.get(k));
        self.position =
            read_doubles::<3>(p.get("Pos")).map(crate::entity_nbt::clamp_loaded_position);
        if let Some(r) = read_doubles::<2>(p.get("Rotation")) {
            self.rotation = [r[0] as f32, r[1] as f32];
        }
        self.motion = read_doubles::<3>(p.get("Motion")).unwrap_or_default();
        self.health = f("Health").map_or(20.0, |v| v as f32);
        self.absorption = f("AbsorptionAmount").map_or(0.0, |v| v as f32);
        self.food = n("foodLevel").map_or(20, |v| v as i32);
        self.saturation = f("foodSaturationLevel").map_or(5.0, |v| v as f32);
        self.food_exhaustion = f("foodExhaustionLevel").unwrap_or(0.0) as f32;
        self.food_tick_timer = n("foodTickTimer").unwrap_or(0) as i32;
        self.xp_level = n("XpLevel").unwrap_or(0) as i32;
        self.xp_progress = f("XpP").unwrap_or(0.0) as f32;
        self.xp_total = n("XpTotal").unwrap_or(0) as i32;
        self.score = n("Score").unwrap_or(0) as i32;
        self.selected_slot = n("SelectedItemSlot").unwrap_or(0) as i32;
        self.fall_distance = f("fall_distance").unwrap_or(0.0);
        self.on_ground = n("OnGround").unwrap_or(0) != 0;
        // LivingEntity.readAdditionalSaveData: `getBooleanOr("FallFlying",
        // false)` sets shared flag 7; any numeric tag counts, as a byte.
        self.fall_flying = n("FallFlying").unwrap_or(0) != 0;
        self.seen_credits = n("seenCredits").unwrap_or(0) != 0;
        self.post_effects = match p.get("post_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|t| match t {
                    Tag::String(s) => Some(s.to_string_lossy()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        self.last_death = p.get("LastDeathLocation").and_then(|d| {
            let Some(Tag::IntArray(pos)) = d.get("pos") else {
                return None;
            };
            Some((
                text(d.get("dimension"))?,
                [*pos.first()?, *pos.get(1)?, *pos.get(2)?],
            ))
        });
        if let Some(mode) = n("playerGameType") {
            self.creative = mode == 1;
            self.game_mode = mode as i32;
        }
        self.previous_game_mode = n("previousPlayerGameType").map(|m| m as i32);
        self.flying = p
            .get("abilities")
            .and_then(|a| number(a.get("flying")))
            .is_some_and(|v| v != 0);
        self.fly_speed = p
            .get("abilities")
            .and_then(|a| float(a.get("flySpeed")))
            .map(|v| v as f32);
        self.abilities = p.get("abilities").map(|a| {
            let flag = |key: &str, default: bool| number(a.get(key)).map_or(default, |v| v != 0);
            let speed = |key: &str, default: f32| {
                float(a.get(key))
                    .map(|v| v as f32)
                    .filter(|v| v.is_finite())
                    .unwrap_or(default)
            };
            crate::lan::abilities::PlayerAbilities {
                invulnerable: flag("invulnerable", false),
                flying: flag("flying", false),
                may_fly: flag("mayfly", false),
                instabuild: flag("instabuild", false),
                may_build: flag("mayBuild", true),
                flying_speed: speed("flySpeed", 0.05),
                walking_speed: speed("walkSpeed", 0.1),
            }
        });
        if let Some(d) = text(p.get("Dimension")) {
            self.dimension = d;
        }
        if let Some(respawn) = respawn::read(p) {
            self.respawn_elsewhere = Some(respawn);
        }
        let mut inventory = Vec::new();
        let mut inventory_components = HashMap::new();
        if let Some(Tag::List(items)) = p.get("Inventory") {
            for it in items {
                let slot = number(it.get("Slot")).unwrap_or(0) as i8 as u8;
                if let Some(stack) = item_from_nbt(it) {
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    // Legacy armor slots 100..103 and offhand -106 (150).
                    let slot = if slot as i8 == -106 { 150 } else { slot };
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        if let Some(eq) = p.get("equipment") {
            let eq = horizon_items::equipment::Equipment::read(eq);
            for (ordinal, slot) in PLAYER_EQUIPMENT_SLOTS {
                if let Some(stack) = eq.get(ordinal) {
                    inventory.retain(|(s, ..)| *s != slot);
                    if let Some(extra) = &stack.extra {
                        inventory_components.insert(slot, extra.clone());
                    }
                    inventory.push((slot, stack.item.to_string(), stack.count, stack.damage));
                }
            }
        }
        self.inventory = inventory;
        self.ender_items = decode_items(p.get("EnderItems"), 27);
        self.recipe_book = crate::recipe_book::Settings::read(p.get("recipeBook"));
        self.inventory_components = inventory_components;
        self.xp_seed = n("XpSeed").unwrap_or(0) as i32;
        self.active_effects = match p.get("active_effects") {
            Some(Tag::List(list)) => list
                .iter()
                .filter_map(|e| Some(read_effect_details(e, &text(e.get("id"))?)))
                .collect(),
            _ => Vec::new(),
        };
        // ServerPlayer.saveParentVehicle: the mount the player rides, by
        // UUID; the vehicle tree stays in the raw compound until it is back.
        self.root_vehicle = p
            .get("RootVehicle")
            .and_then(|r| uuid_from_tag(r.get("Attach")));
    }
    /// The player compound: loaded data with current state written over it.
    /// `LivingEntity.addAdditionalSaveData` / `ServerPlayer.addAdditionalSaveData`:
    /// the live fall fields and the player's empty brain are saved too.
    pub fn player_tag(&self) -> Tag {
        let mut p = compound_or_empty(self.player_extra.clone());
        // Keys rewritten from scratch.
        if let Tag::Compound(map) = &mut p {
            for key in [
                "Inventory",
                "EnderItems",
                "equipment",
                "active_effects",
                "post_effects",
                "Pos",
                "respawn",
                "recipeBook",
                "LastDeathLocation",
                "previousPlayerGameType",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        let mut inventory = Vec::new();
        let mut equipment = horizon_items::equipment::Equipment::default();
        for (slot, id, count, damage) in &self.inventory {
            let extra = self.inventory_components.get(slot).cloned();
            let stack = crate::items::ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra,
            };
            match *slot {
                0..=35 => {
                    let mut entries = vec![("Slot", Tag::Byte(*slot as i8))];
                    entries.extend(encode_item(&stack));
                    inventory.push(Tag::compound(entries));
                }
                _ => {
                    if let Some((ordinal, _)) = PLAYER_EQUIPMENT_SLOTS
                        .iter()
                        .find(|(_, saved)| saved == slot)
                    {
                        // The compound `equipment.write` builds, for the check
                        // alone: a stack past the bound leaves its slot empty.
                        let mut entries = vec![("Slot", Tag::Byte(*slot as i8))];
                        entries.extend(encode_item(&stack));
                        if stack_fits_the_file("equipment", &Tag::compound(entries)) {
                            equipment.set(*ordinal, Some(stack));
                        }
                    }
                }
            }
        }
        // The stored abilities, else GameType.updatePlayerAbilities.
        let mode = self.mode();
        let (creative, spectator) = (mode == 1, mode == 3);
        let mut abilities = compound_or_empty(p.get("abilities").cloned());
        if let Some(a) = self.abilities {
            for (k, v) in [
                ("invulnerable", a.invulnerable),
                ("flying", a.flying),
                ("mayfly", a.may_fly),
                ("instabuild", a.instabuild),
                ("mayBuild", a.may_build),
            ] {
                put(&mut abilities, k, Tag::Byte(i8::from(v)));
            }
            put(
                &mut abilities,
                "flySpeed",
                Tag::Float(a.flying_speed.to_bits()),
            );
            put(
                &mut abilities,
                "walkSpeed",
                Tag::Float(a.walking_speed.to_bits()),
            );
        } else {
            for (k, v) in [
                ("invulnerable", creative || spectator),
                ("mayfly", creative || spectator),
                ("instabuild", creative),
                ("mayBuild", mode < 2),
            ] {
                put(&mut abilities, k, Tag::Byte(i8::from(v)));
            }
            if spectator {
                put(&mut abilities, "flying", Tag::Byte(1));
            } else if creative {
                put(&mut abilities, "flying", Tag::Byte(i8::from(self.flying)));
            } else {
                put(&mut abilities, "flying", Tag::Byte(0));
            }
        }
        if let Tag::Compound(map) = &mut abilities {
            map.entry("mayBuild".into()).or_insert(Tag::Byte(1));
            if let Some(speed) = self.fly_speed {
                map.insert("flySpeed".into(), Tag::Float(speed.to_bits()));
            }
            map.entry("flySpeed".into())
                .or_insert(Tag::Float(0.05f32.to_bits()));
            map.entry("walkSpeed".into())
                .or_insert(Tag::Float(0.1f32.to_bits()));
            map.entry("flying".into()).or_insert(Tag::Byte(0));
        }
        let impulse_grace = self
            .player_impulse_grace
            .map(Tag::Int)
            .or_else(|| p.get("current_impulse_context_reset_grace_time").cloned())
            .unwrap_or(Tag::Int(0));
        let fall_particles = self
            .player_fall_particles
            .map(|v| Tag::Byte(i8::from(v)))
            .or_else(|| p.get("spawn_extra_particles_on_fall").cloned())
            .unwrap_or(Tag::Byte(0));
        let uuid = self.player_uuid.unwrap_or([0, 0, 0, 0]);
        let mut entries: Vec<(&str, Tag)> = vec![
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("UUID", uuid_tag(uuid)),
            ("Rotation", floats(&self.rotation)),
            ("Motion", doubles(&self.motion)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
            // LivingEntity.addAdditionalSaveData: `putBoolean("FallFlying",
            // isFallFlying())`, always written.
            ("FallFlying", Tag::Byte(i8::from(self.fall_flying))),
            ("Health", Tag::Float(self.health.to_bits())),
            // Player inherits LivingEntity.makeBrain: a new empty Brain;
            // Brain.Packed's codec stores its empty memories map.
            (
                "Brain",
                Tag::compound([("memories", Tag::Compound(Default::default()))]),
            ),
            ("current_impulse_context_reset_grace_time", impulse_grace),
            ("spawn_extra_particles_on_fall", fall_particles),
            ("AbsorptionAmount", Tag::Float(self.absorption.to_bits())),
            ("foodLevel", Tag::Int(self.food)),
            ("foodSaturationLevel", Tag::Float(self.saturation.to_bits())),
            (
                "foodExhaustionLevel",
                Tag::Float(self.food_exhaustion.to_bits()),
            ),
            ("foodTickTimer", Tag::Int(self.food_tick_timer)),
            ("XpLevel", Tag::Int(self.xp_level)),
            ("XpP", Tag::Float(self.xp_progress.to_bits())),
            ("XpTotal", Tag::Int(self.xp_total)),
            ("SelectedItemSlot", Tag::Int(self.selected_slot)),
            ("XpSeed", Tag::Int(self.xp_seed)),
            ("Score", Tag::Int(self.score)),
            (
                "active_effects",
                Tag::List(
                    self.active_effects
                        .iter()
                        .map(|e| {
                            let mut effect = vec![("id", string(&e.id))];
                            effect.extend(effect_details(e));
                            Tag::compound(effect)
                        })
                        .collect(),
                ),
            ),
            ("playerGameType", Tag::Int(self.mode())),
            (
                "Inventory",
                keep_writable("inventory", Tag::List(inventory)),
            ),
            (
                "EnderItems",
                keep_writable("ender chest", encode_items(&self.ender_items)),
            ),
            ("recipeBook", self.recipe_book.write()),
            ("abilities", abilities),
            (
                "Dimension",
                string(if self.dimension.is_empty() {
                    "minecraft:overworld"
                } else {
                    &self.dimension
                }),
            ),
        ];
        // LivingEntity.addAdditionalSaveData omits empty equipment and an empty
        // effect list. PlayerEquipment.isEmpty also counts the selected hotbar
        // item, which the codec never writes: holding one saves `equipment: {}`.
        let holding = self
            .inventory
            .iter()
            .any(|(slot, _, count, _)| i32::from(*slot) == self.selected_slot && *count > 0);
        match equipment.write() {
            Some(tag) => entries.push(("equipment", tag)),
            None if holding => entries.push(("equipment", Tag::Compound(Default::default()))),
            None => {}
        }
        if self.active_effects.is_empty() {
            entries.retain(|(key, _)| *key != "active_effects");
        }
        if let Some(pos) = self.position {
            entries.push(("Pos", doubles(&pos)));
        }
        entries.push(("seenCredits", Tag::Byte(i8::from(self.seen_credits))));
        // `output.store` writes the list even when it is empty.
        entries.push((
            "post_effects",
            Tag::List(self.post_effects.iter().map(|e| string(e)).collect()),
        ));
        if let Some((dimension, pos)) = &self.last_death {
            entries.push((
                "LastDeathLocation",
                Tag::compound([
                    ("dimension", string(dimension)),
                    ("pos", Tag::IntArray(pos.to_vec())),
                ]),
            ));
        }
        // A respawn point other than the world spawn: an anchor elsewhere, or a bed.
        if let Some(saved) = &self.respawn_elsewhere {
            entries.push(("respawn", respawn::write(saved)));
        } else if let Some(s) = self.spawn {
            let block = [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ];
            // A point of the player's own in the Overworld other than a bed
            // (`respawn_elsewhere`): `spawnpoint`'s, forced.
            if Some(block) != self.world_spawn {
                entries.push((
                    "respawn",
                    Tag::compound([
                        ("dimension", string("minecraft:overworld")),
                        ("pos", Tag::IntArray(block.to_vec())),
                        ("yaw", Tag::Float(0f32.to_bits())),
                        ("pitch", Tag::Float(0f32.to_bits())),
                        ("forced", Tag::Byte(1)),
                    ]),
                ));
            }
        }
        // ServerPlayer.storeGameTypes: storeNullable, so absent without one.
        if let Some(mode) = self.previous_game_mode {
            entries.push(("previousPlayerGameType", Tag::Int(mode)));
        }
        for (key, value) in entries {
            put(&mut p, key, value);
        }
        if let Tag::Compound(map) = &mut p {
            for (key, value) in [
                ("Air", Tag::Short(300)),
                ("Fire", Tag::Short(-20)),
                ("Invulnerable", Tag::Byte(0)),
                ("PortalCooldown", Tag::Int(0)),
                ("SleepTimer", Tag::Short(0)),
                ("XpSeed", Tag::Int(0)),
                ("seenCredits", Tag::Byte(0)),
                ("HurtTime", Tag::Short(0)),
                ("DeathTime", Tag::Short(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        // ServerPlayer.saveParentVehicle: the savers put the whole
        // RootVehicle in the compound; a mount known by UUID alone (older
        // Horizon files) is still named.
        if let Some(vehicle) = self.root_vehicle
            && p.get("RootVehicle").is_none()
        {
            put(
                &mut p,
                "RootVehicle",
                Tag::compound([("Attach", uuid_tag(vehicle))]),
            );
        }
        p
    }
    /// Write every file; a world without a player UUID gets a random one.
    pub fn write(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, true, false, false)
    }
    /// Dedicated servers save connected players separately.
    pub fn write_without_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, false, false, false)
    }
    /// A crash's save that must leave the player as last saved: no player file, and a legacy
    /// `Data.Player` stays, since before the first normal save it is the owner's only copy.
    pub fn write_keeping_player(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, false, false, true)
    }
    /// The temporary world a Realms upload packs
    /// (`RealmsCreateWorldFlow.createTemporaryWorldFolder`): level.dat as
    /// `WorldData.createTag(null)` writes it — no `singleplayer_uuid` — with the world
    /// generation and game rules, and no player file or weather/clocks the server makes
    /// on its own. The caller writes the data packs and removes the folder.
    pub fn write_upload(&mut self, dir: &Path) -> Result<()> {
        self.write_internal(dir, false, true, false)
    }

    /// The player's file alone, `players/data/<singleplayer_uuid>.dat`
    /// (`PlayerDataStorage.save`): the one level.dat names.
    pub fn write_player(&mut self, dir: &Path) -> Result<()> {
        let uuid = self.ensure_player_uuid();
        let player_file = dir
            .join("players")
            .join("data")
            .join(format!("{}.dat", uuid_text(uuid)));
        back_up_unreadable_player(&player_file);
        write_dat(&player_file, self.player_tag())
    }

    fn write_internal(
        &mut self,
        dir: &Path,
        write_player: bool,
        upload: bool,
        keep_legacy_player: bool,
    ) -> Result<()> {
        fs::create_dir_all(dir)?;
        restore_level_dat(dir);
        let mut root = compound_or_empty(read_dat(&dir.join("level.dat")));
        let legacy = root
            .get("Data")
            .is_some_and(|d| d.get("Player").is_some() || d.get("WorldGenSettings").is_some());
        let mut data = compound_or_empty(root.get("Data").cloned());
        if let Tag::Compound(map) = &mut data {
            // Pre-26.3 keys now stored elsewhere (a legacy Horizon level.dat).
            for key in [
                "Player",
                "WorldGenSettings",
                "DayTime",
                "SpawnX",
                "SpawnY",
                "SpawnZ",
                "SpawnAngle",
                "clearWeatherTime",
                "rainTime",
                "thunderTime",
                "raining",
                "thundering",
                "Difficulty",
                "hardcore",
                "GameRules",
                "DimensionData",
                "WanderingTraderSpawnChance",
                "WanderingTraderSpawnDelay",
                "WanderingTraderId",
                "BorderCenterX",
                "BorderCenterZ",
                "BorderSize",
                "BorderSafeZone",
                "BorderWarningBlocks",
                "BorderWarningTime",
                "BorderSizeLerpTime",
                "BorderSizeLerpTarget",
                "BorderDamagePerBlock",
                "DifficultyLocked",
                "ScheduledEvents",
                "CustomBossEvents",
                "DragonFight",
            ] {
                if keep_legacy_player && key == "Player" {
                    continue;
                }
                map.remove(&JavaString::from(key));
            }
        }
        // An upload's level.dat names no player (`createTag(null)`); every other file
        // names this world's own.
        let uuid = (!upload).then(|| self.ensure_player_uuid());
        // A new world's first spawn point is its world spawn.
        if self.world_spawn.is_none() {
            self.world_spawn = self.spawn.map(|s| {
                [
                    s[0].floor() as i32,
                    s[1].floor() as i32,
                    s[2].floor() as i32,
                ]
            });
        }
        let spawn = self.world_spawn.unwrap_or([0, 64, 0]);
        let mut history: Vec<Tag> = match data.get("version_history") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if history.last() != Some(&Tag::Int(DATA_VERSION)) {
            history.push(Tag::Int(DATA_VERSION));
        }
        let mut brands: Vec<Tag> = match data.get("ServerBrands") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        if !brands.contains(&string("horizon")) {
            brands.push(string("horizon"));
        }
        let mut difficulty = compound_or_empty(data.get("difficulty_settings").cloned());
        put(
            &mut difficulty,
            "difficulty",
            string(if self.difficulty.is_empty() {
                "normal"
            } else {
                &self.difficulty
            }),
        );
        put(
            &mut difficulty,
            "hardcore",
            Tag::Byte(i8::from(self.hardcore)),
        );
        put(
            &mut difficulty,
            "locked",
            Tag::Byte(i8::from(self.difficulty_locked)),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        for (key, value) in [
            ("DataVersion", Tag::Int(DATA_VERSION)),
            (
                "Version",
                Tag::compound([
                    ("Name", string(VERSION_NAME)),
                    ("Id", Tag::Int(DATA_VERSION)),
                    ("Snapshot", Tag::Byte(0)),
                    ("Series", string("main")),
                ]),
            ),
            ("version_history", Tag::List(history)),
            ("version", Tag::Int(19133)),
            ("LevelName", string(&self.name)),
            ("GameType", Tag::Int(self.mode())),
            ("Time", Tag::Long(self.time)),
            ("LastPlayed", Tag::Long(now)),
            (
                "spawn",
                Tag::compound([
                    ("pos", Tag::IntArray(spawn.to_vec())),
                    ("dimension", string(self.spawn_dimension())),
                    ("yaw", Tag::Float(self.world_spawn_angle[0].to_bits())),
                    ("pitch", Tag::Float(self.world_spawn_angle[1].to_bits())),
                ]),
            ),
            ("difficulty_settings", difficulty),
            ("initialized", Tag::Byte(1)),
            ("ServerBrands", Tag::List(brands)),
        ] {
            put(&mut data, key, value);
        }
        level_mods::write(&mut root);
        if let Some(uuid) = uuid {
            put(&mut data, "singleplayer_uuid", uuid_tag(uuid));
        }
        if let Tag::Compound(map) = &mut data {
            map.entry("allowCommands".into()).or_insert(Tag::Byte(0));
            if let Some(allow) = self.allow_commands {
                map.insert("allowCommands".into(), Tag::Byte(i8::from(allow)));
            }
            map.entry("WasModded".into()).or_insert(Tag::Byte(0));
            // lenientOptionalFieldOf("enabled_features", DEFAULT_FLAGS): the
            // tag is left out when the set is vanilla's default, so a
            // `["minecraft:vanilla"]`-only list does not reach the file.
            let vanilla_only = *self.enabled_features == ["minecraft:vanilla"];
            if !self.enabled_features.is_empty() && !vanilla_only {
                map.insert(
                    "enabled_features".into(),
                    Tag::List(self.enabled_features.iter().map(|s| string(s)).collect()),
                );
            }
            map.entry("DataPacks".into()).or_insert_with(|| {
                let (enabled, disabled) = self.data_packs.clone().unwrap_or_else(|| {
                    (
                        vec!["vanilla".into()],
                        [
                            "minecart_improvements",
                            "redstone_experiments",
                            "trade_rebalance",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    )
                });
                let list = |ids: Vec<String>| Tag::List(ids.iter().map(|s| string(s)).collect());
                Tag::compound([("Enabled", list(enabled)), ("Disabled", list(disabled))])
            });
        }
        // Saved data: weather, clocks, and (when missing or legacy) world
        // generation settings and game rules. An upload's temporary world carries only
        // what the server needs before its first tick.
        if !upload {
            write_saved_data(
                dir,
                "weather",
                Tag::compound([
                    ("clear_weather_time", Tag::Int(self.weather.clear_time)),
                    ("rain_time", Tag::Int(self.weather.rain_time)),
                    ("thunder_time", Tag::Int(self.weather.thunder_time)),
                    ("raining", Tag::Byte(i8::from(self.weather.raining))),
                    ("thundering", Tag::Byte(i8::from(self.weather.thundering))),
                ]),
            )?;
            let mut clocks = compound_or_empty(read_saved_data(dir, "world_clocks"));
            let mut overworld = compound_or_empty(clocks.get("minecraft:overworld").cloned());
            put(&mut overworld, "total_ticks", Tag::Long(self.day_time));
            put(&mut clocks, "minecraft:overworld", overworld);
            let mut end = compound_or_empty(clocks.get("minecraft:the_end").cloned());
            put(
                &mut end,
                "total_ticks",
                Tag::Long(self.end_clock.unwrap_or(self.time)),
            );
            put(&mut clocks, "minecraft:the_end", end);
            write_saved_data(dir, "world_clocks", clocks)?;
        }
        if legacy || !saved_data_path(dir, "world_gen_settings").exists() {
            write_world_gen_settings(dir, self.seed, WorldGen::default())?;
        }
        if !saved_data_path(dir, "game_rules").exists() || !self.game_rules.is_empty() {
            // Preserve other namespaces even when their mod is not currently loaded.
            let mut rules = match read_saved_data(dir, "game_rules") {
                Some(Tag::Compound(entries)) => Tag::Compound(
                    entries
                        .into_iter()
                        .filter(|(key, _)| {
                            key.to_string().is_ok_and(|s| {
                                s.split_once(':').is_some_and(|(ns, _)| ns != "minecraft")
                            })
                        })
                        .collect(),
                ),
                _ => Tag::Compound(Compound::new()),
            };
            let registry = &self.rule_registry;
            for rule in registry.available_rules(
                self.enabled_features
                    .iter()
                    .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS),
            ) {
                let tag = match rule.default {
                    crate::gamerules::Value::Bool(default) => Tag::Byte(i8::from(
                        self.game_rules
                            .get(&rule.name)
                            .copied()
                            .unwrap_or(i32::from(default))
                            != 0,
                    )),
                    crate::gamerules::Value::Int(default) => {
                        Tag::Int(self.game_rules.get(&rule.name).copied().unwrap_or(default))
                    }
                };
                put(&mut rules, &rule.name.clone(), tag);
            }
            if self
                .enabled_features
                .iter()
                .any(|f| f == crate::gamerules::MINECART_IMPROVEMENTS)
            {
                let rule = registry.rule("max_minecart_speed").expect("vanilla rule");
                let crate::gamerules::Value::Int(default) = rule.default else {
                    unreachable!()
                };
                let value = self
                    .game_rules
                    .get(&rule.name)
                    .copied()
                    .filter(|v| (rule.min..=rule.max).contains(v))
                    .unwrap_or(default);
                put(&mut rules, &rule.name.clone(), Tag::Int(value));
            }
            write_saved_data(dir, "game_rules", rules)?;
        }
        if write_player {
            self.write_player(dir)?;
        }
        put(&mut root, "Data", data);
        write_dat(&dir.join("level.dat"), root)?;
        Ok(())
    }
}
/// Loads saved chunks before falling back to generation.
pub struct Persistent<G> {
    pub inner: Arc<G>,
    pub save: Arc<SaveDir>,
    pub registry: Arc<crate::assets::Assets>,
    pub biomes: Arc<Biomes>,
}
type LodSurfaces = Arc<[crate::lod::Column; 256]>;
/// The coarsest far tile spacing whose columns read saved chunks: 16 blocks
/// a column, and 4 with Reduced far detail (`LodSampler::set_approximate`).
const SAVED_LOD_STRIDE: i32 = 16;
const SAVED_LOD_STRIDE_REDUCED: i32 = 4;
struct SavedLodSampler<G: ChunkGenerator> {
    generator: Arc<Persistent<G>>,
    inner: Box<dyn crate::streaming::LodSampler>,
    cache: std::collections::VecDeque<((i32, i32), Option<LodSurfaces>)>,
    details: Vec<crate::streaming::LodDetail>,
    /// Reduced far detail: saved chunks only in tiles up to
    /// `SAVED_LOD_STRIDE_REDUCED` blocks a column.
    approximate: bool,
    /// Chunks the generator decorated for far terrain alone, kept from
    /// earlier sessions; saved chunks take precedence over them.
    generated: Arc<crate::lod::cache::DiskCache>,
    /// The generator's far samples from earlier sessions (`lod::bake`).
    bake: Option<Arc<crate::lod::bake::LodBake>>,
}
impl<G: ChunkGenerator> SavedLodSampler<G> {
    fn saved_columns(&mut self, key: (i32, i32)) -> Option<LodSurfaces> {
        let cached = match self.cache.iter().position(|(at, _)| *at == key) {
            Some(index) => self.cache.remove(index).unwrap(),
            None => (
                key,
                self.generator.save.lod_surfaces_shared(
                    key.0,
                    key.1,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
        };
        let columns = cached.1.clone();
        self.cache.push_front(cached);
        self.cache.truncate(128);
        columns
    }
}
impl<G: ChunkGenerator> crate::streaming::LodSampler for SavedLodSampler<G> {
    fn fork(&self) -> Option<Box<dyn crate::streaming::LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            inner: self.inner.fork()?,
            cache: Default::default(),
            details: Vec::new(),
            approximate: self.approximate,
            generated: self.generated.clone(),
            bake: self.bake.clone(),
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
        self.inner.set_approximate(approximate);
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        let mut missing = Vec::new();
        for &key in chunks {
            // Generated chunks from earlier sessions answer only these
            // requests: reading them for every sampled cell of a coarse tile
            // slowed sampling more than it saved.
            if let Some(columns) = self
                .saved_columns(key)
                .or_else(|| self.generated.get(key.0, key.1))
            {
                self.details.push((key, columns));
            } else {
                missing.push(key);
            }
        }
        self.inner.request_detail(&missing, center);
    }
    fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
        let mut details = std::mem::take(&mut self.details);
        for (key, generated) in self.inner.drain_details() {
            // A generation batch also returns neighbouring, unrequested chunks.
            // Their saved builds must take precedence too.
            let columns = self.saved_columns(key).unwrap_or_else(|| {
                self.generated.put(key.0, key.1, &generated);
                generated
            });
            details.push((key, columns));
        }
        details
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod::Column> {
        let approximate = self.approximate;
        let baked = (self.bake.as_ref()).and_then(|b| b.grid(approximate, [x, z], stride, side));
        let mut columns = baked.unwrap_or_else(|| {
            let columns = self.inner.grid(x, z, stride, side);
            if let Some(bake) = &self.bake {
                bake.put_grid(approximate, [x, z], stride, side, &columns);
            }
            columns
        });
        // Saved chunks show in tiles up to 16 blocks a column. With Reduced far
        // detail only up to 4 (within about a kilometre): coarser tiles keep
        // the generator's terrain, which saved chunks were made from, since
        // decoding up to 256 saved chunks for each of them held the whole far
        // view back for minutes in large saves. Edits that far away show once
        // their tiles refine on approach.
        let limit = if self.approximate {
            SAVED_LOD_STRIDE_REDUCED
        } else {
            SAVED_LOD_STRIDE
        };
        if stride <= limit {
            for (index, column) in columns.iter_mut().enumerate() {
                let (wx, wz) = (
                    x + (index % side) as i32 * stride,
                    z + (index / side) as i32 * stride,
                );
                if let Some(saved) = self.saved_columns((wx >> 4, wz >> 4)) {
                    *column = crate::lod::Column::reduced(
                        &saved,
                        (wx & 15) as usize,
                        (wz & 15) as usize,
                        stride as usize,
                    );
                }
            }
        }
        columns
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> crate::lod::Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> crate::lod::Column {
        if width <= 16
            && let Some(columns) = self.saved_columns((x >> 4, z >> 4))
        {
            return crate::lod::Column::reduced(
                &columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.inner.reduced_column(x, z, width)
    }
}

/// Unpack a section's paletted container into `out` (4096 blocks or 64 biome
/// cells): each entry's palette index as stored, unchecked. Returns the
/// palette's length, `None` for a container vanilla does not write. Once a
/// section: far terrain looked both up by name for each block (#6386).
fn lod_palette_indices(container: &Tag, out: &mut [u16], minimum_bits: u32) -> Option<usize> {
    let palette = match container.get("palette")? {
        Tag::List(palette) if (1..=out.len()).contains(&palette.len()) => palette,
        _ => return None,
    };
    if palette.len() == 1 {
        out.fill(0);
        return Some(1);
    }
    let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(minimum_bits) as usize;
    let per_word = 64 / bits;
    let Tag::LongArray(data) = container.get("data")? else {
        return None;
    };
    if data.len() != out.len().div_ceil(per_word) {
        return None;
    }
    for (word, indices) in data.iter().zip(out.chunks_mut(per_word)) {
        let mut word = *word as u64;
        for index in indices {
            *index = (word & ((1 << bits) - 1)) as u16;
            word >>= bits;
        }
    }
    Some(palette.len())
}
fn lod_light_layers(sections: &[(i32, &Tag)], layout: ColumnLayout, sky_light: bool) -> Box<[u8]> {
    let mut layers = vec![(None, None); layout.sections()];
    for &(y, section) in sections {
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(data)) if data.len() == 2048 => Some(&data[..]),
            _ => None,
        };
        layers[(y - (layout.min_y() >> 4)) as usize] = (
            if sky_light { layer("SkyLight") } else { None },
            layer("BlockLight"),
        );
    }
    let mut light = vec![0u8; layout.sections() * 4096].into_boxed_slice();
    let mut above = [if sky_light { 15u8 } else { 0 }; 256];
    let nibble = |data: &[u8], index: usize| (data[index / 2] >> ((index & 1) * 4)) & 15;
    for si in (0..layout.sections()).rev() {
        let (sky, block) = layers[si];
        let section = &mut light[si * 4096..(si + 1) * 4096];
        for (index, value) in section.iter_mut().enumerate() {
            *value = sky.map_or(above[index & 255], |data| nibble(data, index)) << 4
                | block.map_or(0, |data| nibble(data, index));
        }
        // Missing sky layers inherit the bottom layer above, as load_light does.
        for (index, value) in above.iter_mut().enumerate() {
            *value = section[index] >> 4;
        }
    }
    light
}
fn lod_face_light(light: &[u8], x: usize, y: usize, z: usize) -> u16 {
    let index = y * 256 + z * 16 + x;
    let mut sky = light[index] >> 4;
    let mut block = light[index] & 15;
    for (valid, index) in [
        (x > 0, index.wrapping_sub(1)),
        (x < 15, index + 1),
        (z > 0, index.wrapping_sub(16)),
        (z < 15, index + 16),
        (y > 0, index.wrapping_sub(256)),
        (index + 256 < light.len(), index + 256),
    ] {
        if valid {
            sky = sky.max(light[index] >> 4);
            block = block.max(light[index] & 15);
        }
    }
    u16::from(sky) << 4 | u16::from(block) << 12
}
fn decode_lod_surfaces(
    root: &NamedTag,
    assets: &crate::assets::Assets,
    biomes: &Biomes,
    layout: ColumnLayout,
    sky_light: bool,
) -> Option<LodSurfaces> {
    let Tag::List(sections) = root.tag.get("sections")? else {
        return None;
    };
    let water_state = assets.registry.default_state("minecraft:water");
    let mut sections: Vec<_> = sections
        .iter()
        .filter_map(|section| {
            let y = number(section.get("Y"))? as i32;
            layout
                .section_index(y * 16)
                .is_some()
                .then_some((y, section))
        })
        .collect();
    sections.sort_unstable_by_key(|(y, _)| std::cmp::Reverse(*y));
    let mut columns: Box<[crate::lod::Column; 256]> =
        Box::new(std::array::from_fn(|_| crate::lod::Column {
            surface: None,
            spans: crate::lod::Spans::new(),
        }));
    let mut water = [None; 256];
    let light = lod_light_layers(&sections, layout, sky_light);
    let mut states = BTreeMap::new();
    let mut indices = [0u16; 4096];
    for (section_y, section) in sections {
        if let Some(blocks) = section.get("block_states") {
            let Tag::List(palette) = blocks.get("palette")? else {
                return None;
            };
            // Resolve each state once per palette, rather than parsing NBT for every block.
            let palette: Vec<_> = palette
                .iter()
                .map(|entry| {
                    let descriptor = BlockState::from_nbt(entry).ok()?;
                    let state = *states.entry(descriptor.clone()).or_insert_with(|| {
                        assets
                            .registry
                            .id(&descriptor)
                            .or_else(|| assets.registry.default_state(&descriptor.name))
                    });
                    Some(state.map(|state| {
                        let properties = assets.properties.heightmap_state(state);
                        let fluid = matches!(
                            descriptor.name.as_str(),
                            "minecraft:water" | "minecraft:lava"
                        );
                        // Chorus is thin and non-solid but still far-End detail.
                        let chorus = matches!(
                            descriptor.name.as_str(),
                            "minecraft:chorus_plant" | "minecraft:chorus_flower"
                        );
                        let solid = properties.blocks_motion_in_heightmap();
                        // Kelp and seagrass: the water they hold. Left out, the
                        // water stopped short where they reach the surface.
                        match water_state {
                            Some(water) if !fluid && !solid && properties.has_fluid() => {
                                (water, true, true)
                            }
                            _ => (state, fluid, fluid || solid || chorus),
                        }
                    }))
                })
                .collect::<Option<_>>()?;
            let length = lod_palette_indices(blocks, &mut indices, 4)?;
            if indices.iter().any(|&index| usize::from(index) >= length) {
                return None;
            }
            // The air above the ground, mostly: nothing here is far terrain.
            if !palette.iter().flatten().any(|&(_, _, kept)| kept) {
                continue;
            }
            // Each 4x4x4 cell's biome; 0 where the section's biomes do not say.
            let mut cell_biomes = [0u16; 64];
            if let Some(container) = section.get("biomes")
                && let Some(Tag::List(names)) = container.get("palette")
                && lod_palette_indices(container, &mut cell_biomes, 1).is_some()
            {
                let ids: Vec<u16> = names
                    .iter()
                    .map(|entry| match entry {
                        Tag::String(name) => biomes.id(&name.to_string_lossy()),
                        _ => 0,
                    })
                    .collect();
                for biome in &mut cell_biomes {
                    *biome = ids.get(usize::from(*biome)).copied().unwrap_or(0);
                }
            }
            for z in 0..16 {
                for x in 0..16 {
                    let cell = z * 16 + x;
                    let column = &mut columns[cell];
                    for y in (0..16).rev() {
                        let Some((state, fluid, true)) =
                            palette[usize::from(indices[y * 256 + cell])]
                        else {
                            continue;
                        };
                        let biome = cell_biomes[(y / 4) * 16 + (z / 4) * 4 + x / 4];
                        let height = section_y * 16 + y as i32 + 1;
                        if column.surface.is_none() {
                            if fluid {
                                water[cell].get_or_insert((height, state));
                            } else {
                                column.surface = Some(crate::lod::Surface {
                                    height,
                                    state,
                                    water: water[cell],
                                    biome,
                                });
                            }
                        }
                        let light =
                            lod_face_light(&light, x, (height - 1 - layout.min_y()) as usize, z);
                        let min = (height - 1) as i16;
                        let max = height as i16;
                        if let Some(previous) = column.spans.last_mut()
                            && previous.min == max
                            && (previous.state, previous.biome, previous.light)
                                == (state, biome, light)
                        {
                            previous.min = min;
                        } else {
                            column.spans.push(crate::lod::Span {
                                min,
                                max,
                                state,
                                biome,
                                light,
                            });
                        }
                    }
                }
            }
        }
    }
    // Spans are pushed top down, so the reverse puts the topmost last, as
    // `Column::from_chunk` leaves them. Water keeps the chunk's own light,
    // a level less per block, as the generator's far columns shade it.
    for column in &mut *columns {
        column.spans.reverse();
    }
    Some(Arc::from(columns))
}

impl SaveDir {
    /// `lod_surfaces`, decoded once for all far sampling threads: each saved
    /// chunk lies under far tiles of several levels, sampled on any thread.
    fn lod_surfaces_shared(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        if let Some((at, columns)) = self.lod_decoded.lock().unwrap().chunks.get(&(x, z))
            && at.elapsed() < LOD_DECODED_TTL
        {
            return Some(columns.clone());
        }
        let columns = self.lod_surfaces(x, z, assets, biomes)?;
        let mut decoded = self.lod_decoded.lock().unwrap();
        let fresh = (std::time::Instant::now(), columns.clone());
        if decoded.chunks.insert((x, z), fresh).is_none() {
            decoded.order.push_back((x, z));
            while decoded.order.len() > LOD_DECODED {
                if let Some(old) = decoded.order.pop_front() {
                    decoded.chunks.remove(&old);
                }
            }
        }
        Some(columns)
    }
    fn lod_surfaces(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        let root = self.far_record(x, z)?;
        // Proto saves may still be missing decoration from their neighbors.
        if text(root.tag.get("Status"))
            .is_some_and(|status| status != "minecraft:full" && status != "full")
        {
            return None;
        }
        decode_lod_surfaces(
            &root,
            assets,
            biomes,
            self.shape.column(),
            self.shape.sky_light,
        )
    }
}

impl<G: ChunkGenerator> ChunkGenerator for Persistent<G> {
    type Worker = G::Worker;
    fn worker(&self) -> G::Worker {
        let save = self.save.clone();
        self.inner.set_saved_starts(Arc::new(move |pos| {
            let root = save.raw_chunk(pos.x, pos.z)?;
            match root.tag.get("structures")?.get("starts")? {
                Tag::Compound(starts) => Some(starts.clone()),
                _ => None,
            }
        }));
        self.inner.worker()
    }
    fn lod_sampler(self: Arc<Self>) -> Box<dyn crate::streaming::LodSampler> {
        let inner = ChunkGenerator::lod_sampler(self.inner.clone());
        let generated = Arc::new(crate::lod::cache::DiskCache::new(
            self.save.root.join("data/horizon_far_terrain"),
            crate::lod::cache::build_key(self.registry.registry.len(), self.biomes.list.len()),
        ));
        let bake = crate::lod::bake::LodBake::open(
            &self.save.root,
            self.save.shape,
            self.registry.registry.len(),
            self.biomes.list.len(),
        )
        .map(Arc::new);
        Box::new(SavedLodSampler {
            generator: self,
            inner,
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated,
            bake,
        })
    }
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let mut chunk = self.save.load(x, z, self.registry.registry, &self.biomes)?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        chunk.entities.extend(self.save.load_entities(x, z));
        Some(chunk)
    }
    fn lod_surface(&self, worker: &mut G::Worker, x: i32, z: i32) -> Option<crate::lod::Surface> {
        // Resident/visited summaries override this procedural fallback in the LOD cache.
        self.inner.lod_surface(worker, x, z)
    }
    fn prepare_loaded(&self, worker: &mut G::Worker, chunk: &Chunk) {
        self.inner.prepare_loaded(worker, chunk);
    }
    fn all_idle(&self) {
        self.inner.all_idle();
    }
    fn idle(&self, worker: &mut G::Worker) {
        self.inner.idle(worker);
    }
    fn proto(&self, worker: &mut G::Worker, x: i32, z: i32) -> crate::streaming::Proto {
        let mut proto = self.inner.proto(worker, x, z);
        if let Some(saved) = self
            .save
            .load_proto(x, z, self.registry.registry, &self.biomes)
        {
            self.inner.restore_proto(&mut proto, saved);
        }
        proto
    }
    fn is_decorated(&self, proto: &crate::streaming::Proto) -> bool {
        self.inner.is_decorated(proto)
    }
    fn proto_status(
        &self,
        proto: &crate::streaming::Proto,
    ) -> Option<horizon_world::chunk_status::ChunkStatus> {
        self.inner.proto_status(proto)
    }
    fn proto_block(&self, proto: &crate::streaming::Proto, pos: [i32; 3]) -> Option<u32> {
        self.inner.proto_block(proto, pos)
    }
    fn light_view(&self, proto: &crate::streaming::Proto) -> Option<Chunk> {
        self.inner.light_view(proto)
    }
    fn non_air_sections(
        &self,
        proto: &crate::streaming::Proto,
    ) -> Option<crate::section_mask::SectionMask> {
        self.inner.non_air_sections(proto)
    }
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        self.inner.cache_census()
    }
    fn proto_heap_bytes(&self, proto: &crate::streaming::Proto) -> ([usize; 6], [usize; 16]) {
        self.inner.proto_heap_bytes(proto)
    }
    fn save_protos(&self, protos: &[&crate::streaming::Proto]) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let saved: Vec<SavedProto> = protos
            .iter()
            .filter_map(|p| self.inner.saved_proto(p))
            .collect();
        if let Err(e) = self
            .save
            .save_protos(saved, self.registry.registry, &self.biomes)
        {
            log::warn!("Horizon: failed to save proto chunks: {e}");
        }
    }
    fn save_finished(&self, mut chunk: Chunk) {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let key = (chunk.x, chunk.z);
        let entities = std::mem::take(&mut chunk.entities);
        let saved = self
            .save
            .save_later(&[Arc::new(chunk)], self.registry.registry, &self.biomes)
            .and_then(|_| self.save.save_entities_later(vec![(key, entities)]));
        if let Err(e) = saved {
            log::warn!("Horizon: failed to save chunk {} {}: {e}", key.0, key.1);
        }
    }
    fn decorates(&self) -> bool {
        self.inner.decorates()
    }
    fn decorates_beside_final(&self) -> bool {
        self.inner.decorates_beside_final()
    }
    fn proto_beside_final(
        &self,
        worker: &mut G::Worker,
        snapshot: &Chunk,
    ) -> Option<crate::streaming::Proto> {
        self.inner.proto_beside_final(worker, snapshot)
    }
    fn final_writes(
        &self,
        snapshot: &Chunk,
        proto: &crate::streaming::Proto,
    ) -> Vec<([i32; 3], u32)> {
        self.inner.final_writes(snapshot, proto)
    }
    fn final_block_entities(
        &self,
        entries: &[(&crate::streaming::Proto, &[([i32; 3], u32)])],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        self.inner.final_block_entities(entries)
    }
    fn decorate(&self, worker: &mut G::Worker, region: &mut [crate::streaming::Proto; 9]) {
        self.inner.decorate(worker, region);
    }
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        self.inner.finish(proto)
    }
}

/// `IOWorker.createOldDataForRegion`'s test of one record: saved before DataVersion 4997, or
/// with `blending_data`.
fn record_is_old(root: &NamedTag) -> bool {
    let before_blending = match root.tag.get("DataVersion") {
        Some(Tag::Int(version)) => *version < 4997,
        _ => true,
    };
    before_blending || root.tag.get("blending_data").is_some()
}

/// `MobEffectInstance.Details` read under its effect's `id`, the
/// `hidden_effect` chain included.
fn read_effect_details(e: &Tag, id: &str) -> crate::effects::SavedEffect {
    let get = |k: &str| number(e.get(k));
    // `Details` stores `amplifier` as `ExtraCodecs.UNSIGNED_BYTE`: the NBT
    // byte is signed, and `UnsignedBytes.toInt` maps it back to 0..=255. A
    // plain sign-extend turns 255 (byte -1) into -1, which
    // `EffectInstance::new` then clamps to the lowest level.
    let amplifier = match e.get("amplifier") {
        Some(Tag::Byte(v)) => i32::from(*v as u8),
        other => number(other).unwrap_or(0) as i32,
    };
    let particles = get("show_particles").is_none_or(|v| v != 0);
    crate::effects::SavedEffect {
        id: id.to_owned(),
        duration: get("duration").unwrap_or(0) as i32,
        amplifier,
        ambient: get("ambient").unwrap_or(0) != 0,
        particles,
        // Details.CODEC: `show_icon` defaults to `show_particles`.
        icon: get("show_icon").map_or(particles, |v| v != 0),
        hidden: e
            .get("hidden_effect")
            .map(|h| Box::new(read_effect_details(h, id))),
    }
}
/// `MobEffectInstance.Details.MAP_CODEC`'s fields, `hidden_effect` nested.
fn effect_details(e: &crate::effects::SavedEffect) -> Vec<(&'static str, Tag)> {
    let mut details = vec![("duration", Tag::Int(e.duration))];
    // Amplifier defaults to zero; when written it is the codec's unsigned
    // byte: `Mth.clamp` to 0..=255, then `byteValue`.
    if e.amplifier != 0 {
        details.push((
            "amplifier",
            Tag::Byte(e.amplifier.clamp(0, 255) as u8 as i8),
        ));
    }
    // optionalFieldOf("ambient", false) and ("show_particles", true):
    // defaults are not written; show_icon is always written.
    if e.ambient {
        details.push(("ambient", Tag::Byte(1)));
    }
    if !e.particles {
        details.push(("show_particles", Tag::Byte(0)));
    }
    details.push(("show_icon", Tag::Byte(i8::from(e.icon))));
    if let Some(hidden) = &e.hidden {
        details.push(("hidden_effect", Tag::compound(effect_details(hidden))));
    }
    details
}

#[cfg(test)]
mod tests;

/// Far-terrain floating-cell census on a real save: `HORIZON_TEST_RESOURCES`
/// (prepared data), `HORIZON_FAR_SAVE` (a world folder), `HORIZON_FAR_SEED`
/// and `HORIZON_FAR_AT` ("x,z", the camera). Samples the tiles each level
/// draws around the camera as the far sampler gives them (saved chunks up to
/// 16 blocks a cell, the seed beyond) and counts components of drawn cells
/// that touch neither the world's bottom nor the tile's halo.
#[cfg(test)]
mod far_census;

#[cfg(test)]
mod jukebox_song_clock_tests;

/// Added controls (H077): an old chunk's calculated blending data is taken
/// when its save is requested and written with `BlendingData.pack`; the raw
/// compound stays for the rest. The heights are the official 26.3 server's
/// for the constructed edge chunk (12, 0).
#[cfg(test)]
mod calculated_blending_save_tests;

#[cfg(test)]
mod biome_tests;
