//! Per-dimension attachment indexes, retained independently of level swaps.

use super::{Dim, Game};
use horizon_mod_runtime::attachments::{
    AttachmentLifetime, Holder, index::AttachmentIndex, observer::IndexedAttachmentObserver,
};
use mod_api::{AttachmentHolder, ChunkRef};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub(super) type Indexes = HashMap<Dim, Arc<Mutex<AttachmentIndex<AttachmentHolder>>>>;

mod players;

#[cfg(test)]
mod attached_storage_tests;
#[cfg(test)]
mod idle_tests;
#[cfg(test)]
mod mob_tests;
#[cfg(test)]
mod painting_tests;
#[cfg(test)]
mod player_tests;
#[cfg(test)]
mod query_tests;
#[cfg(test)]
mod saved_entity_tests;

impl Game<'_> {
    pub(in crate::game) fn attachment_query(
        &self,
        namespace: &str,
        name: &str,
    ) -> mod_api::AttachmentBatch {
        let Some(index) = self.attachment_indexes.get(&self.dim) else {
            return horizon_mod_runtime::attachments::AttachmentBatch::empty().into();
        };
        let rows = index.lock().unwrap().snapshot_named(namespace, name);
        horizon_mod_runtime::attachments::AttachmentBatch::new(rows).into()
    }

    pub(in crate::game) fn attachment_validate_write(
        &self,
        token: mod_api::AttachmentWriteToken,
    ) -> Result<(), mod_api::AttachmentRowError> {
        let index = self
            .attachment_indexes
            .get(&self.dim)
            .ok_or(mod_api::AttachmentRowError::StaleHolder)?;
        token.validate(&index.lock().unwrap())
    }

    /// Publish saved contents only when an authoritative entity owns their lifetime.
    pub(super) fn bind_saved_entity_attachments(&mut self, id: u32, slot: &mut AttachmentLifetime) {
        if !self.native_authoritative() {
            return;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return;
        };
        let Some(map) = self
            .entity_attachments
            .get_mut(&id)
            .filter(|map| map.has_index_entries())
        else {
            return;
        };
        let index = self.attachment_indexes.entry(self.dim).or_default();
        slot.bind(
            map,
            index,
            AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)),
        );
    }

    /// Publish the dimension's saved maps before listeners observe its load.
    pub(super) fn bind_dimension_attachments(&mut self, dim: Dim) {
        let Some(level) = self.level(dim.id()) else {
            return;
        };
        let index = self.attachment_indexes.entry(dim).or_default();
        self.world
            .attachments
            .bind_index(index, AttachmentHolder::Level(level));
        self.world
            .chunks
            .observe_attachments(Arc::new(IndexedAttachmentObserver::new(
                index,
                move |holder| match holder {
                    Holder::Chunk(pos) => Some(AttachmentHolder::Chunk(ChunkRef::new(level, pos))),
                    _ => None,
                },
            )));
        self.world
            .block_entities
            .observe_attachments(Arc::new(IndexedAttachmentObserver::new(
                index,
                move |identity: horizon_client::block_entities::BlockEntityIdentity| {
                    Some(AttachmentHolder::BlockEntity(
                        mod_api::__private::game::block_entity_ref(
                            level,
                            identity.position,
                            identity.generation,
                        ),
                    ))
                },
            )));
    }
}

#[cfg(test)]
mod block_entity_tests;
#[cfg(test)]
mod boundary_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_entity_refs::tests::with_game;
    use {
        horizon_core::nbt::{Compound, Tag},
        horizon_mod_runtime::attachments::Attachments,
    };

    fn saved() -> Attachments {
        let mut names = Compound::new();
        names.insert("mana".into(), Tag::Compound(Compound::new()));
        let mut namespaces = Compound::new();
        namespaces.insert("test".into(), Tag::Compound(names));
        Attachments::from_nbt(namespaces)
    }

    #[test]
    #[ignore = "daily"]
    fn chunk_residency_uses_the_owning_level_through_cow_and_swaps() {
        use horizon_client::chunk::{Chunk, ChunkExtras};
        use horizon_mod_runtime::attachments::{AttachmentError, AttachmentType, Holder};
        use mod_api::HostTransaction;
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "mana",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let chunk = || {
                let mut chunk = Chunk::empty(2, 3);
                chunk.saved = Some(Arc::new(ChunkExtras {
                    attachments: saved(),
                    ..Default::default()
                }));
                Arc::new(chunk)
            };
            let key = ("test".into(), "mana".into());
            game.native_level_opened(Dim::Overworld);
            let level = game.level(Dim::Overworld.id()).unwrap();
            let overworld = game.attachment_indexes[&Dim::Overworld].clone();
            game.world.chunks.insert((2, 3), chunk());
            let old = overworld.lock().unwrap().snapshot(&key);
            assert_eq!(old.len(), 1);
            assert_eq!(
                old[0].holder,
                AttachmentHolder::Chunk(ChunkRef::new(level, [2, 3]))
            );
            let save = game.world.chunks[&(2, 3)].clone();
            Arc::make_mut(game.world.chunks.get_mut(&(2, 3)).unwrap()).lit = true;
            VALUE
                .set(
                    game.attachment_slot(
                        Holder::Chunk([2, 3]),
                        mod_api::AttachmentAccess::Existing,
                    )
                    .unwrap(),
                    Holder::Chunk([2, 3]),
                    13,
                )
                .unwrap();
            let current = overworld.lock().unwrap().snapshot(&key);
            assert_eq!(current[0].entry.read(&VALUE), Ok(&13));
            let aborted: Result<(), AttachmentError> = game.transaction(|host, _| {
                VALUE.set(host, Holder::Chunk([2, 3]), 14)?;
                let tentative = overworld.lock().unwrap().snapshot(&key);
                assert_eq!(tentative[0].entry.read(&VALUE), Ok(&14));
                Err(AttachmentError::MissingHolder)
            });
            assert!(aborted.is_err());
            let restored = overworld.lock().unwrap().snapshot(&key);
            assert_eq!(restored[0].entry.read(&VALUE), Ok(&13));
            assert!(!Arc::ptr_eq(&old[0].entry, &current[0].entry));
            assert!(Arc::ptr_eq(
                &old[0].entry,
                save.saved
                    .as_ref()
                    .unwrap()
                    .attachments
                    .entry("test", "mana")
                    .unwrap()
                    .unwrap()
            ));
            game.enter_level(Dim::Nether).unwrap();
            let nether_level = game.level(Dim::Nether.id()).unwrap();
            let nether = game.attachment_indexes[&Dim::Nether].clone();
            game.world.chunks.insert((2, 3), chunk());
            assert_eq!(
                nether.lock().unwrap().snapshot(&key)[0].holder,
                AttachmentHolder::Chunk(ChunkRef::new(nether_level, [2, 3]))
            );
            game.enter_level(Dim::Overworld).unwrap();
            game.world.chunks.remove(&(2, 3));
            assert!(overworld.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(nether.lock().unwrap().snapshot(&key).len(), 1);
            assert_eq!(current[0].entry.read(&VALUE), Ok(&13));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn loaded_levels_keep_independent_index_through_swaps_and_close() {
        with_game(|game| {
            let key = ("test".into(), "mana".into());
            game.world.attachments = saved();
            game.native_level_opened(Dim::Overworld);
            let overworld = game.attachment_indexes[&Dim::Overworld].clone();
            let old = overworld.lock().unwrap().snapshot(&key);
            assert_eq!(old.len(), 1);
            let old_level = game.level(Dim::Overworld.id()).unwrap();
            assert_eq!(old[0].holder, AttachmentHolder::Level(old_level));

            game.enter_level(Dim::Nether).unwrap();
            game.world.attachments = saved();
            game.bind_dimension_attachments(Dim::Nether);
            let nether = game.attachment_indexes[&Dim::Nether].clone();
            assert_eq!(nether.lock().unwrap().snapshot(&key).len(), 1);
            game.enter_level(Dim::Overworld).unwrap();
            assert!(Arc::ptr_eq(
                &overworld,
                &game.attachment_indexes[&Dim::Overworld]
            ));
            assert_eq!(overworld.lock().unwrap().snapshot(&key).len(), 1);
            game.native_levels_closing();
            assert!(game.attachment_indexes.is_empty());
            assert_eq!(old.len(), 1);
            game.native_lifecycle_open_started();
            assert!(game.attachment_indexes.is_empty());
            assert_ne!(game.native_lifecycle.epoch(), Some(old_level.epoch()));
        });
    }
}
