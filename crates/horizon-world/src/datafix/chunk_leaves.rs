//! `LeavesFix` (key 1496) and `TrappedChestBlockEntityFix` (key 1624): fixes that
//! read the packed block states of the sections of a chunk.
//!
//! `LeavesFix` visits blocks in the iteration order of fastutil hash sets, and the
//! order decides in which order its new palette entries appear: the sets are
//! emulated here (`IntOpenHashSet` and the key order of an `Int2ObjectOpenHashMap`).
use std::collections::{HashMap, HashSet};

use super::chunk_flatten::side_mask;
use super::dynamic::{as_int, as_long_stream, as_str, ceillog2, set};
use crate::nbt::Tag;

/// `PackedBitStorage`.
#[derive(Clone)]
struct Packed {
    data: Vec<i64>,
    bits: usize,
    mask: i64,
}

impl Packed {
    fn new(bits: usize, data: Vec<i64>) -> Option<Self> {
        (data.len() == (4096 * bits).div_ceil(64)).then(|| Self {
            data,
            bits,
            mask: (1i64 << bits) - 1,
        })
    }
    fn zeroed(bits: usize) -> Self {
        Self::new(bits, vec![0; (4096 * bits).div_ceil(64)]).expect("length")
    }
    fn set(&mut self, index: usize, value: i32) {
        let position = index * self.bits;
        let start = position >> 6;
        let end = ((index + 1) * self.bits - 1) >> 6;
        let start_bit = position ^ (start << 6);
        let value = i64::from(value);
        self.data[start] =
            self.data[start] & !(self.mask << start_bit) | (value & self.mask) << start_bit;
        if start != end {
            let shift = 64 - start_bit;
            let wanted = self.bits - shift;
            self.data[end] =
                ((self.data[end] as u64) >> wanted << wanted) as i64 | (value & self.mask) >> shift;
        }
    }
    fn get(&self, index: usize) -> i32 {
        let position = index * self.bits;
        let start = position >> 6;
        let end = ((index + 1) * self.bits - 1) >> 6;
        let start_bit = position ^ (start << 6);
        if start == end {
            return (((self.data[start] as u64) >> start_bit) as i64 & self.mask) as i32;
        }
        let shift = 64 - start_bit;
        ((((self.data[start] as u64) >> start_bit) as i64 | self.data[end] << shift) & self.mask)
            as i32
    }
}

// ------------------------------------------------------------------ fastutil

fn array_size(expected: usize) -> usize {
    // arraySize(expected, 0.75f): nextPowerOfTwo(ceil(expected / f)), at least 2
    let need = (expected as f64 / 0.75f32 as f64).ceil() as usize;
    need.next_power_of_two().max(2)
}

fn max_fill(n: usize) -> usize {
    ((n as f64 * 0.75f32 as f64).ceil() as usize).min(n - 1)
}

fn mix(x: i32) -> usize {
    let h = x.wrapping_mul(0x9E37_79B9u32 as i32);
    (h ^ ((h as u32) >> 16) as i32) as u32 as usize
}

/// `IntOpenHashSet`: add and iteration order.
struct IntSet {
    key: Vec<i32>,
    n: usize,
    max_fill: usize,
    size: usize,
    contains_null: bool,
}

impl IntSet {
    /// `new IntOpenHashSet()`.
    fn new() -> Self {
        Self::sized(16)
    }
    fn sized(expected: usize) -> Self {
        let n = array_size(expected);
        Self {
            key: vec![0; n + 1],
            n,
            max_fill: max_fill(n),
            size: 0,
            contains_null: false,
        }
    }
    fn add(&mut self, k: i32) -> bool {
        if k == 0 {
            if self.contains_null {
                return false;
            }
            self.contains_null = true;
        } else {
            let mask = self.n - 1;
            let mut pos = mix(k) & mask;
            while self.key[pos] != 0 {
                if self.key[pos] == k {
                    return false;
                }
                pos = (pos + 1) & mask;
            }
            self.key[pos] = k;
        }
        let grow = self.size >= self.max_fill;
        self.size += 1;
        if grow {
            self.rehash(array_size(self.size + 1));
        }
        true
    }
    fn rehash(&mut self, new_n: usize) {
        let mask = new_n - 1;
        let mut new_key = vec![0; new_n + 1];
        let real = if self.contains_null {
            self.size - 1
        } else {
            self.size
        };
        let mut i = self.n;
        for _ in 0..real {
            loop {
                i -= 1;
                if self.key[i] != 0 {
                    break;
                }
            }
            let mut pos = mix(self.key[i]) & mask;
            while new_key[pos] != 0 {
                pos = (pos + 1) & mask;
            }
            new_key[pos] = self.key[i];
        }
        self.n = new_n;
        self.max_fill = max_fill(new_n);
        self.key = new_key;
    }
    /// The elements in iteration order: the null key first, then the table from its end.
    fn order(&self) -> Vec<i32> {
        let mut out = Vec::with_capacity(self.size);
        if self.contains_null {
            out.push(0);
        }
        for pos in (0..self.n).rev() {
            if self.key[pos] != 0 {
                out.push(self.key[pos]);
            }
        }
        out
    }
}

/// The section indices in the iteration order of
/// `new Int2ObjectOpenHashMap<>(hashMapOfTheSections)`.
fn section_order(indices: &[i32]) -> Vec<i32> {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    let mut set = IntSet::sized(sorted.len());
    for i in sorted {
        set.add(i);
    }
    set.order()
}

// ------------------------------------------------------------------ sections

fn leaf_index(name: &str) -> Option<i32> {
    Some(match name {
        "minecraft:acacia_leaves" => 0,
        "minecraft:birch_leaves" => 1,
        "minecraft:dark_oak_leaves" => 2,
        "minecraft:jungle_leaves" => 3,
        "minecraft:oak_leaves" => 4,
        "minecraft:spruce_leaves" => 5,
        _ => return None,
    })
}

const LOGS: [&str; 18] = [
    "minecraft:acacia_bark",
    "minecraft:birch_bark",
    "minecraft:dark_oak_bark",
    "minecraft:jungle_bark",
    "minecraft:oak_bark",
    "minecraft:spruce_bark",
    "minecraft:acacia_log",
    "minecraft:birch_log",
    "minecraft:dark_oak_log",
    "minecraft:jungle_log",
    "minecraft:oak_log",
    "minecraft:spruce_log",
    "minecraft:stripped_acacia_log",
    "minecraft:stripped_birch_log",
    "minecraft:stripped_dark_oak_log",
    "minecraft:stripped_jungle_log",
    "minecraft:stripped_oak_log",
    "minecraft:stripped_spruce_log",
];

/// A section as `LeavesFix.Section` reads it.
struct Section {
    index: i32,
    palette: Vec<Tag>,
    storage: Option<Packed>,
    leaf_ids: HashSet<i32>,
    log_ids: HashSet<i32>,
    state_to_id: HashMap<i32, i32>,
}

fn name_of(state: &Tag) -> String {
    as_str(state.get("Name")).unwrap_or_default()
}

fn state_id(name: &str, persistent: bool, distance: i32) -> i32 {
    leaf_index(name).unwrap_or(0) << 5 | if persistent { 16 } else { 0 } | distance
}

fn leaf_tag(name: &str, persistent: bool, distance: i32) -> Tag {
    Tag::compound([
        (
            "Properties",
            Tag::compound([
                (
                    "persistent",
                    super::dynamic::string(if persistent { "true" } else { "false" }),
                ),
                ("distance", super::dynamic::string(&distance.to_string())),
            ]),
        ),
        ("Name", super::dynamic::string(name)),
    ])
}

impl Section {
    /// `LeavesSection` constructor: reads the palette, and rewrites its leaf states.
    fn read_leaves(section: &Tag) -> Self {
        let mut s = Self::read(section);
        // skippable()
        for i in 0..s.palette.len() {
            let name = name_of(&s.palette[i]);
            if leaf_index(&name).is_some() {
                let persistent = as_str(
                    s.palette[i]
                        .get("Properties")
                        .and_then(|p| p.get("decayable")),
                )
                .is_some_and(|d| d == "false");
                s.leaf_ids.insert(i as i32);
                s.state_to_id
                    .insert(state_id(&name, persistent, 7), i as i32);
                s.palette[i] = leaf_tag(&name, persistent, 7);
            }
            if LOGS.contains(&name.as_str()) {
                s.log_ids.insert(i as i32);
            }
        }
        s.finish_read(section, s.leaf_ids.is_empty() && s.log_ids.is_empty());
        s
    }

    fn read(section: &Tag) -> Self {
        let palette = match section.get("Palette") {
            Some(Tag::List(l)) => l.clone(),
            _ => Vec::new(),
        };
        Self {
            index: as_int(section.get("Y"), 0),
            palette,
            storage: None,
            leaf_ids: HashSet::new(),
            log_ids: HashSet::new(),
            state_to_id: HashMap::new(),
        }
    }

    fn finish_read(&mut self, section: &Tag, skippable: bool) {
        if skippable {
            self.storage = None;
        } else {
            let states = as_long_stream(section.get("BlockStates")).unwrap_or_default();
            let size = ceillog2(self.palette.len()).max(4) as usize;
            self.storage = Packed::new(size, states);
        }
    }

    fn is_skippable(&self) -> bool {
        self.storage.is_none()
    }
    fn block(&self, pos: usize) -> i32 {
        self.storage.as_ref().map_or(0, |s| s.get(pos))
    }
    fn is_leaf(&self, block: i32) -> bool {
        self.leaf_ids.contains(&block)
    }
    fn is_log(&self, block: i32) -> bool {
        self.log_ids.contains(&block)
    }
    fn distance(&self, block: i32) -> i32 {
        if self.is_log(block) {
            return 0;
        }
        self.palette
            .get(block as usize)
            .and_then(|p| p.get("Properties"))
            .and_then(|p| as_str(p.get("distance")))
            .and_then(|d| d.parse().ok())
            .unwrap_or(0)
    }
    fn set_distance(&mut self, pos: usize, block: i32, distance: i32) {
        let base = self.palette[block as usize].clone();
        let name = name_of(&base);
        let persistent = as_str(base.get("Properties").and_then(|p| p.get("persistent")))
            .is_some_and(|p| p == "true");
        let id_key = state_id(&name, persistent, distance);
        if !self.state_to_id.contains_key(&id_key) {
            let id = self.palette.len() as i32;
            self.leaf_ids.insert(id);
            self.state_to_id.insert(id_key, id);
            self.palette.push(leaf_tag(&name, persistent, distance));
        }
        let id = self.state_to_id[&id_key];
        let storage = self.storage.as_mut().expect("storage");
        if 1i64 << storage.bits <= i64::from(id) {
            let mut grown = Packed::zeroed(storage.bits + 1);
            for i in 0..4096 {
                grown.set(i, storage.get(i));
            }
            *storage = grown;
        }
        storage.set(pos, id);
    }
    fn write(&self, section: &mut Tag) {
        let Some(storage) = &self.storage else {
            return;
        };
        set(section, "BlockStates", Tag::LongArray(storage.data.clone()));
        set(section, "Palette", Tag::List(self.palette.clone()));
    }
}

fn get_index(x: i32, y: i32, z: i32) -> i32 {
    y << 8 | z << 4 | x
}

/// `LeavesFix` over a chunk (`{Level: ...}`).
pub(super) fn leaves_fix(chunk: &mut Tag) {
    let Some(level) = chunk.get_mut("Level") else {
        return;
    };
    let Some(Tag::List(raw_sections)) = level.get("Sections").cloned() else {
        return;
    };
    let mut sections: Vec<Section> = raw_sections.iter().map(Section::read_leaves).collect();
    if sections.iter().all(Section::is_skippable) {
        return;
    }
    let mut sides = 0;
    let mut queue: Vec<IntSet> = (0..7).map(|_| IntSet::new()).collect();
    let indices: Vec<i32> = sections.iter().map(|s| s.index).collect();
    for index in section_order(&indices) {
        let Some(section) = sections.iter().find(|s| s.index == index) else {
            continue;
        };
        if section.is_skippable() {
            continue;
        }
        for i in 0..4096usize {
            let block = section.block(i);
            if section.is_log(block) {
                queue[0].add(section.index << 12 | i as i32);
            } else if section.is_leaf(block) {
                let x = (i & 15) as i32;
                let z = (i >> 4 & 15) as i32;
                sides |= side_mask(x == 0, x == 15, z == 0, z == 15);
            }
        }
    }
    const DIRECTIONS: [[i32; 3]; 6] = [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ];
    for i in 1..7 {
        let current = queue[i - 1].order();
        for pos_chunk in current {
            let x = pos_chunk & 15;
            let y = pos_chunk >> 8 & 0xFF;
            let z = pos_chunk >> 4 & 15;
            for d in DIRECTIONS {
                let (nx, ny, nz) = (x + d[0], y + d[1], z + d[2]);
                if !(0..=15).contains(&nx) || !(0..=15).contains(&nz) || !(0..=255).contains(&ny) {
                    continue;
                }
                let Some(section) = sections.iter_mut().find(|s| s.index == ny >> 4) else {
                    continue;
                };
                if section.is_skippable() {
                    continue;
                }
                let pos_section = get_index(nx, ny & 15, nz) as usize;
                let block = section.block(pos_section);
                if section.is_leaf(block) && section.distance(block) > i as i32 {
                    section.set_distance(pos_section, block, i as i32);
                    queue[i].add(get_index(nx, ny, nz));
                }
            }
        }
    }
    if let Some(Tag::List(list)) = level.get_mut("Sections") {
        for raw in list.iter_mut() {
            let y = as_int(raw.get("Y"), 0);
            if let Some(section) = sections.iter().find(|s| s.index == y) {
                section.write(raw);
            }
        }
    }
    if sides != 0 {
        let mut upgrade = level
            .get("UpgradeData")
            .cloned()
            .unwrap_or_else(super::dynamic::empty_map);
        let old = as_int(upgrade.get("Sides"), 0) as i8;
        set(&mut upgrade, "Sides", Tag::Byte(old | sides as i8));
        set(level, "UpgradeData", upgrade);
    }
}

/// `TrappedChestBlockEntityFix` over a chunk (`{Level: ...}`).
pub(super) fn trapped_chest_fix(chunk: &mut Tag) {
    let Some(level) = chunk.get_mut("Level") else {
        return;
    };
    let Some(Tag::List(raw_sections)) = level.get("Sections").cloned() else {
        return;
    };
    let mut chests: HashSet<i32> = HashSet::new();
    for raw in &raw_sections {
        let mut section = Section::read(raw);
        let ids: HashSet<i32> = section
            .palette
            .iter()
            .enumerate()
            .filter(|(_, p)| name_of(p) == "minecraft:trapped_chest")
            .map(|(i, _)| i as i32)
            .collect();
        section.finish_read(raw, ids.is_empty());
        if section.is_skippable() {
            continue;
        }
        for i in 0..4096usize {
            if ids.contains(&section.block(i)) {
                chests.insert(section.index << 12 | i as i32);
            }
        }
    }
    let chunk_x = as_int(level.get("xPos"), 0);
    let chunk_z = as_int(level.get("zPos"), 0);
    if let Some(Tag::List(entities)) = level.get_mut("TileEntities") {
        for be in entities {
            if !super::schema::block_entity_known(be, super::schema::key(1624, 0) - 1) {
                continue;
            }
            let x = as_int(be.get("x"), 0).wrapping_sub(chunk_x << 4);
            let y = as_int(be.get("y"), 0);
            let z = as_int(be.get("z"), 0).wrapping_sub(chunk_z << 4);
            if chests.contains(&get_index(x, y, z)) {
                set(be, "id", super::dynamic::string("minecraft:trapped_chest"));
            }
        }
    }
}
