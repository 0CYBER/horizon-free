//! Ordered guests with indexes for connection and exact player identity.
use super::guest::Guest;
use crate::fast_hash::FxHashMap;
use std::ops::Deref;

/// Collection-owned mutation keeps identity and material-body indexes current.
/// Ordinary state writes compare only the indexed fields; membership or key
/// changes rebuild indexes while preserving join order and first-match lookup.
#[derive(Default)]
pub struct GuestList {
    rows: Vec<Guest>,
    simulation: Vec<usize>,
    connections: FxHashMap<u32, usize>,
    identities: FxHashMap<(u128, u32), usize>,
    residency: Option<(
        crate::join_seq::residency::Journal,
        fn(&str) -> mod_api::DimensionKey,
    )>,
}
impl GuestList {
    /// Bind actual player records, including non-network fake actors. Readiness
    /// and material simulation participation do not decide identity existence.
    pub fn bind_residency(
        &mut self,
        journal: &crate::join_seq::residency::Journal,
        dimension: fn(&str) -> mod_api::DimensionKey,
    ) {
        self.residency = Some((journal.clone(), dimension));
        for index in 0..self.rows.len() {
            self.bind_guest(index);
        }
    }
    fn bind_guest(&mut self, index: usize) -> bool {
        let Some((journal, dimension)) = &self.residency else {
            return false;
        };
        let guest = &mut self.rows[index];
        if guest.released {
            guest.attachments.retire();
            guest.residency.retire();
            return false;
        }
        let uuid = crate::lan::playerdata::uuid_ints(guest.uuid);
        let source = crate::join_seq::residency::Source::Guest(guest.conn);
        if guest.residency.identity().is_some_and(|identity| {
            identity.id.get() != guest.entity_id as u32
                || identity.source != source
                || identity.details.known_uuid != Some(uuid)
        }) {
            guest.attachments.retire();
        }
        guest.residency.bind(
            journal,
            crate::join_seq::residency::Owner {
                dimension: dimension(&guest.dimension),
                kind: crate::join_seq::residency::Kind::Player,
            },
            guest.entity_id as u32,
            source,
            Some(uuid),
        )
    }
    /// Edit one row without lending a mutable row or slice outside this call.
    /// Indexed-key changes are published before the caller can use the list
    /// again, including when the callback unwinds. Ordinary state writes do
    /// not scan the collection or rebuild its indexes.
    pub fn update<T>(&mut self, index: usize, update: impl FnOnce(&mut Guest) -> T) -> T {
        let before = index_fields(&self.rows[index]);
        let registration = self.rows[index].attachments.registration();
        let dimension = self
            .residency
            .as_ref()
            .map(|(_, key)| key(&self.rows[index].dimension));
        let released = self.rows[index].released;
        let edit = Edit {
            list: self,
            index,
            before,
            registration,
            dimension,
            released,
        };
        update(&mut edit.list.rows[index])
    }

    /// Edit the first guest for a connection using the indexed lookup.
    pub fn update_connection<T>(
        &mut self,
        conn: u32,
        update: impl FnOnce(&mut Guest) -> T,
    ) -> Option<T> {
        let index = self.connection_index(conn)?;
        Some(self.update(index, update))
    }

    /// Edit rows in join order, through the same bounded mutation scope.
    pub fn update_all(&mut self, mut update: impl FnMut(&mut Guest)) {
        for index in 0..self.rows.len() {
            self.update(index, &mut update);
        }
    }

    /// Borrow only the tracker and its read-only view inputs for a tracking pass.
    /// The predicate cannot mutate indexed keys, and no whole mutable guest escapes.
    pub(super) fn tracking_viewers<'a>(
        &'a mut self,
        visible: impl Fn(&Guest) -> bool,
        outputs: impl IntoIterator<Item = (&'a mut Vec<super::Body>, Option<&'a mut Vec<[f64; 3]>>)>,
        take_moved: bool,
    ) -> Vec<super::entity::Viewer<'a>> {
        self.tracking_viewers_and_views(visible, outputs, take_moved, ("", &mut []))
    }

    /// [`Self::tracking_viewers`], then the clients that view level `views.0` from another
    /// (#3716, `viewed_levels::entities`), each with its view's tracker, chunks and areas and
    /// one of `views.1`, in the guests' order.
    pub(super) fn tracking_viewers_and_views<'a>(
        &'a mut self,
        visible: impl Fn(&Guest) -> bool,
        outputs: impl IntoIterator<Item = (&'a mut Vec<super::Body>, Option<&'a mut Vec<[f64; 3]>>)>,
        take_moved: bool,
        (dimension, views): (&str, &'a mut [Vec<super::Body>]),
    ) -> Vec<super::entity::Viewer<'a>> {
        use super::viewed_levels::entities::tracks_view;
        let viewing = !views.is_empty();
        let (mut outputs, mut views) = (outputs.into_iter(), views.iter_mut());
        let mut viewers = Vec::new();
        for g in self.rows.iter_mut() {
            let (spectator, horizon) = (g.spectator(), g.is_horizon());
            let (chunks, sources, tracker, out, gone, moved) = if visible(g) {
                let Some((out, gone)) = outputs.next() else {
                    continue;
                };
                let moved = take_moved && std::mem::take(&mut g.moved);
                (&g.sent_chunks, None, &mut g.tracker, out, gone, moved)
            } else if viewing && tracks_view(g, dimension) {
                let (Some(out), Some(view)) = (views.next(), g.viewed.views.get_mut(dimension))
                else {
                    continue;
                };
                // Its own entity is not in the level: every pair is checked each tick.
                (
                    &view.sent,
                    Some(&view.sources[..]),
                    &mut view.tracker,
                    out,
                    None,
                    true,
                )
            } else {
                continue;
            };
            viewers.push(super::entity::Viewer {
                entity_id: g.entity_id,
                spectator,
                permission: g.permission,
                horizon,
                moved,
                position: g.position,
                view_distance: g.view_distance,
                chunks,
                sources,
                tracker,
                out,
                gone,
            });
        }
        viewers
    }

    /// Borrow only queued packets, without exposing the guest's indexed fields.
    pub fn inbox_mut(
        &mut self,
        index: usize,
    ) -> Option<&mut std::collections::VecDeque<super::guest::InboxItem>> {
        Some(&mut self.rows.get_mut(index)?.inbox)
    }

    /// Borrow saved data without exposing indexed identity fields.
    pub fn saved_data_mut(&mut self, conn: u32) -> Option<&mut Option<horizon_core::nbt::Tag>> {
        let index = self.connection_index(conn)?;
        Some(&mut self.rows[index].saved)
    }

    /// Borrow a player's canonical attachments without exposing identity fields.
    pub fn attachments_mut(
        &mut self,
        conn: u32,
    ) -> Option<&mut horizon_mod_runtime::attachments::ResidentAttachments> {
        let index = self.connection_index(conn)?;
        Some(&mut self.rows[index].attachments)
    }

    /// Borrow only the stored menu items at this collection index.
    pub fn ender_items_mut(
        &mut self,
        index: usize,
    ) -> Option<&mut [Option<crate::items::ItemStack>]> {
        Some(&mut self.rows.get_mut(index)?.ender_items)
    }

    /// Borrow only the stored menu items at this collection index.
    pub fn craft_grid_mut(
        &mut self,
        index: usize,
    ) -> Option<&mut [Option<crate::items::ItemStack>]> {
        Some(&mut self.rows.get_mut(index)?.craft_grid)
    }

    /// Borrow only the stored menu items at this collection index.
    pub fn native_menu_mut(
        &mut self,
        index: usize,
    ) -> Option<&mut [Option<crate::items::ItemStack>]> {
        Some(&mut self.rows.get_mut(index)?.native_menu)
    }

    /// Borrow a body without exposing the guest's indexed fields or row replacement.
    pub fn motion_mut(&mut self, conn: u32) -> Option<&mut crate::motion::PlayerMotion> {
        let index = self.connection_index(conn)?;
        self.rows[index].motion.as_deref_mut()
    }

    /// Borrow only advancement state; indexed identity and kind stay inaccessible.
    pub(super) fn advancement_state(
        &mut self,
        index: usize,
    ) -> Option<super::hub::advancements::AdvancementState<'_>> {
        let guest = self.rows.get_mut(index)?;
        Some(super::hub::advancements::AdvancementState {
            progress: &mut guest.advancements,
            changed: &mut guest.advancements_changed,
            unlocked: &mut guest.recipes_unlocked,
            book: &mut guest.recipe_book,
        })
    }

    /// Reorder rows without exposing a mutable slice.
    pub fn swap(&mut self, a: usize, b: usize) {
        self.rows.swap(a, b);
        self.reindex();
    }

    pub fn connection_index(&self, conn: u32) -> Option<usize> {
        self.connections.get(&conn).copied()
    }
    pub fn identity_index(&self, uuid: u128, entity: u32) -> Option<usize> {
        self.identities.get(&(uuid, entity)).copied()
    }
    /// Material player bodies in join order, without visiting connectionless actors.
    pub fn simulation_iter(&self) -> impl Iterator<Item = &Guest> {
        self.simulation_indexed_iter().map(|(_, guest)| guest)
    }
    /// Material players and their collection indexes, without visiting connectionless actors.
    pub fn simulation_indexed_iter(&self) -> impl Iterator<Item = (usize, &Guest)> {
        self.simulation
            .iter()
            .map(|&index| (index, &self.rows[index]))
    }
    pub fn as_slice(&self) -> &[Guest] {
        &self.rows
    }
    pub fn push(&mut self, guest: Guest) {
        let index = self.rows.len();
        self.connections.entry(guest.conn).or_insert(index);
        self.identities
            .entry((guest.uuid, guest.entity_id as u32))
            .or_insert(index);
        if guest.participates_in_player_simulation() {
            self.simulation.push(index);
        }
        self.rows.push(guest);
        if !self.bind_guest(index) {
            self.rows[index].residency.put_back();
        }
    }
    pub fn extend(&mut self, guests: impl IntoIterator<Item = Guest>) {
        for guest in guests {
            self.push(guest);
        }
    }
    /// Move a still-resident record out while saving it; its binding travels with it.
    pub fn take_for_save(&mut self, index: usize) -> Guest {
        let guest = self.rows.remove(index);
        guest.residency.extract();
        self.reindex();
        guest
    }

    pub fn remove(&mut self, index: usize) -> Guest {
        let mut guest = self.take_for_save(index);
        guest.attachments.retire();
        guest.residency.retire();
        guest
    }
    pub fn retain(&mut self, keep: impl FnMut(&Guest) -> bool) {
        // Vec::retain may have removed earlier rows when the predicate unwinds.
        // Publish the surviving order on both the success and unwind paths.
        let repair = Reindex(self);
        repair.0.rows.retain(keep);
    }
    pub fn insert(&mut self, index: usize, guest: Guest) {
        self.rows.insert(index, guest);
        if !self.bind_guest(index) {
            self.rows[index].residency.put_back();
        }
        self.reindex();
    }
    pub fn replace(&mut self, index: usize, guest: Guest) -> Guest {
        let mut old = std::mem::replace(&mut self.rows[index], guest);
        old.residency.retire();
        if !self.bind_guest(index) {
            self.rows[index].residency.put_back();
        }
        self.reindex();
        old.attachments.retire();
        old
    }
    pub fn clear(&mut self) {
        for guest in &mut self.rows {
            guest.attachments.retire();
        }
        self.rows.clear();
        self.simulation.clear();
        self.connections.clear();
        self.identities.clear();
    }
    pub fn reverse(&mut self) {
        self.rows.reverse();
        self.reindex();
    }
    fn reindex(&mut self) {
        self.simulation.clear();
        self.connections.clear();
        self.identities.clear();
        for (index, guest) in self.rows.iter().enumerate() {
            if guest.participates_in_player_simulation() {
                self.simulation.push(index);
            }
            self.connections.entry(guest.conn).or_insert(index);
            self.identities
                .entry((guest.uuid, guest.entity_id as u32))
                .or_insert(index);
        }
    }
}
impl Drop for GuestList {
    fn drop(&mut self) {
        self.clear();
    }
}
impl Deref for GuestList {
    type Target = [Guest];
    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}
impl<'a> IntoIterator for &'a GuestList {
    type Item = &'a Guest;
    type IntoIter = std::slice::Iter<'a, Guest>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.iter()
    }
}
struct Reindex<'a>(&'a mut GuestList);
impl Drop for Reindex<'_> {
    fn drop(&mut self) {
        self.0.reindex();
    }
}

// The guard stays private: callers cannot forget it to bypass publication.
struct Edit<'a> {
    list: &'a mut GuestList,
    index: usize,
    before: (u32, u128, i32, bool),
    registration: Option<horizon_mod_runtime::attachments::AttachmentRegistration>,
    dimension: Option<mod_api::DimensionKey>,
    released: bool,
}
fn index_fields(guest: &Guest) -> (u32, u128, i32, bool) {
    (
        guest.conn,
        guest.uuid,
        guest.entity_id,
        guest.participates_in_player_simulation(),
    )
}
impl Drop for Edit<'_> {
    fn drop(&mut self) {
        if let Some(old) = &self.registration
            && !self.list.rows[self.index]
                .attachments
                .registration()
                .is_some_and(|current| old.same_registration(&current))
        {
            old.unregister();
        }
        let after = index_fields(&self.list.rows[self.index]);
        let identity_changed =
            (after.0, after.1, after.2) != (self.before.0, self.before.1, self.before.2);
        if (after.0, after.1) != (self.before.0, self.before.1)
            || (self.before.2 != 0 && after.2 != self.before.2)
        {
            if self.registration.as_ref().is_some_and(|old| {
                self.list.rows[self.index]
                    .attachments
                    .registration()
                    .is_some_and(|current| old.same_registration(&current))
            }) {
                self.list.rows[self.index].attachments.retire();
            }
            self.list.rows[self.index].residency.renew();
        }
        let dimension = self
            .list
            .residency
            .as_ref()
            .map(|(_, key)| key(&self.list.rows[self.index].dimension));
        if identity_changed
            || dimension != self.dimension
            || self.list.rows[self.index].released != self.released
        {
            self.list.bind_guest(self.index);
        }
        if after != self.before {
            self.list.reindex();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn guest(conn: u32) -> Guest {
        let (tx, _) = std::sync::mpsc::channel();
        Guest::new(
            super::super::Joined {
                conn,
                uuid: u128::from(conn) + 100,
                name: format!("guest{conn}"),
                properties: vec![],
                info: horizon_protocol::packets::configuration::ClientInformation::new(8),
                brand: "horizon".into(),
                sender: super::super::PlayerSender::new(tx, Default::default()),
                latency: Default::default(),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32 + 10,
            [0.0, 64.0, 0.0],
            8,
        )
    }
    fn check(list: &GuestList) {
        for (index, guest) in list.iter().enumerate() {
            assert_eq!(list.connection_index(guest.conn), Some(index));
            assert_eq!(
                list.identity_index(guest.uuid, guest.entity_id as u32),
                Some(index)
            );
        }
    }
    const VALUE: horizon_mod_runtime::attachments::AttachmentType<i32> =
        horizon_mod_runtime::attachments::AttachmentType {
            namespace: "guest_test",
            name: "value",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };

    #[test]
    fn canonical_attachment_retirement_does_not_wait_for_removed_records_or_clones() {
        use horizon_mod_runtime::attachments::{Holder, index::AttachmentIndex};
        use std::sync::{Arc, Mutex};

        for action in ["remove", "replace", "retain", "clear", "drop", "update"] {
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let key = ("guest_test".into(), "value".into());
            let mut player = guest(1);
            VALUE
                .set(&mut player.attachments, Holder::Level, 7)
                .unwrap();
            player.attachments.bind_index(&index, player.uuid);
            let mut saved = player.attachments.clone();
            let before = index.lock().unwrap().snapshot(&key);
            let mut list = GuestList::default();
            list.push(player);
            drop(saved.clone());
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            let mut removed = match action {
                "remove" => Some(list.remove(0)),
                "replace" => Some(list.replace(0, guest(2))),
                "retain" => {
                    list.retain(|_| false);
                    None
                }
                "clear" => {
                    list.clear();
                    None
                }
                "drop" => {
                    drop(std::mem::take(&mut list));
                    None
                }
                "update" => {
                    list.update(0, |g| *g = guest(2));
                    None
                }
                _ => unreachable!(),
            };
            if let Some(player) = &mut removed {
                VALUE
                    .set(&mut player.attachments, Holder::Level, 9)
                    .unwrap();
            }
            assert!(index.lock().unwrap().snapshot(&key).is_empty(), "{action}");
            assert_eq!(before[0].entry.read(&VALUE), Ok(&7));
            assert_eq!(VALUE.get_existing(&mut saved, Holder::Level), Ok(Some(7)));
        }
    }

    #[test]
    fn identity_edits_retire_only_the_retained_old_attachment_registration() {
        use horizon_mod_runtime::attachments::{Holder, index::AttachmentIndex};
        use std::sync::{Arc, Mutex};
        for field in ["connection", "uuid", "entity"] {
            for replace_registration in [false, true] {
                let index = Arc::new(Mutex::new(AttachmentIndex::default()));
                let key = ("guest_test".into(), "value".into());
                let mut player = guest(1);
                VALUE
                    .set(&mut player.attachments, Holder::Level, 7)
                    .unwrap();
                let old = player.attachments.bind_index(&index, player.uuid);
                let mut table =
                    crate::join_seq::residency::table::EntityTable::new(Default::default());
                let mut list = GuestList::default();
                list.bind_residency(&table.producer(), |_| mod_api::DimensionKey::OVERWORLD);
                list.push(player);
                list.update(0, |g| {
                    match field {
                        "connection" => g.conn = 9,
                        "uuid" => g.uuid = 909,
                        _ => g.entity_id = 19,
                    }
                    if replace_registration {
                        g.attachments = Default::default();
                        VALUE.set(&mut g.attachments, Holder::Level, 9).unwrap();
                        g.attachments.bind_index(&index, g.uuid);
                    }
                });
                let entries = index.lock().unwrap().snapshot(&key);
                assert!(
                    !old.unregister(),
                    "old binding already retired: {field}, replacement={replace_registration}"
                );
                if replace_registration {
                    assert_eq!(entries.len(), 1);
                    assert_eq!(entries[0].entry.read(&VALUE), Ok(&9));
                } else {
                    assert!(entries.is_empty(), "{field}");
                }
                assert!(
                    table
                        .server_admitted_identity(list[0].entity_id as u32, |at| resident(
                            &list, at
                        ))
                        .is_some()
                );
            }
        }
    }

    #[test]
    fn displaced_updates_retire_retained_owners() {
        use horizon_mod_runtime::attachments::{Holder, index::AttachmentIndex};
        use std::sync::{Arc, Mutex};

        for action in ["replace", "swap", "capture", "connection", "unwind"] {
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let key = ("guest_test".into(), "value".into());
            let mut player = guest(1);
            VALUE
                .set(&mut player.attachments, Holder::Level, 7)
                .unwrap();
            player.attachments.bind_index(&index, player.uuid);
            let mut list = GuestList::default();
            list.push(player);
            let mut retained = guest(2);
            match action {
                "replace" => retained = list.update(0, |g| std::mem::replace(g, guest(2))),
                "swap" => list.update(0, |g| std::mem::swap(g, &mut retained)),
                "capture" => list.update_all(|g| retained = std::mem::replace(g, guest(2))),
                "connection" => {
                    retained = list
                        .update_connection(1, |g| std::mem::replace(g, guest(2)))
                        .unwrap()
                }
                "unwind" => {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        list.update(0, |g| {
                            std::mem::swap(g, &mut retained);
                            panic!("fixture")
                        })
                    }));
                }
                _ => unreachable!(),
            }
            assert!(index.lock().unwrap().snapshot(&key).is_empty(), "{action}");
            VALUE
                .set(&mut retained.attachments, Holder::Level, 9)
                .unwrap();
            assert!(index.lock().unwrap().snapshot(&key).is_empty(), "{action}");
        }
    }

    #[test]
    fn a_save_take_moves_canonical_binding_and_destruction_retires_it() {
        use horizon_mod_runtime::attachments::{Holder, index::AttachmentIndex};
        use std::sync::{Arc, Mutex};

        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let key = ("guest_test".into(), "value".into());
        let mut player = guest(1);
        VALUE
            .set(&mut player.attachments, Holder::Level, 7)
            .unwrap();
        let binding = player.attachments.bind_index(&index, player.uuid);
        let mut list = GuestList::default();
        list.push(player);
        let player = list.take_for_save(0);
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        list.insert(0, player);
        assert!(binding.unregister(), "the move did not replace the binding");
        list.attachments_mut(1).unwrap().bind_index(&index, 101);
        let mut borrowed = std::mem::take(&mut **list.attachments_mut(1).unwrap());
        let player = list.take_for_save(0);
        drop(player);
        VALUE.set(&mut borrowed, Holder::Level, 9).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
    }

    #[test]
    fn connectionless_actors_do_not_expand_the_material_body_index() {
        let mut list = GuestList::default();
        list.push(guest(1));
        for conn in 2..=257 {
            let mut fake = guest(conn);
            fake.kind = crate::lan::guest::GuestKind::Fake;
            list.push(fake);
        }
        assert_eq!(
            list.simulation_iter().map(|g| g.conn).collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            list.simulation_indexed_iter()
                .map(|(i, g)| (i, g.conn))
                .collect::<Vec<_>>(),
            [(0, 1)]
        );
        list.reverse();
        assert_eq!(
            list.simulation_iter().map(|g| g.conn).collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            list.simulation_indexed_iter()
                .map(|(i, g)| (i, g.conn))
                .collect::<Vec<_>>(),
            [(256, 1)]
        );
        list.remove(256);
        assert_eq!(list.simulation_iter().count(), 0);
        check(&list);
    }

    #[test]
    fn a_panicking_retain_preserves_surviving_lookup_indexes() {
        let mut list = GuestList::default();
        list.extend([guest(1), guest(2), guest(3)]);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            list.retain(|row| match row.conn {
                1 => false,
                2 => panic!("retain predicate failed"),
                _ => true,
            });
        }));
        assert!(result.is_err());
        assert_eq!(list.iter().map(|g| g.conn).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(list.connection_index(1), None);
        assert_eq!(list.identity_index(101, 11), None);
        check(&list);
        assert_eq!(
            list.simulation_indexed_iter()
                .map(|(i, g)| (i, g.conn))
                .collect::<Vec<_>>(),
            [(0, 2), (1, 3)]
        );
    }

    #[test]
    fn membership_changes_keep_both_indexes_and_iteration_order() {
        let mut list = GuestList::default();
        list.extend([guest(1), guest(2), guest(3)]);
        check(&list);
        let removed = list.remove(1);
        check(&list);
        assert_eq!(list.connection_index(2), None);
        assert_eq!(list.identity_index(102, 12), None);
        assert_eq!(list.iter().map(|g| g.conn).collect::<Vec<_>>(), [1, 3]);
        list.insert(1, removed);
        check(&list);
        list.replace(0, guest(4));
        check(&list);
        assert_eq!(list.connection_index(1), None);
        list.reverse();
        check(&list);
        list.update(0, |guest| {
            guest.placed = true;
            guest.position[0] = 42.0;
        });
        check(&list);
        list.retain(|g| g.conn != 2);
        check(&list);
        assert_eq!(list.connection_index(2), None);
        assert_eq!(list.identity_index(102, 12), None);
        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.connection_index(3), None);
        assert_eq!(list.identity_index(103, 13), None);
    }

    fn resident(
        list: &GuestList,
        at: crate::join_seq::residency::table::Location,
    ) -> Option<crate::join_seq::residency::table::SourceEntity<u32>> {
        use crate::join_seq::residency::{Source, table::SourceEntity};
        let Source::Guest(conn) = at.source else {
            return None;
        };
        let guest = &list[list.connection_index(conn)?];
        (guest.residency.location() == Some(at)).then_some(SourceEntity {
            id: guest.entity_id as u32,
            value: conn,
        })
    }

    #[test]
    fn actual_guest_loans_keep_cold_profile_identity_and_retire_before_id_reuse() {
        use crate::join_seq::residency::{
            Journal,
            table::{EntityLookup, EntityTable},
        };
        let mut list = GuestList::default();
        let mut table = EntityTable::new(Journal::default());
        list.bind_residency(&table.producer(), |_| mod_api::DimensionKey::OVERWORLD);
        list.extend([guest(1), guest(2)]);
        list.update(1, |g| g.kind = crate::lan::guest::GuestKind::Fake);
        assert!(!list[0].placed);
        let (original, state, details) = table
            .server_admitted_identity(11, |at| resident(&list, at))
            .unwrap();
        assert!(matches!(state, EntityLookup::Found { value: 1, .. }));
        assert_eq!(details.known_uuid, Some([0, 0, 0, 101]));
        let held = list.take_for_save(0);
        let (at, state, details) = table
            .server_admitted_identity(11, |at| resident(&list, at))
            .unwrap();
        assert_eq!(at, original);
        assert_eq!(state, EntityLookup::Taken);
        assert_eq!(details.known_uuid, Some([0, 0, 0, 101]));
        assert!(matches!(
            table.server_entity_lookup(12, |at| resident(&list, at)),
            EntityLookup::Found { value: 2, .. }
        ));
        list.insert(1, held);
        assert!(
            matches!(table.server_entity_lookup(11, |at| resident(&list, at)), EntityLookup::Found { location, value: 1 } if location == original)
        );
        drop(list.take_for_save(1));
        assert_eq!(
            table.server_entity_lookup::<()>(11, |_| panic!("retired guest must not resolve")),
            EntityLookup::Missing
        );
        list.update(0, |g| {
            g.entity_id = 11;
            g.uuid = 909;
        });
        let (replacement, state, details) = table
            .server_admitted_identity(11, |at| resident(&list, at))
            .unwrap();
        assert_ne!(replacement.join, original.join);
        assert!(matches!(state, EntityLookup::Found { value: 2, .. }));
        assert_eq!(details.known_uuid, Some([0, 0, 0, 909]));
        assert_eq!(
            table.server_entity_lookup::<()>(12, |_| panic!("old key must retire")),
            EntityLookup::Missing
        );
        list.clear();
        assert_eq!(
            table.server_entity_lookup::<()>(11, |_| panic!("cleared guest must not resolve")),
            EntityLookup::Missing
        );
        assert!(table.end_tick().is_ok());
    }

    #[test]
    fn pending_guest_admits_once_and_a_real_saved_record_rebinds_after_epoch_reset() {
        use crate::join_seq::residency::{
            Journal,
            table::{EntityLookup, EntityTable},
        };
        let mut list = GuestList::default();
        let mut pending = guest(1);
        pending.entity_id = 0;
        let join = pending.residency.join();
        let mut table = EntityTable::new(Journal::default());
        list.bind_residency(&table.producer(), |_| mod_api::DimensionKey::OVERWORLD);
        list.push(pending);
        assert_eq!(
            table.server_entity_lookup::<()>(0, |_| panic!("pending id is not admitted")),
            EntityLookup::Missing
        );
        list.update(0, |g| g.entity_id = 31);
        let (at, state, details) = table
            .server_admitted_identity(31, |at| resident(&list, at))
            .unwrap();
        assert_eq!(at.join, join);
        assert!(matches!(state, EntityLookup::Found { value: 1, .. }));
        assert_eq!(details.known_uuid, Some([0, 0, 0, 101]));
        let saved = list.take_for_save(0);
        table.reset();
        list.bind_residency(&table.producer(), |_| mod_api::DimensionKey::OVERWORLD);
        list.push(saved);
        let (at, state, details) = table
            .server_admitted_identity(31, |at| resident(&list, at))
            .unwrap();
        assert_eq!(at.join, join);
        assert!(matches!(state, EntityLookup::Found { value: 1, .. }));
        assert_eq!(details.known_uuid, Some([0, 0, 0, 101]));
        assert!(table.end_tick().is_ok());
    }

    #[test]
    fn key_and_kind_changes_repair_lookups_before_return_and_after_unwind() {
        let mut list = GuestList::default();
        list.extend([guest(1), guest(2)]);
        list.update(0, |row| {
            row.conn = 9;
            row.uuid = 900;
            row.entity_id = 90;
            row.kind = crate::lan::guest::GuestKind::Fake;
        });
        assert_eq!(list.connection_index(1), None);
        assert_eq!(list.identity_index(101, 11), None);
        assert_eq!(list.connection_index(9), Some(0));
        assert_eq!(list.identity_index(900, 90), Some(0));
        assert_eq!(
            list.simulation_iter()
                .map(|row| row.conn)
                .collect::<Vec<_>>(),
            [2]
        );
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            list.update(0, |row| {
                *row = guest(3);
                panic!("mutation fixture");
            });
        }));
        assert!(failed.is_err());
        assert_eq!(list.connection_index(9), None);
        assert_eq!(list.identity_index(900, 90), None);
        assert_eq!(list.connection_index(3), Some(0));
        assert_eq!(
            list.simulation_iter()
                .map(|row| row.conn)
                .collect::<Vec<_>>(),
            [3, 2]
        );
    }

    #[test]
    fn duplicate_keys_keep_first_match_after_supported_reorders_and_replacement() {
        let mut list = GuestList::default();
        list.extend([guest(1), guest(2), guest(1)]);
        list.swap(0, 1);
        assert_eq!(list.connection_index(1), Some(1));
        assert_eq!(list.identity_index(101, 11), Some(1));
        list.update(0, |row| *row = guest(1));
        assert_eq!(list.connection_index(1), Some(0));
        list.remove(0);
        assert_eq!(list.connection_index(1), Some(0));
        list.replace(0, guest(4));
        assert_eq!(list.connection_index(1), Some(1));
        list.update_all(|row| row.position[0] += 1.0);
        assert!(list.iter().all(|row| row.position[0] == 1.0));
        assert_eq!(list.connection_index(1), Some(1));
    }
}
