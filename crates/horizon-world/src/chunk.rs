//! Saved section palettes and storage-level chunk access. Changes here do not
//! run block behavior, lighting, heightmaps, neighbor updates or scheduled ticks.
use crate::{
    DATA_VERSION,
    compression::invalid,
    nbt::{Compound, JavaString, NamedTag, Tag},
    registry::{BlockRegistry, BlockStateId},
    tick_data::{self, TickKind},
};
use horizon_core::{
    position::{BlockPos, ChunkPos},
    storage::PackedStorage,
    ticks::{ChunkTicks, SavedTick, TickTypeId},
};
use std::{collections::BTreeMap, io};

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

#[derive(Clone, Debug)]
pub struct PalettedContainer {
    palette: Vec<Tag>,
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

    pub fn to_nbt(&self) -> Tag {
        let mut map = self.extra.clone();
        map.insert(key("palette"), Tag::List(self.palette.clone()));
        if let Some(storage) = &self.storage {
            map.insert(
                key("data"),
                Tag::LongArray(storage.words().iter().map(|&v| v as i64).collect()),
            );
        }
        Tag::Compound(map)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BlockState {
    pub name: String,
    pub properties: BTreeMap<String, String>,
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
                properties: BTreeMap::new(),
            });
        }
        let Some(Tag::String(name)) = tag.get("id") else {
            return Err(invalid("block state is missing id"));
        };
        let mut properties = BTreeMap::new();
        if let Some(tag) = tag.get("properties") {
            let Tag::Compound(map) = tag else {
                return Err(invalid("block properties are not a compound"));
            };
            for (k, v) in map {
                let Tag::String(v) = v else {
                    return Err(invalid("block property is not a string"));
                };
                properties.insert(
                    k.to_string().map_err(|e| invalid(e.to_string()))?,
                    v.to_string().map_err(|e| invalid(e.to_string()))?,
                );
            }
        }
        Ok(Self {
            name: name.to_string().map_err(|e| invalid(e.to_string()))?,
            properties,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ChunkData {
    root: NamedTag,
    position: ChunkPos,
}
impl ChunkData {
    pub fn new(root: NamedTag) -> io::Result<Self> {
        let Some(Tag::Int(x)) = root.tag.get("xPos") else {
            return Err(invalid("chunk is missing xPos"));
        };
        let Some(Tag::Int(z)) = root.tag.get("zPos") else {
            return Err(invalid("chunk is missing zPos"));
        };
        let position = ChunkPos::new(*x, *z);
        let Some(Tag::Int(version)) = root.tag.get("DataVersion") else {
            return Err(invalid("chunk is missing DataVersion"));
        };
        if *version != DATA_VERSION {
            return Err(invalid(format!(
                "expected DataVersion {DATA_VERSION}, got {version}; data fixing is not implemented"
            )));
        }
        let Some(Tag::List(sections)) = root.tag.get("sections") else {
            return Err(invalid("chunk is missing sections"));
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

    pub fn load_block_section<'r>(
        &self,
        block: BlockPos,
        registry: &'r BlockRegistry,
    ) -> io::Result<Option<BlockSection<'r>>> {
        let Some(index) = self.section_index(block)? else {
            return Ok(None);
        };
        let Tag::List(sections) = self.root.tag.get("sections").unwrap() else {
            unreachable!()
        };
        sections[index]
            .get("block_states")
            .map(|tag| BlockSection::from_nbt(tag, registry))
            .transpose()
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
/// Multiplicative hash for dense state IDs; section palettes are hashed on
/// every block write, where SipHash dominated generation profiles.
#[derive(Default)]
struct StateIdHasher(u64);
impl std::hash::Hasher for StateIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        }
    }
    fn write_u32(&mut self, value: u32) {
        self.0 = (self.0 ^ u64::from(value)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}
type LocalIds =
    std::collections::HashMap<BlockStateId, u32, std::hash::BuildHasherDefault<StateIdHasher>>;
fn local_ids(state: BlockStateId) -> LocalIds {
    let mut ids = LocalIds::default();
    ids.insert(state, 0);
    ids
}
pub struct BlockSection<'r> {
    palette: Vec<BlockStateId>,
    local_ids: LocalIds,
    counts: Vec<u16>,
    free: Vec<u32>,
    storage: Option<PackedStorage>,
    registry: &'r BlockRegistry,
    extra: Compound,
}
impl<'r> BlockSection<'r> {
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
    pub fn uniform(state: BlockStateId, registry: &'r BlockRegistry) -> io::Result<Self> {
        if registry.state(state).is_none() {
            return Err(invalid("state ID outside registry"));
        }
        Ok(Self {
            palette: vec![state],
            local_ids: local_ids(state),
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
    pub fn get(&self, index: usize) -> io::Result<BlockStateId> {
        if index >= 4096 {
            return Err(invalid("section index out of bounds"));
        }
        let id = self.storage.as_ref().map_or(0, |s| s.get(index).unwrap());
        Ok(self.palette[id as usize])
    }
    pub fn set(&mut self, index: usize, state: BlockStateId) -> io::Result<BlockStateId> {
        let old = self.get(index)?;
        if state.0 as usize >= self.registry.len() {
            return Err(invalid("state ID outside registry"));
        }
        if old == state {
            return Ok(old);
        }
        let old_id = self.local_ids[&old];
        if self.counts[old_id as usize] == 1 && !self.local_ids.contains_key(&state) {
            // Replacing the last occurrence can reuse its slot without repacking.
            self.palette[old_id as usize] = state;
            self.local_ids.remove(&old);
            self.local_ids.insert(state, old_id);
            return Ok(old);
        }
        let id = if let Some(id) = self.local_ids.get(&state) {
            *id
        } else if let Some(id) = self.free.pop() {
            self.palette[id as usize] = state;
            self.local_ids.insert(state, id);
            id
        } else {
            let id = self.palette.len() as u32;
            let next_bits = bits_for_palette(self.palette.len() + 1, 4);
            if next_bits != self.bits() {
                let mut storage =
                    PackedStorage::new(next_bits, 4096).map_err(|e| invalid(e.to_string()))?;
                if let Some(old) = &self.storage {
                    for i in 0..4096 {
                        storage.set(i, old.get(i).unwrap()).unwrap();
                    }
                }
                self.storage = Some(storage);
            }
            self.palette.push(state);
            self.counts.push(0);
            self.local_ids.insert(state, id);
            id
        };
        self.storage
            .as_mut()
            .unwrap()
            .set(index, id)
            .map_err(|e| invalid(e.to_string()))?;
        self.counts[old_id as usize] -= 1;
        self.counts[id as usize] += 1;
        if self.counts[old_id as usize] == 0 {
            self.local_ids.remove(&old);
            self.free.push(old_id);
        }
        if self.counts[id as usize] == 4096 {
            self.palette = vec![state];
            self.local_ids = local_ids(state);
            self.counts = vec![4096];
            self.free.clear();
            self.storage = None;
        }
        Ok(old)
    }
    pub fn compact(&mut self) -> io::Result<()> {
        let values: Vec<_> = (0..4096).map(|i| self.get(i).unwrap()).collect();
        self.palette.clear();
        self.local_ids.clear();
        self.counts.clear();
        self.free.clear();
        for state in &values {
            if !self.local_ids.contains_key(state) {
                self.local_ids.insert(*state, self.palette.len() as u32);
                self.palette.push(*state);
                self.counts.push(0);
            }
            self.counts[self.local_ids[state] as usize] += 1;
        }
        let bits = bits_for_palette(self.palette.len(), 4);
        self.storage = if bits == 0 {
            None
        } else {
            let mut storage = PackedStorage::new(bits, 4096).map_err(|e| invalid(e.to_string()))?;
            for (i, state) in values.iter().enumerate() {
                storage.set(i, self.local_ids[state]).unwrap();
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
mod tests {
    use super::*;
    #[test]
    fn palette_grows_across_word_widths_and_compacts_to_uniform() {
        let mut palette = PalettedContainer::uniform(Tag::Int(0), 4096, 4).unwrap();
        for i in 0..4096 {
            palette.set(i, Tag::Int(i as i32)).unwrap();
        }
        assert_eq!(palette.bits(), 12);
        let mut loaded = PalettedContainer::from_nbt(&palette.to_nbt(), 4096, 4).unwrap();
        for i in 0..4096 {
            assert_eq!(loaded.get(i).unwrap(), &Tag::Int(i as i32));
            loaded.set(i, Tag::Int(7)).unwrap();
        }
        loaded.compact().unwrap();
        assert_eq!(loaded.bits(), 0);
        assert_eq!(loaded.palette_len(), 1);
        assert!(loaded.to_nbt().get("data").is_none());
        assert_eq!(loaded.get(4095).unwrap(), &Tag::Int(7));
    }
    #[test]
    fn invalid_palette_data_fails_before_access() {
        let mut tag = Tag::compound([("palette", Tag::List(vec![Tag::Int(0), Tag::Int(1)]))]);
        assert!(PalettedContainer::from_nbt(&tag, 4096, 4).is_err());
        let Tag::Compound(map) = &mut tag else {
            unreachable!()
        };
        map.insert(key("data"), Tag::LongArray(vec![-1; 256]));
        assert!(PalettedContainer::from_nbt(&tag, 4096, 4).is_err());
    }
}
