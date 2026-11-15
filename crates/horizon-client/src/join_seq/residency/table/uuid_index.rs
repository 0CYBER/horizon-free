//! Reverse lookup of the UUIDs of admitted entities, scoped to one level.
use super::{DimensionKey, ResidentRow, SavedUuidUpdate};
use crate::join_seq::residency::Kind;
use horizon_core::fast_hash::FxHashMap;
use std::{
    collections::{BTreeSet, hash_map::Entry},
    num::NonZeroU32,
    ops::Bound::{Excluded, Unbounded},
};

type LevelUuid = (DimensionKey, [i32; 4]);

enum Residents {
    One(NonZeroU32),
    // A departure can still be indexed when the replacement is admitted.
    Multiple(BTreeSet<NonZeroU32>),
}

#[derive(Default)]
pub(super) struct UuidIndex {
    saved: FxHashMap<(DimensionKey, NonZeroU32), [i32; 4]>,
    residents: FxHashMap<LevelUuid, Residents>,
}

impl UuidIndex {
    fn key(&self, id: NonZeroU32, row: ResidentRow) -> Option<LevelUuid> {
        let owner = row.location.owner;
        if owner.kind == Kind::DragonPart {
            return None;
        }
        let uuid = row
            .details
            .known_uuid
            .or_else(|| self.saved.get(&(owner.dimension, id)).copied())?;
        Some((owner.dimension, uuid))
    }

    pub(super) fn admit(&mut self, id: NonZeroU32, row: ResidentRow) {
        if let Some(key) = self.key(id, row) {
            self.insert(key, id);
        }
    }

    pub(super) fn retire(&mut self, id: NonZeroU32, row: ResidentRow) {
        if let Some(key) = self.key(id, row) {
            self.remove(key, id);
        }
    }

    pub(super) fn remember(&mut self, update: SavedUuidUpdate, row: Option<ResidentRow>) {
        let SavedUuidUpdate {
            dimension,
            id,
            uuid,
        } = update;
        let before = row.and_then(|row| self.key(id, row));
        if let Some(uuid) = uuid {
            self.saved.insert((dimension, id), uuid);
        } else {
            self.saved.remove(&(dimension, id));
        }
        let after = row.and_then(|row| self.key(id, row));
        if before == after {
            return;
        }
        if let Some(key) = before {
            self.remove(key, id);
        }
        if let Some(key) = after {
            self.insert(key, id);
        }
    }

    pub(super) fn candidate(
        &self,
        dimension: DimensionKey,
        uuid: [i32; 4],
        after: Option<NonZeroU32>,
    ) -> Option<NonZeroU32> {
        match self.residents.get(&(dimension, uuid))? {
            Residents::One(id) => after.is_none_or(|after| *id > after).then_some(*id),
            Residents::Multiple(ids) => match after {
                Some(after) => ids.range((Excluded(after), Unbounded)).next().copied(),
                None => ids.first().copied(),
            },
        }
    }

    fn insert(&mut self, key: LevelUuid, id: NonZeroU32) {
        match self.residents.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(Residents::One(id));
            }
            Entry::Occupied(mut entry) => match entry.get_mut() {
                Residents::One(existing) if *existing != id => {
                    let ids = BTreeSet::from([*existing, id]);
                    entry.insert(Residents::Multiple(ids));
                }
                Residents::Multiple(ids) => {
                    ids.insert(id);
                }
                Residents::One(_) => {}
            },
        }
    }

    fn remove(&mut self, key: LevelUuid, id: NonZeroU32) {
        let Entry::Occupied(mut entry) = self.residents.entry(key) else {
            return;
        };
        match entry.get_mut() {
            Residents::One(existing) if *existing == id => {
                entry.remove();
            }
            Residents::Multiple(ids) => {
                ids.remove(&id);
                if ids.len() == 1
                    && let Some(&remaining) = ids.first()
                {
                    entry.insert(Residents::One(remaining));
                }
            }
            Residents::One(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_seq::{
        JoinSeq,
        residency::{
            Change, Event, Identity, Journal, Owner,
            table::{EntityLookup, EntityTable, SourceEntity},
        },
    };

    const HOME: DimensionKey = DimensionKey::OVERWORLD;
    const AWAY: DimensionKey = DimensionKey::NETHER;
    const FIRST: [i32; 4] = [1, 2, 3, 4];
    const SECOND: [i32; 4] = [5, 6, 7, 8];

    fn publish(journal: &Journal, dimension: DimensionKey, kind: Kind, change: Change) {
        journal.push(Event {
            owner: Owner { dimension, kind },
            change,
        });
    }

    #[test]
    fn uuid_candidates_follow_saved_changes_and_resident_ownership() {
        let journal = Journal::default();
        let mut table = EntityTable::new(journal.clone());
        let entity = Identity::new(21, JoinSeq::next()).unwrap();
        let id = entity.id.get();

        journal.set_saved_uuid(HOME, id, Some(FIRST));
        assert_eq!(table.level_uuid_candidate(HOME, FIRST, None), None);
        publish(&journal, HOME, Kind::Mob, Change::Install(entity));
        assert_eq!(
            table.level_uuid_candidate(HOME, FIRST, None),
            Some(entity.id)
        );
        assert_eq!(table.level_uuid_candidate(AWAY, FIRST, None), None);

        journal.set_saved_uuid(HOME, id, Some(SECOND));
        assert_eq!(table.level_uuid_candidate(HOME, FIRST, None), None);
        assert_eq!(
            table.level_uuid_candidate(HOME, SECOND, None),
            Some(entity.id)
        );
        publish(&journal, HOME, Kind::Mob, Change::Remove(entity));
        assert_eq!(table.level_uuid_candidate(HOME, SECOND, None), None);

        journal.set_saved_uuid(HOME, id, None);
        journal.set_saved_uuid(AWAY, id, Some(SECOND));
        publish(&journal, AWAY, Kind::Mob, Change::Install(entity));
        assert_eq!(
            table.level_uuid_candidate(AWAY, SECOND, None),
            Some(entity.id)
        );
        assert_eq!(table.level_uuid_candidate(HOME, SECOND, None), None);

        assert_eq!(
            table.server_entity_lookup(id, |_| None::<SourceEntity<()>>),
            EntityLookup::Missing
        );
        assert_eq!(table.level_uuid_candidate(AWAY, SECOND, None), None);

        table.reset();
        let fresh = table.producer();
        publish(&fresh, AWAY, Kind::Mob, Change::Install(entity));
        assert_eq!(table.level_uuid_candidate(AWAY, SECOND, None), None);
    }

    #[test]
    fn uuid_candidates_keep_the_replacement_and_exclude_dragon_parts() {
        let journal = Journal::default();
        let mut table = EntityTable::new(journal.clone());
        let dragon = Identity::new(20, JoinSeq::next())
            .unwrap()
            .with_known_uuid(Some(FIRST));
        let part = Identity::new(21, JoinSeq::next())
            .unwrap()
            .with_known_uuid(Some(SECOND));
        let replacement = Identity::new(50, JoinSeq::next()).unwrap();
        journal.set_saved_uuid(HOME, replacement.id.get(), Some(FIRST));
        journal.set_saved_uuid(HOME, part.id.get(), Some(SECOND));
        publish(&journal, HOME, Kind::Dragon, Change::Install(dragon));
        publish(&journal, HOME, Kind::DragonPart, Change::Install(part));
        publish(&journal, HOME, Kind::Mob, Change::Install(replacement));

        assert_eq!(
            table.level_uuid_candidate(HOME, FIRST, None),
            Some(dragon.id)
        );
        assert_eq!(
            table.level_uuid_candidate(HOME, FIRST, Some(dragon.id)),
            Some(replacement.id)
        );
        assert_eq!(table.level_uuid_candidate(HOME, SECOND, None), None);

        publish(&journal, HOME, Kind::Dragon, Change::Remove(dragon));
        assert_eq!(
            table.level_uuid_candidate(HOME, FIRST, None),
            Some(replacement.id)
        );
    }
}
