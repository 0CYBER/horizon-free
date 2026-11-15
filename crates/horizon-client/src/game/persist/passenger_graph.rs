//! The ordered relationship shared by saved entity trees and live non-mob riders.
//! Creation, boarding, joining the level and saving are separate decisions.
use super::{Game, PassengerSlot, Tag, entity_nbt};
use crate::game::{commands::Target, native_entities::target_id};

pub(super) fn take_passengers(tag: &mut Tag) -> Option<Tag> {
    if let Tag::Compound(c) = tag {
        c.remove(&"Passengers".into())
    } else {
        None
    }
}

impl Game<'_> {
    pub(in crate::game) fn passenger_present(&self, id: u32) -> bool {
        self.level_entity(id as i32).is_some_and(|t| match t {
            Target::Mob(id) => self
                .mobs
                .iter()
                .any(|m| m.id == id && m.alive() && !m.removed),
            _ => true,
        })
    }

    /// Mob riding state remains authoritative for existing AI/dismount callers.
    /// Other live kinds use the same ordered slots the loader already retains.
    pub(in crate::game) fn passenger_vehicle(&self, id: u32) -> Option<u32> {
        if !self.passenger_present(id) {
            return None;
        }
        if let Some(m) = self.mobs.iter().find(|m| m.id == id) {
            return m.extra.mount.filter(|v| self.passenger_present(*v));
        }
        self.persist
            .passenger_slots
            .iter()
            .find_map(|(&vehicle, slots)| {
                (self.passenger_present(vehicle)
                    && slots
                        .iter()
                        .any(|slot| matches!(slot, PassengerSlot::Live(rider) if *rider == id)))
                .then_some(vehicle)
            })
    }

    pub(in crate::game) fn ordered_live_passengers(&self, vehicle: u32) -> Vec<u32> {
        let mut ids = Vec::new();
        if let Some(slots) = self.persist.passenger_slots.get(&vehicle) {
            for slot in slots {
                if let PassengerSlot::Live(id) = slot
                    && self.passenger_vehicle(*id) == Some(vehicle)
                {
                    ids.push(*id);
                }
            }
        }
        for m in &self.mobs {
            if m.extra.mount == Some(vehicle)
                && self.passenger_present(m.id)
                && !ids.contains(&m.id)
            {
                ids.push(m.id);
            }
        }
        ids
    }

    pub(in crate::game) fn unlink_passenger(&mut self, id: u32) {
        if let Some(stand) = self.armor_stands.iter_mut().find(|a| a.id == id) {
            stand.vehicle = None;
        }
        for slots in self.persist.passenger_slots.values_mut() {
            slots.retain(|s| !matches!(s, PassengerSlot::Live(rider) if *rider == id));
        }
    }

    /// Pinned EntityType: player has a null factory; FishingHook has a real
    /// factory, but its type cannot serialize. Registry authority stays shared.
    pub(super) fn passenger_creatable(&self, tag: &Tag) -> bool {
        entity_nbt::kind(tag).is_some_and(|kind| {
            kind != "minecraft:player"
                && (self.entity_type_exists(&kind) || self.res.mob_types().contains_key(&kind))
        })
    }

    pub(super) fn saved_vehicle_accepts(&self, tag: &Tag) -> bool {
        entity_nbt::kind(tag).is_some_and(|kind| {
            kind != "minecraft:marker" && crate::game::spawner::entity_type_saves(&kind)
        })
    }

    pub(super) fn load_passengers(&mut self, vehicle: u32, passengers: Tag) {
        let Tag::List(list) = passengers else { return };
        let Some(target) = self.level_entity(vehicle as i32) else {
            return;
        };
        let kind = self.target_type(target);
        // Marker.couldAcceptPassenger refuses even forced boarding. A no-save
        // vehicle is refused on a server too. Its unlinked children never join.
        if kind == "minecraft:marker" || !crate::game::spawner::entity_type_saves(&kind) {
            return;
        }
        self.load_entity_types();
        let mut slots = Vec::new();
        for mut tag in list {
            if !self.passenger_creatable(&tag) {
                continue;
            }
            if entity_nbt::uuid(&tag).is_some_and(|u| self.target_with_uuid(u).is_some()) {
                // #11807: joined-level duplicate skips tracking, not saving.
                slots.push(PassengerSlot::Saved(tag));
                continue;
            }
            let Some(kind) = entity_nbt::kind(&tag) else {
                continue;
            };
            if tag.get("Pos").is_none()
                && let Tag::Compound(c) = &mut tag
            {
                c.insert("Pos".into(), Tag::List(vec![Tag::Double(0); 3]));
            }
            let Some(at) = entity_nbt::position(&tag) else {
                continue;
            };
            let children = take_passengers(&mut tag);
            let before = self.next_entity;
            let key = super::chunk_of(at);
            let uuid = entity_nbt::uuid(&tag);
            let unknown_before = self.persist.unknown.get(&key).map_or(0, Vec::len);
            if let Err(tag) = self.load_unlisted(key, &kind, uuid, tag)
                && let Err(tag) = self.load_listed(&kind, uuid, self.level.player_uuid, tag)
            {
                self.load_other(tag, &kind);
            }
            let Some(id) = ((before + 1)..=self.next_entity).find(|id| self.passenger_present(*id))
            else {
                if let Some(unknown) = self.persist.unknown.get_mut(&key) {
                    unknown.truncate(unknown_before);
                }
                continue;
            };
            self.link_loaded_passenger(id, vehicle);
            slots.push(PassengerSlot::Live(id));
            if let Some(children) = children {
                self.load_passengers(id, children);
            }
        }
        if !slots.is_empty() {
            self.persist.passenger_slots.insert(vehicle, slots);
        }
    }

    pub(in crate::game) fn link_loaded_passenger(&mut self, id: u32, vehicle: u32) {
        self.unlink_passenger(id);
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.mount = Some(vehicle);
            m.extra.passenger = true;
        }
        if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id) {
            a.vehicle = Some(vehicle);
        }
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == vehicle) {
            m.extra.rider.get_or_insert(id);
        }
        self.persist
            .passenger_slots
            .entry(vehicle)
            .or_default()
            .push(PassengerSlot::Live(id));
    }

    /// Entity.rideTick: non-mob carriers use the same position writer and
    /// attachment authority as existing riders. Walk parents before children.
    pub(in crate::game) fn position_saved_passengers(&mut self) {
        let mut riders: Vec<u32> = self
            .persist
            .passenger_slots
            .values()
            .flatten()
            .filter_map(|slot| match slot {
                PassengerSlot::Live(id) => Some(*id),
                _ => None,
            })
            .collect();
        riders.sort_unstable();
        riders.dedup();
        riders.sort_by_key(|id| {
            let mut path = vec![*id];
            let mut next = self.passenger_vehicle(*id);
            while let Some(parent) = next {
                if path.contains(&parent) {
                    break;
                }
                path.push(parent);
                next = self.passenger_vehicle(parent);
            }
            path.len()
        });
        let index: std::collections::HashMap<u32, usize> = self
            .mobs
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        for id in riders {
            let Some(parent) = self.passenger_vehicle(id) else {
                self.unlink_passenger(id);
                continue;
            };
            if let Some(&i) = index.get(&id) {
                // Parents have already moved, including non-mob vehicles of
                // mobs. Reuse the real mob attachment and specialized seats.
                self.position_saved_mob_rider(i, &index);
                continue;
            }
            let Some(target) = self.level_entity(id as i32) else {
                continue;
            };
            let seat = self.passenger_riding_position(parent, None, None, &index);
            if let Some(at) = seat {
                self.place_saved_passenger(
                    target,
                    horizon_core::geometry::Vec3::new(at.x, at.y, at.z),
                );
            }
        }
    }

    /// Entity.positionRider's MoveFunction is setPos, not a teleport: no
    /// teleport hook, player transition or recursively repeated displacement.
    pub(in crate::game) fn place_saved_passenger(
        &mut self,
        target: Target,
        at: horizon_core::geometry::Vec3,
    ) {
        use crate::game::entity::{Entity, with_entity_mut};
        let Some(id) = target_id(target) else { return };
        for handle in self.entity_handles() {
            with_entity_mut!(self, handle, entity => {
                if entity.id() == id {
                    *entity.position_mut() = at;
                    *entity.velocity_mut() = horizon_core::geometry::Vec3::ZERO;
                }
            });
        }
        match target {
            Target::Other(id) => {
                if let Some(e) = self.others.iter_mut().find(|e| e.id == id) {
                    e.position = at;
                }
            }
            // BlockAttachedEntity.setPos floors the anchor then recalculates
            // the bounding-box centre; it does not retain the requested point.
            Target::Frame(id) => {
                if let Some(e) = self.item_frames.iter_mut().find(|e| e.id == id) {
                    e.pos = [
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ];
                }
            }
            Target::Painting(id) => {
                if let Some(e) = self.paintings.iter_mut().find(|e| e.id == id) {
                    e.pos = [
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ];
                }
            }
            Target::Fangs(id) => {
                if let Some(e) = self.fangs.list.iter_mut().find(|e| e.id == id) {
                    e.pos = [at.x, at.y, at.z];
                }
            }
            Target::Crystal(id) => {
                if let Some(e) = self.crystals.iter_mut().find(|e| e.id == id) {
                    e.position = at;
                }
            }
            Target::Dragon(id) => {
                if let Some(e) = self.dragon.as_mut().filter(|e| e.id == id) {
                    e.position = at;
                }
            }
            Target::Cushion(id) => {
                if let Some(e) = self.cushions.iter_mut().find(|e| e.id == id) {
                    e.position = at;
                }
            }
            Target::Spawner(id) => {
                if let Some(e) = self.ominous_spawners.iter_mut().find(|e| e.id == id) {
                    e.position = [at.x, at.y, at.z];
                }
            }
            Target::Cloud(id) => {
                if let Some(e) = self.breath_clouds.iter_mut().find(|e| e.id == id) {
                    e.at = [at.x, at.y, at.z];
                }
            }
            _ => {}
        }
    }

    /// Dimension travel clones every live rider, including noSave types.
    /// Filesystem/RootVehicle serialization deliberately uses the saved tree
    /// instead. Both snapshots use the existing per-kind body writers.
    pub(in crate::game) fn vehicle_transfer_tag(&mut self, id: u32) -> Option<Tag> {
        let target = self.level_entity(id as i32)?;
        let mut body = self.entity_nbt(target)?;
        take_passengers(&mut body);
        let slots = self
            .persist
            .passenger_slots
            .get(&id)
            .cloned()
            .unwrap_or_default();
        let mut included = Vec::new();
        let mut children = Vec::new();
        for slot in slots {
            match slot {
                PassengerSlot::Live(child) if self.passenger_vehicle(child) == Some(id) => {
                    included.push(child);
                    if let Some(tag) = self.vehicle_transfer_tag(child) {
                        children.push(tag);
                    }
                }
                PassengerSlot::Saved(tag) => children.push(tag),
                _ => {}
            }
        }
        let mut extra: Vec<u32> = self
            .ordered_live_passengers(id)
            .into_iter()
            .filter(|child| !included.contains(child))
            .collect();
        if let Some(driver) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.extra.rider)
            && let Some(i) = extra.iter().position(|child| *child == driver)
        {
            extra.remove(i);
            extra.insert(0, driver);
        }
        for child in extra {
            if let Some(tag) = self.vehicle_transfer_tag(child) {
                children.push(tag);
            }
        }
        super::add_passengers(&mut body, children);
        super::write_attachments(&mut body, &self.entity_attachments);
        Some(body)
    }

    pub(in crate::game) fn passenger_tree(&self, root: u32) -> Vec<u32> {
        let mut tree = vec![root];
        let mut cursor = 0;
        while cursor < tree.len() {
            for id in self.ordered_live_passengers(tree[cursor]) {
                if !tree.contains(&id) {
                    tree.push(id);
                }
            }
            cursor += 1;
        }
        tree
    }

    /// Remove a saved subtree without running death/loot adapters. The typed
    /// entity walk supplies listed kinds; the existing off-walk carriers leave too.
    pub(super) fn discard_passenger_entities(&mut self, gone: &[u32]) {
        self.take_passenger_entities(gone, false);
    }
    pub(in crate::game) fn take_passenger_entities(&mut self, gone: &[u32], travelling: bool) {
        use crate::game::entity::{Entity, with_entity};
        for handle in self.entity_handles_rev() {
            if with_entity!(self, handle, entity => gone.contains(&entity.id())) {
                self.discard_entity(handle);
            }
        }
        self.item_frames.retain(|e| !gone.contains(&e.id));
        self.paintings.retain(|e| !gone.contains(&e.id));
        self.others.retain(|e| !gone.contains(&e.id));
        self.cushions.retain(|e| !gone.contains(&e.id));
        self.fangs.list.retain(|e| !gone.contains(&e.id));
        self.crystals.retain(|e| !gone.contains(&e.id));
        self.breath_clouds.retain(|e| !gone.contains(&e.id));
        self.ominous_spawners.retain(|e| !gone.contains(&e.id));
        if self.dragon.as_ref().is_some_and(|e| gone.contains(&e.id)) {
            self.dragon = None;
        }
        for &id in gone {
            self.release_vehicle_container(id);
            self.unlink_passenger(id);
            self.persist.passenger_slots.remove(&id);
            if travelling {
                self.detach_entity(id);
            } else {
                self.forget_entity(id);
            }
        }
    }

    pub(super) fn saved_passenger_body(&mut self, id: u32) -> Option<Tag> {
        let target = self.level_entity(id as i32)?;
        if !self.passenger_present(id)
            || !crate::game::spawner::entity_type_saves(&self.target_type(target))
        {
            return None;
        }
        self.entity_nbt(target)
    }

    pub(super) fn saved_entity_is_root(&self, tag: &Tag) -> bool {
        if entity_nbt::kind(tag).is_some_and(|k| !crate::game::spawner::entity_type_saves(&k)) {
            return false;
        }
        let id = entity_nbt::uuid(tag)
            .and_then(|u| self.target_with_uuid(u))
            .and_then(target_id);
        id.is_none_or(|id| self.passenger_vehicle(id).is_none())
    }
}
