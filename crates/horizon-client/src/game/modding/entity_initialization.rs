//! `entity_constructing` (`EntityEvent.EntityConstructing`): vanilla's `Entity.<init>`, once per
//! entity on the side that makes it. Horizon has no one constructor, so the hook is told where
//! each kind of entity starts: a spawned mob (`spawn_mob`), an entity read from disk or a
//! summon's NBT (`remember_uuid`, before its saved attachments are read), every other new
//! entity in the join pass just before `entity_join_level`, a LAN guest's player as it is
//! loaded or respawned, and on a client of a server the entities and players the server adds.
//! The first attachment access of an entity not yet told tells it first, so no mod reads an
//! attachment before the listeners have set it.

use super::Game;
use mod_api::Holder;
use std::collections::{HashMap, HashSet};

#[cfg(test)]
#[path = "entity_initialization/tests.rs"]
mod tests;

#[derive(Default)]
pub(super) struct EntityInitialization {
    /// The object holders already told (entity IDs and separate player UUIDs). An entity is forgotten when it leaves the world (`forget_entity`:
    /// death, a chunk unloading), so the next one made from its data is
    /// told again, as vanilla constructs a new entity for each of those.
    initialized: HashSet<Holder>,
    /// Each LAN guest's connection when its player was last told: a new connection is a new
    /// `ServerPlayer`.
    players: HashMap<u128, u32>,
    /// The holder being told now: its attachments are reachable before it is in the level.
    pub(super) active: Option<Holder>,
}

impl Game<'_> {
    /// Tell the listeners about a new entity of type `kind`, unless it was already told. Marked
    /// before the dispatch, so a listener's own attachment access doesn't tell it again.
    pub(super) fn initialize_entity(&mut self, holder: Holder, kind: &str) {
        if !mod_api::__private::listened(mod_api::HookId::EntityConstructing) {
            return;
        }
        if !matches!(holder, Holder::Entity(_) | Holder::Player(_)) {
            return;
        }
        if !self.entity_initialization.initialized.insert(holder) {
            return;
        }
        let previous = self.entity_initialization.active.replace(holder);
        self.native_entity_constructing(holder, kind);
        self.entity_initialization.active = previous;
        // Listener stores skip binding while their holder is being constructed.
        // A resident part has no separate join callback, so finish its binding
        // here even when construction was triggered by an inspection.
        if self.native_authoritative()
            && let Holder::Entity(id) = holder
            && self
                .entity_attachments
                .get(&id)
                .is_some_and(|map| map.has_index_entries())
            && let Err(error) = self.bind_resident_entity_attachments(id)
        {
            log::warn!("constructor attachment binding failed for entity {id}: {error:?}");
        }
    }

    /// `initialize_entity` for a holder whose attachments are about to be read, its type
    /// looked up: an entity that started on a path that doesn't tell (one made and touched
    /// within a pass, before the join pass) is told before the read.
    pub(super) fn initialize_entity_on_access(&mut self, holder: Holder) {
        if !mod_api::__private::listened(mod_api::HookId::EntityConstructing) {
            return;
        }
        match holder {
            Holder::Entity(id) => {
                let Some(kind) = self
                    .entity(id)
                    .and_then(|entity| self.entity_kind(entity).map(|kind| kind.id(self)))
                else {
                    return;
                };
                self.initialize_entity(holder, &kind);
            }
            Holder::Player(_) => self.initialize_entity(holder, "minecraft:player"),
            _ => {}
        }
    }

    pub(super) fn forget_entity_initialization(&mut self, holder: Holder) {
        if let Holder::Entity(id) = holder
            && let Some(mob) = self.mobs.iter_mut().find(|mob| mob.id == id)
        {
            mob.accepted_hit = None;
        }
        self.entity_initialization.initialized.remove(&holder);
    }

    /// Forget destroyed objects in this level before its UUID metadata is pruned.
    /// The supplied set includes reachable off-level objects. Other levels and
    /// player lifetimes are owned by their own cleanup boundaries.
    pub(super) fn prune_entity_initializations(&mut self, retained: &HashSet<u32>) {
        self.entity_initialization
            .initialized
            .retain(|holder| match holder {
                Holder::Entity(id) => !self.persist.uuids.contains_key(id) || retained.contains(id),
                _ => true,
            });
    }

    /// A LAN guest's player, its file just read for connection `conn`: told once per
    /// connection (`PlayerList.placeNewPlayer` constructs the `ServerPlayer`, then
    /// `loadPlayerData` reads its file).
    pub(super) fn initialize_server_player(&mut self, uuid: u128, conn: u32) {
        if self.entity_initialization.players.insert(uuid, conn) != Some(conn) {
            self.forget_entity_initialization(Holder::Player(uuid));
        }
        self.initialize_guest_player_under_data(uuid, conn);
    }

    /// `PlayerList.respawn`: the guest gets a new `ServerPlayer`, told again, and the data it
    /// keeps (`copy_on_death`, already in place) is restored over it.
    pub(super) fn reinitialize_server_player(&mut self, uuid: u128, conn: u32) {
        self.forget_entity_initialization(Holder::Player(uuid));
        self.initialize_guest_player_under_data(uuid, conn);
    }

    /// Tell a guest's player whose attachments already hold its data, then read that data
    /// back over what the listeners set: the end state of vanilla's order, where the player is
    /// constructed before its data is read (`Entity.load`, `restoreFrom`).
    fn initialize_guest_player_under_data(&mut self, uuid: u128, conn: u32) {
        if !mod_api::__private::listened(mod_api::HookId::EntityConstructing) {
            return;
        }
        let data = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.guests.attachments_mut(conn))
            .map(|attachments| attachments.to_nbt());
        self.initialize_entity(Holder::Player(uuid), "minecraft:player");
        if let Some(data) = data
            && let Some(attachments) = self
                .lan
                .as_mut()
                .and_then(|host| host.hub.guests.attachments_mut(conn))
        {
            attachments.read_saved_over(data);
        }
    }
}

/// Explicit ownership passed alongside one travelling object's NBT, never encoded in NBT.
/// Not Clone: a transfer cannot install the same runtime state twice.
#[derive(Debug)]
pub(super) struct TransferredEntityState {
    attachments: Option<horizon_mod_runtime::attachments::Attachments>,
    initialized: bool,
    accepted_hit: Option<std::sync::Arc<horizon_client::health::AcceptedHit>>,
    died: bool,
}

impl TransferredEntityState {
    /// Move the single accepted-hit owner onto a reconstructed living body.
    pub(in crate::game) fn restore_mob(&mut self, mob: &mut horizon_client::mobs::Mob) {
        if mob.health > 0.0 {
            self.died = false;
        }
        if let Some(hit) = self.accepted_hit.take() {
            mob.last_damage = hit.committed;
            mob.accepted_hit = Some(hit);
        }
    }
}

impl Game<'_> {
    /// Consume the source object's state before forgetting its persistence metadata.
    /// Retiring the publisher immediately makes retained query write tokens stale.
    pub(super) fn take_transferred_entity_state(&mut self, id: u32) -> TransferredEntityState {
        let mut attachments = self.entity_attachments.remove(&id);
        if let Some(map) = &mut attachments {
            map.unbind_index();
        }
        let accepted_hit = self
            .mob_object_mut(id)
            .and_then(|mob| mob.accepted_hit.take());
        let died = self.died.remove(&id);
        TransferredEntityState {
            attachments,
            accepted_hit,
            died,
            initialized: self
                .entity_initialization
                .initialized
                .remove(&Holder::Entity(id)),
        }
    }

    /// Install before `remember_uuid`: the destination is the same logical entity,
    /// so neither construction nor the saved overlay may replace its runtime values.
    pub(super) fn adopt_transferred_entity_state(
        &mut self,
        id: u32,
        state: TransferredEntityState,
    ) {
        self.journal_arrival(id);
        if state.died {
            self.died.insert(id);
        }
        // Even an empty map marks this as runtime state for `remember_uuid`.
        self.entity_attachments
            .insert(id, state.attachments.unwrap_or_default());
        if state.initialized {
            self.entity_initialization
                .initialized
                .insert(Holder::Entity(id));
        }
    }
}

impl Game<'_> {
    pub(super) fn forget_remote_entity_construction(&mut self, id: i32) {
        self.forget_entity_initialization(Holder::Entity(id as u32));
        if let Some(uuid) = self
            .remote
            .as_ref()
            .and_then(|r| r.level_players.get(&id))
            .copied()
        {
            self.forget_entity_initialization(Holder::Player(uuid));
        }
    }
}
