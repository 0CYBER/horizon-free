//! `ChunkPalettedStorageFix` (key 1451.1): the chunk sections of the 1.12 format
//! (`Blocks`, `Data`, `Add` byte arrays) become a palette and packed `BlockStates`,
//! with the block entities that carried block state (note blocks, flower pots,
//! beds, banners, skulls) folded into the states, as vanilla does.
//!
//! Vanilla compares block states by identity (`CrudeIncrementalIntIdentityHashBiMap`
//! and an identity set): states are instances here, numbered in one space with the
//! instances of the `BlockStateData` table.
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use super::block_state_data::{self as bsd, State};
use super::dynamic::{as_bool, as_byte_buffer, as_int, as_str, ceillog2, key, set};
use crate::nbt::Tag;

/// `MappingConstants`.
struct Consts {
    /// Instances: the `BlockStateData` ones first, then the constants below.
    states: Vec<State>,
    air: u32,
    pumpkin: u32,
    snowy_podzol: u32,
    snowy_grass: u32,
    snowy_mycelium: u32,
    upper: HashMap<&'static str, u32>,
    flower_pot: HashMap<String, u32>,
    skull: HashMap<String, u32>,
    door: HashMap<String, u32>,
    note_block: HashMap<String, u32>,
    bed: HashMap<String, u32>,
    banner: HashMap<String, u32>,
}

const DYE_COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

fn consts() -> &'static Consts {
    static C: OnceLock<Consts> = OnceLock::new();
    C.get_or_init(|| {
        let mut states = bsd::data().instances.clone();
        let mut add = |name: &str, props: &[(&str, &str)]| -> u32 {
            states.push(State::new(name, props));
            (states.len() - 1) as u32
        };
        let air = add("minecraft:air", &[]);
        let pumpkin = add("minecraft:pumpkin", &[]);
        let snowy_podzol = add("minecraft:podzol", &[("snowy", "true")]);
        let snowy_grass = add("minecraft:grass_block", &[("snowy", "true")]);
        let snowy_mycelium = add("minecraft:mycelium", &[("snowy", "true")]);
        let mut upper = HashMap::new();
        for name in [
            "minecraft:sunflower",
            "minecraft:lilac",
            "minecraft:tall_grass",
            "minecraft:large_fern",
            "minecraft:rose_bush",
            "minecraft:peony",
        ] {
            upper.insert(name, add(name, &[("half", "upper")]));
        }
        let mut flower_pot = HashMap::new();
        let pots: [(&str, &str); 22] = [
            ("minecraft:air0", "minecraft:flower_pot"),
            ("minecraft:red_flower0", "minecraft:potted_poppy"),
            ("minecraft:red_flower1", "minecraft:potted_blue_orchid"),
            ("minecraft:red_flower2", "minecraft:potted_allium"),
            ("minecraft:red_flower3", "minecraft:potted_azure_bluet"),
            ("minecraft:red_flower4", "minecraft:potted_red_tulip"),
            ("minecraft:red_flower5", "minecraft:potted_orange_tulip"),
            ("minecraft:red_flower6", "minecraft:potted_white_tulip"),
            ("minecraft:red_flower7", "minecraft:potted_pink_tulip"),
            ("minecraft:red_flower8", "minecraft:potted_oxeye_daisy"),
            ("minecraft:yellow_flower0", "minecraft:potted_dandelion"),
            ("minecraft:sapling0", "minecraft:potted_oak_sapling"),
            ("minecraft:sapling1", "minecraft:potted_spruce_sapling"),
            ("minecraft:sapling2", "minecraft:potted_birch_sapling"),
            ("minecraft:sapling3", "minecraft:potted_jungle_sapling"),
            ("minecraft:sapling4", "minecraft:potted_acacia_sapling"),
            ("minecraft:sapling5", "minecraft:potted_dark_oak_sapling"),
            ("minecraft:red_mushroom0", "minecraft:potted_red_mushroom"),
            (
                "minecraft:brown_mushroom0",
                "minecraft:potted_brown_mushroom",
            ),
            ("minecraft:deadbush0", "minecraft:potted_dead_bush"),
            ("minecraft:tallgrass2", "minecraft:potted_fern"),
            ("minecraft:cactus0", "minecraft:potted_cactus"),
        ];
        for (k, name) in pots {
            flower_pot.insert(k.to_owned(), add(name, &[]));
        }
        let mut skull = HashMap::new();
        for (i, name, ty) in [
            (0, "skeleton", "skull"),
            (1, "wither_skeleton", "skull"),
            (2, "zombie", "head"),
            (3, "player", "head"),
            (4, "creeper", "head"),
            (5, "dragon", "head"),
        ] {
            for facing in ["north", "east", "south", "west"] {
                skull.insert(
                    format!("{i}{facing}"),
                    add(
                        &format!("minecraft:{name}_wall_{ty}"),
                        &[("facing", facing)],
                    ),
                );
            }
            for rot in 0..16 {
                skull.insert(
                    format!("{i}{rot}"),
                    add(
                        &format!("minecraft:{name}_{ty}"),
                        &[("rotation", &rot.to_string())],
                    ),
                );
            }
        }
        let mut door = HashMap::new();
        for ty in [
            "oak_door",
            "iron_door",
            "spruce_door",
            "birch_door",
            "jungle_door",
            "acacia_door",
            "dark_oak_door",
        ] {
            let id = format!("minecraft:{ty}");
            for facing in ["east", "north", "south", "west"] {
                for half in ["lower", "upper"] {
                    for hinge in ["left", "right"] {
                        for open in ["false", "true"] {
                            for powered in ["false", "true"] {
                                door.insert(
                                    format!("{id}{facing}{half}{hinge}{open}{powered}"),
                                    add(
                                        &id,
                                        &[
                                            ("facing", facing),
                                            ("half", half),
                                            ("hinge", hinge),
                                            ("open", open),
                                            ("powered", powered),
                                        ],
                                    ),
                                );
                            }
                        }
                    }
                }
            }
        }
        let mut note_block = HashMap::new();
        for i in 0..26 {
            for powered in ["true", "false"] {
                note_block.insert(
                    format!("{powered}{i}"),
                    add(
                        "minecraft:note_block",
                        &[("powered", powered), ("note", &i.to_string())],
                    ),
                );
            }
        }
        let mut bed = HashMap::new();
        for (id, color) in DYE_COLORS.iter().enumerate() {
            if *color == "red" {
                continue;
            }
            let name = format!("minecraft:{color}_bed");
            for (facing, occupied, part) in [
                ("south", "false", "foot"),
                ("west", "false", "foot"),
                ("north", "false", "foot"),
                ("east", "false", "foot"),
                ("south", "false", "head"),
                ("west", "false", "head"),
                ("north", "false", "head"),
                ("east", "false", "head"),
                ("south", "true", "head"),
                ("west", "true", "head"),
                ("north", "true", "head"),
                ("east", "true", "head"),
            ] {
                bed.insert(
                    format!("{facing}{occupied}{part}{id}"),
                    add(
                        &name,
                        &[("facing", facing), ("occupied", occupied), ("part", part)],
                    ),
                );
            }
        }
        let mut banner = HashMap::new();
        for (id, color) in DYE_COLORS.iter().enumerate() {
            if *color == "white" {
                continue;
            }
            let color_id = 15 - id;
            for i in 0..16 {
                banner.insert(
                    format!("{i}_{color_id}"),
                    add(
                        &format!("minecraft:{color}_banner"),
                        &[("rotation", &i.to_string())],
                    ),
                );
            }
            for facing in ["north", "south", "west", "east"] {
                banner.insert(
                    format!("{facing}_{color_id}"),
                    add(
                        &format!("minecraft:{color}_wall_banner"),
                        &[("facing", facing)],
                    ),
                );
            }
        }
        Consts {
            states,
            air,
            pumpkin,
            snowy_podzol,
            snowy_grass,
            snowy_mycelium,
            upper,
            flower_pot,
            skull,
            door,
            note_block,
            bed,
            banner,
        }
    })
}

/// The block ids (`id >> 4`) whose blocks need a second look.
const FIX: [usize; 18] = [
    2, 3, 110, 140, 144, 25, 86, 26, 176, 177, 175, 64, 71, 193, 194, 195, 196, 197,
];
const VIRTUAL: &[usize] = &[
    54, 146, 25, 26, 51, 53, 67, 108, 109, 114, 128, 134, 135, 136, 156, 163, 164, 180, 203, 55,
    85, 113, 188, 189, 190, 191, 192, 93, 94, 101, 102, 160, 106, 107, 183, 184, 185, 186, 187,
    132, 139, 199,
];

fn is_virtual(block: usize) -> bool {
    VIRTUAL.contains(&block)
}

/// `getSideMask` (LeavesFix has the same copy).
pub(super) fn side_mask(west: bool, east: bool, north: bool, south: bool) -> i32 {
    let mut s = 0;
    if north {
        if east {
            s |= 2;
        } else if west {
            s |= 128;
        } else {
            s |= 1;
        }
    } else if south {
        if west {
            s |= 32;
        } else if east {
            s |= 8;
        } else {
            s |= 16;
        }
    } else if east {
        s |= 4;
    } else if west {
        s |= 64;
    }
    s
}

/// A `DataLayer`: 4 bits per block.
struct Nibbles(Vec<u8>);

impl Nibbles {
    fn get(&self, x: usize, y: usize, z: usize) -> i32 {
        let position = y << 8 | z << 4 | x;
        let byte = self.0[position >> 1];
        if position & 1 == 0 {
            i32::from(byte & 15)
        } else {
            i32::from(byte >> 4 & 15)
        }
    }
}

struct Section {
    y: i32,
    has_data: bool,
    /// The `listTag`: instances in the order they were first seen.
    list: Vec<u32>,
    seen: HashSet<u32>,
    /// `palette`: instance by palette id.
    palette: Vec<u32>,
    palette_ids: HashMap<u32, i32>,
    to_fix: Vec<(usize, Vec<usize>)>,
    update: Vec<i32>,
    buffer: Vec<i32>,
    raw: Tag,
}

impl Section {
    fn new(raw: Tag) -> Self {
        let y = as_int(raw.get("Y"), 0);
        let has_data = raw.get("Blocks").is_some();
        Self {
            y,
            has_data,
            list: Vec::new(),
            seen: HashSet::new(),
            palette: Vec::new(),
            palette_ids: HashMap::new(),
            to_fix: Vec::new(),
            update: Vec::new(),
            buffer: vec![0; 4096],
            raw,
        }
    }

    fn get_block(&self, pos: usize) -> u32 {
        if pos <= 4095 {
            self.palette
                .get(self.buffer[pos] as usize)
                .copied()
                .unwrap_or_else(|| consts().air)
        } else {
            consts().air
        }
    }

    fn set_block(&mut self, idx: usize, state: u32) {
        if self.seen.insert(state) {
            self.list.push(state);
        }
        let id = match self.palette_ids.get(&state) {
            Some(id) => *id,
            None => {
                self.palette.push(state);
                let id = (self.palette.len() - 1) as i32;
                self.palette_ids.insert(state, id);
                id
            }
        };
        self.buffer[idx] = id;
    }

    fn add_fix(&mut self, block: usize, position: usize) {
        match self.to_fix.iter_mut().find(|(b, _)| *b == block) {
            Some((_, list)) => list.push(position),
            None => self.to_fix.push((block, vec![position])),
        }
    }

    fn upgrade(&mut self, mut sides: i32) -> i32 {
        if !self.has_data {
            return sides;
        }
        let blocks = as_byte_buffer(self.raw.get("Blocks")).unwrap_or_default();
        let nibbles = |name: &str| {
            as_byte_buffer(self.raw.get(name))
                .map(Nibbles)
                .unwrap_or_else(|| Nibbles(vec![0; 2048]))
        };
        let data = nibbles("Data");
        let add_blocks = nibbles("Add");
        let c = consts();
        self.seen.insert(c.air);
        self.palette.push(c.air);
        self.palette_ids.insert(c.air, 0);
        self.list.push(c.air);
        let d = bsd::data();
        for idx in 0..4096usize {
            let (xx, yy, zz) = (idx & 15, idx >> 8 & 15, idx >> 4 & 15);
            let id = add_blocks.get(xx, yy, zz) << 12
                | i32::from(blocks.get(idx).copied().unwrap_or(0)) << 4
                | data.get(xx, yy, zz);
            let block = (id >> 4) as usize;
            if FIX.contains(&block) {
                self.add_fix(block, idx);
            }
            if is_virtual(block) {
                let s = side_mask(xx == 0, xx == 15, zz == 0, zz == 15);
                if s == 0 {
                    self.update.push(idx as i32);
                } else {
                    sides |= s;
                }
            }
            self.set_block(idx, d.instance(id));
        }
        sides
    }

    fn write(&self) -> Tag {
        let mut section = self.raw.clone();
        if !self.has_data {
            return section;
        }
        let c = consts();
        let palette: Vec<Tag> = self
            .list
            .iter()
            .map(|s| {
                let state = &c.states[*s as usize];
                if state.name == "%%FILTER_ME%%" {
                    c.states[c.air as usize].to_tag()
                } else {
                    state.to_tag()
                }
            })
            .collect();
        set(&mut section, "Palette", Tag::List(palette));
        let bits = ceillog2(self.seen.len()).max(4) as usize;
        let mut storage = vec![0i64; (4096 * bits).div_ceil(64)];
        let mask = (1i64 << bits) - 1;
        for (index, value) in self.buffer.iter().enumerate() {
            let value = i64::from(*value);
            let position = index * bits;
            let start = position >> 6;
            let end = ((index + 1) * bits - 1) >> 6;
            let start_bit = position ^ (start << 6);
            storage[start] = storage[start] & !(mask << start_bit) | (value & mask) << start_bit;
            if start != end {
                let shift = 64 - start_bit;
                let wanted = bits - shift;
                storage[end] =
                    ((storage[end] as u64) >> wanted << wanted) as i64 | (value & mask) >> shift;
            }
        }
        set(&mut section, "BlockStates", Tag::LongArray(storage));
        for f in ["Blocks", "Data", "Add"] {
            super::dynamic::remove(&mut section, f);
        }
        section
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Down,
    Up,
}

fn relative(pos: i64, direction: Direction) -> i64 {
    let y = (pos >> 8)
        + if matches!(direction, Direction::Up) {
            1
        } else {
            -1
        };
    if (0..=255).contains(&y) {
        pos & 0xFF | y << 8
    } else {
        -1
    }
}

struct UpgradeChunk {
    sides: i32,
    sections: Vec<Option<Section>>,
    /// `Int2ObjectLinkedOpenHashMap`: insertion order, a repeated key keeps its place.
    block_entities: Vec<(i64, Tag)>,
}

impl UpgradeChunk {
    fn block_entity(&self, pos: i64) -> Option<&Tag> {
        self.block_entities
            .iter()
            .find(|(k, _)| *k == pos)
            .map(|(_, v)| v)
    }
    fn remove_block_entity(&mut self, pos: i64) -> Option<Tag> {
        let i = self.block_entities.iter().position(|(k, _)| *k == pos)?;
        Some(self.block_entities.remove(i).1)
    }
    fn section(&self, pos: i64) -> Option<&Section> {
        self.sections.get((pos >> 12) as usize)?.as_ref()
    }
    fn get_block(&self, pos: i64) -> u32 {
        if (0..=65535).contains(&pos) {
            match self.section(pos) {
                Some(s) => s.get_block((pos & 4095) as usize),
                None => consts().air,
            }
        } else {
            consts().air
        }
    }
    fn set_block(&mut self, pos: i64, block: u32) {
        if (0..=65535).contains(&pos)
            && let Some(Some(s)) = self.sections.get_mut((pos >> 12) as usize)
        {
            s.set_block((pos & 4095) as usize, block);
        }
    }
    fn name(&self, pos: i64) -> &str {
        &consts().states[self.get_block(pos) as usize].name
    }
    fn state(&self, pos: i64) -> &State {
        &consts().states[self.get_block(pos) as usize]
    }
}

/// `ChunkPalettedStorageFix.fix` over a chunk (`{Level: ...}`).
pub(super) fn fix(chunk: &mut Tag) {
    let Some(level) = chunk.get_mut("Level") else {
        return;
    };
    if !matches!(level.get("Sections"), Some(Tag::List(_))) {
        return;
    }
    let new_level = upgrade_level(level);
    *level = new_level;
}

fn upgrade_level(level: &Tag) -> Tag {
    let c = consts();
    let x0 = as_int(level.get("xPos"), 0) << 4;
    let z0 = as_int(level.get("zPos"), 0) << 4;
    let mut chunk = UpgradeChunk {
        sides: 0,
        sections: (0..16).map(|_| None).collect(),
        block_entities: Vec::new(),
    };
    if let Some(Tag::List(entities)) = level.get("TileEntities") {
        for entity in entities {
            let x = i64::from(as_int(entity.get("x"), 0).wrapping_sub(x0) & 15);
            let y = i64::from(as_int(entity.get("y"), 0));
            let z = i64::from(as_int(entity.get("z"), 0).wrapping_sub(z0) & 15);
            let k = (y << 8) | (z << 4) | x;
            match chunk.block_entities.iter_mut().find(|(key, _)| *key == k) {
                Some((_, v)) => *v = entity.clone(),
                None => chunk.block_entities.push((k, entity.clone())),
            }
        }
    }
    let converted_from_alpha = as_bool(level.get("convertedFromAlphaFormat"), false);
    if let Some(Tag::List(sections)) = level.get("Sections") {
        for raw in sections {
            let mut section = Section::new(raw.clone());
            chunk.sides = section.upgrade(chunk.sides);
            let y = section.y;
            if (0..16).contains(&y) {
                chunk.sections[y as usize] = Some(section);
            }
        }
    }
    for si in 0..16 {
        let Some(section) = &chunk.sections[si] else {
            continue;
        };
        let to_fix = section.to_fix.clone();
        let section_y = section.y as i64;
        for (block, positions) in to_fix {
            let dy = section_y << 12;
            for pos in positions {
                let pos = pos as i64 | dy;
                fix_block(&mut chunk, c, block, pos, converted_from_alpha);
            }
        }
    }
    // write()
    let mut out = level.clone();
    if chunk.block_entities.is_empty() {
        super::dynamic::remove(&mut out, "TileEntities");
    } else {
        set(
            &mut out,
            "TileEntities",
            Tag::List(
                chunk
                    .block_entities
                    .iter()
                    .map(|(_, v)| v.clone())
                    .collect(),
            ),
        );
    }
    let mut indices = crate::nbt::Compound::new();
    let mut sections = Vec::new();
    for section in chunk.sections.iter().flatten() {
        sections.push(section.write());
        indices.insert(
            key(&section.y.to_string()),
            Tag::IntArray(section.update.clone()),
        );
    }
    let upgrade_data = Tag::compound([
        ("Sides", Tag::Byte(chunk.sides as i8)),
        ("Indices", Tag::Compound(indices)),
    ]);
    set(&mut out, "UpgradeData", upgrade_data);
    set(&mut out, "Sections", Tag::List(sections));
    out
}

fn fix_block(chunk: &mut UpgradeChunk, c: &Consts, block: usize, pos: i64, alpha: bool) {
    let up = relative(pos, Direction::Up);
    let down = relative(pos, Direction::Down);
    let is_snow = |n: &str| n == "minecraft:snow" || n == "minecraft:snow_layer";
    match block {
        2 => {
            if chunk.name(pos) == "minecraft:grass_block" && is_snow(chunk.name(up)) {
                chunk.set_block(pos, c.snowy_grass);
            }
        }
        3 => {
            if chunk.name(pos) == "minecraft:podzol" && is_snow(chunk.name(up)) {
                chunk.set_block(pos, c.snowy_podzol);
            }
        }
        25 => {
            if let Some(entity) = chunk.remove_block_entity(pos) {
                let note = as_int(entity.get("note"), 0).clamp(0, 24) as i8;
                let k = format!("{}{}", as_bool(entity.get("powered"), false), note);
                let state = c
                    .note_block
                    .get(&k)
                    .or_else(|| c.note_block.get("false0"))
                    .copied()
                    .expect("note block");
                chunk.set_block(pos, state);
            }
        }
        26 => {
            if let Some(entity) = chunk.block_entity(pos) {
                let color = as_int(entity.get("color"), 0);
                let state = chunk.state(pos);
                if color != 14 && (0..16).contains(&color) {
                    let k = format!(
                        "{}{}{}{}",
                        state.property("facing"),
                        state.property("occupied"),
                        state.property("part"),
                        color
                    );
                    if let Some(s) = c.bed.get(&k).copied() {
                        chunk.set_block(pos, s);
                    }
                }
            }
        }
        64 | 71 | 193..=197 => {
            if chunk.name(pos).ends_with("_door") {
                let lower = chunk.state(pos).clone();
                if lower.property("half") == "lower" {
                    let above = up;
                    let upper = chunk.state(above).clone();
                    if lower.name == upper.name {
                        let facing = lower.property("facing");
                        let open = lower.property("open");
                        let hinge = if alpha {
                            "left"
                        } else {
                            upper.property("hinge")
                        };
                        let powered = if alpha {
                            "false"
                        } else {
                            upper.property("powered")
                        };
                        let name = &lower.name;
                        let lo = c
                            .door
                            .get(&format!("{name}{facing}lower{hinge}{open}{powered}"))
                            .copied();
                        let hi = c
                            .door
                            .get(&format!("{name}{facing}upper{hinge}{open}{powered}"))
                            .copied();
                        if let (Some(lo), Some(hi)) = (lo, hi) {
                            chunk.set_block(pos, lo);
                            chunk.set_block(above, hi);
                        }
                    }
                }
            }
        }
        86 => {
            if chunk.name(pos) == "minecraft:carved_pumpkin" {
                let below = chunk.name(down);
                if below == "minecraft:grass_block" || below == "minecraft:dirt" {
                    chunk.set_block(pos, c.pumpkin);
                }
            }
        }
        110 => {
            if chunk.name(pos) == "minecraft:mycelium" && is_snow(chunk.name(up)) {
                chunk.set_block(pos, c.snowy_mycelium);
            }
        }
        140 => {
            if let Some(entity) = chunk.remove_block_entity(pos) {
                let item = as_str(entity.get("Item")).unwrap_or_default();
                let k = format!("{}{}", item, as_int(entity.get("Data"), 0));
                let state = c
                    .flower_pot
                    .get(&k)
                    .or_else(|| c.flower_pot.get("minecraft:air0"))
                    .copied()
                    .expect("flower pot");
                chunk.set_block(pos, state);
            }
        }
        144 => {
            if let Some(entity) = chunk.block_entity(pos) {
                let ty = as_int(entity.get("SkullType"), 0).to_string();
                let facing = chunk.state(pos).property("facing").to_owned();
                let k = if facing != "up" && facing != "down" {
                    format!("{ty}{facing}")
                } else {
                    format!("{ty}{}", as_int(entity.get("Rot"), 0))
                };
                // `entity.remove(...)` on an immutable Dynamic changes nothing.
                let state = c
                    .skull
                    .get(&k)
                    .or_else(|| c.skull.get("0north"))
                    .copied()
                    .expect("skull");
                chunk.set_block(pos, state);
            }
        }
        175 => {
            if chunk.state(pos).property("half") == "upper" {
                let variant = chunk.name(down).to_owned();
                if let Some(s) = c.upper.get(variant.as_str()).copied() {
                    chunk.set_block(pos, s);
                }
            }
        }
        176 | 177 => {
            if let Some(entity) = chunk.block_entity(pos) {
                let color = as_int(entity.get("Base"), 0);
                let state = chunk.state(pos);
                if color != 15 && (0..16).contains(&color) {
                    let prop = state.property(if block == 176 { "rotation" } else { "facing" });
                    let k = format!("{prop}_{color}");
                    if let Some(s) = c.banner.get(&k).copied() {
                        chunk.set_block(pos, s);
                    }
                }
            }
        }
        _ => {}
    }
}
