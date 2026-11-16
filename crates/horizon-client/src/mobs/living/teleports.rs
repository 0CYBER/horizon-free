//! A mob's position writes inside its tick that are not its walk: `Entity.teleportTo`
//! (`snapTo`: an enderman's, a pet's to its owner, a cat's morning gift, a shulker's, a dismount)
//! and `setPos` (a villager laid in its bed or standing up from it, a size change's nudge). Each
//! is one position update of its own (`setPosRaw`, then `EntityInLevelCallback.onMove`), which
//! `entity_entering_section` tells, so a mob that teleports twice and then walks has made three.
//! Mob code moves the mob with [`snap_to`] or [`set_pos`]; each write is captured by the
//! thread-local section-write collector while the game ticks mobs with the hook listened to.

use super::super::Mob;
use horizon_core::geometry::Vec3;

/// `snapTo`: the position and the old position (`xo`) move together, and the motion stays.
pub fn snap_to(m: &mut Mob, at: Vec3) {
    snap_to_with(m, at, None);
}

/// `snapTo` with a live section observer supplied by the owning mob tick.
pub fn snap_to_with(m: &mut Mob, at: Vec3, observer: Option<super::section_writes::Observer>) {
    m.previous = at;
    set_pos_with(m, at, observer);
}

/// `setPos`: the position moves, the old position (`xo`) stays.
pub fn set_pos(m: &mut Mob, at: Vec3) {
    set_pos_with(m, at, None);
}

/// `setPos` with a live section observer supplied by the owning mob tick.
pub fn set_pos_with(m: &mut Mob, at: Vec3, observer: Option<super::section_writes::Observer>) {
    let from = m.position;
    m.position = at;
    super::section_writes::record_with(m.id, m.join, from, at, observer);
}
