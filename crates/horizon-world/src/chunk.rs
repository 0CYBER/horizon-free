//! Saved section palettes and storage-level chunk access. Changes here do not
//! run block behavior, lighting, heightmaps, neighbor updates or scheduled ticks.
pub use crate::block_state_properties::BlockProperties;
use horizon_core::{
    position::{BlockPos, ChunkPos},
    storage::PackedStorage,
    ticks::{ChunkTicks, SavedTick, TickTypeId},
};
use std::collections::HashMap;
use std::io;
use {
    crate::{
        DATA_VERSION,
        compression::invalid,
        registry::{BlockRegistry, BlockStateId},
        tick_data,
        tick_data::TickKind,
    },
    horizon_core::nbt::{Compound, JavaString, NamedTag, Tag},
};

fn key(value: &str) -> JavaString {
    JavaString::from(value)
}
fn bits_for_palette(len: usize, min: u8) -> u8 {
    if len <= 1 {
        0
    } else {
        ((usize::BITS - (len - 1).leading_zeros()) as u8).max(min)
    }
}

/// `BlockEntity.getPosFromTag(chunk, tag)`: a saved block entity's `x`, `y`,
/// `z`, moved into `chunk` when the tag names another chunk. Vanilla warns
/// about such a save and keeps only the low four bits of each coordinate, so
/// the entity stays in the chunk that held it instead of landing in one that
/// never did.
pub fn block_entity_pos(pos: (i32, i32, i32), chunk: [i32; 2]) -> (i32, i32, i32) {
    let (x, y, z) = pos;
    if (x >> 4, z >> 4) == (chunk[0], chunk[1]) {
        return pos;
    }
    log::warn!(
        "Block entity at {x} {y} {z} found in a wrong chunk, expected position from chunk {chunk:?}"
    );
    (chunk[0] * 16 + (x & 15), y, chunk[1] * 16 + (z & 15))
}

#[derive(Clone, Debug)]
pub struct PalettedContainer {
    palette: Vec<Tag>,
    // Only loaded input can contain equal tags in multiple slots. Remember the
    // first slot of each such tag without changing the stored ids or bit width.
    // New slots appended by `set` are already unique and need no entry here.
    aliases: Option<Box<[u32]>>,
    storage: Option<PackedStorage>,
    entries: usize,
    min_bits: u8,
    extra: Compound,
}
impl PalettedContainer {
    pub fn uniform(value: Tag, entries: usize, min_bits: u8) -> io::Result<Self> {
        if entries == 0 || !(1..=32).contains(&min_bits) {
            return Err(invalid("invalid palette dimensions"));
        }
        if matches!(value, Tag::End) {
            return Err(invalid("End cannot be a palette entry"));
        }
        Ok(Self {
            palette: vec![value],
            aliases: None,
            storage: None,
            entries,
            min_bits,
            extra: Compound::new(),
        })
    }

    pub fn from_nbt(tag: &Tag, entries: usize, min_bits: u8) -> io::Result<Self> {
        let Tag::Compound(map) = tag else {
            return Err(invalid("palette container is not a compound"));
        };
        let Some(Tag::List(palette)) = map.get(&key("palette")) else {
            return Err(invalid("missing palette list"));
        };
        let first = palette.first().ok_or_else(|| invalid("empty palette"))?;
        let mut result = Self::uniform(first.clone(), entries, min_bits)?;
        if palette.len() > i32::MAX as usize || palette.iter().any(|t| matches!(t, Tag::End)) {
            return Err(invalid("invalid palette"));
        }
        result.palette = palette.clone();
        let bits = bits_for_palette(palette.len(), min_bits);
        if bits > 0 {
            let Some(Tag::LongArray(words)) = map.get(&key("data")) else {
                return Err(invalid("nonuniform palette is missing data"));
            };
            let storage =
                PackedStorage::from_words(bits, entries, words.iter().map(|&w| w as u64).collect())
                    .map_err(|e| invalid(e.to_string()))?;
            for index in 0..entries {
                if storage.get(index).unwrap() as usize >= palette.len() {
                    return Err(invalid("palette index outside palette"));
                }
            }
            result.aliases = duplicate_palette_slots(palette);
            result.storage = Some(storage);
        }
        result.extra = map.clone();
        result.extra.remove(&key("palette"));
        result.extra.remove(&key("data"));
        Ok(result)
    }

    pub fn bits(&self) -> u8 {
        self.storage.as_ref().map_or(0, PackedStorage::bits)
    }
    pub fn palette_len(&self) -> usize {
        self.palette.len()
    }
    pub fn get(&self, index: usize) -> io::Result<&Tag> {
        if index >= self.entries {
            return Err(invalid("section index out of bounds"));
        }
        let palette_index = self.storage.as_ref().map_or(0, |s| s.get(index).unwrap()) as usize;
        Ok(&self.palette[palette_index])
    }

    pub fn set(&mut self, index: usize, value: Tag) -> io::Result<Tag> {
        let old = self.get(index)?.clone();
        if matches!(value, Tag::End) {
            return Err(invalid("End cannot be a palette entry"));
        }
        let id = if let Some(id) = self.palette.iter().position(|v| *v == value) {
            id
        } else {
            if self.palette.len() >= i32::MAX as usize {
                return Err(invalid("palette too large"));
            }
            let next_bits = bits_for_palette(self.palette.len() + 1, self.min_bits);
            if next_bits != self.bits() {
                let mut next = PackedStorage::new(next_bits, self.entries)
                    .map_err(|e| invalid(e.to_string()))?;
                if let Some(previous) = &self.storage {
                    for i in 0..self.entries {
                        next.set(i, previous.get(i).unwrap()).unwrap();
                    }
                }
                self.storage = Some(next);
            }
            self.palette.push(value);
            self.palette.len() - 1
        };
        if let Some(storage) = &mut self.storage {
            storage
                .set(index, id as u32)
                .map_err(|e| invalid(e.to_string()))?;
        }
        Ok(old)
    }

    /// Rebuild a minimal palette in first-use order, preserving every entry.
    pub fn compact(&mut self) -> io::Result<()> {
        let values = (0..self.entries)
            .map(|i| self.get(i).cloned())
            .collect::<io::Result<Vec<_>>>()?;
        let mut compact = Self::uniform(values[0].clone(), self.entries, self.min_bits)?;
        for (i, value) in values.into_iter().enumerate() {
            compact.set(i, value)?;
        }
        compact.extra = std::mem::take(&mut self.extra);
        *self = compact;
        Ok(())
    }

    /// `PalettedContainer.pack`: the cells re-encoded into a fresh palette in
    /// first-use order, so a slot no cell holds any more is not written and the
    /// bit count is the one for the entries that remain. Saving the palette as
    /// it is would keep a state a block edit replaced and pack wider than
    /// vanilla writes for the same cells.
    pub fn to_nbt(&self) -> Tag {
        let mut map = self.extra.clone();
        let Some(storage) = &self.storage else {
            map.insert(key("palette"), Tag::List(vec![self.palette[0].clone()]));
            return Tag::Compound(map);
        };
        let mut ids = storage.unpack();
        if let Some(aliases) = &self.aliases {
            for id in &mut ids {
                if let Some(&first) = aliases.get(*id as usize) {
                    *id = first;
                }
            }
        }
        let mut palette = Vec::with_capacity(self.palette.len().min(self.entries));
        let mut remap = vec![u32::MAX; self.palette.len()];
        for id in &mut ids {
            let saved = &mut remap[*id as usize];
            if *saved == u32::MAX {
                *saved = palette.len() as u32;
                palette.push(self.palette[*id as usize].clone());
            }
            *id = *saved;
        }
        let bits = bits_for_palette(palette.len(), self.min_bits);
        map.insert(key("palette"), Tag::List(palette));
        if bits > 0 {
            let mut storage = PackedStorage::new(bits, self.entries).unwrap();
            for (index, id) in ids.into_iter().enumerate() {
                storage.set(index, id).unwrap();
            }
            map.insert(
                key("data"),
                Tag::LongArray(storage.words().iter().map(|&v| v as i64).collect()),
            );
        }
        Tag::Compound(map)
    }
}

/// Resolve duplicate imported palette tags once, not once per cell on every save.
fn duplicate_palette_slots(palette: &[Tag]) -> Option<Box<[u32]>> {
    use std::collections::hash_map::Entry;

    let mut seen = HashMap::with_capacity(palette.len());
    let mut aliases: Option<Box<[u32]>> = None;
    for (id, tag) in (0u32..).zip(palette) {
        match seen.entry(tag) {
            Entry::Vacant(slot) => {
                slot.insert(id);
            }
            Entry::Occupied(slot) => {
                let aliases = aliases.get_or_insert_with(|| (0..palette.len() as u32).collect());
                aliases[id as usize] = *slot.get();
            }
        }
    }
    aliases
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockState {
    pub name: String,
    pub properties: BlockProperties,
}
impl BlockState {
    pub fn to_nbt(&self) -> Tag {
        let mut map = Compound::new();
        map.insert(key("id"), Tag::String(key(&self.name)));
        if !self.properties.is_empty() {
            map.insert(
                key("properties"),
                Tag::Compound(
                    self.properties
                        .iter()
                        .map(|(k, v)| (key(k), Tag::String(key(v))))
                        .collect(),
                ),
            );
        }
        Tag::Compound(map)
    }
    pub fn from_nbt(tag: &Tag) -> io::Result<Self> {
        if let Tag::String(name) = tag {
            return Ok(Self {
                name: name.to_string().map_err(|e| invalid(e.to_string()))?,
                properties: BlockProperties::new(),
            });
        }
        let Some(Tag::String(name)) = tag.get("id") else {
            return Err(invalid("block state is missing id"));
        };
        let mut properties = Vec::new();
        if let Some(tag) = tag.get("properties") {
            let Tag::Compound(map) = tag else {
                return Err(invalid("block properties are not a compound"));
            };
            for (k, v) in map {
                let Tag::String(v) = v else {
                    return Err(invalid("block property is not a string"));
                };
                properties.push((
                    k.to_string().map_err(|e| invalid(e.to_string()))?,
                    v.to_string().map_err(|e| invalid(e.to_string()))?,
                ));
            }
        }
        Ok(Self {
            name: name.to_string().map_err(|e| invalid(e.to_string()))?,
            properties: properties.into_iter().collect(),
        })
    }
}

/// `SerializableChunkData.postLoadChunk`: the entities a saved full chunk keeps in its own
/// `entities` list, which `ServerLevel.addLegacyChunkEntities` adds as the chunk loads. Only a
/// chunk last saved before 1.17 gave entities files of their own (`entities/`) has any; saving a
/// full chunk writes no such list.
pub fn legacy_entities(root: &Tag) -> Vec<Tag> {
    match root.get("entities") {
        Some(Tag::List(entities)) => entities.clone(),
        _ => Vec::new(),
    }
}

#[derive(Clone, Debug)]
pub struct ChunkData {
    root: NamedTag,
    position: ChunkPos,
}
impl ChunkData {
    pub fn new(mut root: NamedTag) -> io::Result<Self> {
        // `SerializableChunkData.parse` reads the position with
        // `getIntOr("xPos", 0)` and `getIntOr("zPos", 0)`, so a chunk that does
        // not carry them is the chunk at 0, 0; `Tag.asInt` also takes the
        // narrower integer tags, not only `Int`.
        let position = ChunkPos::new(
            int_or_zero(&root.tag, "xPos"),
            int_or_zero(&root.tag, "zPos"),
        );
        let Some(Tag::Int(version)) = root.tag.get("DataVersion") else {
            return Err(invalid("chunk is missing DataVersion"));
        };
        if *version != DATA_VERSION {
            return Err(invalid(format!(
                "expected DataVersion {DATA_VERSION}, got {version}; data fixing is not implemented"
            )));
        }
        // `SerializableChunkData.parse` reads `getListOrEmpty("sections")`, so a
        // chunk without one is a chunk with no sections. Keep an empty list in
        // the tag: that is what the writer would put there, and every reader
        // below finds it.
        if !matches!(root.tag.get("sections"), Some(Tag::List(_))) {
            let Tag::Compound(map) = &mut root.tag else {
                unreachable!()
            };
            map.insert(key("sections"), Tag::List(Vec::new()));
        }
        let Tag::List(sections) = root.tag.get("sections").unwrap() else {
            unreachable!()
        };
        let mut seen = std::collections::BTreeSet::new();
        for section in sections {
            let Some(Tag::Byte(y)) = section.get("Y") else {
                return Err(invalid("section is missing byte Y"));
            };
            if !seen.insert(*y) {
                return Err(invalid("duplicate section Y"));
            }
        }
        Ok(Self { root, position })
    }
    pub fn position(&self) -> ChunkPos {
        self.position
    }
    pub fn into_nbt(self) -> NamedTag {
        self.root
    }
    pub fn root(&self) -> &NamedTag {
        &self.root
    }

    pub fn saved_ticks(
        &self,
        kind: TickKind,
        resolve: impl FnMut(&str) -> Option<TickTypeId>,
    ) -> io::Result<ChunkTicks> {
        let Some(tag) = self.root.tag.get(kind.field()) else {
            return Ok(ChunkTicks::default());
        };
        let mut ticks = tick_data::read_list(tag, resolve)?;
        ticks.retain(|tick| tick.pos.chunk() == self.position);
        Ok(ChunkTicks::from_saved(ticks))
    }

    pub fn set_saved_ticks(
        &mut self,
        kind: TickKind,
        ticks: &[SavedTick],
        name: impl FnMut(TickTypeId) -> Option<String>,
    ) -> io::Result<()> {
        if ticks.iter().any(|tick| tick.pos.chunk() != self.position) {
            return Err(invalid("saved tick is outside this chunk"));
        }
        let tag = tick_data::write_list(ticks, name)?;
        let Tag::Compound(map) = &mut self.root.tag else {
            unreachable!()
        };
        map.insert(key(kind.field()), tag);
        Ok(())
    }

    fn section_index(&self, block: BlockPos) -> io::Result<Option<usize>> {
        if block.chunk() != self.position {
            return Err(invalid("block is outside this chunk"));
        }
        let section_y = i8::try_from(block.y >> 4)
            .map_err(|_| invalid("block height outside saved section range"))?;
        let Tag::List(sections) = self.root.tag.get("sections").unwrap() else {
            unreachable!()
        };
        Ok(sections
            .iter()
            .position(|s| s.get("Y") == Some(&Tag::Byte(section_y))))
    }

    pub fn block(&self, block: BlockPos) -> io::Result<Option<BlockState>> {
        let Some(index) = self.section_index(block)? else {
            return Ok(None);
        };
        let Tag::List(sections) = self.root.tag.get("sections").unwrap() else {
            unreachable!()
        };
        let Some(container) = sections[index].get("block_states") else {
            return Ok(None);
        };
        let palette = PalettedContainer::from_nbt(container, 4096, 4)?;
        Ok(Some(BlockState::from_nbt(
            palette.get(block.section_index())?,
        )?))
    }

    /// Storage-only mutation. The caller must arrange simulation-derived updates
    /// before treating this as a gameplay block placement.
    pub fn set_block_data(&mut self, block: BlockPos, state: BlockState) -> io::Result<BlockState> {
        let index = self
            .section_index(block)?
            .ok_or_else(|| invalid("section is not present"))?;
        let Tag::List(sections) = self.root.tag.get_mut("sections").unwrap() else {
            unreachable!()
        };
        let container = sections[index]
            .get_mut("block_states")
            .ok_or_else(|| invalid("section has no block-state data"))?;
        let mut palette = PalettedContainer::from_nbt(container, 4096, 4)?;
        let old = BlockState::from_nbt(palette.get(block.section_index())?)?;
        palette.set(block.section_index(), state.to_nbt())?;
        *container = palette.to_nbt();
        Ok(old)
    }
}

/// Resident section: 4,096 blocks stored as numeric palette entries. Uniform
/// sections have no word array; property strings live in the shared registry.
/// Palettes are almost always tiny, so a local id is a scan of that vector
/// (the resident container does the same) with the last lookup cached: a run
/// of one state costs a compare, and no section allocates a hash table.
fn id_in(
    palette: &[BlockStateId],
    counts: &[u16],
    last: &mut Option<(BlockStateId, u32)>,
    state: BlockStateId,
) -> Option<u32> {
    if let Some((seen, id)) = *last
        && seen == state
        && counts[id as usize] != 0
    {
        return Some(id);
    }
    // A dead slot is still on `free`; reviving it without removing it there
    // would let the next new state overwrite it. Only live slots count.
    let id = palette
        .iter()
        .zip(counts)
        .position(|(&v, &count)| count != 0 && v == state)? as u32;
    *last = Some((state, id));
    Some(id)
}
#[cold]
/// `CompoundTag.getIntOr(name, 0)`: the tag as Java's `Tag.asInt` reads it,
/// which is any integer tag, or 0.
fn int_or_zero(tag: &Tag, name: &str) -> i32 {
    match tag.get(name) {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        _ => 0,
    }
}
fn out_of_bounds() -> io::Error {
    invalid("section index out of bounds")
}
pub struct BlockSection<'r> {
    palette: Vec<BlockStateId>,
    last: Option<(BlockStateId, u32)>,
    counts: Vec<u16>,
    free: Vec<u32>,
    storage: Option<PackedStorage>,
    registry: &'r BlockRegistry,
    extra: Compound,
}
impl<'r> BlockSection<'r> {
    /// The section's heap bytes: its packed storage, and its palette, counts
    /// and free list (`horizon-ctl memory`; extra NBT not counted).
    pub fn heap_bytes(&self) -> [usize; 2] {
        [
            self.storage.as_ref().map_or(0, PackedStorage::heap_bytes),
            self.palette.capacity() * std::mem::size_of::<BlockStateId>()
                + self.counts.capacity() * std::mem::size_of::<u16>()
                + self.free.capacity() * std::mem::size_of::<u32>(),
        ]
    }
    /// Bits per block of its packed storage, 0 for a single-state section.
    pub fn storage_bits(&self) -> u8 {
        self.storage.as_ref().map_or(0, PackedStorage::bits)
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn uniform(state: BlockStateId, registry: &'r BlockRegistry) -> io::Result<Self> {
        if registry.state(state).is_none() {
            return Err(invalid("state ID outside registry"));
        }
        Ok(Self {
            palette: vec![state],
            last: Some((state, 0)),
            counts: vec![4096],
            free: Vec::new(),
            storage: None,
            registry,
            extra: Compound::new(),
        })
    }
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        let container = PalettedContainer::from_nbt(tag, 4096, 4)?;
        let resolved = container
            .palette
            .iter()
            .map(|tag| registry.resolve(&BlockState::from_nbt(tag)?))
            .collect::<io::Result<Vec<_>>>()?;
        let first_index = container.storage.as_ref().map_or(0, |s| s.get(0).unwrap()) as usize;
        let mut section = Self::uniform(resolved[first_index], registry)?;
        for index in 0..4096 {
            let id = container
                .storage
                .as_ref()
                .map_or(0, |s| s.get(index).unwrap()) as usize;
            section.set(index, resolved[id])?;
        }
        section.extra = container.extra;
        Ok(section)
    }
    /// Test current distinct states without unpacking all 4,096 cells. The
    /// predicate should be pure; unused palette slots are intentionally skipped.
    pub fn any_state(&self, mut predicate: impl FnMut(BlockStateId) -> bool) -> bool {
        self.palette
            .iter()
            .zip(&self.counts)
            .any(|(&state, &count)| count != 0 && predicate(state))
    }
    pub fn bits(&self) -> u8 {
        self.storage.as_ref().map_or(0, PackedStorage::bits)
    }
    #[inline]
    pub fn get(&self, index: usize) -> io::Result<BlockStateId> {
        if index >= 4096 {
            return Err(out_of_bounds());
        }
        Ok(self.state(index))
    }
    /// Every state in index order, as [`state`](Self::state) reads them one by
    /// one, unpacked word by word.
    pub(crate) fn fill_states(&self, out: &mut [u32; 4096]) {
        match &self.storage {
            None => out.fill(self.palette[0].0),
            Some(storage) => {
                storage.unpack_into(out);
                for v in out.iter_mut() {
                    *v = self.palette[*v as usize].0;
                }
            }
        }
    }
    /// The state at `index`, which must be below 4096 (a `local_index`).
    #[inline]
    pub(crate) fn state(&self, index: usize) -> BlockStateId {
        debug_assert!(index < 4096);
        let id = self.storage.as_ref().map_or(0, |s| s.read(index));
        self.palette[id as usize]
    }
    /// The state of every block, when the section stores only one (no
    /// word array).
    #[inline]
    pub(crate) fn uniform_state(&self) -> Option<BlockStateId> {
        self.storage.is_none().then(|| self.palette[0])
    }
    pub fn set(&mut self, index: usize, state: BlockStateId) -> io::Result<BlockStateId> {
        if index >= 4096 {
            return Err(invalid("section index out of bounds"));
        }
        if state.0 as usize >= self.registry.len() {
            return Err(invalid("state ID outside registry"));
        }
        // A uniform section has one palette slot, all 4,096 blocks and no free
        // slots, so a write either matches it or grows it to two states.
        let Some(storage) = self.storage.as_ref() else {
            debug_assert!(self.palette.len() == 1 && self.counts == [4096] && self.free.is_empty());
            let old = self.palette[0];
            if old == state {
                return Ok(old);
            }
            let bits = bits_for_palette(2, 4);
            let mut storage = PackedStorage::new(bits, 4096).map_err(|e| invalid(e.to_string()))?;
            storage.write(index, 1);
            self.storage = Some(storage);
            self.palette.push(state);
            self.counts[0] = 4095;
            self.counts.push(1);
            self.last = Some((state, 1));
            return Ok(old);
        };
        // The slot at `index` is the old state's id, so no palette lookup is
        // needed for it; only the new state has to be resolved. Reading and
        // writing that slot share one position computation; growing the palette
        // re-packs the storage, which looks the position up again.
        let mut slot = storage.slot_of(index);
        let old_id = storage.read_at(slot);
        let old = self.palette[old_id as usize];
        if old == state {
            return Ok(old);
        }
        let id = match id_in(&self.palette, &self.counts, &mut self.last, state) {
            Some(id) => id,
            None if self.counts[old_id as usize] == 1 => {
                // Replacing the last occurrence can reuse its slot without repacking.
                self.palette[old_id as usize] = state;
                self.last = Some((state, old_id));
                return Ok(old);
            }
            None => {
                if let Some(id) = self.free.pop() {
                    self.palette[id as usize] = state;
                    self.last = Some((state, id));
                    id
                } else {
                    let id = self.palette.len() as u32;
                    let next_bits = bits_for_palette(self.palette.len() + 1, 4);
                    if next_bits != self.bits() {
                        let mut storage = PackedStorage::new(next_bits, 4096)
                            .map_err(|e| invalid(e.to_string()))?;
                        for i in 0..4096 {
                            storage.write(i, self.storage.as_ref().unwrap().read(i));
                        }
                        slot = storage.slot_of(index);
                        self.storage = Some(storage);
                    }
                    self.palette.push(state);
                    self.counts.push(0);
                    self.last = Some((state, id));
                    id
                }
            }
        };
        self.storage.as_mut().unwrap().write_at(slot, id);
        // Both counts are read once and written back once: the two checks below
        // use the values this call produced.
        let old_count = self.counts[old_id as usize] - 1;
        self.counts[old_id as usize] = old_count;
        let new_count = self.counts[id as usize] + 1;
        self.counts[id as usize] = new_count;
        if old_count == 0 {
            self.free.push(old_id);
            if self.last == Some((old, old_id)) {
                self.last = None;
            }
        }
        if new_count == 4096 {
            self.palette = vec![state];
            self.last = Some((state, 0));
            self.counts = vec![4096];
            self.free.clear();
            self.storage = None;
        }
        Ok(old)
    }
    /// Write `len` consecutive values at (`x`, `y_start..y_start+len`, `z`) with
    /// `state`, all inside this section. Resolves the palette id once, then writes
    /// the rest directly; returns how many previous blocks were `air` so the
    /// caller can update its non-air count. The final palette and counts match
    /// `len` calls to `set` in the same top-down order.
    pub fn fill_run(
        &mut self,
        x: i32,
        y_start: i32,
        z: i32,
        len: i32,
        state: BlockStateId,
        air: BlockStateId,
    ) -> io::Result<u16> {
        if !(0..16).contains(&x)
            || !(0..16).contains(&z)
            || len <= 0
            || y_start < 0
            || y_start + len > 16
        {
            return Err(invalid("section fill outside its bounds"));
        }
        let index = |y: i32| ((y as usize) << 8) | ((z as usize) << 4) | x as usize;
        let top = y_start + len - 1;
        let mut added = 0u16;
        // The first write takes the general path: it may add the state to the
        // palette, allocate storage, or find the section already uniform.
        let old = self.set(index(top), state)?;
        if old == air {
            added += 1;
        }
        let Some(id) = id_in(&self.palette, &self.counts, &mut self.last, state) else {
            return Ok(added);
        };
        // Ids are unique among live palette slots, so a cell already holds
        // `state` exactly when it holds `id`, and held `air` exactly when it
        // held `air_id` (written as a sentinel: no slot is that large).
        let air_id = id_in(&self.palette, &self.counts, &mut self.last, air).unwrap_or(u32::MAX);
        let head = index(top);
        // A run in one column moves 256 entries per Y. While a word holds a
        // whole number of those steps (every width generation produces: 1, 2, 4,
        // 8, 16 and 32 bits per entry) the bit shift stays put, so the run walks
        // its words directly. Other widths step the shift as well and go one
        // `set` per block: the palette edits and non-air counts are the same,
        // and the run's state is already in the palette by now.
        const STRIDE: usize = 1 << 8;
        let fixed = match self.storage.as_ref() {
            // No word array: every block already is what the first write wrote.
            None => return Ok(added),
            Some(storage) => storage.fixed_shift(STRIDE),
        };
        if !fixed {
            // A cell that already held the run's state is not counted, so a run
            // of air over air adds nothing here; the top block above is counted
            // either way.
            for y in (y_start..top).rev() {
                if self.set(index(y), state)? == air && state != air {
                    added += 1;
                }
            }
            return Ok(added);
        }
        let counts = &mut self.counts;
        let free = &mut self.free;
        let last = &mut self.last;
        let palette = &self.palette;
        // `counts[id]` is read nowhere in the loop, so its shares are added up.
        let mut gained = 0u16;
        let mut run = self
            .storage
            .as_mut()
            .expect("the section is not uniform")
            .word_run(head, STRIDE, id);
        // Counted down rather than iterated over the Y range: the range's own
        // bookkeeping was a sixth of the loop's instructions.
        let mut left = top - y_start;
        while left > 0 {
            left -= 1;
            let old_id = run.write_back();
            if old_id == id {
                continue;
            }
            gained += 1;
            let old_count = counts[old_id as usize] - 1;
            counts[old_id as usize] = old_count;
            if old_id == air_id {
                added += 1;
            }
            if old_count == 0 {
                free.push(old_id);
                if *last == Some((palette[old_id as usize], old_id)) {
                    *last = None;
                }
            }
        }
        counts[id as usize] += gained;
        if self.counts[id as usize] == 4096 {
            self.palette = vec![state];
            self.last = Some((state, 0));
            self.counts = vec![4096];
            self.free.clear();
            self.storage = None;
        }
        Ok(added)
    }
    pub fn compact(&mut self) -> io::Result<()> {
        let values: Vec<_> = (0..4096).map(|i| self.get(i).unwrap()).collect();
        self.palette.clear();
        self.last = None;
        self.counts.clear();
        self.free.clear();
        for state in &values {
            if let Some(id) = id_in(&self.palette, &self.counts, &mut self.last, *state) {
                self.counts[id as usize] += 1;
            } else {
                self.palette.push(*state);
                self.counts.push(1);
                self.last = Some((*state, self.palette.len() as u32 - 1));
            }
        }
        let bits = bits_for_palette(self.palette.len(), 4);
        self.storage = if bits == 0 {
            None
        } else {
            let mut storage = PackedStorage::new(bits, 4096).map_err(|e| invalid(e.to_string()))?;
            let mut last = None;
            for (i, state) in values.iter().enumerate() {
                let id = id_in(&self.palette, &self.counts, &mut last, *state).unwrap();
                storage.set(i, id).unwrap();
            }
            Some(storage)
        };
        Ok(())
    }
    pub fn to_nbt(&self) -> io::Result<Tag> {
        let mut map = self.extra.clone();
        let palette = self
            .palette
            .iter()
            .map(|id| {
                self.registry
                    .state(*id)
                    .map(BlockState::to_nbt)
                    .ok_or_else(|| invalid("state ID outside registry"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        map.insert(key("palette"), Tag::List(palette));
        if let Some(storage) = &self.storage {
            map.insert(
                key("data"),
                Tag::LongArray(storage.words().iter().map(|&w| w as i64).collect()),
            );
        }
        Ok(Tag::Compound(map))
    }
}

#[cfg(test)]
mod tests;
