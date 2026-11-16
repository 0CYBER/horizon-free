//! Saved UUIDs by runtime ID. The level publishes changes to the residency index.
use horizon_client::join_seq::residency::Journal;
use mod_api::DimensionKey;
use std::{collections::HashMap, ops::Deref};

#[derive(Default)]
pub(in crate::game) struct EntityUuids {
    by_id: HashMap<u32, [i32; 4]>,
    binding: Option<(Journal, DimensionKey)>,
}

impl EntityUuids {
    pub(in crate::game) fn bind(&mut self, journal: &Journal, dimension: DimensionKey) {
        if let Some((old, previous)) = &self.binding
            && old.same_epoch(journal)
        {
            if *previous == dimension {
                return;
            }
            self.clear_binding();
        }
        self.binding = Some((journal.clone(), dimension));
        for (&id, &uuid) in &self.by_id {
            journal.set_saved_uuid(dimension, id, Some(uuid));
        }
    }

    pub(in crate::game) fn insert(&mut self, id: u32, uuid: [i32; 4]) -> Option<[i32; 4]> {
        let previous = self.by_id.insert(id, uuid);
        if previous != Some(uuid)
            && let Some((journal, dimension)) = &self.binding
        {
            journal.set_saved_uuid(*dimension, id, Some(uuid));
        }
        previous
    }

    pub(in crate::game) fn get_or_insert_with(
        &mut self,
        id: u32,
        make: impl FnOnce() -> [i32; 4],
    ) -> [i32; 4] {
        if let Some(&uuid) = self.by_id.get(&id) {
            return uuid;
        }
        let uuid = make();
        self.insert(id, uuid);
        uuid
    }

    pub(in crate::game) fn remove(&mut self, id: &u32) -> Option<[i32; 4]> {
        let previous = self.by_id.remove(id)?;
        if let Some((journal, dimension)) = &self.binding {
            journal.set_saved_uuid(*dimension, *id, None);
        }
        Some(previous)
    }

    pub(in crate::game) fn retain(&mut self, mut keep: impl FnMut(&u32, &[i32; 4]) -> bool) {
        self.by_id.retain(|id, uuid| {
            let kept = keep(id, uuid);
            if !kept && let Some((journal, dimension)) = &self.binding {
                journal.set_saved_uuid(*dimension, *id, None);
            }
            kept
        });
    }

    fn clear_binding(&mut self) {
        if let Some((journal, dimension)) = self.binding.take() {
            for &id in self.by_id.keys() {
                journal.set_saved_uuid(dimension, id, None);
            }
        }
    }
}

impl Deref for EntityUuids {
    type Target = HashMap<u32, [i32; 4]>;

    fn deref(&self) -> &Self::Target {
        &self.by_id
    }
}

impl Drop for EntityUuids {
    fn drop(&mut self) {
        self.clear_binding();
    }
}
