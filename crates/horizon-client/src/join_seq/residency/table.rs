//! A journal-fed index. Every hit is checked against its named storage owner;
//! there is no owner reconciliation scan or reference back to the game.
use super::{
    Change, DimensionKey, Event, Identity, IdentityDetails, Journal, Owner, SavedUuidUpdate,
};

#[path = "table/uuid_index.rs"]
mod uuid_index;
use crate::join_seq::JoinSeq;
use horizon_core::fast_hash::FxHashMap;
use std::num::NonZeroU32;
use uuid_index::UuidIndex;

/// Stable storage identity, never a cached element index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Location {
    pub owner: Owner,
    pub join: JoinSeq,
    pub source: super::Source,
}

/// A candidate obtained by locating the join in exactly the named owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceEntity<T> {
    pub id: u32,
    pub value: T,
}

/// A validated result or a narrowly scoped temporary absence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityLookup<T> {
    Missing,
    Taken,
    Found { location: Location, value: T },
}

/// Unfinished operations expired at the tick boundary, never retained as Taken.
/// `lifts` counts outstanding passes, including nested passes of the same owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnbalancedPass {
    pub extractions: usize,
    pub lifts: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ResidentRow {
    location: Location,
    details: IdentityDetails,
    admission: JoinSeq,
}

/// One game's index, shared by active and parked dimensions through the journal.
/// Runtime ids are allocated by the game's one counter, not per dimension.
pub struct EntityTable {
    journal: Journal,
    pending: Vec<Event>,
    saved_uuid_updates: Vec<SavedUuidUpdate>,
    uuids: UuidIndex,
    rows: FxHashMap<NonZeroU32, ResidentRow>,
    extracted: FxHashMap<NonZeroU32, Location>,
    lifted: FxHashMap<Owner, usize>,
}
impl EntityTable {
    /// Observe this game's existing structural-event sink.
    pub fn new(journal: Journal) -> Self {
        Self {
            journal,
            pending: Vec::new(),
            saved_uuid_updates: Vec::new(),
            uuids: UuidIndex::default(),
            rows: FxHashMap::default(),
            extracted: FxHashMap::default(),
            lifted: FxHashMap::default(),
        }
    }
    /// Check the current owner and id once, returning its already located value.
    /// The callback must only inspect the supplied owner, not scan other kinds
    /// or dimensions. A missing or mismatched source retires the stale cache row.
    pub fn entity_lookup<T>(
        &mut self,
        dimension: &DimensionKey,
        id: u32,
        locate: impl FnOnce(Location) -> Option<SourceEntity<T>>,
    ) -> EntityLookup<T> {
        self.lookup(Some(dimension), id, locate)
    }
    /// Resolve an id across all loaded dimensions. The resolver still validates
    /// exactly the named owner; it must not swap levels or search other owners.
    pub fn server_entity_lookup<T>(
        &mut self,
        id: u32,
        locate: impl FnOnce(Location) -> Option<SourceEntity<T>>,
    ) -> EntityLookup<T> {
        self.lookup(None, id, locate)
    }
    fn lookup<T>(
        &mut self,
        dimension: Option<&DimensionKey>,
        id: u32,
        locate: impl FnOnce(Location) -> Option<SourceEntity<T>>,
    ) -> EntityLookup<T> {
        let Some(id) = NonZeroU32::new(id) else {
            return EntityLookup::Missing;
        };
        self.drain();
        let Some(&ResidentRow { location, .. }) = self.rows.get(&id) else {
            return EntityLookup::Missing;
        };
        if dimension.is_some_and(|dimension| location.owner.dimension != *dimension) {
            return EntityLookup::Missing;
        }
        if let Some(source) = locate(location) {
            if source.id == id.get() {
                return EntityLookup::Found {
                    location,
                    value: source.value,
                };
            }
            // Finding a different entity is corruption, not a lifted absence.
        } else if self.extracted.get(&id) == Some(&location)
            || self.lifted.contains_key(&location.owner)
        {
            return EntityLookup::Taken;
        }
        forget_row(&mut self.rows, &mut self.uuids, id, location);
        forget(&mut self.extracted, id, location);
        EntityLookup::Missing
    }
    /// Apply pending operations in mutation order, reusing the event buffer.
    pub fn drain(&mut self) {
        self.journal
            .drain_batches(&mut self.pending, Some(&mut self.saved_uuid_updates));
        // Saved UUIDs and residency are independent inputs. A UUID may be
        // written before an owner admits the object, or while it is resident.
        for update in self.saved_uuid_updates.drain(..) {
            self.uuids
                .remember(update, self.rows.get(&update.id).copied());
        }
        for event in self.pending.drain(..) {
            let location = |identity: Identity| Location {
                owner: event.owner,
                join: identity.join,
                source: identity.source,
            };
            match event.change {
                Change::Install(identity) | Change::PutBack(identity) => {
                    let location = location(identity);
                    let row = ResidentRow {
                        location,
                        details: identity.details,
                        admission: self
                            .rows
                            .get(&identity.id)
                            .map_or_else(JoinSeq::next, |row| row.admission),
                    };
                    if let Some(old) = self.rows.insert(identity.id, row) {
                        debug_assert_eq!(old.location, location, "duplicate loaded entity id");
                        debug_assert_eq!(
                            old.details, identity.details,
                            "immutable identity details changed without replacement"
                        );
                    } else {
                        self.uuids.admit(identity.id, row);
                    }
                    self.extracted.remove(&identity.id);
                }
                Change::Remove(identity) => {
                    let location = location(identity);
                    forget_row(&mut self.rows, &mut self.uuids, identity.id, location);
                    forget(&mut self.extracted, identity.id, location);
                }
                Change::Extract(identity) => {
                    let location = location(identity);
                    if self.rows.get(&identity.id).map(|row| row.location) == Some(location) {
                        self.extracted.insert(identity.id, location);
                    }
                }
                Change::Lifted => {
                    *self.lifted.entry(event.owner).or_default() += 1;
                }
                Change::Restored => {
                    if let Some(depth) = self.lifted.get_mut(&event.owner) {
                        *depth -= 1;
                        if *depth == 0 {
                            self.lifted.remove(&event.owner);
                        }
                    }
                }
            }
        }
    }
    /// Drain and expire open markers before reporting an unfinished operation.
    /// Log the counts in every build; debug builds then assert, while release
    /// returns a typed error. Later lookups never remain permanently Taken.
    /// Valid ticks allocate nothing; work scales with markers, not resident rows.
    pub fn end_tick(&mut self) -> Result<(), UnbalancedPass> {
        self.drain();
        let leaked = self.extracted.len() + self.lifted.len();
        let unfinished = (leaked != 0).then(|| UnbalancedPass {
            extractions: self.extracted.len(),
            lifts: self.lifted.values().sum(),
        });
        self.extracted.clear();
        self.lifted.clear();
        if let Some(error) = unfinished {
            log::error!("entity residency operation did not finish: {error:?}");
            debug_assert_eq!(leaked, 0, "entity residency operation did not finish");
            return Err(error);
        }
        Ok(())
    }
    /// Resolve once and return immutable metadata from that same primary row.
    /// Real loans keep metadata even without an available physical payload.
    pub fn server_admitted_identity<T>(
        &mut self,
        id: u32,
        locate: impl FnOnce(Location) -> Option<SourceEntity<T>>,
    ) -> Option<(Location, EntityLookup<T>, IdentityDetails)> {
        let state = self.lookup(None, id, locate);
        if matches!(state, EntityLookup::Missing) {
            return None;
        }
        let row = self.rows.get(&NonZeroU32::new(id)?)?;
        Some((row.location, state, row.details))
    }

    /// Candidate-only metadata, including Taken. Collect into a reusable buffer
    /// and release this borrow before invoking any source or UUID resolver.
    pub fn admitted_metadata(
        &mut self,
    ) -> impl Iterator<Item = (u32, Location, IdentityDetails)> + '_ {
        self.drain();
        self.rows
            .iter()
            .map(|(id, row)| (id.get(), row.location, row.details))
    }

    /// The next admitted ID for this level and UUID. The caller still checks
    /// the owner and visibility, including temporarily taken payloads.
    pub fn level_uuid_candidate(
        &mut self,
        dimension: DimensionKey,
        uuid: [i32; 4],
        after: Option<NonZeroU32>,
    ) -> Option<NonZeroU32> {
        self.drain();
        self.uuids.candidate(dimension, uuid, after)
    }

    /// Candidates in lookup insertion order, independent of family and runtime id.
    /// Extraction/restoration preserves order; removal and re-admission moves to the end.
    /// Dragon parts are not members of vanilla's entity lookup.
    pub fn level_entities(&mut self, dimension: DimensionKey) -> Vec<u32> {
        self.drain();
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .filter(|(_, row)| {
                row.location.owner.dimension == dimension
                    && row.location.owner.kind != super::Kind::DragonPart
            })
            .map(|(id, row)| (row.admission, id.get()))
            .collect();
        rows.sort_unstable_by_key(|(admission, _)| *admission);
        rows.into_iter().map(|(_, id)| id).collect()
    }

    /// Each admitted id's lookup admission: what an `EntitySection` keeps its
    /// entities in, so a chunk saved in this order loads back in it.
    pub fn admission_order(&mut self) -> impl Fn(u32) -> Option<JoinSeq> + '_ {
        self.drain();
        |id| Some(self.rows.get(&NonZeroU32::new(id)?)?.admission)
    }

    /// Clone the current producer authority when binding storage.
    pub fn producer(&self) -> Journal {
        self.journal.clone()
    }
    /// Retire the epoch and clear cached state. Rebind every retained owner.
    pub fn reset(&mut self) {
        self.journal.reset_epoch();
        self.pending.clear();
        self.saved_uuid_updates.clear();
        self.uuids = UuidIndex::default();
        self.rows.clear();
        self.extracted.clear();
        self.lifted.clear();
    }
}
fn forget_row(
    rows: &mut FxHashMap<NonZeroU32, ResidentRow>,
    uuids: &mut UuidIndex,
    id: NonZeroU32,
    location: Location,
) {
    if let Some(&row) = rows.get(&id)
        && row.location == location
    {
        uuids.retire(id, row);
        rows.remove(&id);
    }
}
fn forget(map: &mut FxHashMap<NonZeroU32, Location>, id: NonZeroU32, location: Location) {
    if map.get(&id) == Some(&location) {
        map.remove(&id);
    }
}

#[cfg(test)]
#[path = "table/completion_tests.rs"]
mod completion_tests;
#[cfg(test)]
#[path = "table/metadata_tests.rs"]
mod metadata_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join_seq::residency::{DimensionKey, Kind};
    use crate::join_seq::{JoinList, Joined, RestoreOrder};
    const HOME: Owner = Owner {
        dimension: DimensionKey::OVERWORLD,
        kind: Kind::Mob,
    };

    struct Fixture {
        journal: Journal,
        table: EntityTable,
        source: FxHashMap<Location, SourceEntity<usize>>,
        dimension: DimensionKey,
    }
    impl Fixture {
        fn new() -> Self {
            let journal = Journal::default();
            Self {
                table: EntityTable::new(journal.clone()),
                journal,
                source: FxHashMap::default(),
                dimension: HOME.dimension,
            }
        }
        fn admit(&mut self, id: u32, owner: Owner) -> Location {
            let location = Location {
                owner,
                join: JoinSeq::next(),
                source: Default::default(),
            };
            self.source.insert(location, SourceEntity { id, value: 42 });
            self.emit(
                location,
                Change::Install(Identity::new(id, location.join).unwrap()),
            );
            location
        }
        fn emit(&self, location: Location, change: Change) {
            self.journal.push(Event {
                owner: location.owner,
                change,
            });
        }
        fn entity_lookup(&mut self, id: u32) -> EntityLookup<usize> {
            self.table.entity_lookup(&self.dimension, id, |location| {
                self.source.get(&location).copied()
            })
        }
    }

    #[test]
    fn lookup_iteration_preserves_insertion_order_not_id_or_family_order() {
        let mut f = Fixture::new();
        let first = f.admit(90, HOME);
        let boat = Owner {
            kind: Kind::Boat,
            ..HOME
        };
        f.admit(3, boat);
        f.admit(40, HOME);
        f.admit(
            2,
            Owner {
                kind: Kind::DragonPart,
                ..HOME
            },
        );
        f.admit(
            1,
            Owner {
                dimension: DimensionKey::NETHER,
                ..HOME
            },
        );
        // Also compared to the runtime vanilla EntityLookup by the parity harness.
        println!("lookup-order: {:?}", f.table.level_entities(HOME.dimension));
        assert_eq!(f.table.level_entities(HOME.dimension), vec![90, 3, 40]);
        let identity = Identity::new(90, first.join).unwrap();
        f.emit(first, Change::Extract(identity));
        f.emit(first, Change::PutBack(identity));
        println!("lookup-order: {:?}", f.table.level_entities(HOME.dimension));
        assert_eq!(f.table.level_entities(HOME.dimension), vec![90, 3, 40]);
        f.emit(first, Change::Remove(identity));
        f.emit(first, Change::Install(identity));
        println!("lookup-order: {:?}", f.table.level_entities(HOME.dimension));
        assert_eq!(f.table.level_entities(HOME.dimension), vec![3, 40, 90]);
        f.emit(first, Change::Remove(identity));
        let away = f.admit(
            90,
            Owner {
                dimension: DimensionKey::NETHER,
                ..HOME
            },
        );
        println!("lookup-order: {:?}", f.table.level_entities(HOME.dimension));
        println!(
            "lookup-order: {:?}",
            f.table.level_entities(DimensionKey::NETHER)
        );
        f.emit(away, Change::Remove(Identity::new(90, away.join).unwrap()));
        f.admit(90, HOME);
        println!("lookup-order: {:?}", f.table.level_entities(HOME.dimension));
    }

    #[test]
    fn server_lookup_follows_a_transfer_without_a_dimension_scan() {
        let mut f = Fixture::new();
        let old = f.admit(7, HOME);
        let away = Owner {
            dimension: DimensionKey::NETHER,
            kind: Kind::Boat,
        };
        f.emit(old, Change::Remove(Identity::new(7, old.join).unwrap()));
        f.source.remove(&old);
        let arrived = f.admit(7, away);
        assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
        assert_eq!(
            f.table.server_entity_lookup(7, |at| {
                assert_eq!(at, arrived);
                f.source.get(&at).copied()
            }),
            EntityLookup::Found {
                location: arrived,
                value: 42
            }
        );
        f.emit(arrived, Change::Lifted);
        f.source.remove(&arrived);
        assert_eq!(
            f.table.server_entity_lookup::<()>(7, |_| None),
            EntityLookup::Taken
        );
        assert_eq!(
            f.table
                .server_entity_lookup::<()>(0, |_| panic!("zero reached storage")),
            EntityLookup::Missing
        );
    }

    struct Stored(u32, JoinSeq);
    impl Joined for Stored {
        fn join_seq(&self) -> JoinSeq {
            self.1
        }
        fn set_join_seq(&mut self, join: JoinSeq) {
            self.1 = join;
        }
    }
    fn locate(list: &JoinList<Stored>, at: Location) -> Option<SourceEntity<usize>> {
        list.locate(at.join).map(|i| SourceEntity {
            id: list[i].0,
            value: i,
        })
    }

    #[test]
    fn a_spawn_during_a_real_lift_is_found_while_the_survivor_is_taken() {
        let mut list = JoinList::default();
        list.push(Stored(7, JoinSeq::UNJOINED));
        let mut table = EntityTable::new(Journal::default());
        list.bind_residency(&table.producer(), HOME, |v| (v.0, v.1));
        let first = Location {
            owner: HOME,
            join: list[0].1,
            source: Default::default(),
        };
        let taken = list.lift();
        assert_eq!(
            table.entity_lookup(&HOME.dimension, 7, |at| locate(&list, at)),
            EntityLookup::Taken
        );
        list.push(Stored(8, JoinSeq::UNJOINED));
        let second = Location {
            owner: HOME,
            join: list[0].1,
            source: Default::default(),
        };
        assert_eq!(
            table.entity_lookup(&HOME.dimension, 8, |at| locate(&list, at)),
            EntityLookup::Found {
                location: second,
                value: 0
            }
        );
        list.restore(taken, RestoreOrder::SurvivorsFirst);
        table.end_tick().unwrap();
        assert_eq!(
            table.entity_lookup(&HOME.dimension, 7, |at| locate(&list, at)),
            EntityLookup::Found {
                location: first,
                value: 0
            }
        );
        assert_eq!(
            table.entity_lookup(&HOME.dimension, 8, |at| locate(&list, at)),
            EntityLookup::Found {
                location: second,
                value: 1
            }
        );
    }

    #[test]
    fn nested_real_lifts_remain_taken_until_the_outer_restore() {
        let mut list = JoinList::default();
        list.push(Stored(7, JoinSeq::UNJOINED));
        let mut table = EntityTable::new(Journal::default());
        list.bind_residency(&table.producer(), HOME, |v| (v.0, v.1));
        let outer = list.lift();
        let inner = list.lift();
        list.restore(inner, RestoreOrder::SurvivorsFirst);
        assert_eq!(
            table.entity_lookup(&HOME.dimension, 7, |at| locate(&list, at)),
            EntityLookup::Taken
        );
        list.restore(outer, RestoreOrder::SurvivorsFirst);
        assert!(matches!(
            table.entity_lookup(&HOME.dimension, 7, |at| locate(&list, at)),
            EntityLookup::Found { value: 0, .. }
        ));
        table.end_tick().unwrap();
    }

    #[test]
    fn validated_hits_keep_the_located_value_and_purge_both_kinds_of_stale_row() {
        let mut f = Fixture::new();
        let location = f.admit(7, HOME);
        assert_eq!(
            f.entity_lookup(7),
            EntityLookup::Found {
                location,
                value: 42
            }
        );
        for wrong_source in [None, Some(SourceEntity { id: 8, value: 99 })] {
            match wrong_source {
                None => {
                    f.source.remove(&location);
                }
                Some(source) => {
                    f.source.insert(location, source);
                }
            }
            assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
            // A physical row alone must not resurrect a purged cache entry.
            f.source.insert(location, SourceEntity { id: 7, value: 42 });
            assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
            f.emit(
                location,
                Change::Install(Identity::new(7, location.join).unwrap()),
            );
            assert_eq!(
                f.entity_lookup(7),
                EntityLookup::Found {
                    location,
                    value: 42
                }
            );
        }
        f.admit(1, HOME);
        assert_eq!(
            f.table
                .entity_lookup::<()>(&HOME.dimension, 0, |_| panic!("zero id inspected storage")),
            EntityLookup::Missing
        );
    }

    #[test]
    fn extraction_and_lift_only_hide_missing_sources_and_finish_in_order() {
        let mut f = Fixture::new();
        let location = f.admit(7, HOME);
        let identity = Identity::new(7, location.join).unwrap();
        let source = f.source.remove(&location).unwrap();
        f.emit(location, Change::Extract(identity));
        assert_eq!(f.entity_lookup(7), EntityLookup::Taken);
        f.source.insert(location, source);
        // Exposed replay storage is readable even before its marker closes.
        assert!(
            matches!(f.entity_lookup(7), EntityLookup::Found { location: at, .. } if at == location)
        );
        f.emit(location, Change::PutBack(identity));
        f.table.end_tick().unwrap();
        f.source.remove(&location);
        f.emit(location, Change::Lifted);
        let spawned = f.admit(8, HOME);
        assert_eq!(f.entity_lookup(7), EntityLookup::Taken);
        assert_eq!(
            f.entity_lookup(8),
            EntityLookup::Found {
                location: spawned,
                value: 42
            }
        );
        f.source.insert(location, source);
        f.emit(location, Change::Restored);
        f.table.end_tick().unwrap();
        assert!(
            matches!(f.entity_lookup(7), EntityLookup::Found { location: at, .. } if at == location)
        );
        // Retirement is authoritative, even if an unreported old view remains.
        f.emit(location, Change::Extract(identity));
        f.emit(location, Change::Remove(identity));
        assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
        f.table.end_tick().unwrap();
        f.source.remove(&location);
    }

    #[test]
    fn an_id_mismatch_is_not_disguised_as_a_temporary_absence() {
        for lift in [false, true] {
            let mut f = Fixture::new();
            let location = f.admit(7, HOME);
            f.source.get_mut(&location).unwrap().id = 8;
            let identity = Identity::new(7, location.join).unwrap();
            f.emit(
                location,
                if lift {
                    Change::Lifted
                } else {
                    Change::Extract(identity)
                },
            );
            assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
            f.source.get_mut(&location).unwrap().id = 7;
            assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
            if lift {
                f.emit(location, Change::Restored);
            }
            f.table.end_tick().unwrap();
        }
    }

    #[test]
    fn parked_transfer_and_late_retirement_cannot_erase_a_new_owner() {
        let mut f = Fixture::new();
        let old = f.admit(7, HOME);
        assert!(matches!(f.entity_lookup(7), EntityLookup::Found { .. }));
        let new = Location {
            owner: Owner {
                dimension: DimensionKey::NETHER,
                kind: Kind::Other,
            },
            ..old
        };
        let source = f.source.remove(&old).unwrap();
        let identity = Identity::new(7, old.join).unwrap();
        f.emit(old, Change::Remove(identity));
        f.source.insert(new, source);
        f.dimension = new.owner.dimension;
        f.emit(new, Change::Install(identity));
        assert_eq!(
            f.table.entity_lookup::<()>(&HOME.dimension, 7, |_| panic!(
                "wrong dimension inspected storage"
            )),
            EntityLookup::Missing
        );
        assert!(
            matches!(f.entity_lookup(7), EntityLookup::Found { location, .. } if location == new)
        );
        f.emit(old, Change::Remove(identity));
        assert!(
            matches!(f.entity_lookup(7), EntityLookup::Found { location, .. } if location == new)
        );
        f.source.remove(&new);
        f.emit(new, Change::Remove(identity));
        let replacement = f.admit(7, new.owner);
        f.emit(new, Change::Remove(identity));
        assert!(
            matches!(f.entity_lookup(7), EntityLookup::Found { location, .. } if location == replacement)
        );
    }

    #[test]
    fn end_tick_expires_leaks_before_debug_panics_and_release_lookups_miss() {
        for lift in [false, true] {
            let mut f = Fixture::new();
            let location = f.admit(7, HOME);
            f.source.remove(&location);
            let identity = Identity::new(7, location.join).unwrap();
            f.emit(
                location,
                if lift {
                    Change::Lifted
                } else {
                    Change::Extract(identity)
                },
            );
            assert_eq!(f.entity_lookup(7), EntityLookup::Taken);
            f.admit(8, HOME);
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.table.end_tick()));
            assert_eq!(result.is_err(), cfg!(debug_assertions));
            if !cfg!(debug_assertions) {
                assert_eq!(
                    result.unwrap(),
                    Err(UnbalancedPass {
                        extractions: usize::from(!lift),
                        lifts: usize::from(lift),
                    })
                );
            }
            let mut leftover = Vec::new();
            f.journal.drain_into(&mut leftover);
            assert!(leftover.is_empty(), "end_tick did not drain the journal");
            assert!(f.table.pending.is_empty());
            assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
            f.table.end_tick().unwrap();
        }
    }

    #[test]
    fn reset_discards_pending_state_and_requires_fresh_producers() {
        let mut f = Fixture::new();
        let old = f.admit(7, HOME);
        assert!(matches!(f.entity_lookup(7), EntityLookup::Found { .. }));
        f.emit(old, Change::Extract(Identity::new(7, old.join).unwrap()));
        f.emit(old, Change::Lifted);
        f.table.drain();
        f.admit(9, HOME);
        f.table.reset();
        assert_eq!(f.entity_lookup(7), EntityLookup::Missing);
        assert_eq!(f.entity_lookup(9), EntityLookup::Missing);
        f.table.end_tick().unwrap();
        f.admit(8, HOME);
        assert_eq!(f.entity_lookup(8), EntityLookup::Missing);
        f.journal = f.table.producer();
        let new = f.admit(8, HOME);
        assert_eq!(
            f.entity_lookup(8),
            EntityLookup::Found {
                location: new,
                value: 42
            }
        );
    }
}
