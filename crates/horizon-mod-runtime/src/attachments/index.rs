//! Dense attachment membership with immutable, shared-entry snapshots.
use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{
    AttachmentEntry,
    index_key::{BucketKey, Lookup, Named},
};
use horizon_core::{fast_hash::FxHashMap, nbt::JavaString};

/// An already-resolved attachment namespace and name.
pub type AttachmentKey = (JavaString, JavaString);

/// A loaded holder and the immutable value it had when a snapshot was acquired.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentRow<H> {
    /// The ready-to-use holder identity; iteration needs no lookup or conversion.
    pub holder: H,
    /// Shared with the holder's attachment map, never copied by snapshot acquisition.
    pub entry: Arc<AttachmentEntry>,
    generation: u64,
}

/// Dense rows and residency probes captured as one immutable bucket version.
#[derive(Clone)]
pub struct AttachmentSnapshot<H> {
    rows: Vec<AttachmentRow<H>>,
    slots: FxHashMap<H, usize>,
}
impl<H> Default for AttachmentSnapshot<H> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            slots: FxHashMap::default(),
        }
    }
}
impl<H> std::ops::Deref for AttachmentSnapshot<H> {
    type Target = Vec<AttachmentRow<H>>;
    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}
impl<H: Copy + Eq + Hash> AttachmentSnapshot<H> {
    /// Build a captured bucket from transferred rows, preserving their order.
    ///
    /// rows must have distinct holders.
    pub fn from_rows(rows: Vec<AttachmentRow<H>>) -> Self {
        let mut snapshot = Self::default();
        for row in rows {
            snapshot.push(row);
        }
        snapshot
    }

    fn push(&mut self, row: AttachmentRow<H>) {
        let previous = self.slots.insert(row.holder, self.rows.len());
        debug_assert!(previous.is_none(), "duplicate attachment residency");
        self.rows.push(row);
    }
    fn replace(&mut self, slot: usize, row: AttachmentRow<H>) -> AttachmentRow<H> {
        let old = std::mem::replace(&mut self.rows[slot], row);
        self.slots.remove(&old.holder);
        let row = &self.rows[slot];
        self.slots.insert(row.holder, slot);
        old
    }
    fn swap_remove(&mut self, slot: usize) -> AttachmentRow<H> {
        let old = self.rows.swap_remove(slot);
        self.slots.remove(&old.holder);
        if let Some(row) = self.rows.get(slot) {
            self.slots.insert(row.holder, slot);
        }
        reclaim_sparse_map(&mut self.slots);
        old
    }
    /// Probe captured membership by holder in expected O(1) time.
    pub fn slot_for(&self, holder: H) -> Option<usize> {
        self.slots.get(&holder).copied()
    }

    /// Probe captured membership using the existing residency token.
    pub fn slot_of(&self, owner: &AttachmentOwner<H>) -> Option<usize> {
        self.slot_for(owner.holder)
            .filter(|&slot| self.rows[slot].generation == owner.generation)
    }
}

/// Authority to publish one registration's changes. Cloning or dropping this token has
/// no membership effect. Re-registering the holder invalidates all earlier tokens.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentOwner<H> {
    holder: H,
    generation: u64,
}

/// A mutation result and the displaced or rejected entries it keeps alive.
/// Always move the whole change out of the index guard's scope before extracting
/// its result or dropping it: an entry's final Drop may execute arbitrary mod code.
#[must_use = "release the index guard before extracting or dropping this change"]
#[non_exhaustive]
pub struct AttachmentChange<T> {
    result: T,
    first: Option<Arc<AttachmentEntry>>,
    overflow: Vec<Arc<AttachmentEntry>>,
}

impl<T> AttachmentChange<T> {
    /// Start an allocation-free aggregate, usually with `()` as the result.
    pub fn new(result: T) -> Self {
        Self {
            result,
            first: None,
            overflow: Vec::new(),
        }
    }

    /// Retain another change's payloads and return its result without dropping any
    /// entries. This permits grouped publication under one guard, with all retired
    /// values dropped only after the complete group has been published and unlocked.
    pub fn merge<U>(&mut self, other: AttachmentChange<U>) -> U {
        if let Some(entry) = other.first {
            self.retire(entry);
        }
        self.overflow.extend(other.overflow);
        other.result
    }

    /// Release retired values and extract the result, after the guard has dropped.
    /// Do not chain this onto a mutation through a temporary mutex guard.
    pub fn into_result(self) -> T {
        self.result
    }

    fn retire(&mut self, entry: Arc<AttachmentEntry>) {
        if self.first.is_none() {
            self.first = Some(entry);
        } else {
            self.overflow.push(entry);
        }
    }
}

impl<H> AttachmentOwner<H> {
    /// Rebuild a transferred residency identity; the index still validates it.
    #[doc(hidden)]
    pub fn from_parts(holder: H, generation: u64) -> Self {
        Self { holder, generation }
    }
    /// The fields needed to transfer this identity.
    #[doc(hidden)]
    pub fn parts(&self) -> (&H, u64) {
        (&self.holder, self.generation)
    }
    /// Rebuild a snapshot row from a transferred identity and value.
    #[doc(hidden)]
    pub fn into_row(self, entry: Arc<AttachmentEntry>) -> AttachmentRow<H> {
        AttachmentRow {
            holder: self.holder,
            generation: self.generation,
            entry,
        }
    }
}

impl<H: Copy> AttachmentRow<H> {
    /// Capture non-forgeable authority to validate this residency before a write.
    pub fn write_token(&self) -> AttachmentOwner<H> {
        AttachmentOwner {
            holder: self.holder,
            generation: self.generation,
        }
    }
}

fn next_generation(next: u64) -> Option<u64> {
    next.checked_add(1)
}

struct Owner {
    generation: u64,
    slots: HashMap<Arc<AttachmentKey>, usize>,
}

fn reclaim_sparse_map<K: Eq + Hash, V, S: BuildHasher + Default>(map: &mut HashMap<K, V, S>) {
    if map.is_empty() {
        *map = HashMap::default();
    } else if map.len() < map.capacity() / 4 {
        map.shrink_to_fit();
    }
}

/// A dimension's loaded attachment memberships, independent of holder storage.
///
/// Snapshots clone one Arc, and borrowed row iteration needs no locks or allocations.
/// Mutation copies a bucket's rows and probe map only when a snapshot retains that version;
/// entries themselves remain shared. With no retained version, updates are in place.
/// All key/holder lookups are expected O(1). An owner's key-to-slot map is the only
/// reverse index; unregistration visits only that holder's keys, not all types.
/// Key strings are shared across every membership in a bucket. Sparse tables and
/// row vectors shrink below quarter capacity, amortizing reclamation over removals.
/// Registration and unload are explicit lifecycle operations, never Drop effects.
///
/// Rows append on first insertion. Replacement preserves the slot; removal fills it
/// from the final row. Order is deterministic given the ordered registration/mutation
/// history, but is neither sorted nor stable across removals. Callers restoring many
/// holders must supply a defined registration order rather than HashMap iteration.
#[non_exhaustive]
pub struct AttachmentIndex<H> {
    buckets: HashMap<BucketKey, Arc<AttachmentSnapshot<H>>>,
    owners: FxHashMap<H, Owner>,
    empty: Arc<AttachmentSnapshot<H>>,
    #[cfg(feature = "test-work-count")]
    work: AtomicU64,
}

impl<H> Default for AttachmentIndex<H> {
    fn default() -> Self {
        Self {
            buckets: HashMap::new(),
            owners: FxHashMap::default(),
            empty: Arc::new(AttachmentSnapshot::default()),
            #[cfg(feature = "test-work-count")]
            work: AtomicU64::new(0),
        }
    }
}

impl<H: Copy + Eq + Hash> AttachmentIndex<H> {
    #[inline]
    fn record_work(&self) {
        #[cfg(feature = "test-work-count")]
        self.work.fetch_add(1, Ordering::Relaxed);
    }

    /// Number of lookup, validation or mutation calls, for owning-thread idle proofs.
    #[doc(hidden)]
    #[cfg(feature = "test-work-count")]
    pub fn work_count(&self) -> u64 {
        self.work.load(Ordering::Relaxed)
    }

    /// Install the authoritative map, replacing the holder's previous membership.
    /// Only lifecycle code may register; a bound map publishes through its token.
    /// Rebinding invalidates stale save/COW bindings, even when their keys match.
    /// Shared keys keep their dense slots and entry pointers; a new generation
    /// replaces row metadata even when its value is unchanged.
    /// Any rebind may therefore copy a bucket a retained batch still holds: at
    /// placement and travel, and after a chunk-extras copy-on-write.
    pub fn register(
        &mut self,
        holder: H,
        entries: impl IntoIterator<Item = (AttachmentKey, Arc<AttachmentEntry>)>,
    ) -> AttachmentChange<AttachmentOwner<H>> {
        self.record_work();
        let mut omitted = self
            .owners
            .remove(&holder)
            .map_or_else(HashMap::new, |owner| owner.slots);
        // Process-wide identity also refuses tokens against another dimension's index.
        static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
        let generation = NEXT_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, next_generation)
            .expect("attachment residency generation exhausted");
        let token = AttachmentOwner { holder, generation };
        self.owners.insert(
            holder,
            Owner {
                generation: token.generation,
                slots: omitted.clone(),
            },
        );
        let mut change = AttachmentChange::new(token);
        for (key, entry) in entries {
            omitted.remove(&key);
            let update = self.set(&change.result, &key, entry);
            change.merge(update);
        }
        for (key, _) in omitted {
            let update = self.remove(&change.result, &key);
            change.merge(update);
        }
        change
    }

    /// Capture current membership and values. Missing keys share the same empty Arc;
    /// acquiring an empty snapshot neither allocates nor registers the key.
    pub fn snapshot(&self, key: &AttachmentKey) -> Arc<AttachmentSnapshot<H>> {
        self.record_work();
        self.buckets
            .get(key as &dyn Lookup)
            .unwrap_or(&self.empty)
            .clone()
    }

    /// Capture a bucket by UTF-8 names without allocating UTF-16 lookup keys.
    /// Both present and absent types use expected O(1) lookup, with no full scan.
    pub fn snapshot_named(&self, namespace: &str, name: &str) -> Arc<AttachmentSnapshot<H>> {
        self.record_work();
        self.buckets
            .get(&Named(namespace, name) as &dyn Lookup)
            .unwrap_or(&self.empty)
            .clone()
    }

    /// Refuse a captured row after unload or re-registration, including in another index.
    /// Call under the same owning-thread access as the subsequent holder write. This
    /// only checks residency; writes must still use the holder's normal journal path.
    pub fn validate_row(&self, row: &AttachmentRow<H>) -> Result<(), super::AttachmentRowError> {
        self.validate_owner(&row.write_token())
    }

    /// Check a non-forgeable residency token without inspecting attachment membership.
    pub fn validate_owner(
        &self,
        token: &AttachmentOwner<H>,
    ) -> Result<(), super::AttachmentRowError> {
        self.record_work();
        if self
            .owners
            .get(&token.holder)
            .is_some_and(|owner| owner.generation == token.generation)
        {
            Ok(())
        } else {
            Err(super::AttachmentRowError::StaleHolder)
        }
    }

    /// Publish insertion or replacement. Return false for a stale or foreign token,
    /// leaving the index unchanged. Replacement does not reorder the holder's row.
    pub fn set(
        &mut self,
        token: &AttachmentOwner<H>,
        key: &AttachmentKey,
        entry: Arc<AttachmentEntry>,
    ) -> AttachmentChange<bool> {
        self.record_work();
        let Some(owner) = self
            .owners
            .get_mut(&token.holder)
            .filter(|owner| owner.generation == token.generation)
        else {
            let mut rejected = AttachmentChange::new(false);
            rejected.retire(entry);
            return rejected;
        };
        let mut change = AttachmentChange::new(true);
        let row = AttachmentRow {
            holder: token.holder,
            entry,
            generation: token.generation,
        };
        if let Some(&slot) = owner.slots.get(key) {
            let rows = self.buckets.get_mut(key as &dyn Lookup).unwrap();
            if !Arc::ptr_eq(&rows[slot].entry, &row.entry)
                || rows[slot].generation != row.generation
            {
                let old = Arc::make_mut(rows).replace(slot, row);
                change.retire(old.entry);
            }
        } else {
            let shared_key = self
                .buckets
                .get_key_value(key as &dyn Lookup)
                .map_or_else(|| Arc::new(key.clone()), |(key, _)| key.0.clone());
            let rows = self
                .buckets
                .entry(BucketKey(shared_key.clone()))
                .or_default();
            owner.slots.insert(shared_key, rows.len());
            Arc::make_mut(rows).push(row);
        }
        change
    }

    /// Remove one membership. The result is false only for a stale or foreign token.
    pub fn remove(
        &mut self,
        token: &AttachmentOwner<H>,
        key: &AttachmentKey,
    ) -> AttachmentChange<bool> {
        self.record_work();
        let Some(owner) = self
            .owners
            .get_mut(&token.holder)
            .filter(|owner| owner.generation == token.generation)
        else {
            return AttachmentChange::new(false);
        };
        let mut change = AttachmentChange::new(true);
        if let Some(slot) = owner.slots.remove(key) {
            reclaim_sparse_map(&mut owner.slots);
            change.retire(self.remove_membership(key, slot));
        }
        change
    }

    /// Explicitly unload this registration. Stale or foreign tokens cannot unload
    /// a replacement owner. Retained snapshots keep their original rows and values.
    pub fn unregister(&mut self, token: &AttachmentOwner<H>) -> AttachmentChange<bool> {
        self.record_work();
        let Some(owner) = self.owners.get(&token.holder) else {
            return AttachmentChange::new(false);
        };
        if owner.generation != token.generation {
            return AttachmentChange::new(false);
        }
        self.unload(token.holder)
    }

    /// Remove the current registration at an authoritative storage boundary, even
    /// when the live payload has detached from its original map binding through COW.
    /// Unlike token-checked unregistration, this is only for trusted holder unload,
    /// never for a map observer or Drop. Return false if the holder was not registered.
    pub fn unload(&mut self, holder: H) -> AttachmentChange<bool> {
        self.record_work();
        let Some(owner) = self.owners.remove(&holder) else {
            return AttachmentChange::new(false);
        };
        let mut change = AttachmentChange::new(true);
        for (key, slot) in owner.slots {
            change.retire(self.remove_membership(&key, slot));
        }
        reclaim_sparse_map(&mut self.owners);
        change
    }

    fn remove_membership(&mut self, key: &AttachmentKey, slot: usize) -> Arc<AttachmentEntry> {
        let rows = Arc::make_mut(self.buckets.get_mut(key as &dyn Lookup).unwrap());
        let retired = rows.swap_remove(slot).entry;
        if let Some(moved) = rows.get(slot) {
            *self
                .owners
                .get_mut(&moved.holder)
                .unwrap()
                .slots
                .get_mut(key)
                .unwrap() = slot;
        }
        if rows.is_empty() {
            self.buckets.remove(key as &dyn Lookup);
            reclaim_sparse_map(&mut self.buckets);
        } else if rows.len() < rows.capacity() / 4 {
            rows.rows.shrink_to_fit();
        }
        retired
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn residency_generation_refuses_to_wrap() {
        assert_eq!(super::next_generation(u64::MAX - 1), Some(u64::MAX));
        assert_eq!(super::next_generation(u64::MAX), None);
    }
    use super::*;
    use crate::attachments::{
        AttachmentCodec, AttachmentError, AttachmentType, Attachments, Holder,
    };
    use horizon_core::nbt::Tag;

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "value",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("value".into(), Tag::Int(*n))].into()),
            decode: |form| match form.get(&"value".into()) {
                Some(Tag::Int(n)) => Ok(*n),
                _ => Err(AttachmentError::Decode),
            },
        }),
        copy_on_death: false,
        sync: false,
    };

    fn key(name: &str) -> AttachmentKey {
        ("test".into(), name.into())
    }

    fn entry(n: i32) -> Arc<AttachmentEntry> {
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Level, n).unwrap();
        map.entry("test", "value").unwrap().unwrap().clone()
    }

    fn values(rows: &[AttachmentRow<u32>]) -> Vec<(u32, i32)> {
        rows.iter()
            .map(|row| (row.holder, *row.entry.read(&VALUE).unwrap()))
            .collect()
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "duplicate attachment residency")]
    fn snapshot_from_rows_refuses_duplicate_residencies() {
        let row = AttachmentOwner::from_parts(1u32, 7).into_row(entry(7));
        AttachmentSnapshot::from_rows(vec![row.clone(), row]);
    }

    #[test]
    fn snapshot_probes_follow_mutations_and_retain_old_residencies() {
        let mut index = AttachmentIndex::default();
        let k = key("value");
        let a = index.register(1, [(k.clone(), entry(7))]).into_result();
        let b = index.register(2, [(k.clone(), entry(9))]).into_result();
        let c = index.register(3, [(k.clone(), entry(11))]).into_result();
        let old = index.snapshot(&k);
        assert_eq!(values(&old), [(1, 7), (2, 9), (3, 11)]);
        assert_eq!(
            [old.slot_of(&a), old.slot_of(&b), old.slot_of(&c)],
            [Some(0), Some(1), Some(2)]
        );
        assert_eq!(old.slot_of(&AttachmentOwner::from_parts(4, 0)), None);
        assert!(index.remove(&b, &k).into_result());
        let live = index.snapshot(&k);
        assert_eq!(values(&live), [(1, 7), (3, 11)]);
        assert_eq!(live.slot_of(&c), Some(1));
        assert_eq!(live.slot_of(&b), None);
        assert_eq!(old.slot_of(&c), Some(2));
        let transferred = AttachmentSnapshot::from_rows(old.iter().cloned().collect());
        assert_eq!(values(&transferred), [(1, 7), (2, 9), (3, 11)]);
        assert_eq!(transferred.slot_of(&b), Some(1));
        assert_eq!(transferred.slot_of(&c), Some(2));
        assert_eq!(values(&old), [(1, 7), (2, 9), (3, 11)]);
        assert!(index.unregister(&c).into_result());
        let seven = index.snapshot(&k);
        assert!(index.set(&a, &k, entry(8)).into_result());
        let eight = index.snapshot(&k);
        assert_eq!(values(&seven), [(1, 7)]);
        assert_eq!(values(&eight), [(1, 8)]);
        let fresh = index.register(1, [(k.clone(), entry(8))]).into_result();
        assert!(!index.set(&a, &k, entry(99)).into_result());
        let rebound = index.snapshot(&k);
        assert_eq!(values(&rebound), [(1, 8)]);
        assert_eq!(rebound.slot_of(&a), None);
        assert_eq!(rebound.slot_of(&fresh), Some(0));
        assert_eq!(eight.slot_of(&a), Some(0));
        assert!(index.remove(&fresh, &k).into_result());
        assert_eq!(index.snapshot(&k).len(), 0);
        assert_eq!(eight.len(), 1);
        assert!(index.unload(1).into_result());
        assert_eq!(rebound.slot_of(&fresh), Some(0));
    }

    #[test]
    fn ten_thousand_unbound_holders_have_no_idle_memberships() {
        let index = AttachmentIndex::<u32>::default();
        let holders: Vec<_> = (0..10_000).collect();
        #[cfg(feature = "test-work-count")]
        let before = index.work_count();
        for _ in 0..100 {
            // Unbound holders have no lifecycle publication to perform on idle ticks.
            std::hint::black_box(&holders);
        }
        assert!(index.buckets.is_empty());
        assert!(index.owners.is_empty());
        #[cfg(feature = "test-work-count")]
        assert_eq!(index.work_count(), before);
    }

    #[test]
    fn snapshots_keep_membership_and_values_while_current_rows_change() {
        let mut index = AttachmentIndex::default();
        let original = entry(10);
        let a = index
            .register(1, [(key("value"), original.clone())])
            .into_result();
        let b = index.register(2, [(key("value"), entry(20))]).into_result();
        let before = index.snapshot(&key("value"));
        assert!(Arc::ptr_eq(&before[0].entry, &original));
        assert!(index.set(&a, &key("value"), entry(11)).into_result());
        assert!(index.remove(&b, &key("value")).into_result());
        index.register(3, [(key("value"), entry(30))]).into_result();
        assert_eq!(values(&before), [(1, 10), (2, 20)]);
        assert_eq!(values(&index.snapshot(&key("value"))), [(1, 11), (3, 30)]);
    }

    #[test]
    fn saved_entries_join_without_a_typed_read_and_remain_snapshot_values() {
        let mut source = Attachments::default();
        VALUE.set(&mut source, Holder::Level, 42).unwrap();
        let nbt = source.to_nbt();
        let loaded = Attachments::from_nbt(nbt.clone());
        let raw = loaded.entry("test", "value").unwrap().unwrap().clone();
        let mut index = AttachmentIndex::default();
        let owner = index
            .register(1, [(key("value"), raw.clone())])
            .into_result();
        let before = index.snapshot(&key("value"));
        assert!(Arc::ptr_eq(&raw, &before[0].entry));
        index.set(&owner, &key("value"), entry(99)).into_result();
        assert_eq!(values(&before), [(1, 42)]);
        assert_eq!(values(&index.snapshot(&key("value"))), [(1, 99)]);
        assert_eq!(loaded.to_nbt(), nbt);
    }

    #[test]
    fn writes_copy_rows_once_per_retained_version_and_keep_other_entries_shared() {
        let mut index = AttachmentIndex::default();
        let a = index.register(1, [(key("value"), entry(1))]).into_result();
        let untouched = entry(2);
        index
            .register(2, [(key("value"), untouched.clone())])
            .into_result();
        let before = index.snapshot(&key("value"));
        index.set(&a, &key("value"), entry(10)).into_result();
        let current = index.snapshot(&key("value"));
        assert!(!Arc::ptr_eq(&before, &current));
        assert!(Arc::ptr_eq(&untouched, &current[1].entry));
        let current_version = Arc::as_ptr(&current);
        drop(current);
        index.set(&a, &key("value"), entry(11)).into_result();
        let after = index.snapshot(&key("value"));
        assert_eq!(Arc::as_ptr(&after), current_version);
        assert_eq!(values(&before), [(1, 1), (2, 2)]);
        assert_eq!(values(&after), [(1, 11), (2, 2)]);
    }

    #[test]
    fn swap_removal_repairs_slots_and_replacement_preserves_order() {
        let mut index = AttachmentIndex::default();
        let a = index.register(1, [(key("value"), entry(1))]).into_result();
        let b = index.register(2, [(key("value"), entry(2))]).into_result();
        let c = index.register(3, [(key("value"), entry(3))]).into_result();
        let d = index.register(4, [(key("value"), entry(4))]).into_result();
        assert!(index.remove(&b, &key("value")).into_result());
        assert!(index.set(&d, &key("value"), entry(40)).into_result());
        assert_eq!(
            values(&index.snapshot(&key("value"))),
            [(1, 1), (4, 40), (3, 3)]
        );
        assert!(index.unregister(&a).into_result());
        assert!(index.set(&c, &key("value"), entry(30)).into_result());
        assert_eq!(values(&index.snapshot(&key("value"))), [(3, 30), (4, 40)]);
        assert!(index.remove(&d, &key("value")).into_result());
        assert_eq!(values(&index.snapshot(&key("value"))), [(3, 30)]);
    }

    #[test]
    fn rebind_rejects_stale_tokens_and_unloads_only_the_owners_keys() {
        let mut index = AttachmentIndex::default();
        let old = index
            .register(1, [(key("a"), entry(1)), (key("b"), entry(2))])
            .into_result();
        index.register(2, [(key("b"), entry(20))]).into_result();
        let retained = index.snapshot(&key("a"));
        let current = index
            .register(1, [(key("b"), entry(22)), (key("c"), entry(3))])
            .into_result();
        assert!(!index.set(&old, &key("b"), entry(99)).into_result());
        assert!(!index.remove(&old, &key("b")).into_result());
        assert!(!index.unregister(&old).into_result());
        assert_eq!(values(&retained), [(1, 1)]);
        assert!(index.snapshot(&key("a")).is_empty());
        assert_eq!(values(&index.snapshot(&key("b"))), [(1, 22), (2, 20)]);
        assert_eq!(values(&index.snapshot(&key("c"))), [(1, 3)]);
        let clone = current.clone();
        assert_eq!(values(&index.snapshot(&key("c"))), [(1, 3)]);
        assert!(index.unregister(&clone).into_result());
        assert!(!index.set(&clone, &key("a"), entry(9)).into_result());
        assert_eq!(values(&index.snapshot(&key("b"))), [(2, 20)]);
    }

    #[test]
    fn multi_key_mass_unload_repairs_survivors_reverse_slots() {
        let mut index = AttachmentIndex::default();
        let owners: Vec<_> = (0..64)
            .map(|i| {
                let mut entries = vec![(key("a"), entry(i))];
                if i % 2 == 0 {
                    entries.push((key("b"), entry(i + 100)));
                }
                index.register(i as u32, entries).into_result()
            })
            .collect();
        let mut alive = [true; 64];
        for step in 0..64 {
            let removed = step * 17 % 64;
            alive[removed] = false;
            assert!(index.unload(removed as u32).into_result());
            for (i, owner) in owners.iter().enumerate().filter(|(i, _)| alive[*i]) {
                assert!(
                    index
                        .set(owner, &key("a"), entry(i as i32 + 1000))
                        .into_result()
                );
            }
            for (name, offset) in [("a", 1000), ("b", 100)] {
                let mut actual = values(&index.snapshot(&key(name)));
                actual.sort_unstable();
                let expected: Vec<_> = (0..64)
                    .filter(|&i| alive[i] && (name == "a" || i % 2 == 0))
                    .map(|i| (i as u32, i as i32 + offset))
                    .collect();
                assert_eq!(actual, expected);
            }
        }
        assert!(index.snapshot(&key("a")).is_empty());
        assert!(index.snapshot(&key("b")).is_empty());
    }

    #[test]
    fn same_key_rebind_preserves_slots_and_entries_but_refreshes_row_generation() {
        let mut index = AttachmentIndex::default();
        let original = entry(1);
        let old = index
            .register(1, [(key("value"), original.clone())])
            .into_result();
        index.register(2, [(key("value"), entry(2))]).into_result();
        let retained = index.snapshot(&key("value"));
        let refreshed = index.register(1, [(key("value"), original)]).into_result();
        assert!(Arc::ptr_eq(
            &retained[0].entry,
            &index.snapshot(&key("value"))[0].entry
        ));
        assert_eq!(
            index.validate_row(&retained[0]),
            Err(super::super::AttachmentRowError::StaleHolder)
        );
        assert!(!index.set(&old, &key("value"), entry(99)).into_result());
        let current = index.register(1, [(key("value"), entry(10))]).into_result();
        assert!(!index.unregister(&refreshed).into_result());
        assert_eq!(values(&retained), [(1, 1), (2, 2)]);
        assert_eq!(values(&index.snapshot(&key("value"))), [(1, 10), (2, 2)]);
        assert!(index.unregister(&current).into_result());
    }

    #[test]
    fn trusted_unload_removes_detached_registration_but_preserves_snapshots() {
        let mut index = AttachmentIndex::default();
        let binding = index.register(1, [(key("value"), entry(7))]).into_result();
        let stale = binding.clone();
        let retained = index.snapshot(&key("value"));
        assert!(index.unload(1).into_result());
        assert!(!index.unload(1).into_result());
        assert!(index.snapshot(&key("value")).is_empty());
        assert_eq!(values(&retained), [(1, 7)]);
        assert!(!index.set(&stale, &key("value"), entry(99)).into_result());
        assert!(!index.unregister(&stale).into_result());
        index.register(1, [(key("value"), entry(8))]).into_result();
        assert!(!index.set(&stale, &key("value"), entry(99)).into_result());
        assert!(!index.unregister(&stale).into_result());
        assert_eq!(values(&index.snapshot(&key("value"))), [(1, 8)]);
    }

    #[test]
    fn tokens_from_another_index_cannot_publish_or_unload() {
        let mut first = AttachmentIndex::default();
        let foreign = first.register(1, [(key("value"), entry(1))]).into_result();
        let mut second = AttachmentIndex::default();
        second.register(1, [(key("value"), entry(2))]).into_result();
        assert!(!second.set(&foreign, &key("value"), entry(99)).into_result());
        assert!(!second.remove(&foreign, &key("value")).into_result());
        assert!(!second.unregister(&foreign).into_result());
        assert_eq!(values(&second.snapshot(&key("value"))), [(1, 2)]);
    }

    #[test]
    fn locked_mutations_defer_destructors_until_complete_publication_and_unlock() {
        use std::sync::{Mutex, Weak};

        #[derive(Clone)]
        struct Probe {
            index: Weak<Mutex<AttachmentIndex<u32>>>,
            events: Arc<Mutex<Vec<Option<[usize; 2]>>>>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let index = self.index.upgrade().unwrap();
                let state = index.try_lock().ok().map(|index| {
                    [
                        index.snapshot(&key("a")).len(),
                        index.snapshot(&key("b")).len(),
                    ]
                });
                self.events.lock().unwrap().push(state);
            }
        }
        static PROBE: AttachmentType<Probe> = AttachmentType {
            namespace: "test",
            name: "probe",
            default: || unreachable!(),
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        fn apply<T>(
            index: &Mutex<AttachmentIndex<u32>>,
            mutation: impl FnOnce(&mut AttachmentIndex<u32>) -> AttachmentChange<T>,
        ) -> T {
            let change = {
                let mut guard = index.lock().unwrap();
                mutation(&mut guard)
            };
            change.into_result()
        }
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let probe_entry = || {
            let mut map = Attachments::default();
            PROBE
                .set(
                    &mut map,
                    Holder::Level,
                    Probe {
                        index: Arc::downgrade(&index),
                        events: events.clone(),
                    },
                )
                .unwrap();
            map.entry("test", "probe").unwrap().unwrap().clone()
        };
        let initial = [(key("a"), probe_entry()), (key("b"), probe_entry())];
        let old = apply(&index, |index| index.register(1, initial));
        let replacement = [(key("a"), probe_entry())];
        let current = apply(&index, |index| index.register(1, replacement));
        assert_eq!(*events.lock().unwrap(), [Some([1, 0]), Some([1, 0])]);
        let rejected = probe_entry();
        assert!(!apply(&index, |index| index.set(&old, &key("a"), rejected)));
        assert_eq!(events.lock().unwrap().last(), Some(&Some([1, 0])));
        let next = probe_entry();
        apply(&index, |index| {
            let mut group = AttachmentChange::new(());
            assert!(group.merge(index.set(&current, &key("b"), next)));
            assert!(group.merge(index.remove(&current, &key("a"))));
            group
        });
        assert_eq!(events.lock().unwrap().last(), Some(&Some([0, 1])));
        assert!(apply(&index, |index| index.unload(1)));
        assert_eq!(events.lock().unwrap().last(), Some(&Some([0, 0])));
        let last = [(key("a"), probe_entry())];
        let last = apply(&index, |index| index.register(1, last));
        assert!(apply(&index, |index| index.unregister(&last)));
        let observed = events.lock().unwrap();
        assert_eq!(observed.len(), 6);
        assert!(observed.iter().all(Option::is_some));
        assert_eq!(observed.last(), Some(&Some([0, 0])));
    }

    #[test]
    fn missing_keys_reuse_empty_snapshot_and_do_not_become_memberships() {
        let mut index = AttachmentIndex::<u32>::default();
        let empty = index.snapshot(&key("missing"));
        assert!(empty.is_empty());
        assert!(Arc::ptr_eq(&empty, &index.snapshot(&key("other"))));
        let owner = index.register(1, []).into_result();
        assert!(index.remove(&owner, &key("missing")).into_result());
        assert!(index.unregister(&owner).into_result());
        assert!(Arc::ptr_eq(&empty, &index.snapshot(&key("missing"))));
        index
            .register(1, [(key("missing"), entry(7)), (key("missing"), entry(8))])
            .into_result();
        assert_eq!(values(&index.snapshot(&key("missing"))), [(1, 8)]);
        assert!(empty.is_empty());
    }
}
