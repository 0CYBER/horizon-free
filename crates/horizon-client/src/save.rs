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
use crate::{
    Result,
    biome::Biomes,
    chunk::{BIOME_STRATEGY, BLOCK_STRATEGY, Chunk, Light, MIN_Y, SECTIONS},
    streaming::ChunkGenerator,
};
use horizon_core::{
    paletted::PalettedContainer,
    position::{BlockPos, ChunkPos},
    ticks::{SavedTick, TickPriority, TickTypeId},
};
use horizon_world::{
    DATA_VERSION,
    chunk::BlockState,
    chunk_status::ChunkStatus,
    compression::Compression,
    heightmap::{Heightmap, HeightmapChunk, HeightmapType},
    nbt::{self, Compound, JavaString, Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile, RegionWriter},
    registry::{BlockRegistry, BlockStateId},
    worldgen::terrain::TerrainBlockStates,
};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

pub const VERSION_NAME: &str = "26.3";

/// Snapshot live block entities and custom names for a save batch without
/// mutating the world. Each map is visited once, retaining its iteration order
/// within each chunk (including repeated chunk coordinates in the input).
/// An empty batch skips both maps; one chunk uses the direct filter path.
pub fn chunks_with_block_entities<'a>(
    chunks: impl IntoIterator<Item = &'a Arc<Chunk>>,
    entities: &HashMap<(i32, i32, i32), crate::containers::BlockEntity>,
    names: &HashMap<(i32, i32, i32), String>,
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

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}
pub fn text(tag: Option<&Tag>) -> Option<String> {
    match tag {
        Some(Tag::String(s)) => Some(s.to_string_lossy()),
        _ => None,
    }
}
/// Any numeric tag as i64 (vanilla's `getIntOr` etc. accept every number type).
pub fn number(tag: Option<&Tag>) -> Option<i64> {
    Some(match tag? {
        Tag::Byte(v) => i64::from(*v),
        Tag::Short(v) => i64::from(*v),
        Tag::Int(v) => i64::from(*v),
        Tag::Long(v) => *v,
        Tag::Float(b) => f32::from_bits(*b) as i64,
        Tag::Double(b) => f64::from_bits(*b) as i64,
        _ => return None,
    })
}
pub(crate) fn float(tag: Option<&Tag>) -> Option<f64> {
    Some(match tag? {
        Tag::Float(b) => f64::from(f32::from_bits(*b)),
        Tag::Double(b) => f64::from_bits(*b),
        other => number(Some(other))? as f64,
    })
}
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

/// `dimensions/<namespace>/<path>` inside a world folder.
pub fn dimension_dir(world: &Path, dimension: &str) -> PathBuf {
    let (namespace, path) = dimension
        .split_once(':')
        .unwrap_or(("minecraft", dimension));
    world.join("dimensions").join(namespace).join(path)
}
/// `DirectoryLock`: `session.lock` holds "\u{2603}" (UTF-16BE) under an exclusive
/// file lock for as long as the world is open, so two instances cannot share it.
pub struct SessionLock(#[allow(dead_code)] fs::File);
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
}
/// Bring a world last saved by an older game version to the current format, as
/// 26.3 does when it opens one: the file fixes and level.dat first
/// (`FileFixerUpper`), then the DataFixers upgrades of every chunk, entity chunk,
/// POI chunk, player, saved data, advancements and statistics file. Vanilla runs
/// the latter as it reads each file; Horizon runs them all on opening, which reads
/// the same data. Worlds older than the ported fixes (1.17) are refused.
pub fn upgrade_older_world(world: &Path) -> Result<()> {
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
    horizon_world::datafix::files::upgrade_world(world)
        .map_err(|e| format!("upgrading {}: {e}", world.display()))?;
    Ok(())
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
    air: u16,
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
            air: registry
                .default_state("minecraft:air")
                .map_or(0, |s| s.0 as u16),
        }
    }
    /// Which states carry a block entity (`BlockFacts`), to drop entries for
    /// unknown block entities whose block is gone.
    pub fn with_block_entities(mut self, facts: &crate::blocks::BlockFacts) -> Self {
        self.block_entity = facts.states.iter().map(|s| s.block_entity).collect();
        self
    }
}

/// Save data loaded with a chunk that Horizon keeps but does not simulate.
#[derive(Clone, Debug, Default)]
pub struct ChunkExtras {
    /// Chunk root keys written back as loaded (structures, PostProcessing,
    /// InhabitedTime, blending data, ...).
    pub root: Compound,
    /// Block entities Horizon does not model.
    pub block_entities: Vec<Tag>,
    /// The NBT of the modelled block entities as loaded, to restore the fields and
    /// detail Horizon does not model when it saves them (`block_entity_nbt::overlay`).
    pub block_entity_sources: Vec<((i32, i32, i32), Tag)>,
    /// Every saved block entity's position in the order of the save's list,
    /// the order `SerializableChunkData` puts them in the chunk's maps.
    pub block_entity_order: Vec<(i32, i32, i32)>,
    /// Scheduled ticks (`block_ticks`/`fluid_ticks`): delays relative to the
    /// save time. Registered with the level's tick scheduler on arrival and
    /// replaced with the scheduler's pending ticks when saving.
    pub block_ticks: Vec<SavedTick>,
    pub fluid_ticks: Vec<SavedTick>,
    /// Game time written as `LastUpdate`.
    pub last_update: i64,
}

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
    let fluid = *FluidKind::ALL.get(kind as usize)?;
    Some(
        match fluid {
            FluidKind::Empty => return None,
            FluidKind::FlowingWater => "minecraft:flowing_water",
            FluidKind::Water => "minecraft:water",
            FluidKind::FlowingLava => "minecraft:flowing_lava",
            FluidKind::Lava => "minecraft:lava",
        }
        .to_owned(),
    )
}
fn fluid_id(name: &str) -> Option<u32> {
    use horizon_world::fluid::FluidKind;
    Some(match name {
        "minecraft:flowing_water" => FluidKind::FlowingWater,
        "minecraft:water" => FluidKind::Water,
        "minecraft:flowing_lava" => FluidKind::FlowingLava,
        "minecraft:lava" => FluidKind::Lava,
        _ => return None,
    } as u32)
}

fn encode_item(stack: &crate::items::ItemStack) -> Vec<(&'static str, Tag)> {
    let mut entries = vec![
        ("id", string(&stack.item)),
        ("count", Tag::Int(stack.count as i32)),
    ];
    if let Some(components) = crate::item_data::encode_components(stack) {
        entries.push(("components", components));
    }
    entries
}
/// ItemStack.CODEC (id, count, components).
pub fn item_to_nbt(stack: &crate::items::ItemStack) -> Tag {
    Tag::compound(encode_item(stack))
}
pub fn item_from_nbt(tag: &Tag) -> Option<crate::items::ItemStack> {
    let id = text(tag.get("id"))?;
    if id == "minecraft:air" {
        return None;
    }
    let count = number(tag.get("count"))
        .or_else(|| number(tag.get("Count")))
        .unwrap_or(1)
        .max(1) as u32;
    let (damage, extra) = crate::item_data::decode_components(tag.get("components"));
    Some(crate::items::ItemStack {
        item: Arc::from(id.as_str()),
        count,
        damage,
        extra,
    })
}
pub fn encode_items(slots: &[Option<crate::items::ItemStack>]) -> Tag {
    Tag::List(
        slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let mut entries = vec![("Slot", Tag::Byte(i as i8))];
                entries.extend(encode_item(s.as_ref()?));
                Some(Tag::compound(entries))
            })
            .collect(),
    )
}
pub fn decode_items(tag: Option<&Tag>, size: usize) -> Vec<Option<crate::items::ItemStack>> {
    let mut slots = vec![None; size];
    if let Some(Tag::List(items)) = tag {
        for it in items {
            let slot = number(it.get("Slot")).unwrap_or(0) as u8 as usize;
            if let Some(cell) = slots.get_mut(slot) {
                *cell = item_from_nbt(it);
            }
        }
    }
    slots
}
/// The block entity type ID for the block it sits in (a chest-like container
/// is a chest, trapped chest or barrel).
fn block_entity_id(e: &crate::containers::BlockEntity, block: &str) -> String {
    match (e, block) {
        (
            crate::containers::BlockEntity::Chest(_),
            "minecraft:trapped_chest" | "minecraft:barrel" | "minecraft:dropper",
        ) => block.to_owned(),
        (crate::containers::BlockEntity::Chest(_), b) if b.ends_with("shulker_box") => {
            "minecraft:shulker_box".to_owned()
        }
        _ => e.id().to_owned(),
    }
}
pub fn encode_block_entity(
    p: (i32, i32, i32),
    e: &crate::containers::BlockEntity,
    block: &str,
) -> Tag {
    use crate::containers::BlockEntity;
    let mut entries = vec![
        ("id", string(&block_entity_id(e, block))),
        ("x", Tag::Int(p.0)),
        ("y", Tag::Int(p.1)),
        ("z", Tag::Int(p.2)),
        ("keepPacked", Tag::Byte(0)),
        ("components", Tag::Compound(Compound::new())),
    ];
    match e {
        BlockEntity::Chest(items) => entries.push(("Items", encode_items(items))),
        BlockEntity::Furnace(f) => {
            entries.push(("Items", encode_items(&f.items)));
            entries.push(("lit_time_remaining", Tag::Int(f.lit_remaining)));
            entries.push(("lit_total_time", Tag::Int(f.lit_total)));
            entries.push(("cooking_time_spent", Tag::Int(f.cooking_timer)));
            entries.push(("cooking_total_time", Tag::Int(f.cooking_total)));
            entries.push(("speed_multiplier", Tag::Float(f.speed.to_bits())));
            entries.push(("RecipesUsed", Tag::Compound(Compound::new())));
        }
        BlockEntity::Brewing(b) => {
            entries.push(("Items", encode_items(&b.items)));
            entries.push(("BrewTime", Tag::Int(b.brew_time)));
            entries.push(("total_brew_time", Tag::Int(b.total_brew_time)));
            entries.push(("Fuel", Tag::Int(b.fuel)));
            entries.push(("total_fuel", Tag::Int(b.total_fuel)));
            entries.push(("speed_multiplier", Tag::Float(b.speed.to_bits())));
        }
        BlockEntity::Sign(sign) => entries.extend(sign.encode()),
        BlockEntity::Jukebox(j) => {
            if let Some(record) = &j.record {
                entries.push(("RecordItem", item_to_nbt(record)));
            }
            entries.push(("ticks_since_song_started", Tag::Long(j.ticks)));
        }
        BlockEntity::Beacon(b) => {
            if let Some(p) = &b.primary {
                entries.push(("primary_effect", string(p)));
            }
            if let Some(s) = &b.secondary {
                entries.push(("secondary_effect", string(s)));
            }
            entries.push(("Levels", Tag::Int(b.levels)));
        }
        BlockEntity::Lectern(l) => {
            if let Some(book) = &l.book {
                entries.push(("Book", item_to_nbt(book)));
                entries.push(("Page", Tag::Int(l.page)));
            }
        }
        BlockEntity::Sculk(s) => entries.extend(s.to_nbt()),
        BlockEntity::Crafter(c) => {
            entries.push(("Items", encode_items(&c.items)));
            entries.push((
                "disabled_slots",
                Tag::IntArray(
                    (0..9)
                        .filter(|&i| c.disabled[i])
                        .map(|i| i as i32)
                        .collect(),
                ),
            ));
            entries.push(("triggered", Tag::Int(i32::from(c.triggered))));
            entries.push(("crafting_ticks_remaining", Tag::Int(c.crafting_ticks)));
        }
        BlockEntity::Banner(b) => {
            if !b.layers.is_empty() {
                entries.push(("patterns", crate::banner::layers_to_nbt(&b.layers)));
            }
            if let Some(name) = &b.name {
                entries.push(("CustomName", string(name)));
            }
        }
        BlockEntity::CreakingHeart(h) => {
            if let Some((_, uuid)) = h.creaking {
                entries.push(("creaking", uuid_tag(uuid)));
            }
        }
        BlockEntity::Spawner(s) => entries.extend(s.save()),
        BlockEntity::Beehive(h) => entries.extend(h.to_nbt()),
    }
    Tag::compound(entries)
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
pub fn decode_block_entity(tag: &Tag) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    use crate::containers::BlockEntity;
    let int = |k: &str| number(tag.get(k)).map(|v| v as i32);
    let id = text(tag.get("id"))?;
    let pos = (int("x")?, int("y")?, int("z")?);
    if id == "minecraft:jukebox" {
        let record = tag.get("RecordItem").and_then(item_from_nbt);
        let ticks = number(tag.get("ticks_since_song_started")).unwrap_or(0);
        let playing = record.is_some();
        return Some((
            pos,
            BlockEntity::Jukebox(crate::containers::Jukebox {
                record,
                ticks,
                playing,
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
        let lectern = crate::containers::Lectern {
            book: tag.get("Book").and_then(item_from_nbt),
            page: int("Page").unwrap_or(0),
        };
        return Some((pos, BlockEntity::Lectern(lectern)));
    }
    if id == "minecraft:banner" {
        return Some((
            pos,
            BlockEntity::Banner(crate::containers::Banner {
                layers: crate::banner::layers_from_nbt(tag.get("patterns")),
                name: text(tag.get("CustomName")),
            }),
        ));
    }
    if id == "minecraft:beacon" {
        let beacon = crate::containers::Beacon {
            levels: int("Levels").unwrap_or(0),
            primary: text(tag.get("primary_effect")),
            secondary: text(tag.get("secondary_effect")),
            last_check_y: i32::MIN / 2,
            ..Default::default()
        };
        return Some((pos, BlockEntity::Beacon(Box::new(beacon))));
    }
    // A spawner's block entity id is not its block's name.
    let block = match id.as_str() {
        "minecraft:mob_spawner" => "minecraft:spawner",
        other => other,
    };
    let mut entity = BlockEntity::for_block(block)?;
    match &mut entity {
        BlockEntity::Chest(items) => *items = decode_items(tag.get("Items"), items.len()),
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
            b.speed = match tag.get("speed_multiplier") {
                Some(Tag::Float(f)) => f32::from_bits(*f),
                _ => 1.0,
            };
        }
        BlockEntity::Sign(sign) => sign.decode(tag),
        BlockEntity::Sculk(s) => s.load(tag),
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
            c.triggered = int("triggered").unwrap_or(0) != 0;
            c.state_triggered = None;
            c.crafting_ticks = int("crafting_ticks_remaining").unwrap_or(0);
        }
        BlockEntity::Jukebox(_)
        | BlockEntity::Beacon(_)
        | BlockEntity::Lectern(_)
        | BlockEntity::Banner(_)
        | BlockEntity::CreakingHeart(_) => {}
        BlockEntity::Spawner(s) => **s = crate::mobs::spawner::Spawner::load(tag),
        BlockEntity::Beehive(h) => **h = crate::beehive::Beehive::from_nbt(tag),
    }
    Some((pos, entity))
}

struct HeightmapView<'a> {
    chunk: &'a Chunk,
    tables: &'a SaveTables,
}
impl HeightmapChunk for HeightmapView<'_> {
    type State = crate::assets::StateFacts;
    fn min_y(&self) -> i32 {
        MIN_Y
    }
    fn height(&self) -> i32 {
        SECTIONS as i32 * 16
    }
    fn highest_section_y(&self) -> i32 {
        let top = self
            .chunk
            .sections
            .iter()
            .rposition(|s| !s.is_uniform(self.tables.air))
            .unwrap_or(0);
        MIN_Y + top as i32 * 16
    }
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State {
        self.tables
            .heightmaps
            .heightmap_state(self.chunk.block(x, y, z))
    }
}
/// Heightmaps kept after world generation (`ChunkStatus.FULL.heightmapsAfter`).
fn heightmaps(chunk: &Chunk, tables: &SaveTables) -> Tag {
    let kinds = [
        HeightmapType::WorldSurface,
        HeightmapType::OceanFloor,
        HeightmapType::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves,
    ];
    let mut maps: Vec<Heightmap> = kinds
        .iter()
        .filter_map(|k| Heightmap::new(*k, MIN_Y, SECTIONS as i32 * 16).ok())
        .collect();
    let view = HeightmapView { chunk, tables };
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
/// The heightmaps a ClientboundLevelChunkPacketData carries (the types with
/// sendToClient: WORLD_SURFACE, MOTION_BLOCKING, MOTION_BLOCKING_NO_LEAVES),
/// as (network id, packed longs) in Heightmap.Types order.
pub fn client_heightmaps(chunk: &Chunk, tables: &SaveTables) -> Vec<(i32, Vec<i64>)> {
    let kinds = [
        (1, HeightmapType::WorldSurface),
        (4, HeightmapType::MotionBlocking),
        (5, HeightmapType::MotionBlockingNoLeaves),
    ];
    let mut maps: Vec<(i32, Heightmap)> = kinds
        .iter()
        .filter_map(|(id, k)| {
            Heightmap::new(*k, MIN_Y, SECTIONS as i32 * 16)
                .ok()
                .map(|m| (*id, m))
        })
        .collect();
    let view = HeightmapView { chunk, tables };
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
            let v = match light {
                Light::Uniform(v) => *v,
                Light::Full(a) => a[j],
            };
            if sky { v >> 4 } else { v & 15 }
        };
        *byte = value(2 * i) | value(2 * i + 1) << 4;
    }
    out
}

pub fn encode(chunk: &Chunk, registry: &BlockRegistry, biomes: &Biomes) -> NamedTag {
    encode_with(chunk, registry, biomes, None, true)
}
/// The chunk NBT vanilla's `SerializableChunkData.write` produces for a full
/// chunk. `sky_light` is false in dimensions without skylight.
pub fn encode_with(
    chunk: &Chunk,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    sky_light: bool,
) -> NamedTag {
    let mut sections = Vec::new();
    let air = tables.map_or(0, |t| t.air);
    // Sky layers below the highest section with light other than 15 are
    // written (even all-zero ones): vanilla derives missing layers from the
    // layer above them.
    let top_sky = chunk.sections.iter().rposition(|s| match &s.light {
        Light::Uniform(v) => v >> 4 != 15,
        Light::Full(a) => a.iter().any(|v| v >> 4 != 15),
    });
    for (si, section) in chunk.sections.iter().enumerate() {
        let y = (MIN_Y >> 4) + si as i32;
        // PalettedContainer.pack: distinct states in index order, indices at
        // the saved width.
        let packed = section.blocks.pack();
        let mut states = vec![(
            "palette",
            Tag::List(
                packed
                    .palette
                    .iter()
                    .map(|&id| {
                        let state = registry.state(BlockStateId(id));
                        match state {
                            // BlockState.CODEC: the block name for default states.
                            Some(s)
                                if tables.is_some_and(|t| {
                                    t.default_state.get(id as usize) == Some(&true)
                                }) =>
                            {
                                string(&s.name)
                            }
                            Some(s) => s.to_nbt(),
                            None => string("minecraft:air"),
                        }
                    })
                    .collect(),
            ),
        )];
        if let Some(data) = &packed.data {
            states.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        let biome_packed = section.biomes.pack();
        let mut biome_tag = vec![(
            "palette",
            Tag::List(
                biome_packed
                    .palette
                    .iter()
                    .map(|&b| string(&biomes.get(b as u16).name))
                    .collect(),
            ),
        )];
        if let Some(data) = &biome_packed.data {
            biome_tag.push((
                "data",
                Tag::LongArray(data.iter().map(|&w| w as i64).collect()),
            ));
        }
        let mut entries = vec![
            ("Y", Tag::Byte(y as i8)),
            ("block_states", Tag::compound(states)),
            ("biomes", Tag::compound(biome_tag)),
        ];
        if chunk.lit {
            let block_lit = match &section.light {
                Light::Uniform(v) => v & 15 != 0,
                Light::Full(a) => a.iter().any(|v| v & 15 != 0),
            };
            if block_lit {
                entries.push(("BlockLight", Tag::ByteArray(nibbles(&section.light, false))));
            }
            if sky_light && top_sky.is_some_and(|top| si <= top + 1) {
                entries.push(("SkyLight", Tag::ByteArray(nibbles(&section.light, true))));
            }
        }
        let _ = air;
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
        .map(|(p, e)| {
            let block = block_name(p);
            let mut current = encode_block_entity(*p, e, &block);
            if let Some((_, name)) = chunk.block_names.iter().find(|(q, _)| q == p)
                && let Tag::Compound(map) = &mut current
            {
                map.insert("CustomName".into(), string(name));
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
                        |(q, e)| encode_block_entity(q, &e, &block),
                    );
                    merge_block_entity(original, &loaded, &current)
                }
                None => current,
            };
            crate::block_entity_nbt::canonical(&merged).unwrap_or(merged)
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
        ("yPos", Tag::Int(MIN_Y >> 4)),
        ("Status", string("minecraft:full")),
        ("LastUpdate", Tag::Long(extras.map_or(0, |e| e.last_update))),
        ("isLightOn", Tag::Byte(i8::from(chunk.lit))),
        ("sections", Tag::List(sections)),
        ("block_entities", Tag::List(block_entities)),
    ] {
        put(&mut root, key, value);
    }
    if let Some(Tag::Compound(map)) = Some(&mut root) {
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
        map.entry("PostProcessing".into())
            .or_insert_with(|| Tag::List(vec![Tag::List(Vec::new()); SECTIONS]));
    }
    if let Some(t) = tables {
        put(&mut root, "Heightmaps", heightmaps(chunk, t));
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
    decode_with(root, registry, biomes, None)
}
/// Only full chunks load here; partially generated ones (vanilla's world
/// border of proto chunks) resume generation through `decode_proto`.
pub fn decode_with(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
) -> Option<Chunk> {
    if text(root.tag.get("Status")).is_some_and(|s| s != "minecraft:full" && s != "full") {
        return None;
    }
    decode_column(root, registry, biomes, tables)
}
/// `decode_with` whatever the chunk's status.
fn decode_column(
    root: &NamedTag,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
) -> Option<Chunk> {
    let tag = &root.tag;
    let Tag::Int(x) = tag.get("xPos")? else {
        return None;
    };
    let Tag::Int(z) = tag.get("zPos")? else {
        return None;
    };
    let mut chunk = Chunk::empty(*x, *z);
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
    let air = registry.default_state("minecraft:air")?.0 as u16;
    let mut cache: HashMap<String, u16> = HashMap::new();
    let Some(Tag::List(sections)) = tag.get("sections") else {
        return Some(chunk);
    };
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let si = y as i32 - (MIN_Y >> 4);
        if !(0..SECTIONS as i32).contains(&si) {
            continue;
        }
        if let Some(states) = section.get("block_states")
            && let Some(Tag::List(palette)) = states.get("palette")
        {
            let ids: Vec<u16> = palette
                .iter()
                .map(|p| {
                    let state = BlockState::from_nbt(p).ok();
                    let key = state
                        .as_ref()
                        .map(|s| format!("{}{:?}", s.name, s.properties))
                        .unwrap_or_default();
                    *cache.entry(key).or_insert_with(|| {
                        state
                            .and_then(|s| {
                                registry.id(&s).or_else(|| {
                                    // Missing properties take the block's defaults.
                                    let default = registry.default_state(&s.name)?;
                                    let mut full = registry.state(default)?.clone();
                                    for (k, v) in &s.properties {
                                        if full.properties.contains_key(k) {
                                            full.properties.insert(k.clone(), v.clone());
                                        }
                                    }
                                    registry.id(&full).or(Some(default))
                                })
                            })
                            .map_or(air, |id| id.0 as u16)
                    })
                })
                .collect();
            let target = Arc::make_mut(&mut chunk.sections[si as usize]);
            let data = match states.get("data") {
                Some(Tag::LongArray(data)) => {
                    Some(data.iter().map(|&w| w as u64).collect::<Vec<_>>())
                }
                _ => None,
            };
            let palette: Vec<u32> = ids.iter().map(|&id| u32::from(id)).collect();
            if let Ok(blocks) =
                PalettedContainer::from_packed(BLOCK_STRATEGY, palette, data.as_deref())
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
                    Tag::String(s) => biomes.id(&s.to_string_lossy()),
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
            let target = Arc::make_mut(&mut chunk.sections[si as usize]);
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
            let int = |k: &str| number(e.get(k)).map(|v| v as i32);
            if let (Some(x), Some(y), Some(z)) = (int("x"), int("y"), int("z")) {
                extras.block_entity_order.push((x, y, z));
            }
            match decode_block_entity(e) {
                Some(be) => {
                    if let Some(name) = text(e.get("CustomName")) {
                        chunk.block_names.push((be.0, name));
                    }
                    extras.block_entity_sources.push((be.0, e.clone()));
                    chunk.block_entities.push(be);
                }
                None => extras.block_entities.push(e.clone()),
            }
        }
    }
    if let Tag::Compound(map) = tag {
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

/// A proto chunk saved before it became final. Decoration writes into a
/// chunk's neighbours, and decorated chunks wait for theirs, so the chunks
/// past the finished ones hold writes that generating them again would lose
/// (whole structure pieces, trees, ores). Vanilla saves its proto chunks too.
pub struct SavedProto {
    /// Blocks, scheduled ticks (in `saved`) and post-processing marks.
    pub column: Chunk,
    /// Decorated itself (`features`), not only written to by its neighbours
    /// (`terrain`).
    pub decorated: bool,
    /// Block entity data in placement order, with `x`, `y` and `z`.
    pub block_entities: Vec<Tag>,
    /// Entities placed by structures, with absolute `Pos`.
    pub entities: Vec<Tag>,
}
pub fn encode_proto(
    proto: &SavedProto,
    registry: &BlockRegistry,
    biomes: &Biomes,
    tables: Option<&SaveTables>,
    sky_light: bool,
) -> NamedTag {
    let mut root = encode_with(&proto.column, registry, biomes, tables, sky_light);
    let status = if proto.decorated {
        ChunkStatus::Features
    } else {
        ChunkStatus::Terrain
    };
    // ProtoChunk.postProcessing: one list per section, packed section offsets.
    let mut marks = vec![Vec::new(); SECTIONS];
    for &[x, y, z] in &proto.column.postprocess {
        if let Some(list) = marks.get_mut(((y - MIN_Y) >> 4) as usize) {
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
) -> Option<SavedProto> {
    let status = ChunkStatus::parse(&text(root.tag.get("Status"))?).ok()?;
    if !(ChunkStatus::Terrain..ChunkStatus::Full).contains(&status) {
        return None;
    }
    let mut column = decode_column(root, registry, biomes, tables)?;
    column.block_entities.clear();
    let base = number(root.tag.get("yPos")).map_or(MIN_Y, |y| y as i32 * 16);
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
    let mut layers: Vec<(Option<&[u8]>, Option<&[u8]>)> = vec![(None, None); SECTIONS];
    for section in sections {
        let Some(y) = number(section.get("Y")) else {
            continue;
        };
        let si = y as i32 - (MIN_Y >> 4);
        if !(0..SECTIONS as i32).contains(&si) {
            continue;
        }
        let layer = |name: &str| match section.get(name) {
            Some(Tag::ByteArray(a)) if a.len() == 2048 => Some(&a[..]),
            _ => None,
        };
        layers[si as usize] = (layer("SkyLight"), layer("BlockLight"));
    }
    let sky_default = if sky_light { 15u8 } else { 0 };
    // Sky nibbles of the section above's bottom layer, going down.
    let mut above: [u8; 256] = [sky_default; 256];
    for si in (0..SECTIONS).rev() {
        let (sky, block) = layers[si];
        let mut data = Box::new([0u8; 4096]);
        let nibble = |a: &[u8], i: usize| (a[i >> 1] >> ((i & 1) * 4)) & 15;
        for (i, packed) in data.iter_mut().enumerate() {
            let s = match sky {
                Some(a) if sky_light => nibble(a, i),
                _ => above[i & 255],
            };
            let b = block.map_or(0, |a| nibble(a, i));
            *packed = s << 4 | b;
        }
        for (i, a) in above.iter_mut().enumerate() {
            *a = data[i] >> 4;
        }
        let mut section = (*chunk.sections[si]).clone();
        section.light = Light::Full(data);
        section.compact();
        chunk.sections[si] = Arc::new(section);
    }
    chunk.lit = true;
    chunk.light_loaded = true;
}

type RegionCache = Mutex<HashMap<(i32, i32), Option<Arc<Mutex<RegionFile>>>>>;
/// Rewrite every region file in `folder` touched by `records`, keeping its
/// other chunks. `None` deletes a chunk.
fn write_region_records(
    root: &Path,
    cache: &RegionCache,
    folder: &str,
    records: Vec<((i32, i32), Option<Arc<NamedTag>>)>,
) -> Result<()> {
    let mut by_region: BTreeMap<(i32, i32), Vec<((i32, i32), Option<Arc<NamedTag>>)>> =
        BTreeMap::new();
    for (key, tag) in records {
        by_region
            .entry((key.0 >> 5, key.1 >> 5))
            .or_default()
            .push((key, tag));
    }
    if by_region.is_empty() {
        return Ok(());
    }
    let staging = root.join(folder).join(".staging");
    fs::create_dir_all(&staging)?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    for ((rx, rz), list) in by_region {
        let path = region_path(root, folder, rx, rz);
        let mut records: BTreeMap<(i32, i32), ChunkRecord> = BTreeMap::new();
        let only_deletes = list.iter().all(|(_, tag)| tag.is_none());
        if let Ok(mut existing) = RegionFile::open(&path) {
            let positions: Vec<ChunkPos> = existing.chunks().map(|(p, _)| p).collect();
            if only_deletes
                && !positions
                    .iter()
                    .any(|p| list.iter().any(|(k, _)| *k == (p.x, p.z)))
            {
                continue;
            }
            for p in positions {
                if let Ok(Some(r)) = existing.read(p) {
                    records.insert((p.x, p.z), r);
                }
            }
        } else if only_deletes {
            continue;
        }
        // Deleting chunks the region doesn't hold changes nothing: leave
        // the file alone (unloading empty entity chunks asks for that).
        let mut changed = false;
        for (key, tag) in list {
            match tag {
                Some(tag) => {
                    changed = true;
                    records.insert(
                        key,
                        ChunkRecord::from_nbt(
                            &tag,
                            Compression::Zlib,
                            timestamp,
                            Limits::default(),
                        )?,
                    );
                }
                None => {
                    changed |= records.remove(&key).is_some();
                }
            }
        }
        if !changed {
            continue;
        }
        if records.is_empty() {
            let _ = fs::remove_file(&path);
        } else {
            let temporary = staging.join(format!("r.{rx}.{rz}.mca"));
            let _ = fs::remove_file(&temporary);
            let mut writer = RegionWriter::create(&temporary)?;
            for ((x, z), r) in &records {
                writer.write(ChunkPos::new(*x, *z), r)?;
            }
            writer.finish()?;
            fs::rename(&temporary, &path)?;
        }
        cache
            .lock()
            .map_err(|_| "region cache poisoned")?
            .remove(&(rx, rz));
    }
    let _ = fs::remove_dir(&staging);
    Ok(())
}

fn region_path(root: &Path, folder: &str, rx: i32, rz: i32) -> PathBuf {
    root.join(format!("{folder}/r.{rx}.{rz}.mca"))
}

/// A record waiting for the writer thread.
#[derive(Clone)]
enum Record {
    Tag(Arc<NamedTag>),
    /// A chunk to encode on the writer thread.
    Chunk(Arc<Chunk>, Arc<Encoding>),
    Delete,
}
/// What encoding a chunk needs, captured when it is queued.
struct Encoding {
    registry: &'static BlockRegistry,
    biomes: Arc<Biomes>,
    tables: Option<Arc<SaveTables>>,
    sky_light: bool,
}
impl Record {
    fn tag(&self) -> Option<Arc<NamedTag>> {
        match self {
            Record::Tag(t) => Some(t.clone()),
            Record::Chunk(c, e) => Some(Arc::new(encode_with(
                c,
                e.registry,
                &e.biomes,
                e.tables.as_deref(),
                e.sky_light,
            ))),
            Record::Delete => None,
        }
    }
}
/// Chunk and entity records waiting for the writer thread, by (entities?,
/// x, z): reads see them before the region files do.
type PendingWrites = Mutex<HashMap<(bool, i32, i32), (u64, Record)>>;
type WriteBatch = (bool, Vec<((i32, i32), Record, u64)>);

/// The background thread that encodes chunks and rewrites region files, in
/// submission order.
struct Writer {
    send: mpsc::Sender<WriteBatch>,
    thread: std::thread::JoinHandle<()>,
}

/// Writes still queued or running, with a condition to wait for none.
#[derive(Default)]
struct Outstanding {
    count: Mutex<usize>,
    done: Condvar,
}
/// One dimension folder (`dimensions/minecraft/<name>`). Region files are
/// read lazily by worker threads.
pub struct SaveDir {
    pub root: PathBuf,
    regions: Arc<RegionCache>,
    entity_regions: Arc<RegionCache>,
    pending: Arc<PendingWrites>,
    sequence: AtomicU64,
    writer: Mutex<Option<Writer>>,
    outstanding: Arc<Outstanding>,
    /// Set by `Persistent` before the first load.
    pub tables: OnceLock<Arc<SaveTables>>,
    /// False in dimensions without skylight (no SkyLight layers are written).
    pub sky_light: bool,
}
impl SaveDir {
    pub fn new(root: PathBuf) -> Self {
        let sky_light = !root.ends_with("the_nether") && !root.ends_with("DIM-1");
        Self {
            root,
            regions: Default::default(),
            entity_regions: Default::default(),
            pending: Default::default(),
            sequence: AtomicU64::new(0),
            writer: Mutex::new(None),
            outstanding: Default::default(),
            tables: OnceLock::new(),
            sky_light,
        }
    }
    fn region_path(&self, folder: &str, rx: i32, rz: i32) -> PathBuf {
        region_path(&self.root, folder, rx, rz)
    }
    fn read_record(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        let pending = self
            .pending
            .lock()
            .ok()?
            .get(&(folder != "region", x, z))
            .map(|(_, r)| r.clone());
        if let Some(record) = pending {
            return record.tag().map(|t| (*t).clone());
        }
        let key = (x >> 5, z >> 5);
        let cache = if folder == "region" {
            &self.regions
        } else {
            &self.entity_regions
        };
        let region = {
            let mut regions = cache.lock().ok()?;
            regions
                .entry(key)
                .or_insert_with(|| {
                    RegionFile::open(self.region_path(folder, key.0, key.1))
                        .ok()
                        .map(|r| Arc::new(Mutex::new(r)))
                })
                .clone()?
        };
        let record = region.lock().ok()?.read(ChunkPos::new(x, z)).ok()??;
        record.nbt(Limits::default()).ok()
    }
    pub fn load(&self, x: i32, z: i32, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
        let root = self.read_record("region", x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        let mut chunk = decode_with(&root, registry, biomes, tables)?;
        load_light(&root.tag, &mut chunk, self.sky_light);
        Some(chunk)
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
        decode_proto(&root, registry, biomes, self.tables.get().map(|t| &**t))
    }
    /// Queue proto chunks for the writer thread; loads see them meanwhile.
    pub fn save_protos(
        &self,
        protos: &[SavedProto],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<()> {
        let tables = self.tables.get().map(|t| &**t);
        let records = protos
            .iter()
            .map(|p| {
                let tag = encode_proto(p, registry, biomes, tables, self.sky_light);
                ((p.column.x, p.column.z), Some(tag))
            })
            .collect();
        self.write_records("region", records, false)
    }
    /// Entities saved for a chunk (`entities/`; older chunks may still keep
    /// them in the chunk's own `entities` list).
    pub fn load_entities(&self, x: i32, z: i32) -> Vec<Tag> {
        let mut out = Vec::new();
        if let Some(root) = self.read_record("entities", x, z)
            && let Some(Tag::List(list)) = root.tag.get("Entities")
        {
            out.extend(list.iter().cloned());
        }
        out
    }
    /// Queue `records` for the writer thread; with `wait`, return once
    /// they and every earlier write are on disk.
    fn write_records(
        &self,
        folder: &str,
        records: Vec<((i32, i32), Option<NamedTag>)>,
        wait: bool,
    ) -> Result<()> {
        let records = records
            .into_iter()
            .map(|(k, t)| (k, t.map_or(Record::Delete, |t| Record::Tag(Arc::new(t)))))
            .collect();
        self.queue(folder != "region", records, wait)
    }
    fn queue(&self, entities: bool, records: Vec<((i32, i32), Record)>, wait: bool) -> Result<()> {
        if !records.is_empty() {
            let mut batch = Vec::with_capacity(records.len());
            {
                let mut pending = self.pending.lock().map_err(|_| "pending writes poisoned")?;
                for (key, record) in records {
                    let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
                    pending.insert((entities, key.0, key.1), (seq, record.clone()));
                    batch.push((key, record, seq));
                }
            }
            *self
                .outstanding
                .count
                .lock()
                .map_err(|_| "writer poisoned")? += 1;
            let mut writer = self.writer.lock().map_err(|_| "writer poisoned")?;
            let writer = writer.get_or_insert_with(|| self.start_writer());
            if writer.send.send((entities, batch)).is_err() {
                return Err("the region writer stopped".into());
            }
        }
        if wait {
            self.flush();
        }
        Ok(())
    }
    fn start_writer(&self) -> Writer {
        let (send, receive) = mpsc::channel::<WriteBatch>();
        let root = self.root.clone();
        let (regions, entity_regions) = (self.regions.clone(), self.entity_regions.clone());
        let pending = self.pending.clone();
        let outstanding = self.outstanding.clone();
        let thread = std::thread::Builder::new()
            .name("horizon-region-writer".into())
            .spawn(move || {
                for (entities, batch) in receive {
                    let (folder, cache) = if entities {
                        ("entities", &entity_regions)
                    } else {
                        ("region", &regions)
                    };
                    let records = batch.iter().map(|(k, r, _)| (*k, r.tag())).collect();
                    if let Err(e) = write_region_records(&root, cache, folder, records) {
                        log::warn!("Horizon: failed to write {folder} files: {e}");
                    }
                    // Readers find the files (and a fresh region cache) from here on.
                    if let Ok(mut pending) = pending.lock() {
                        for (k, _, seq) in &batch {
                            if pending
                                .get(&(entities, k.0, k.1))
                                .is_some_and(|p| p.0 == *seq)
                            {
                                pending.remove(&(entities, k.0, k.1));
                            }
                        }
                    }
                    if let Ok(mut count) = outstanding.count.lock() {
                        *count -= 1;
                        outstanding.done.notify_all();
                    }
                }
            })
            .expect("spawn region writer");
        Writer { send, thread }
    }
    /// Wait until every queued write is on disk.
    pub fn flush(&self) {
        let Ok(mut count) = self.outstanding.count.lock() else {
            return;
        };
        while *count > 0 {
            count = match self.outstanding.done.wait(count) {
                Ok(c) => c,
                Err(_) => return,
            };
        }
    }
    /// Rewrite every region touched by `chunks`, keeping their other chunks.
    pub fn save(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<usize> {
        let tables = self.tables.get().map(|t| &**t);
        let records = chunks
            .iter()
            .map(|c| {
                (
                    (c.x, c.z),
                    Some(encode_with(c, registry, biomes, tables, self.sky_light)),
                )
            })
            .collect();
        self.write_records("region", records, true)?;
        Ok(chunks.len())
    }
    /// `save` on the writer thread: returns before the files are written;
    /// loads see the chunks meanwhile.
    pub fn save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<usize> {
        let encoding = Arc::new(Encoding {
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            sky_light: self.sky_light,
        });
        let records = chunks
            .iter()
            .map(|c| ((c.x, c.z), Record::Chunk(c.clone(), encoding.clone())))
            .collect();
        self.queue(false, records, false)?;
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

impl Drop for SaveDir {
    /// Finish queued writes before the world is let go.
    fn drop(&mut self) {
        if let Some(writer) = self.writer.get_mut().ok().and_then(Option::take) {
            drop(writer.send);
            let _ = writer.thread.join();
        }
    }
}

pub fn read_dat(path: &Path) -> Option<Tag> {
    let bytes = fs::read(path).ok()?;
    let raw = match Compression::Gzip.decode(&bytes, 64 << 20) {
        Ok(raw) => raw,
        Err(_) => bytes,
    };
    Some(nbt::read_named(&raw, Limits::default()).ok()?.0.tag)
}
/// Write a gzip NBT file the way vanilla does (`<name>_new`, keep `_old`).
pub fn write_dat(path: &Path, tag: Tag) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag,
        },
        Limits::default(),
    )
    .map_err(|e| e.to_string())?;
    let compressed = Compression::Gzip.encode(&bytes)?;
    let file = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("data.dat");
    let temporary = path.with_file_name(format!("{file}_new"));
    fs::write(&temporary, compressed)?;
    if path.exists() {
        let old = match file.strip_suffix(".dat") {
            Some(stem) => path.with_file_name(format!("{stem}.dat_old")),
            None => path.with_file_name(format!("{file}_old")),
        };
        let _ = fs::rename(path, old);
    }
    fs::rename(temporary, path)?;
    Ok(())
}
/// `SavedDataStorage`: `{data: ..., DataVersion}` under `data/minecraft/`.
fn saved_data_path(world: &Path, name: &str) -> PathBuf {
    world
        .join("data")
        .join("minecraft")
        .join(format!("{name}.dat"))
}
fn read_saved_data(world: &Path, name: &str) -> Option<Tag> {
    read_dat(&saved_data_path(world, name))?
        .get("data")
        .cloned()
}
/// The world's generator settings: world_gen_settings.dat (26.3) or
/// level.dat's Data.WorldGenSettings.
pub fn world_gen_settings(world: &Path) -> Option<Tag> {
    read_saved_data(world, "world_gen_settings").or_else(|| {
        read_dat(&world.join("level.dat"))?
            .get("Data")?
            .get("WorldGenSettings")
            .cloned()
    })
}
fn write_saved_data(world: &Path, name: &str, data: Tag) -> Result<()> {
    write_dat(
        &saved_data_path(world, name),
        Tag::compound([("data", data), ("DataVersion", Tag::Int(DATA_VERSION))]),
    )
}
pub fn uuid_text(u: [i32; 4]) -> String {
    let hex: String = u.iter().map(|v| format!("{:08x}", *v as u32)).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
pub fn uuid_from_text(s: &str) -> Option<[i32; 4]> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0i32; 4];
    for (i, v) in out.iter_mut().enumerate() {
        *v = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok()? as i32;
    }
    Some(out)
}
/// A random (version 4) UUID as vanilla's four-int form.
pub fn random_uuid() -> [i32; 4] {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut s = t ^ (std::process::id() as u64).rotate_left(32) ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let (a, b) = (next(), next());
    let a = (a & !0xF000) | 0x4000;
    let b = (b & !(0xC000_0000_0000_0000)) | 0x8000_0000_0000_0000;
    [(a >> 32) as i32, a as i32, (b >> 32) as i32, b as i32]
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
pub(crate) fn doubles(values: &[f64]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Double(v.to_bits())).collect())
}
pub(crate) fn floats(values: &[f32]) -> Tag {
    Tag::List(values.iter().map(|v| Tag::Float(v.to_bits())).collect())
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

/// Default game rules of a new 26.3 world (`data/minecraft/game_rules.dat`).
const GAME_RULES: [(&str, i32); 58] = [
    ("spawn_wandering_traders", 1),
    ("block_drops", 1),
    ("reduced_debug_info", 0),
    ("show_death_messages", 1),
    ("spawner_blocks_work", 1),
    ("spawn_monsters", 1),
    ("tnt_explodes", 1),
    ("player_movement_check", 1),
    ("immediate_respawn", 0),
    ("block_explosion_drop_decay", 1),
    ("spread_vines", 1),
    ("max_entity_cramming", 24),
    ("forgive_dead_players", 1),
    ("fall_damage", 1),
    ("send_command_feedback", 1),
    ("global_sound_events", 1),
    ("elytra_movement_check", 1),
    ("fire_spread_radius_around_player", 128),
    ("freeze_damage", 1),
    ("mob_explosion_drop_decay", 1),
    ("natural_health_regeneration", 1),
    ("players_nether_portal_default_delay", 80),
    ("mob_drops", 1),
    ("log_admin_commands", 1),
    ("mob_griefing", 1),
    ("spawn_mobs", 1),
    ("pvp", 1),
    ("spectators_generate_chunks", 1),
    ("max_command_sequence_length", 65536),
    ("players_nether_portal_creative_delay", 0),
    ("players_sleeping_percentage", 100),
    ("advance_weather", 1),
    ("max_block_modifications", 32768),
    ("max_command_forks", 65536),
    ("drowning_damage", 1),
    ("command_block_output", 1),
    ("locator_bar", 1),
    ("show_advancement_messages", 1),
    ("respawn_radius", 10),
    ("raids", 1),
    ("spawn_phantoms", 1),
    ("max_snow_accumulation_height", 1),
    ("limited_crafting", 0),
    ("allow_entering_nether_using_portals", 1),
    ("lava_source_conversion", 0),
    ("tnt_explosion_drop_decay", 0),
    ("universal_anger", 0),
    ("keep_inventory", 0),
    ("spawn_patrols", 1),
    ("random_tick_speed", 3),
    ("fire_damage", 1),
    ("entity_drops", 1),
    ("advance_time", 1),
    ("command_blocks_work", 1),
    ("spawn_wardens", 1),
    ("water_source_conversion", 1),
    ("projectiles_can_break_blocks", 1),
    ("ender_pearls_vanish_on_death", 1),
];
/// Integer-valued rules; the others are booleans stored as bytes.
const INT_RULES: [&str; 11] = [
    "max_snow_accumulation_height",
    "random_tick_speed",
    "max_entity_cramming",
    "fire_spread_radius_around_player",
    "players_nether_portal_default_delay",
    "max_command_sequence_length",
    "players_nether_portal_creative_delay",
    "players_sleeping_percentage",
    "max_block_modifications",
    "max_command_forks",
    "respawn_radius",
];

/// World and player state stored in `level.dat`, the saved-data files and
/// the player file.
/// The world options CreateWorldScreen sets (`WorldOptions` and the world
/// preset) that `world_gen_settings` keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldGen {
    /// The `minecraft:flat` preset (Superflat) instead of `normal`.
    pub flat: bool,
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
            flat: text(generator.and_then(|g| g.get("type")))
                .is_some_and(|t| t.trim_start_matches("minecraft:") == "flat"),
            structures: flag("generate_structures", true),
            bonus_chest: flag("bonus_chest", false),
        })
    }
}
impl Default for WorldGen {
    fn default() -> Self {
        Self {
            flat: false,
            structures: true,
            bonus_chest: false,
        }
    }
}
/// Writes a new world's `world_gen_settings` (seed, options and dimensions).
pub fn write_world_gen_settings(dir: &Path, seed: i64, options: WorldGen) -> Result<()> {
    fs::create_dir_all(dir)?;
    // worldgen/world_preset/flat.json's overworld: the classic flat layers.
    let layer = |block: &str, height: i32| {
        Tag::compound([("block", string(block)), ("height", Tag::Int(height))])
    };
    let flat = Tag::compound([
        ("type", string("minecraft:overworld")),
        (
            "generator",
            Tag::compound([
                ("type", string("minecraft:flat")),
                (
                    "settings",
                    Tag::compound([
                        ("biome", string("minecraft:plains")),
                        ("features", Tag::Byte(0)),
                        ("lakes", Tag::Byte(0)),
                        (
                            "layers",
                            Tag::List(vec![
                                layer("minecraft:bedrock", 1),
                                layer("minecraft:dirt", 2),
                                layer("minecraft:grass_block", 1),
                            ]),
                        ),
                        (
                            "structure_overrides",
                            Tag::List(vec![
                                string("minecraft:strongholds"),
                                string("minecraft:villages"),
                            ]),
                        ),
                    ]),
                ),
            ]),
        ),
    ]);
    let dimension = |kind: &str, settings: &str, source: Tag| {
        Tag::compound([
            ("type", string(kind)),
            (
                "generator",
                Tag::compound([
                    ("type", string("minecraft:noise")),
                    ("settings", string(settings)),
                    ("biome_source", source),
                ]),
            ),
        ])
    };
    let preset = |p: &str| {
        Tag::compound([
            ("type", string("minecraft:multi_noise")),
            ("preset", string(p)),
        ])
    };
    write_saved_data(
        dir,
        "world_gen_settings",
        Tag::compound([
            ("seed", Tag::Long(seed)),
            (
                "generate_structures",
                Tag::Byte(i8::from(options.structures)),
            ),
            ("bonus_chest", Tag::Byte(i8::from(options.bonus_chest))),
            (
                "dimensions",
                Tag::compound([
                    (
                        "minecraft:overworld",
                        if options.flat {
                            flat
                        } else {
                            dimension(
                                "minecraft:overworld",
                                "minecraft:overworld",
                                preset("minecraft:overworld"),
                            )
                        },
                    ),
                    (
                        "minecraft:the_nether",
                        dimension(
                            "minecraft:the_nether",
                            "minecraft:nether",
                            preset("minecraft:nether"),
                        ),
                    ),
                    (
                        "minecraft:the_end",
                        dimension(
                            "minecraft:the_end",
                            "minecraft:end",
                            Tag::compound([("type", string("minecraft:the_end"))]),
                        ),
                    ),
                ]),
            ),
        ]),
    )
}
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub name: String,
    pub seed: i64,
    pub creative: bool,
    /// The game mode id (0 survival, 1 creative, 2 adventure, 3 spectator);
    /// `creative` wins when the two disagree.
    pub game_mode: i32,
    pub time: i64,
    pub day_time: i64,
    /// The player's respawn point (bed, or the world spawn).
    pub spawn: Option<[f64; 3]>,
    /// A respawn point in another dimension than the Overworld (a respawn
    /// anchor): dimension, block, yaw and `forced`.
    pub respawn_elsewhere: Option<(String, [i32; 3], f32, bool)>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    /// (slot, item, count, damage): 0..36 inventory, 100..103 armor (feet..head), 150 offhand.
    pub inventory: Vec<(u8, String, u32, u32)>,
    /// Extra item components of `inventory` entries, by slot.
    pub inventory_components: HashMap<u8, Box<crate::item_data::StackData>>,
    /// Player.XpSeed: the enchanting table seed.
    pub xp_seed: i32,
    /// Player.active_effects: (id, duration, amplifier, ambient, show_icon).
    pub active_effects: Vec<(String, i32, i32, bool, bool)>,
    /// Player.EnderItems: the ender chest inventory (27 slots).
    pub ender_items: Vec<Option<crate::items::ItemStack>>,
    /// The `recipeBook` compound: open/filter settings and unlocked recipes.
    pub recipe_book: crate::recipe_book::Settings,
    pub xp_level: i32,
    pub xp_progress: f32,
    pub xp_total: i32,
    pub weather: crate::world::Weather,
    /// Player.Dimension (minecraft:overworld / minecraft:the_nether).
    pub dimension: String,
    /// World spawn (`level.dat` `spawn`), distinct from a bed respawn point.
    pub world_spawn: Option<[i32; 3]>,
    pub difficulty: String,
    pub hardcore: bool,
    /// `allowCommands` chosen on the Create New World screen; unset keeps
    /// the saved value (off for a new world).
    pub allow_commands: Option<bool>,
    /// Game rule values by name without namespace (`keep_inventory`, ...).
    pub game_rules: BTreeMap<String, i32>,
    pub player_uuid: Option<[i32; 4]>,
    pub selected_slot: i32,
    pub motion: [f64; 3],
    pub fall_distance: f64,
    pub on_ground: bool,
    pub food_exhaustion: f32,
    pub food_tick_timer: i32,
    /// The player compound as loaded, for fields Horizon does not model
    /// (ender chest, effects, abilities, recipe book, attributes, ...).
    pub player_extra: Option<Tag>,
}
impl Level {
    pub fn game_rule(&self, name: &str) -> i32 {
        self.game_rules
            .get(name)
            .copied()
            .or_else(|| GAME_RULES.iter().find(|(n, _)| *n == name).map(|(_, v)| *v))
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
        let root = read_dat(&dir.join("level.dat"))?;
        let data = root.get("Data")?;
        let n = |t: Option<&Tag>| number(t).unwrap_or(0);
        let mut level = Self {
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
        level.world_spawn = match data.get("spawn").and_then(|s| s.get("pos")) {
            Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
            _ => data.get("SpawnX").map(|_| {
                [
                    n(data.get("SpawnX")) as i32,
                    n(data.get("SpawnY")) as i32,
                    n(data.get("SpawnZ")) as i32,
                ]
            }),
        };
        if let Some(d) = data.get("difficulty_settings") {
            level.difficulty = text(d.get("difficulty")).unwrap_or_else(|| "normal".into());
            level.hardcore = n(d.get("hardcore")) != 0;
        }
        let mut rules: BTreeMap<String, i32> = GAME_RULES
            .iter()
            .map(|(k, v)| ((*k).to_owned(), *v))
            .collect();
        let saved_rules = read_saved_data(dir, "game_rules");
        if let Some(Tag::Compound(map)) = &saved_rules {
            for (k, v) in map {
                let name = k.to_string_lossy();
                let name = name.strip_prefix("minecraft:").unwrap_or(&name).to_owned();
                if let Some(v) = number(Some(v)) {
                    rules.insert(name, v as i32);
                }
            }
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
        // The player: players/data/<singleplayer_uuid>.dat, the newest player
        // file of a server world, or the legacy Data.Player.
        let uuid = uuid_from_tag(data.get("singleplayer_uuid"));
        let player_dir = dir.join("players").join("data");
        let owner = uuid.and_then(|u| read_dat(&player_dir.join(format!("{}.dat", uuid_text(u)))));
        let player = owner
            .or_else(|| {
                fs::read_dir(&player_dir)
                    .ok()
                    .and_then(|d| {
                        d.flatten()
                            .filter(|e| e.path().extension().is_some_and(|x| x == "dat"))
                            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
                    })
                    .and_then(|e| read_dat(&e.path()))
            })
            .or_else(|| data.get("Player").cloned());
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
        let n = |k: &str| number(p.get(k));
        let f = |k: &str| float(p.get(k));
        self.position = read_doubles::<3>(p.get("Pos"));
        if let Some(r) = read_doubles::<2>(p.get("Rotation")) {
            self.rotation = [r[0] as f32, r[1] as f32];
        }
        self.motion = read_doubles::<3>(p.get("Motion")).unwrap_or_default();
        self.health = f("Health").map_or(20.0, |v| v as f32);
        self.food = n("foodLevel").map_or(20, |v| v as i32);
        self.saturation = f("foodSaturationLevel").map_or(5.0, |v| v as f32);
        self.food_exhaustion = f("foodExhaustionLevel").unwrap_or(0.0) as f32;
        self.food_tick_timer = n("foodTickTimer").unwrap_or(0) as i32;
        self.xp_level = n("XpLevel").unwrap_or(0) as i32;
        self.xp_progress = f("XpP").unwrap_or(0.0) as f32;
        self.xp_total = n("XpTotal").unwrap_or(0) as i32;
        self.selected_slot = n("SelectedItemSlot").unwrap_or(0) as i32;
        self.fall_distance = f("fall_distance").unwrap_or(0.0);
        self.on_ground = n("OnGround").unwrap_or(0) != 0;
        if let Some(mode) = n("playerGameType") {
            self.creative = mode == 1;
            self.game_mode = mode as i32;
        }
        if let Some(d) = text(p.get("Dimension")) {
            self.dimension = d;
        }
        if let Some(respawn) = p.get("respawn")
            && let Some(Tag::IntArray(pos)) = respawn.get("pos")
            && pos.len() == 3
        {
            let dimension =
                text(respawn.get("dimension")).unwrap_or_else(|| "minecraft:overworld".into());
            if dimension == "minecraft:overworld" {
                self.spawn = Some([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]);
            } else {
                let yaw = match respawn.get("yaw") {
                    Some(Tag::Float(b)) => f32::from_bits(*b),
                    _ => 0.0,
                };
                let forced = number(respawn.get("forced")).is_some_and(|v| v != 0);
                self.respawn_elsewhere = Some((dimension, [pos[0], pos[1], pos[2]], yaw, forced));
            }
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
            for (name, slot) in [
                ("feet", 100u8),
                ("legs", 101),
                ("chest", 102),
                ("head", 103),
                ("offhand", 150),
            ] {
                if let Some(stack) = eq.get(name).and_then(item_from_nbt) {
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
                .filter_map(|e| {
                    let id = text(e.get("id"))?;
                    let get = |k: &str| number(e.get(k));
                    Some((
                        id,
                        get("duration").unwrap_or(0) as i32,
                        get("amplifier").unwrap_or(0) as i32,
                        get("ambient").unwrap_or(0) != 0,
                        get("show_icon").is_none_or(|v| v != 0),
                    ))
                })
                .collect(),
            _ => Vec::new(),
        };
    }
    /// The player compound: the loaded one with Horizon's state written over it.
    pub fn player_tag(&self) -> Tag {
        let mut p = compound_or_empty(self.player_extra.clone());
        // Keys rewritten from scratch.
        if let Tag::Compound(map) = &mut p {
            for key in [
                "Inventory",
                "EnderItems",
                "equipment",
                "Pos",
                "respawn",
                "recipeBook",
            ] {
                map.remove(&JavaString::from(key));
            }
        }
        let mut inventory = Vec::new();
        let mut equipment = Vec::new();
        for (slot, id, count, damage) in &self.inventory {
            let extra = self.inventory_components.get(slot).cloned();
            let stack = crate::items::ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra,
            };
            match slot {
                0..=35 => {
                    let mut entries = vec![("Slot", Tag::Byte(*slot as i8))];
                    entries.extend(encode_item(&stack));
                    inventory.push(Tag::compound(entries));
                }
                100 => equipment.push(("feet", item_to_nbt(&stack))),
                101 => equipment.push(("legs", item_to_nbt(&stack))),
                102 => equipment.push(("chest", item_to_nbt(&stack))),
                103 => equipment.push(("head", item_to_nbt(&stack))),
                150 => equipment.push(("offhand", item_to_nbt(&stack))),
                _ => {}
            }
        }
        // GameType.updatePlayerAbilities.
        let mode = self.mode();
        let (creative, spectator) = (mode == 1, mode == 3);
        let mut abilities = compound_or_empty(p.get("abilities").cloned());
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
        } else if !creative {
            put(&mut abilities, "flying", Tag::Byte(0));
        }
        if let Tag::Compound(map) = &mut abilities {
            map.entry("mayBuild".into()).or_insert(Tag::Byte(1));
            map.entry("flySpeed".into())
                .or_insert(Tag::Float(0.05f32.to_bits()));
            map.entry("walkSpeed".into())
                .or_insert(Tag::Float(0.1f32.to_bits()));
            map.entry("flying".into()).or_insert(Tag::Byte(0));
        }
        let uuid = self.player_uuid.unwrap_or([0, 0, 0, 0]);
        let mut entries: Vec<(&str, Tag)> = vec![
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("UUID", uuid_tag(uuid)),
            ("Rotation", floats(&self.rotation)),
            ("Motion", doubles(&self.motion)),
            ("OnGround", Tag::Byte(i8::from(self.on_ground))),
            ("fall_distance", Tag::Double(self.fall_distance.to_bits())),
            ("Health", Tag::Float(self.health.to_bits())),
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
            (
                "active_effects",
                Tag::List(
                    self.active_effects
                        .iter()
                        .map(|(id, duration, amplifier, ambient, icon)| {
                            Tag::compound([
                                ("id", string(id)),
                                ("duration", Tag::Int(*duration)),
                                ("amplifier", Tag::Byte(*amplifier as i8)),
                                ("ambient", Tag::Byte(i8::from(*ambient))),
                                ("show_particles", Tag::Byte(1)),
                                ("show_icon", Tag::Byte(i8::from(*icon))),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("playerGameType", Tag::Int(self.mode())),
            ("Inventory", Tag::List(inventory)),
            ("EnderItems", encode_items(&self.ender_items)),
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
        if holding || !equipment.is_empty() {
            entries.push(("equipment", Tag::compound(equipment)));
        }
        if self.active_effects.is_empty() {
            entries.retain(|(key, _)| *key != "active_effects");
        }
        if let Some(pos) = self.position {
            entries.push(("Pos", doubles(&pos)));
        }
        // A respawn point other than the world spawn: an anchor elsewhere, or a bed.
        if let Some((dimension, block, yaw, forced)) = &self.respawn_elsewhere {
            let mut respawn = vec![
                ("dimension", string(dimension)),
                ("pos", Tag::IntArray(block.to_vec())),
                ("yaw", Tag::Float(yaw.to_bits())),
                ("pitch", Tag::Float(0f32.to_bits())),
            ];
            if *forced {
                respawn.push(("forced", Tag::Byte(1)));
            }
            entries.push(("respawn", Tag::compound(respawn)));
        } else if let Some(s) = self.spawn {
            let block = [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ];
            if Some(block) != self.world_spawn {
                entries.push((
                    "respawn",
                    Tag::compound([
                        ("dimension", string("minecraft:overworld")),
                        ("pos", Tag::IntArray(block.to_vec())),
                        ("yaw", Tag::Float(0f32.to_bits())),
                        ("pitch", Tag::Float(0f32.to_bits())),
                    ]),
                ));
            }
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
                ("Score", Tag::Int(0)),
                ("SleepTimer", Tag::Short(0)),
                ("XpSeed", Tag::Int(0)),
                ("seenCredits", Tag::Byte(0)),
                ("HurtTime", Tag::Short(0)),
                ("DeathTime", Tag::Short(0)),
                ("AbsorptionAmount", Tag::Float(0)),
                ("FallFlying", Tag::Byte(0)),
            ] {
                map.entry(key.into()).or_insert(value);
            }
        }
        p
    }
    /// Write every file; a world without a player UUID gets a random one.
    pub fn write(&mut self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let existing = read_dat(&dir.join("level.dat")).and_then(|r| r.get("Data").cloned());
        let legacy = existing
            .as_ref()
            .is_some_and(|d| d.get("Player").is_some() || d.get("WorldGenSettings").is_some());
        let mut data = compound_or_empty(existing);
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
                map.remove(&JavaString::from(key));
            }
        }
        // A new world's player has the offline profile id of its name
        // (UUIDUtil.createOfflinePlayerUUID), as in an offline single-player game.
        let uuid = *self.player_uuid.get_or_insert_with(|| {
            let u = horizon_protocol::crypto::offline_uuid("Player");
            [
                (u >> 96) as u32 as i32,
                (u >> 64) as u32 as i32,
                (u >> 32) as u32 as i32,
                u as u32 as i32,
            ]
        });
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
        if let Tag::Compound(map) = &mut difficulty {
            map.entry("locked".into()).or_insert(Tag::Byte(0));
        }
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
                    ("dimension", string("minecraft:overworld")),
                    ("yaw", Tag::Float(0f32.to_bits())),
                    ("pitch", Tag::Float(0f32.to_bits())),
                ]),
            ),
            ("difficulty_settings", difficulty),
            ("initialized", Tag::Byte(1)),
            ("singleplayer_uuid", uuid_tag(uuid)),
            ("ServerBrands", Tag::List(brands)),
        ] {
            put(&mut data, key, value);
        }
        if let Tag::Compound(map) = &mut data {
            map.entry("allowCommands".into()).or_insert(Tag::Byte(0));
            if let Some(allow) = self.allow_commands {
                map.insert("allowCommands".into(), Tag::Byte(i8::from(allow)));
            }
            map.entry("WasModded".into()).or_insert(Tag::Byte(0));
            map.entry("DataPacks".into()).or_insert_with(|| {
                Tag::compound([
                    ("Enabled", Tag::List(vec![string("vanilla")])),
                    (
                        "Disabled",
                        Tag::List(
                            [
                                "minecart_improvements",
                                "redstone_experiments",
                                "trade_rebalance",
                            ]
                            .iter()
                            .map(|s| string(s))
                            .collect(),
                        ),
                    ),
                ])
            });
        }
        // Saved data: weather, clocks, and (when missing or legacy) world
        // generation settings and game rules.
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
        if let Tag::Compound(map) = &mut clocks {
            map.entry("minecraft:the_end".into())
                .or_insert_with(|| Tag::compound([("total_ticks", Tag::Long(self.time))]));
        }
        write_saved_data(dir, "world_clocks", clocks)?;
        if legacy || !saved_data_path(dir, "world_gen_settings").exists() {
            write_world_gen_settings(dir, self.seed, WorldGen::default())?;
        }
        if !saved_data_path(dir, "game_rules").exists() || !self.game_rules.is_empty() {
            // Only rules vanilla knows: an unknown key makes it reject the file.
            let mut rules = Tag::Compound(Compound::new());
            for (name, default) in GAME_RULES {
                let value = self.game_rules.get(name).copied().unwrap_or(default);
                let tag = if INT_RULES.contains(&name) {
                    Tag::Int(value)
                } else {
                    Tag::Byte(i8::from(value != 0))
                };
                put(&mut rules, &format!("minecraft:{name}"), tag);
            }
            write_saved_data(dir, "game_rules", rules)?;
        }
        write_dat(
            &dir.join("players")
                .join("data")
                .join(format!("{}.dat", uuid_text(uuid))),
            self.player_tag(),
        )?;
        write_dat(&dir.join("level.dat"), Tag::compound([("Data", data)]))?;
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
impl<G: ChunkGenerator> ChunkGenerator for Persistent<G> {
    type Worker = G::Worker;
    fn worker(&self) -> G::Worker {
        self.inner.worker()
    }
    fn load(&self, x: i32, z: i32) -> Option<Chunk> {
        self.save
            .tables
            .get_or_init(|| Arc::new(SaveTables::new(&self.registry)));
        let mut chunk = self.save.load(x, z, self.registry.registry, &self.biomes)?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        chunk.entities = self.save.load_entities(x, z);
        Some(chunk)
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
            .save_protos(&saved, self.registry.registry, &self.biomes)
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
    fn decorate(&self, worker: &mut G::Worker, region: &mut [crate::streaming::Proto; 9]) {
        self.inner.decorate(worker, region);
    }
    fn finish(&self, proto: crate::streaming::Proto) -> Chunk {
        self.inner.finish(proto)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn save_batch_preserves_order_coordinates_and_ownership() {
        use crate::containers::BlockEntity;
        let mut entities = HashMap::new();
        let mut names = HashMap::new();
        for x in [i32::MIN, -17, -16, -1, 0, 15, 16, i32::MAX] {
            for z in [-17, -16, -1, 0, 15, 16] {
                let p = (x, 64, z);
                entities.insert(p, BlockEntity::Chest(vec![None; 27]));
                names.insert(p, format!("container {x},{z}"));
            }
        }
        // Names without a modelled entity must also survive the snapshot.
        names.insert((-2, 80, -3), "orphan name".into());
        let mut source = Chunk::empty(-1, -1);
        source
            .block_entities
            .push(((99, 0, 99), BlockEntity::Chest(vec![])));
        source.block_names.push(((99, 0, 99), "stale".into()));
        source.saved = Some(Arc::new(ChunkExtras::default()));
        let source = Arc::new(source);
        let batch = vec![
            source.clone(),
            Arc::new(Chunk::empty(1, 1)),
            Arc::new(Chunk::empty(-2, 0)),
            source.clone(), // Preserve input order and repeated coordinates.
            Arc::new(Chunk::empty(i32::MIN >> 4, -2)),
            Arc::new(Chunk::empty(i32::MAX >> 4, 1)),
            Arc::new(Chunk::empty(100, 100)),
        ];
        let encode = |entries: &[((i32, i32, i32), BlockEntity)]| {
            entries
                .iter()
                .map(|(p, e)| encode_block_entity(*p, e, "minecraft:chest"))
                .collect::<Vec<_>>()
        };
        for input in [&batch[..0], &batch[..1], &batch[..]] {
            let actual = chunks_with_block_entities(input, &entities, &names);
            assert_eq!(actual.len(), input.len());
            for (c, original) in actual.iter().zip(input) {
                let expected: Vec<_> = entities
                    .iter()
                    .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                    .map(|(p, e)| (*p, e.clone()))
                    .collect();
                let expected_names: Vec<_> = names
                    .iter()
                    .filter(|(p, _)| (p.0 >> 4, p.2 >> 4) == (c.x, c.z))
                    .map(|(p, n)| (*p, n.clone()))
                    .collect();
                assert_eq!((c.x, c.z), (original.x, original.z));
                assert_eq!(encode(&c.block_entities), encode(&expected));
                assert_eq!(c.block_names, expected_names);
                assert!(Arc::ptr_eq(&c.sections[0], &original.sections[0]));
                if let Some(saved) = &original.saved {
                    assert!(Arc::ptr_eq(c.saved.as_ref().unwrap(), saved));
                }
            }
        }
        let snapshot = chunks_with_block_entities(&batch, &entities, &names);
        entities.clear();
        names.clear();
        assert!(!snapshot[0].block_entities.is_empty());
        assert!(!snapshot[0].block_names.is_empty());
        assert_eq!(source.block_names[0].1, "stale");
        assert_eq!(source.block_entities[0].0, (99, 0, 99));
    }

    #[test]
    fn older_worlds_are_upgraded_on_opening() {
        let dir = std::env::temp_dir().join(format!("horizon-dv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let level =
            |v: i32| Tag::compound([("Data", Tag::compound([("DataVersion", Tag::Int(v))]))]);
        let version = |dir: &std::path::Path| {
            super::read_dat(&dir.join("level.dat"))
                .and_then(|r| super::number(r.get("Data")?.get("DataVersion")))
        };
        // 1.12.2 and 1.16.5: upgraded too, every server fix is ported.
        for old in [1343, 2586] {
            super::write_dat(&dir.join("level.dat"), level(old)).unwrap();
            super::upgrade_older_world(&dir).unwrap();
            assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
        }
        // 1.21.1: upgraded to the current version.
        super::write_dat(&dir.join("level.dat"), level(3955)).unwrap();
        super::upgrade_older_world(&dir).unwrap();
        assert_eq!(version(&dir), Some(i64::from(super::DATA_VERSION)));
        super::upgrade_older_world(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn session_lock_is_exclusive() {
        let dir = std::env::temp_dir().join(format!("horizon-lock-{}", std::process::id()));
        let first = super::SessionLock::acquire(&dir).expect("first lock");
        assert!(super::SessionLock::acquire(&dir).is_err());
        assert_eq!(
            std::fs::read(dir.join("session.lock")).unwrap(),
            [0x26, 0x03]
        );
        drop(first);
        assert!(super::SessionLock::acquire(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn crafter_block_entity_round_trips() {
        let mut crafter = crate::containers::Crafter::default();
        crafter.items[2] = Some(crate::items::ItemStack::new("minecraft:stick", 5));
        crafter.disabled[4] = true;
        crafter.triggered = true;
        crafter.crafting_ticks = 3;
        let tag = encode_block_entity(
            (7, 8, 9),
            &crate::containers::BlockEntity::Crafter(crafter),
            "minecraft:crafter",
        );
        let (pos, back) = decode_block_entity(&tag).expect("crafter");
        assert_eq!(pos, (7, 8, 9));
        let crate::containers::BlockEntity::Crafter(back) = back else {
            panic!("not a crafter");
        };
        assert_eq!(back.items[2].as_ref().map(|s| s.count), Some(5));
        assert!(back.disabled[4] && !back.disabled[3]);
        assert!(back.triggered);
        assert_eq!(back.crafting_ticks, 3);
        assert_eq!(back.redstone_signal(), 2);
    }
    #[test]
    fn spawner_block_entity_round_trips() {
        // A dungeon spawner as vanilla saves it (`id` is mob_spawner, not the block).
        let saved = horizon_world::nbt::parse_snbt(
            r#"{id:"minecraft:mob_spawner",x:67,y:-52,z:-64,Delay:20s,SpawnData:{entity:{id:"minecraft:zombie"}},SpawnPotentials:[]}"#,
        )
        .unwrap()
        .0;
        let (pos, back) = decode_block_entity(&saved).expect("spawner");
        assert_eq!(pos, (67, -52, -64));
        let crate::containers::BlockEntity::Spawner(spawner) = &back else {
            panic!("not a spawner");
        };
        let next = spawner.next_data().and_then(|d| d.id());
        assert_eq!(next.as_deref(), Some("minecraft:zombie"));
        let tag = encode_block_entity(pos, &back, "minecraft:spawner");
        let (_, again) = decode_block_entity(&tag).expect("spawner again");
        let crate::containers::BlockEntity::Spawner(again) = again else {
            panic!("not a spawner");
        };
        assert_eq!(again.update_tag(), spawner.update_tag());
    }
    #[test]
    fn banner_block_entity_round_trips() {
        let banner = crate::containers::Banner {
            layers: vec![crate::banner::BannerLayer::new("minecraft:creeper", "lime")],
            name: Some("Flag".into()),
        };
        let tag = encode_block_entity(
            (1, 2, 3),
            &crate::containers::BlockEntity::Banner(banner.clone()),
            "minecraft:red_banner",
        );
        let (pos, back) = decode_block_entity(&tag).expect("banner");
        assert_eq!(pos, (1, 2, 3));
        let crate::containers::BlockEntity::Banner(back) = back else {
            panic!("not a banner");
        };
        assert_eq!(back.layers, banner.layers);
        assert_eq!(back.name, banner.name);
    }
    use super::*;

    #[test]
    fn uuid_text_round_trips() {
        let u = [-158327916, 1572358581, -2125825222, -487948715];
        assert_eq!(uuid_from_text(&uuid_text(u)), Some(u));
        assert_eq!(
            uuid_text([0, 1, 2, 3]),
            "00000000-0000-0001-0000-000200000003"
        );
        let r = random_uuid();
        assert_eq!((r[1] >> 12) & 0xF, 4, "version 4");
    }
    #[test]
    fn creaking_heart_keeps_its_protector_uuid() {
        use crate::containers::{BlockEntity, CreakingHeart};
        let heart = CreakingHeart {
            creaking: Some((Some(7), [1, 2, 3, 4])),
            ..Default::default()
        };
        let tag = encode_block_entity(
            (3, 70, -5),
            &BlockEntity::CreakingHeart(Box::new(heart)),
            "minecraft:creaking_heart",
        );
        assert_eq!(
            tag.get("creaking"),
            Some(&Tag::IntArray(vec![1, 2, 3, 4])),
            "vanilla stores the protector's UUID under `creaking`"
        );
        let (pos, entity) = decode_block_entity(&tag).expect("heart decodes");
        assert_eq!(pos, (3, 70, -5));
        let BlockEntity::CreakingHeart(h) = entity else {
            panic!("a creaking heart");
        };
        // The entity id is resolved again once the creaking is loaded.
        assert_eq!(h.creaking, Some((None, [1, 2, 3, 4])));
        let lonely = encode_block_entity(
            (0, 0, 0),
            &BlockEntity::CreakingHeart(Box::default()),
            "minecraft:creaking_heart",
        );
        assert!(lonely.get("creaking").is_none());
    }
    #[test]
    fn light_nibbles_use_vanilla_order() {
        let mut a = Box::new([0u8; 4096]);
        a[0] = 0xF3; // sky 15, block 3 at x=0
        a[1] = 0x52; // sky 5, block 2 at x=1
        let light = Light::Full(a);
        assert_eq!(nibbles(&light, false)[0], 0x23);
        assert_eq!(nibbles(&light, true)[0], 0x5F);
    }
    #[test]
    fn stored_light_loads_with_vanilla_defaults() {
        let mut sky = vec![0u8; 2048];
        sky[0] = 0x5A; // x=0 -> 10, x=1 -> 5
        let section = |y: i8, entries: Vec<(&str, Tag)>| {
            let mut all = vec![("Y", Tag::Byte(y))];
            all.extend(entries);
            Tag::compound(all)
        };
        let tag = Tag::compound([
            ("isLightOn", Tag::Byte(1)),
            (
                "sections",
                Tag::List(vec![
                    section(
                        -4,
                        vec![
                            ("SkyLight", Tag::ByteArray(sky)),
                            ("BlockLight", Tag::ByteArray(vec![0x21; 2048])),
                        ],
                    ),
                    // No sky layer: repeats the bottom layer of the section above.
                    section(-3, vec![]),
                ]),
            ),
        ]);
        let mut chunk = Chunk::empty(0, 0);
        load_light(&tag, &mut chunk, true);
        assert!(chunk.lit && chunk.light_loaded);
        let at = |c: &Chunk, si: usize, i: usize| c.sections[si].light(i);
        assert_eq!(at(&chunk, 0, 0), 0xA1, "stored sky 10, block 1");
        assert_eq!(at(&chunk, 0, 1), 0x52, "stored sky 5, block 2");
        // Section -3 has no layers; above it (-2..) sky defaults to 15.
        assert_eq!(at(&chunk, 1, 0), 0xF0);
        assert_eq!(at(&chunk, SECTIONS - 1, 5), 0xF0);
        let mut dark = Chunk::empty(0, 0);
        load_light(&tag, &mut dark, false);
        assert_eq!(at(&dark, 1, 0), 0x00, "no skylight dimension");
        let mut unlit = Chunk::empty(0, 0);
        load_light(
            &Tag::compound([("isLightOn", Tag::Byte(0))]),
            &mut unlit,
            true,
        );
        assert!(!unlit.lit && !unlit.light_loaded);
    }
    #[test]
    fn level_round_trips_through_the_vanilla_files() {
        let dir = std::env::temp_dir().join(format!("horizon-level-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut level = Level {
            name: "Test".into(),
            seed: -42,
            time: 1234,
            day_time: 25000,
            creative: true,
            position: Some([1.5, 70.0, -3.25]),
            rotation: [90.0, 10.0],
            health: 17.0,
            food: 15,
            saturation: 2.5,
            inventory: vec![
                (0, "minecraft:stone".into(), 12, 0),
                (5, "minecraft:iron_sword".into(), 1, 3),
                (103, "minecraft:iron_helmet".into(), 1, 0),
            ],
            xp_level: 3,
            xp_progress: 0.5,
            xp_total: 30,
            dimension: "minecraft:the_nether".into(),
            world_spawn: Some([8, 64, 8]),
            spawn: Some([20.0, 70.0, 20.0]),
            ..Level::default()
        };
        level.weather.raining = true;
        level.weather.rain_time = 500;
        level.game_rules.insert("keep_inventory".into(), 1);
        level.write(&dir).unwrap();
        let back = Level::read(&dir).unwrap();
        assert_eq!(back.name, "Test");
        assert_eq!(back.seed, -42);
        assert_eq!((back.time, back.day_time), (1234, 25000));
        assert!(back.creative);
        assert_eq!(back.position, Some([1.5, 70.0, -3.25]));
        assert_eq!(back.rotation, [90.0, 10.0]);
        assert_eq!((back.health, back.food, back.saturation), (17.0, 15, 2.5));
        let mut inv = back.inventory.clone();
        inv.sort();
        assert_eq!(
            inv,
            vec![
                (0, "minecraft:stone".into(), 12, 0),
                (5, "minecraft:iron_sword".into(), 1, 3),
                (103, "minecraft:iron_helmet".into(), 1, 0)
            ]
        );
        assert_eq!(
            (back.xp_level, back.xp_progress, back.xp_total),
            (3, 0.5, 30)
        );
        assert_eq!(back.dimension, "minecraft:the_nether");
        assert_eq!(back.world_spawn, Some([8, 64, 8]));
        assert_eq!(back.spawn, Some([20.0, 70.0, 20.0]));
        assert!(back.weather.raining);
        assert_eq!(back.weather.rain_time, 500);
        assert_eq!(back.game_rule("keep_inventory"), 1);
        assert_eq!(back.game_rule("random_tick_speed"), 3);
        // Unknown player data survives another save.
        let mut extra = back.player_extra.clone().unwrap();
        put(
            &mut extra,
            "ShoulderEntityLeft",
            Tag::compound([("id", string("minecraft:parrot"))]),
        );
        let mut ender = vec![None; 27];
        ender[3] = Some(crate::items::ItemStack::new("minecraft:diamond", 2));
        let mut again = Level {
            player_extra: Some(extra),
            ender_items: ender,
            ..back
        };
        again.write(&dir).unwrap();
        let third = Level::read(&dir).unwrap();
        assert!(matches!(
            third
                .player_extra
                .as_ref()
                .and_then(|p| p.get("ShoulderEntityLeft")),
            Some(Tag::Compound(_))
        ));
        // EnderItems round-trip by slot.
        assert_eq!(
            third.ender_items[3].as_ref().map(|s| (&*s.item, s.count)),
            Some(("minecraft:diamond", 2))
        );
        assert!(dir.join("data/minecraft/world_gen_settings.dat").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
