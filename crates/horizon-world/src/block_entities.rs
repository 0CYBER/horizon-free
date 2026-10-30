//! Proto-chunk block-entity storage. Active payloads are caller-owned runtime
//! types; this module does not create entities or execute their behavior.
use crate::{compression::invalid, nbt::Tag};
use horizon_core::position::{BlockPos, ChunkPos};
use std::{
    collections::{HashMap, HashSet},
    io,
};

/// Resolve native block-entity coordinates, relocating mismatched X/Z into the
/// owning chunk while retaining local offsets and the full Y coordinate. The
/// supplied tag is not rewritten. NumericTag float/double reads use Mth.floor.
pub fn position_from_tag(chunk: ChunkPos, tag: &Tag) -> io::Result<BlockPos> {
    if !matches!(tag, Tag::Compound(_)) {
        return Err(invalid("block entity is not a compound"));
    }
    let read = |name| match tag.get(name) {
        Some(Tag::Byte(v)) => i32::from(*v),
        Some(Tag::Short(v)) => i32::from(*v),
        Some(Tag::Int(v)) => *v,
        Some(Tag::Long(v)) => *v as i32,
        Some(Tag::Float(bits)) => f64::from(f32::from_bits(*bits)).floor() as i32,
        Some(Tag::Double(bits)) => f64::from_bits(*bits).floor() as i32,
        _ => 0,
    };
    let (mut x, y, mut z) = (read("x"), read("y"), read("z"));
    if x >> 4 != chunk.x || z >> 4 != chunk.z {
        x = chunk.x.wrapping_shl(4).wrapping_add(x & 15);
        z = chunk.z.wrapping_shl(4).wrapping_add(z & 15);
    }
    Ok(BlockPos::new(x, y, z))
}

pub struct ProtoBlockEntities<E> {
    position: ChunkPos,
    pending: HashMap<BlockPos, Tag>,
    active: HashMap<BlockPos, E>,
}
impl<E> ProtoBlockEntities<E> {
    pub fn new(position: ChunkPos) -> Self {
        Self {
            position,
            pending: HashMap::new(),
            active: HashMap::new(),
        }
    }
    /// Ignore a pending record when an active entity already owns its resolved
    /// position. Inserting an active entity removes any earlier pending record.
    pub fn set_pending(&mut self, tag: Tag) -> io::Result<()> {
        let position = position_from_tag(self.position, &tag)?;
        if !self.active.contains_key(&position) {
            self.pending.insert(position, tag);
        }
        Ok(())
    }
    /// Position must be the payload's immutable world position, as supplied by
    /// the runtime's block-entity constructor. No chunk/build-height gate applies.
    pub fn set_active(&mut self, position: BlockPos, entity: E) -> Option<E> {
        self.pending.remove(&position);
        self.active.insert(position, entity)
    }
    pub fn active(&self, position: BlockPos) -> Option<&E> {
        self.active.get(&position)
    }
    pub fn pending(&self, position: BlockPos) -> Option<&Tag> {
        self.pending.get(&position)
    }
    /// Mutating coordinate fields does not re-key an existing pending entry.
    pub fn pending_mut(&mut self, position: BlockPos) -> Option<&mut Tag> {
        self.pending.get_mut(&position)
    }
    pub fn pending_entries(&self) -> &HashMap<BlockPos, Tag> {
        &self.pending
    }
    pub fn positions(&self) -> HashSet<BlockPos> {
        self.pending
            .keys()
            .chain(self.active.keys())
            .copied()
            .collect()
    }
    pub fn remove(&mut self, position: BlockPos) -> Option<E> {
        self.pending.remove(&position);
        self.active.remove(&position)
    }
    /// Serialize an active entity when present, otherwise clone its pending tag.
    /// The caller supplies runtime serialization; this does not instantiate NBT.
    pub fn for_saving<T>(
        &self,
        position: BlockPos,
        serialize: impl FnOnce(&E) -> Result<Tag, T>,
    ) -> Result<Option<Tag>, T> {
        if let Some(entity) = self.active.get(&position) {
            serialize(entity).map(Some)
        } else {
            Ok(self.pending.get(&position).cloned())
        }
    }
}
