//! World persistence in vanilla's Anvil layout: `region/r.X.Z.mca` files with
//! zlib-compressed chunk NBT (paletted `block_states`/`biomes` per section) and
//! a gzip `level.dat`. Light is not stored (`isLightOn: 0`), so both Horizon and
//! vanilla relight saved chunks on load.
use crate::{
    Result,
    biome::Biomes,
    chunk::{Blocks, Chunk, MIN_Y, SECTIONS},
    streaming::ChunkGenerator,
};
use horizon_core::position::ChunkPos;
use horizon_world::{
    DATA_VERSION,
    chunk::BlockState,
    compression::Compression,
    nbt::{self, Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile, RegionWriter},
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

fn pack(indices: &[u16], bits: u32) -> Vec<i64> {
    let per_long = (64 / bits) as usize;
    let mut longs = vec![0i64; indices.len().div_ceil(per_long)];
    for (i, &v) in indices.iter().enumerate() {
        longs[i / per_long] |= (i64::from(v) & ((1i64 << bits) - 1)) << ((i % per_long) as u32 * bits);
    }
    longs
}
fn unpack(longs: &[i64], bits: u32, count: usize) -> Vec<u16> {
    let per_long = (64 / bits) as usize;
    let mask = (1u64 << bits) - 1;
    (0..count)
        .map(|i| {
            longs
                .get(i / per_long)
                .map_or(0, |&l| ((l as u64 >> ((i % per_long) as u32 * bits)) & mask) as u16)
        })
        .collect()
}
fn bits_for(len: usize, min: u32) -> u32 {
    (usize::BITS - (len.max(1) - 1).leading_zeros()).max(min)
}
fn string(s: &str) -> Tag {
    Tag::String(s.into())
}

fn encode_items(slots: &[Option<crate::items::ItemStack>]) -> Tag {
    Tag::List(
        slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let s = s.as_ref()?;
                let mut entries = vec![
                    ("Slot", Tag::Byte(i as i8)),
                    ("id", string(&s.item)),
                    ("count", Tag::Int(s.count as i32)),
                ];
                if s.damage > 0 {
                    entries.push(("components", Tag::compound([("minecraft:damage", Tag::Int(s.damage as i32))])));
                }
                Some(Tag::compound(entries))
            })
            .collect(),
    )
}
fn decode_items(tag: Option<&Tag>, size: usize) -> Vec<Option<crate::items::ItemStack>> {
    let mut slots = vec![None; size];
    if let Some(Tag::List(items)) = tag {
        for it in items {
            let Some(Tag::Byte(slot)) = it.get("Slot") else { continue };
            let Some(Tag::String(id)) = it.get("id") else { continue };
            let count = match it.get("count") {
                Some(Tag::Int(c)) => *c as u32,
                Some(Tag::Byte(c)) => *c as u32,
                _ => 1,
            };
            let damage = match it.get("components").and_then(|c| c.get("minecraft:damage")) {
                Some(Tag::Int(d)) => *d as u32,
                _ => 0,
            };
            if let Some(cell) = slots.get_mut(*slot as u8 as usize) {
                *cell = Some(crate::items::ItemStack { item: Arc::from(id.to_string_lossy().as_str()), count, damage });
            }
        }
    }
    slots
}
fn encode_block_entity(p: (i32, i32, i32), e: &crate::containers::BlockEntity) -> Tag {
    use crate::containers::BlockEntity;
    let mut entries = vec![
        ("id", string(e.id())),
        ("x", Tag::Int(p.0)),
        ("y", Tag::Int(p.1)),
        ("z", Tag::Int(p.2)),
        ("keepPacked", Tag::Byte(0)),
    ];
    match e {
        BlockEntity::Chest(items) => entries.push(("Items", encode_items(items))),
        BlockEntity::Furnace(f) => {
            entries.push(("Items", encode_items(&f.items)));
            entries.push(("lit_time_remaining", Tag::Short(f.lit_remaining.clamp(0, i32::from(i16::MAX)) as i16)));
            entries.push(("lit_total_time", Tag::Short(f.lit_total.clamp(0, i32::from(i16::MAX)) as i16)));
            entries.push(("cooking_time_spent", Tag::Short(f.cooking_timer as i16)));
            entries.push(("cooking_total_time", Tag::Short(f.cooking_total as i16)));
        }
    }
    Tag::compound(entries)
}
fn decode_block_entity(tag: &Tag) -> Option<((i32, i32, i32), crate::containers::BlockEntity)> {
    use crate::containers::BlockEntity;
    let int = |k: &str| match tag.get(k) {
        Some(Tag::Int(v)) => Some(*v),
        Some(Tag::Short(v)) => Some(i32::from(*v)),
        _ => None,
    };
    let Some(Tag::String(id)) = tag.get("id") else { return None };
    let id = id.to_string_lossy();
    let pos = (int("x")?, int("y")?, int("z")?);
    let mut entity = BlockEntity::for_block(&id)?;
    match &mut entity {
        BlockEntity::Chest(items) => *items = decode_items(tag.get("Items"), 27),
        BlockEntity::Furnace(f) => {
            let items = decode_items(tag.get("Items"), 3);
            f.items = [items[0].clone(), items[1].clone(), items[2].clone()];
            f.lit_remaining = int("lit_time_remaining").unwrap_or(0);
            f.lit_total = int("lit_total_time").unwrap_or(0);
            f.cooking_timer = int("cooking_time_spent").unwrap_or(0);
            f.cooking_total = int("cooking_total_time").unwrap_or(200);
        }
    }
    Some((pos, entity))
}
pub fn encode(chunk: &Chunk, registry: &BlockRegistry, biomes: &Biomes) -> NamedTag {
    let mut sections = Vec::new();
    for (si, section) in chunk.sections.iter().enumerate() {
        let y = (MIN_Y >> 4) + si as i32;
        let ids: Vec<u16> = match &section.blocks {
            Blocks::Uniform(v) => vec![*v],
            Blocks::Full(a) => a.to_vec(),
        };
        let mut palette: Vec<u16> = Vec::new();
        let mut lookup: HashMap<u16, u16> = HashMap::new();
        let indices: Vec<u16> = ids
            .iter()
            .map(|id| {
                *lookup.entry(*id).or_insert_with(|| {
                    palette.push(*id);
                    (palette.len() - 1) as u16
                })
            })
            .collect();
        let mut states = vec![(
            "palette",
            Tag::List(
                palette
                    .iter()
                    .map(|id| {
                        registry
                            .state(BlockStateId(u32::from(*id)))
                            .map_or_else(|| Tag::compound([("Name", string("minecraft:air"))]), BlockState::to_nbt)
                    })
                    .collect(),
            ),
        )];
        if palette.len() > 1 {
            states.push(("data", Tag::LongArray(pack(&indices, bits_for(palette.len(), 4)))));
        }
        let quart: Vec<u16> = (0..64).map(|i| chunk.biomes[si * 64 + i]).collect();
        let mut biome_palette: Vec<u16> = Vec::new();
        let biome_indices: Vec<u16> = quart
            .iter()
            .map(|b| match biome_palette.iter().position(|p| p == b) {
                Some(i) => i as u16,
                None => {
                    biome_palette.push(*b);
                    (biome_palette.len() - 1) as u16
                }
            })
            .collect();
        let mut biome_tag = vec![(
            "palette",
            Tag::List(biome_palette.iter().map(|b| string(&biomes.get(*b).name)).collect()),
        )];
        if biome_palette.len() > 1 {
            biome_tag.push(("data", Tag::LongArray(pack(&biome_indices, bits_for(biome_palette.len(), 0)))));
        }
        sections.push(Tag::compound([
            ("Y", Tag::Byte(y as i8)),
            ("block_states", Tag::compound(states)),
            ("biomes", Tag::compound(biome_tag)),
        ]));
    }
    NamedTag {
        name: "".into(),
        tag: Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("xPos", Tag::Int(chunk.x)),
            ("zPos", Tag::Int(chunk.z)),
            ("yPos", Tag::Int(MIN_Y >> 4)),
            ("Status", string("minecraft:full")),
            ("LastUpdate", Tag::Long(0)),
            ("InhabitedTime", Tag::Long(0)),
            ("isLightOn", Tag::Byte(0)),
            ("sections", Tag::List(sections)),
            ("block_entities", Tag::List(chunk.block_entities.iter().map(|(p, e)| encode_block_entity(*p, e)).collect())),
        ]),
    }
}
pub fn decode(root: &NamedTag, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
    let tag = &root.tag;
    let Tag::Int(x) = tag.get("xPos")? else { return None };
    let Tag::Int(z) = tag.get("zPos")? else { return None };
    let mut chunk = Chunk::empty(*x, *z);
    let air = registry.default_state("minecraft:air")?.0 as u16;
    let mut cache: HashMap<String, u16> = HashMap::new();
    let mut biome_data = (*chunk.biomes).clone();
    let Some(Tag::List(sections)) = tag.get("sections") else { return Some(chunk) };
    for section in sections {
        let Some(Tag::Byte(y)) = section.get("Y") else { continue };
        let si = i32::from(*y) - (MIN_Y >> 4);
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
                    let key = state.as_ref().map(|s| format!("{}{:?}", s.name, s.properties)).unwrap_or_default();
                    *cache.entry(key).or_insert_with(|| {
                        state
                            .and_then(|s| registry.id(&s).or_else(|| registry.default_state(&s.name)))
                            .map_or(air, |id| id.0 as u16)
                    })
                })
                .collect();
            let target = Arc::make_mut(&mut chunk.sections[si as usize]);
            if ids.len() <= 1 {
                target.blocks = Blocks::Uniform(ids.first().copied().unwrap_or(air));
            } else if let Some(Tag::LongArray(data)) = states.get("data") {
                let indices = unpack(data, bits_for(ids.len(), 4), 4096);
                let mut full = Box::new([air; 4096]);
                for (i, idx) in indices.iter().enumerate() {
                    full[i] = ids.get(*idx as usize).copied().unwrap_or(air);
                }
                target.blocks = Blocks::Full(full);
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
            let indices = match b.get("data") {
                Some(Tag::LongArray(data)) if ids.len() > 1 => unpack(data, bits_for(ids.len(), 0), 64),
                _ => vec![0; 64],
            };
            for (i, idx) in indices.iter().enumerate() {
                biome_data[si as usize * 64 + i] = ids.get(*idx as usize).copied().unwrap_or(0);
            }
        }
    }
    chunk.biomes = Arc::new(biome_data);
    if let Some(Tag::List(entities)) = tag.get("block_entities") {
        chunk.block_entities = entities.iter().filter_map(decode_block_entity).collect();
    }
    Some(chunk)
}
/// The world folder. Region files are read lazily by worker threads.
pub struct SaveDir {
    pub root: PathBuf,
    regions: Mutex<HashMap<(i32, i32), Option<Arc<Mutex<RegionFile>>>>>,
}
impl SaveDir {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            regions: Mutex::new(HashMap::new()),
        }
    }
    fn region_path(&self, rx: i32, rz: i32) -> PathBuf {
        self.root.join(format!("region/r.{rx}.{rz}.mca"))
    }
    pub fn load(&self, x: i32, z: i32, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
        let key = (x >> 5, z >> 5);
        let region = {
            let mut regions = self.regions.lock().ok()?;
            regions
                .entry(key)
                .or_insert_with(|| RegionFile::open(self.region_path(key.0, key.1)).ok().map(|r| Arc::new(Mutex::new(r))))
                .clone()?
        };
        let record = region.lock().ok()?.read(ChunkPos::new(x, z)).ok()??;
        let root = record.nbt(Limits::default()).ok()?;
        decode(&root, registry, biomes)
    }
    /// Rewrite every region touched by `chunks`, keeping their other chunks.
    pub fn save(&self, chunks: &[Arc<Chunk>], registry: &BlockRegistry, biomes: &Biomes) -> Result<usize> {
        let mut by_region: BTreeMap<(i32, i32), Vec<&Arc<Chunk>>> = BTreeMap::new();
        for c in chunks {
            by_region.entry((c.x >> 5, c.z >> 5)).or_default().push(c);
        }
        let staging = self.root.join("region/.staging");
        fs::create_dir_all(&staging)?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as u32);
        for ((rx, rz), list) in by_region {
            let path = self.region_path(rx, rz);
            let mut records: BTreeMap<(i32, i32), ChunkRecord> = BTreeMap::new();
            if let Ok(mut existing) = RegionFile::open(&path) {
                let positions: Vec<ChunkPos> = existing.chunks().map(|(p, _)| p).collect();
                for p in positions {
                    if let Ok(Some(r)) = existing.read(p) {
                        records.insert((p.x, p.z), r);
                    }
                }
            }
            for c in list {
                let record = ChunkRecord::from_nbt(&encode(c, registry, biomes), Compression::Zlib, timestamp, Limits::default())?;
                records.insert((c.x, c.z), record);
            }
            let temporary = staging.join(format!("r.{rx}.{rz}.mca"));
            let _ = fs::remove_file(&temporary);
            let mut writer = RegionWriter::create(&temporary)?;
            for ((x, z), r) in &records {
                writer.write(ChunkPos::new(*x, *z), r)?;
            }
            writer.finish()?;
            fs::rename(&temporary, &path)?;
            self.regions.lock().map_err(|_| "region cache poisoned")?.remove(&(rx, rz));
        }
        Ok(chunks.len())
    }
}
/// Player and world state stored in `level.dat`.
#[derive(Clone, Debug, Default)]
pub struct Level {
    pub name: String,
    pub seed: i64,
    pub creative: bool,
    pub time: i64,
    pub day_time: i64,
    pub spawn: Option<[f64; 3]>,
    pub position: Option<[f64; 3]>,
    pub rotation: [f32; 2],
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub inventory: Vec<(u8, String, u32, u32)>,
}
impl Level {
    pub fn read(dir: &Path) -> Option<Self> {
        let bytes = fs::read(dir.join("level.dat")).ok()?;
        let raw = Compression::Gzip.decode(&bytes, 64 << 20).ok()?;
        let (root, _) = nbt::read_named(&raw, Limits::default()).ok()?;
        let data = root.tag.get("Data")?;
        let long = |t: Option<&Tag>| match t {
            Some(Tag::Long(v)) => *v,
            Some(Tag::Int(v)) => i64::from(*v),
            _ => 0,
        };
        let doubles = |t: Option<&Tag>| match t {
            Some(Tag::List(l)) if l.len() == 3 => {
                let v: Vec<f64> = l.iter().map(|d| if let Tag::Double(b) = d { f64::from_bits(*b) } else { 0.0 }).collect();
                Some([v[0], v[1], v[2]])
            }
            _ => None,
        };
        let player = data.get("Player");
        let float = |t: Option<&Tag>| if let Some(Tag::Float(b)) = t { f32::from_bits(*b) } else { 0.0 };
        let mut inventory = Vec::new();
        if let Some(Tag::List(items)) = player.and_then(|p| p.get("Inventory")) {
            for it in items {
                let slot = if let Some(Tag::Byte(s)) = it.get("Slot") { *s as u8 } else { continue };
                let id = if let Some(Tag::String(s)) = it.get("id") { s.to_string_lossy() } else { continue };
                let count = long(it.get("count")).max(1) as u32;
                let damage = it
                    .get("components")
                    .and_then(|c| c.get("minecraft:damage"))
                    .map_or(0, |d| long(Some(d)) as u32);
                inventory.push((slot, id, count, damage));
            }
        }
        let rotation = match player.and_then(|p| p.get("Rotation")) {
            Some(Tag::List(r)) if r.len() == 2 => [float(r.first()), float(r.get(1))],
            _ => [0.0; 2],
        };
        Some(Self {
            name: match data.get("LevelName") {
                Some(Tag::String(s)) => s.to_string_lossy(),
                _ => "World".into(),
            },
            seed: long(data.get("WorldGenSettings").and_then(|w| w.get("seed"))),
            creative: long(data.get("GameType")) == 1,
            time: long(data.get("Time")),
            day_time: long(data.get("DayTime")),
            spawn: Some([long(data.get("SpawnX")) as f64, long(data.get("SpawnY")) as f64, long(data.get("SpawnZ")) as f64]),
            position: doubles(player.and_then(|p| p.get("Pos"))),
            rotation,
            health: player.map_or(20.0, |p| float(p.get("Health"))),
            food: player.map_or(20, |p| long(p.get("foodLevel")) as i32),
            saturation: player.map_or(5.0, |p| float(p.get("foodSaturationLevel"))),
            inventory,
        })
    }
    pub fn write(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let spawn = self.spawn.unwrap_or([0.0, 64.0, 0.0]);
        let mut player = vec![
            ("Health", Tag::Float(self.health.to_bits())),
            ("foodLevel", Tag::Int(self.food)),
            ("foodSaturationLevel", Tag::Float(self.saturation.to_bits())),
            ("playerGameType", Tag::Int(i32::from(self.creative))),
            (
                "Rotation",
                Tag::List(vec![Tag::Float(self.rotation[0].to_bits()), Tag::Float(self.rotation[1].to_bits())]),
            ),
            (
                "Inventory",
                Tag::List(
                    self.inventory
                        .iter()
                        .map(|(slot, id, count, damage)| {
                            let mut entries = vec![
                                ("Slot", Tag::Byte(*slot as i8)),
                                ("id", string(id)),
                                ("count", Tag::Int(*count as i32)),
                            ];
                            if *damage > 0 {
                                entries.push(("components", Tag::compound([("minecraft:damage", Tag::Int(*damage as i32))])));
                            }
                            Tag::compound(entries)
                        })
                        .collect(),
                ),
            ),
            ("Dimension", string("minecraft:overworld")),
        ];
        if let Some(p) = self.position {
            player.push(("Pos", Tag::List(p.iter().map(|v| Tag::Double(v.to_bits())).collect())));
        }
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound([(
                "Data",
                Tag::compound([
                    ("DataVersion", Tag::Int(DATA_VERSION)),
                    ("version", Tag::Int(19133)),
                    ("LevelName", string(&self.name)),
                    ("GameType", Tag::Int(i32::from(self.creative))),
                    ("Time", Tag::Long(self.time)),
                    ("DayTime", Tag::Long(self.day_time)),
                    ("SpawnX", Tag::Int(spawn[0].floor() as i32)),
                    ("SpawnY", Tag::Int(spawn[1].floor() as i32)),
                    ("SpawnZ", Tag::Int(spawn[2].floor() as i32)),
                    ("initialized", Tag::Byte(1)),
                    ("allowCommands", Tag::Byte(0)),
                    ("hardcore", Tag::Byte(0)),
                    ("Difficulty", Tag::Byte(2)),
                    ("WorldGenSettings", Tag::compound([("seed", Tag::Long(self.seed)), ("generate_features", Tag::Byte(1)), ("bonus_chest", Tag::Byte(0))])),
                    ("Player", Tag::compound(player)),
                ]),
            )]),
        };
        let bytes = nbt::write_named(&root, Limits::default()).map_err(|e| e.to_string())?;
        let compressed = Compression::Gzip.encode(&bytes)?;
        let temporary = dir.join("level.dat_new");
        fs::write(&temporary, compressed)?;
        if dir.join("level.dat").exists() {
            let _ = fs::rename(dir.join("level.dat"), dir.join("level.dat_old"));
        }
        fs::rename(temporary, dir.join("level.dat"))?;
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
        let mut chunk = self.save.load(x, z, &self.registry.registry, &self.biomes)?;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        Some(chunk)
    }
    fn proto(&self, worker: &mut G::Worker, x: i32, z: i32) -> crate::streaming::Proto {
        self.inner.proto(worker, x, z)
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
