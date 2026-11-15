//! `entity_entering_section` (`EntityEvent.EnteringSection`): vanilla tells it from
//! `Entity.setPosRaw`, whose `EntityInLevelCallback.onMove` (`PersistentEntitySectionManager`'s
//! callback on a server, `TransientEntitySectionManager`'s on a client) compares the section the
//! entity is filed under with the one its new position is in, and moves it at once when they
//! differ. Horizon's kinds keep their positions in their own fields, so the hook is told from
//! the places that write them: [`EntitySections`] files each entity under its section, and each
//! position-write boundary hands it the entity's new position immediately after the write.
//!
//! The boundaries: an entity joining (the server's `native_joins_flush`, a client's
//! `AddEntity`) files it untold. The mob and item loops report each committed write while their
//! rows are resident again; minecarts, boats and the remaining server entities report after each
//! entity's own update. A client reports interpolation writes per entity. A teleport, a piston
//! push, a mod's position write, each move a server player sent and each packet that moves an
//! entity on a client are one each.
//! Every boundary that sees another section records a crossing from the section the entity was
//! filed under and tells it before the write returns, so two moves told apart are two crossings,
//! and a move out and back is two.
//!
//! A listener runs synchronously after the new section is filed. What it moves or removes is
//! deferred by the surrounding entity write and is never re-entered from its own call; listeners
//! should queue such work themselves until the write completes.
//!
//! With no listener each boundary is one subscription check and nothing is filed. With one, a
//! boundary is a hash lookup and the reusable crossing record is told without a per-frame scan.

#[path = "entity_sections/observer_cache.rs"]
pub(super) mod observer_cache;

use super::Game;
use crate::mobs::living::section_writes::PositionWrite;
use mod_api::__private::game::BackendRefOf;
use mod_api::{DimensionKey, EntityRef, WorldEpoch};
use std::collections::HashMap;
use std::collections::hash_map::Entry;

#[cfg(test)]
#[path = "entity_sections/tests.rs"]
mod tests;

/// `SectionPos.blockToSectionCoord(Mth.floor(v))`: `Mth.floor` truncates (saturating, NaN to 0,
/// as Java's cast does) and steps down for a negative fraction with Java's wrapping `int`.
fn section_coord(v: f64) -> i32 {
    let truncated = v as i32;
    let block = if v < f64::from(truncated) {
        truncated.wrapping_sub(1)
    } else {
        truncated
    };
    block >> 4
}

fn section_of([x, y, z]: [f64; 3]) -> [i32; 3] {
    [section_coord(x), section_coord(y), section_coord(z)]
}

/// A section change a boundary found: the entity, its old and its new section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Crossing {
    pub(super) id: u32,
    pub(super) old: [i32; 3],
    pub(super) new: [i32; 3],
}

/// Where a boundary is: the world open and the level the entity is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Scope {
    epoch: WorldEpoch,
    level: DimensionKey,
}

/// The section each entity is filed under, and the crossings not told yet.
#[derive(Default)]
pub(super) struct EntitySections {
    /// The world open the table was filled in: another one starts it over.
    epoch: Option<WorldEpoch>,
    /// Each entity's level and section.
    filed: HashMap<u32, (DimensionKey, [i32; 3])>,
    /// Crossings raised while a listener is already running. Ordinary writes are told
    /// immediately; a nested write waits until the current callback returns.
    queue: Vec<Crossing>,
    /// The batch being told; kept for its capacity.
    telling: Vec<Crossing>,
    /// A telling is under way: a listener's moves wait for the next one.
    busy: bool,
    /// The mobs' position updates during their family's tick; kept for its capacity.
    pub(super) ticked: Vec<PositionWrite>,
    /// A projectile row is lent during its individual tick. Position reads during its write
    /// boundary use this value until the row is restored.
    pub(super) write_position: Option<(u32, [f64; 3])>,
    /// Projectile ids seen by the current pass; kept to forget rows removed during settlement.
    pub(super) projectile_ids: Vec<u32>,
    /// Projectile ids resident after settlement; kept to avoid a fresh set per pass.
    pub(super) projectile_live: horizon_client::fast_hash::FxHashSet<u32>,
}

impl EntitySections {
    /// The table for `scope`'s world: another world's entries are dropped.
    fn table(&mut self, scope: Scope) -> &mut HashMap<u32, (DimensionKey, [i32; 3])> {
        if self.epoch != Some(scope.epoch) {
            self.epoch = Some(scope.epoch);
            self.filed.clear();
            self.queue.clear();
        }
        &mut self.filed
    }

    /// `addEntity`: entity `id` is filed at `at`, untold. A new entity under an id seen before
    /// (a client's `AddEntity` replacing one) starts over too.
    pub(super) fn placed(&mut self, scope: Scope, id: u32, at: [f64; 3]) {
        self.table(scope).insert(id, (scope.level, section_of(at)));
    }

    /// `setPosRaw`: entity `id` is at `to` now. Filed under another section of the same level,
    /// that is a crossing. One not filed in this level yet (it changed level, or its join is
    /// still to be told) is filed where it was, `from` when the boundary knows it, so a move
    /// before its join is told is a crossing too; otherwise it is filed untold.
    pub(super) fn moved(&mut self, scope: Scope, id: u32, from: Option<[f64; 3]>, to: [f64; 3]) {
        let new = section_of(to);
        let old = match self.table(scope).entry(id) {
            Entry::Occupied(mut entry) => {
                let (level, section) = entry.get_mut();
                if *level != scope.level {
                    *level = scope.level;
                    *section = new;
                    return;
                }
                std::mem::replace(section, new)
            }
            Entry::Vacant(entry) => {
                entry.insert((scope.level, new));
                match from {
                    Some(from) => section_of(from),
                    None => return,
                }
            }
        };
        if old != new {
            self.queue.push(Crossing { id, old, new });
        }
    }

    /// `id` is gone (removed, or respawned as a new entity under the same id): it is filed
    /// again where it next appears, untold.
    pub(super) fn forget(&mut self, id: u32) {
        self.filed.remove(&id);
        // A nested listener may have queued a crossing immediately before it
        // removed this entity.  Do not deliver that stale write on the next
        // boundary (the hook never fires for an entity being removed).
        self.queue.retain(|crossing| crossing.id != id);
    }

    /// The crossings to tell now, or none while a telling is under way.
    fn begin_telling(&mut self) -> Option<Vec<Crossing>> {
        if self.busy || self.queue.is_empty() {
            return None;
        }
        self.busy = true;
        let mut batch = std::mem::take(&mut self.telling);
        std::mem::swap(&mut batch, &mut self.queue);
        Some(batch)
    }

    /// The telling is over; `batch` goes back for its capacity.
    fn end_telling(&mut self, mut batch: Vec<Crossing>) {
        batch.clear();
        self.telling = batch;
        self.busy = false;
    }
}

/// The lightweight host used while a mob's world view is borrowed.  Entity
/// reads are answered from the write that is currently being delivered; all
/// other host operations retain their normal typed refusal instead of trying
/// to borrow the game through the live mob view.
struct LiveSectionHost {
    epoch: WorldEpoch,
    current: Option<(u32, [f64; 3])>,
}

mod_api::impl_host_areas!(LiveSectionHost);
mod_api::impl_server_host_areas!(LiveSectionHost);
mod_api::impl_backend_areas!(LiveSectionHost, except(entities));

impl mod_api::prelude::EntitiesBackend for LiveSectionHost {
    fn entity_position(&self, entity: EntityRef) -> Option<[f64; 3]> {
        (entity.epoch() == self.epoch)
            .then_some(self.current)
            .flatten()
            .and_then(|(id, position)| (id == entity.id()).then_some(position))
    }
}

impl mod_api::Backend for LiveSectionHost {}

impl mod_api::Host for LiveSectionHost {
    fn backend(&self) -> Option<mod_api::BackendRef<'_>> {
        Some(mod_api::BackendRef::of(self))
    }
}

impl mod_api::ServerHost for LiveSectionHost {}

/// Delivers a mob's captured teleport at the write boundary.  The observer
/// owns only the section table, so it can run while the mob's borrowed world
/// view is still live.
pub(crate) struct LiveSectionObserver {
    sections: std::rc::Rc<std::cell::RefCell<EntitySections>>,
    scope: Scope,
    current: Option<(u32, [f64; 3])>,
}

impl LiveSectionObserver {
    pub(crate) fn new(
        sections: std::rc::Rc<std::cell::RefCell<EntitySections>>,
        scope: Scope,
    ) -> Self {
        Self {
            sections,
            scope,
            current: None,
        }
    }

    fn tell(&mut self) {
        let Some(mut batch) = self.sections.borrow_mut().begin_telling() else {
            return;
        };
        let mut host = LiveSectionHost {
            epoch: self.scope.epoch,
            current: self.current,
        };
        for crossing in &batch {
            host.current = self.current.filter(|(id, _)| *id == crossing.id);
            let chunk_changed =
                crossing.old[0] != crossing.new[0] || crossing.old[2] != crossing.new[2];
            mod_api::dispatch_entity_entering_section(
                mod_api::HostSide::Server(&mut host),
                EntityRef::new(self.scope.epoch, crossing.id),
                crossing.old,
                crossing.new,
                chunk_changed,
            );
        }
        batch.clear();
        let mut sections = self.sections.borrow_mut();
        sections.telling = batch;
        sections.busy = false;
    }
}

impl crate::mobs::living::section_writes::WriteObserver for LiveSectionObserver {
    fn position_written(
        &mut self,
        id: u32,
        from: horizon_core::geometry::Vec3,
        to: horizon_core::geometry::Vec3,
    ) {
        self.current = Some((id, [to.x, to.y, to.z]));
        self.sections.borrow_mut().moved(
            self.scope,
            id,
            Some([from.x, from.y, from.z]),
            [to.x, to.y, to.z],
        );
        self.tell();
        self.current = None;
    }
}

impl Game<'_> {
    /// Where boundaries are now: None with no listener (nothing is filed) or no world open.
    pub(super) fn entity_section_scope(&self) -> Option<Scope> {
        if !mod_api::__private::listened(mod_api::HookId::EntityEnteringSection) {
            return None;
        }
        Some(Scope {
            epoch: self.native_lifecycle.epoch()?,
            level: self.native_level_key(),
        })
    }

    /// Entity `id` was put from `from` (when known) at `to`; tell it immediately after filing.
    pub(super) fn entity_section_moved(&mut self, id: u32, from: Option<[f64; 3]>, to: [f64; 3]) {
        if let Some(scope) = self.entity_section_scope() {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, from, to);
            self.tell_entity_sections();
        }
    }

    /// A server player's moves this tick (`moves`: each one's delta, in order), which left it at
    /// `end`: each is one `setPos`, so a move out of a section and one back are two crossings.
    pub(super) fn entity_section_walked(
        &mut self,
        id: u32,
        end: [f64; 3],
        moves: impl Iterator<Item = [f64; 3]> + Clone,
    ) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let mut from = moves
            .clone()
            .fold(end, |p, d| [p[0] - d[0], p[1] - d[1], p[2] - d[2]]);
        for d in moves {
            let to = [from[0] + d[0], from[1] + d[1], from[2] + d[2]];
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, Some(from), to);
            self.tell_entity_sections();
            from = to;
        }
    }

    /// Entity `id`'s position was written: it is wherever it is now (any kind, either side).
    pub(super) fn entity_section_moved_id(&mut self, id: u32) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id)) {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, None, at);
            self.tell_entity_sections();
        }
    }

    /// Entity `id` is gone, or replaced under the same id: it starts over untold.
    pub(super) fn entity_section_forget(&mut self, id: u32) {
        self.native_lifecycle.entity_sections.forget(id);
        self.native_lifecycle
            .entity_sections
            .projectile_live
            .remove(&id);
    }

    /// Tell the writes from one lifted mob while its owning list is resident.
    ///
    /// A mob tick cannot borrow `Game` while its world view is live, so the
    /// caller restores the pass before entering here. The resident row is
    /// temporarily moved to each recorded write while its crossing is told;
    /// this gives listeners the same position they would see at the write and
    /// preserves a listener's own position change.
    pub(super) fn entity_sections_ticked_live(
        &mut self,
        sections: &mut EntitySections,
        moves: &mut Vec<PositionWrite>,
    ) {
        let Some(scope) = self.entity_section_scope() else {
            moves.clear();
            return;
        };
        let mut indices = horizon_client::fast_hash::FxHashMap::default();
        indices.extend(
            self.mobs
                .iter()
                .enumerate()
                .map(|(index, mob)| (mob.join, index)),
        );
        for write in moves.drain(..) {
            if write.delivered {
                continue;
            }
            let Some(&index) = indices.get(&write.join) else {
                sections.forget(write.id);
                continue;
            };
            let final_position = self.mobs[index].position;
            self.mobs.update(index, |mob| mob.position = write.to);
            sections.moved(
                scope,
                write.id,
                write.from.map(|from| [from.x, from.y, from.z]),
                [write.to.x, write.to.y, write.to.z],
            );
            self.tell_entity_sections_in(sections);
            indices.clear();
            indices.extend(
                self.mobs
                    .iter()
                    .enumerate()
                    .map(|(index, mob)| (mob.join, index)),
            );
            if let Some(&index) = indices.get(&write.join) {
                self.mobs.update(index, |mob| {
                    if mob.position == write.to {
                        mob.position = final_position;
                    }
                });
            }
        }
    }

    /// Entity `id`'s position was written from `from` to `to` (`setPosRaw`) inside a family tick
    /// or while a rider is carried by a vehicle teleport. File and tell it at this write.
    pub(super) fn entity_section_step(&mut self, id: u32, from: [f64; 3], to: [f64; 3]) {
        if id != 0
            && let Some(scope) = self.entity_section_scope()
        {
            self.native_lifecycle
                .entity_sections
                .moved(scope, id, Some(from), to);
            self.tell_entity_sections();
        }
    }

    /// Tell a write whose row is temporarily lent to a family pass. The callback sees the
    /// committed position without removing and reinserting the surrounding list tail.
    pub(super) fn entity_section_step_visible(
        &mut self,
        id: u32,
        from: [f64; 3],
        to: [f64; 3],
    ) -> bool {
        let scope = self.entity_section_scope();
        self.native_lifecycle.entity_sections.write_position = Some((id, to));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.entity_section_step(id, from, to);
        }));
        self.native_lifecycle.entity_sections.write_position = None;
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
        scope.is_none_or(|_| {
            self.native_lifecycle
                .entity_sections
                .filed
                .contains_key(&id)
        })
    }

    /// `Entity.teleport` placed as `/tp` places it ([`Game::place_teleport_in_level`]), then filed
    /// and told as one position update (`/tp`, a mob's ender pearl). Its old position, as the
    /// placement returns it.
    pub(super) fn place_teleport_filed(
        &mut self,
        t: super::commands::Target,
        pos: [f64; 3],
        rotation: Option<[f32; 2]>,
        relative: [bool; 3],
        turn_riders: bool,
    ) -> Option<[f64; 3]> {
        let from = self.place_teleport_in_level(t, pos, rotation, relative, turn_riders);
        if let (Some(entity), Some(to)) = (self.network_entity_id(t), self.target_position(t)) {
            self.entity_section_moved(entity as u32, from, to);
        }
        from
    }

    /// An entity joined (`addEntity`): filed untold where it is now, which a join listener may
    /// have changed from `at`, where it joined.
    pub(super) fn entity_section_joined(&mut self, id: u32, at: [f64; 3]) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let now = self.entity_position(EntityRef::new(scope.epoch, id));
        self.native_lifecycle
            .entity_sections
            .placed(scope, id, now.unwrap_or(at));
    }

    /// The projectiles tick ([`Game::tick_projectiles_now`]), and each one's move is filed.
    pub(super) fn tick_projectiles(&mut self) -> super::Result<()> {
        self.tick_projectiles_now()
    }

    /// A client's entities after its tick: each one where its interpolation or its own move
    /// put it (its live position, not the packet's), and this window's player.
    pub(super) fn client_entity_sections_stepped(&mut self) {
        let Some(scope) = self.entity_section_scope() else {
            return;
        };
        let own = self.own_entity_id();
        let mut sections = std::mem::take(&mut self.native_lifecycle.entity_sections);
        if let Some(r) = self.remote.as_ref() {
            for &id in r.entities.keys().filter(|&&id| id != own && id > 0) {
                if let Some(at) = self.entity_position(EntityRef::new(scope.epoch, id as u32)) {
                    sections.moved(scope, id as u32, None, at);
                }
            }
        }
        if let Some(player) = self.player.as_ref()
            && own > 0
        {
            let at = player.position;
            sections.moved(scope, own as u32, None, [at.x, at.y, at.z]);
        }
        self.native_lifecycle.entity_sections = sections;
        self.tell_entity_sections();
    }

    /// A client handles one packet from its server ([`Game::apply_packet`]). A packet that adds,
    /// moves or removes an entity is a position-update boundary: `handleAddEntity` files the
    /// entity untold, a move (`handleMoveEntity`, `handleEntityPositionSync`,
    /// `handleTeleportEntity`, `handleMovePlayer`'s `PlayerPosition`, a vehicle's) is told when
    /// it put the entity in another section, and `handleRespawn`'s new player starts over.
    /// An entity a move only starts interpolating moves at the client's entity tick.
    pub(in crate::game) fn handle_packet(
        &mut self,
        packet: horizon_protocol::packets::play::Clientbound,
    ) -> super::Result<()> {
        use horizon_protocol::packets::play::Clientbound as C;
        if self.entity_section_scope().is_none() {
            return self.apply_packet(packet);
        }
        if let C::RemoveEntities(p) = &packet {
            for &id in &p.ids {
                self.entity_section_forget(id as u32);
            }
        }
        let packet_section = match &packet {
            C::AddEntity(p) => Some((true, p.id as u32, p.position)),
            _ => None,
        };
        let respawned = matches!(packet, C::Respawn(_));
        let result = self.apply_packet(packet);
        if result.is_ok()
            && let Some((joined, id, at)) = packet_section
        {
            if joined {
                self.entity_section_joined(id, at);
            } else {
                self.entity_section_moved_id(id);
            }
        }
        if respawned {
            self.entity_section_forget(self.own_entity_id() as u32);
        }
        result
    }

    /// Tells `entity_entering_section` each crossing queued by a nested write, in order. Called
    /// from inside a listener it does nothing: the outer write is still in progress.
    pub(super) fn tell_entity_sections(&mut self) {
        let Some(batch) = self.native_lifecycle.entity_sections.begin_telling() else {
            return;
        };
        if let Some(epoch) = self.native_lifecycle.epoch() {
            for c in &batch {
                let chunk_changed = c.old[0] != c.new[0] || c.old[2] != c.new[2];
                self.native_entity_entering_section(
                    EntityRef::new(epoch, c.id),
                    c.old,
                    c.new,
                    chunk_changed,
                );
            }
        }
        self.native_lifecycle.entity_sections.end_telling(batch);
    }

    fn tell_entity_sections_in(&mut self, sections: &mut EntitySections) {
        let Some(batch) = sections.begin_telling() else {
            return;
        };
        if let Some(epoch) = self.native_lifecycle.epoch() {
            for c in &batch {
                let chunk_changed = c.old[0] != c.new[0] || c.old[2] != c.new[2];
                self.native_entity_entering_section(
                    EntityRef::new(epoch, c.id),
                    c.old,
                    c.new,
                    chunk_changed,
                );
            }
        }
        sections.end_telling(batch);
    }
}
