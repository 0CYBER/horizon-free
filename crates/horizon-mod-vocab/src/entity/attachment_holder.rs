//! Scoped holder identities shared by attachment lifecycle and query batches.

use crate::{BlockEntityRef, ChunkRef, EntityRef, LevelRef, PlayerRef};

/// A holder's existing scoped handle, without retaining or loading the holder.
///
/// Unlike [`crate::Holder`], these identities are not relative to whichever level a
/// host currently has in place. Keep the ordinary handle rules when using a retained
/// identity: a removed entity, unloaded chunk or closed world may no longer resolve.
/// Copying the identity never extends the holder's lifetime.
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AttachmentHolder {
    /// An entity's attachment holder.
    Entity(EntityRef),
    /// A player, keeping both their UUID and their existing entity handle.
    Player(PlayerRef),
    /// A block entity in its level.
    BlockEntity(BlockEntityRef),
    /// A chunk in its level.
    Chunk(ChunkRef),
    /// A level of the open world.
    Level(LevelRef),
}
