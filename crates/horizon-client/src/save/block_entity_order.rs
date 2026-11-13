//! A saved block entity's place in its chunk's load order.
use super::{Tag, block_entity_pos, number};

/// `BlockEntity.getPosFromTag`: the saved `x`, `y`, `z`, moved into `chunk` when the tag names
/// another chunk, so the load order lists the block entity where it loads.
pub(super) fn saved_at(e: &Tag, chunk: [i32; 2]) -> Option<(i32, i32, i32)> {
    let int = |k: &str| number(e.get(k)).map(|v| v as i32);
    Some(block_entity_pos((int("x")?, int("y")?, int("z")?), chunk))
}
