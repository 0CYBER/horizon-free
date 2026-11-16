use super::Game;

/// A block entity's `mark_motion` id: its position, packed into the 32 bits beside the tag
/// (x and z wrap at 2048 blocks and y at 1024; two block entities that far apart in one frame's
/// view are told apart by their vertex counts, as `PackEntities` matches only equal runs).
pub(super) fn motion_block_entity(pos: [i32; 3]) -> u64 {
    motion_at(Game::MOTION_BLOCK_ENTITY, pos)
}

/// `tag` with a block position packed into its low 32 bits (`motion_block_entity`).
pub(super) fn motion_at(tag: u64, pos: [i32; 3]) -> u64 {
    let [x, y, z] = pos.map(|c| c as u32);
    tag | u64::from((x & 0x7FF) | (z & 0x7FF) << 11 | (y & 0x3FF) << 22)
}
