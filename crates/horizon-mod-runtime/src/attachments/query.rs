//! Owned snapshot batches and lock-free borrowed typed traversal.
use std::{fmt, hash::Hash, slice, sync::Arc};

use super::{
    AttachmentError, AttachmentType,
    index::{AttachmentIndex, AttachmentOwner, AttachmentRow, AttachmentSnapshot},
};

// no wire: owned by the lower runtime crate, which cannot depend on mod_api's derive;
// mod_api supplies the explicit tagged Wire implementation, including bounded static text.
/// A failed row read or write through a retained batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachmentRowError {
    /// The stored binding could not be read as the requested type.
    Value(AttachmentError),
    /// The holder unloaded, was replaced, or belongs to another index.
    StaleHolder,
    /// The physical holder storage is temporarily unavailable; retry after the pass.
    HolderBusy,
    /// This holder family cannot own attachments.
    UnsupportedHolder(&'static str),
}

impl fmt::Display for AttachmentRowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(error) => write!(f, "attachment row read failed: {error:?}"),
            Self::StaleHolder => f.write_str("attachment row holder is stale"),
            Self::HolderBusy => f.write_str("attachment row holder is busy"),
            Self::UnsupportedHolder(kind) => write!(f, "unsupported attachment holder: {kind}"),
        }
    }
}
impl std::error::Error for AttachmentRowError {}

/// An immutable bucket snapshot. Construction moves the Arc returned by
/// `AttachmentIndex::snapshot`; it copies neither rows nor values.
/// Later mutations affect only fresh batches, not these captured bindings.
#[derive(Clone)]
#[non_exhaustive]
pub struct AttachmentBatch<H> {
    rows: Option<Arc<AttachmentSnapshot<H>>>,
}

impl<H> AttachmentBatch<H> {
    /// Wrap a captured index bucket without copying its contents.
    pub fn new(rows: Arc<AttachmentSnapshot<H>>) -> Self {
        Self { rows: Some(rows) }
    }

    /// An empty batch requiring no allocation, even without a live index.
    pub fn empty() -> Self {
        Self { rows: None }
    }

    /// Borrow the captured rows for an owned boundary conversion.
    /// Native query construction does not invoke that conversion.
    pub fn rows(&self) -> &[AttachmentRow<H>] {
        self.rows.as_deref().map_or(&[], |rows| rows.as_slice())
    }

    /// Bind the key once, checking its policy before any row is read.
    /// The query borrows only the key, never the index or host.
    pub fn query<T: Send + Sync + 'static>(
        self,
        key: &AttachmentType<T>,
    ) -> Result<AttachmentQuery<'_, H, T>, AttachmentError> {
        key.validate_policy()?;
        Ok(AttachmentQuery { batch: self, key })
    }
}

/// Typed view of a captured batch. Saved values decode lazily into the entry's
/// read cache; errors preserve saved data and never initialize a default.
/// This cache does not claim the canonical stored type, dirty or sync the holder.
#[non_exhaustive]
pub struct AttachmentQuery<'key, H, T: 'static> {
    batch: AttachmentBatch<H>,
    key: &'key AttachmentType<T>,
}

impl<H: Copy, T: Send + Sync + 'static> AttachmentQuery<'_, H, T> {
    /// Traverse captured index order without locks, holder lookups, value clones
    /// or per-item allocation. A first saved decode may allocate in the codec/cache.
    pub fn iter(&self) -> AttachmentIter<'_, H, T> {
        AttachmentIter {
            rows: self.batch.rows().iter(),
            key: self.key,
        }
    }
}

impl<H: Copy + Eq + Hash, T: Clone + Send + Sync + 'static> AttachmentQuery<'_, H, T> {
    /// Read an owned captured value with an expected O(1) holder probe before decode.
    /// Absent or not yet synced holders return `Ok(None)`, without initializing
    /// defaults. Decode/type errors are preserved as `AttachmentRowError::Value`.
    /// `None` does not mean the holder is alive: absent and not-live holders
    /// alike return `Ok(None)` when absent from captured membership, without a
    /// host liveness check. Only scoped residency probes report `StaleHolder`.
    /// Later replacements, removals and rebinds require a fresh snapshot.
    pub fn get(&self, holder: H) -> Result<Option<T>, AttachmentRowError> {
        let Some(rows) = self.batch.rows.as_deref() else {
            return Ok(None);
        };
        let Some(slot) = rows.slot_for(holder) else {
            return Ok(None);
        };
        rows[slot]
            .entry
            .read_validated(self.key)
            .map(|value| Some(value.clone()))
            .map_err(AttachmentRowError::Value)
    }
}

/// Borrowing iterator over a batch, not an iterator owning its borrowed values.
#[non_exhaustive]
pub struct AttachmentIter<'q, H, T: 'static> {
    rows: slice::Iter<'q, AttachmentRow<H>>,
    key: &'q AttachmentType<T>,
}

impl<'q, H: Copy, T: Send + Sync + 'static> Iterator for AttachmentIter<'q, H, T> {
    type Item = Result<AttachmentRowRef<'q, H, T>, AttachmentRowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(|row| {
            row.entry
                .read_validated(self.key)
                .map(|value| AttachmentRowRef {
                    holder: row.holder,
                    value,
                    row,
                })
                .map_err(AttachmentRowError::Value)
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}
impl<H: Copy, T: Send + Sync + 'static> ExactSizeIterator for AttachmentIter<'_, H, T> {}

/// A Copy holder identity and a value borrowed directly from its captured entry.
/// Interior mutability inside T is not deep-frozen by a snapshot.
#[non_exhaustive]
pub struct AttachmentRowRef<'q, H, T> {
    /// Ready-to-use holder; no conversion or resolution is done during traversal.
    pub holder: H,
    /// The captured value, not a clone of T.
    pub value: &'q T,
    row: &'q AttachmentRow<H>,
}

impl<H: Copy + Eq + Hash, T> AttachmentRowRef<'_, H, T> {
    /// Capture non-forgeable residency authority for validation by the host.
    /// Value replacement or attachment removal alone does not stale the token,
    /// but unloading or re-registering the holder does.
    pub fn write_token(&self) -> AttachmentOwner<H> {
        self.row.write_token()
    }

    /// Validate residency under the owning index immediately before writing.
    pub fn validate_write(&self, index: &AttachmentIndex<H>) -> Result<(), AttachmentRowError> {
        index.validate_row(self.row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{AttachmentCodec, AttachmentEntry, AttachmentPolicy, Decoded};
    use horizon_core::nbt::{Compound, Tag};
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    struct CountingAllocator;
    thread_local! {
        static ALLOCATIONS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    }
    fn count_allocation() {
        let _ = ALLOCATIONS.try_with(|count| {
            if let Some(n) = count.get() {
                count.set(Some(n + 1));
            }
        });
    }
    // SAFETY: every operation delegates unchanged to the system allocator.
    unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            count_allocation();
            unsafe { std::alloc::System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
            unsafe { std::alloc::System.dealloc(ptr, layout) }
        }
        unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
            count_allocation();
            unsafe { std::alloc::System.alloc_zeroed(layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
            count_allocation();
            unsafe { std::alloc::System.realloc(ptr, layout, size) }
        }
    }
    #[global_allocator]
    static ALLOCATOR: CountingAllocator = CountingAllocator;

    static DECODES: AtomicUsize = AtomicUsize::new(0);
    static KEY: AttachmentType<u32> = AttachmentType {
        namespace: "query",
        name: "value",
        default: || panic!("query must not initialize defaults"),
        codec: Some(AttachmentCodec {
            encode: |_| unreachable!(),
            decode: |saved| {
                DECODES.fetch_add(1, Ordering::Relaxed);
                match saved.get(&"n".into()) {
                    Some(Tag::Int(n)) => Ok(*n as u32),
                    _ => Err(AttachmentError::Decode),
                }
            },
        }),
        copy_on_death: false,
        sync: false,
    };
    pub(super) fn key() -> super::super::index::AttachmentKey {
        ("query".into(), "value".into())
    }
    pub(super) fn entry(value: impl Send + Sync + 'static) -> Arc<AttachmentEntry> {
        Arc::new(AttachmentEntry {
            state: super::super::Entry::Decoded(Decoded {
                value: Arc::new(value),
                encoded: None,
                policy: AttachmentPolicy {
                    copy_on_death: false,
                    sync: false,
                },
            }),
        })
    }
    pub(super) fn query(index: &AttachmentIndex<u32>) -> AttachmentQuery<'static, u32, u32> {
        AttachmentBatch::new(index.snapshot(&key()))
            .query(&KEY)
            .unwrap()
    }
    fn values(query: &AttachmentQuery<'_, u32, u32>) -> Vec<(u32, u32)> {
        query
            .iter()
            .map(|row| {
                let row = row.unwrap();
                (row.holder, *row.value)
            })
            .collect()
    }

    #[test]
    fn captured_order_values_and_stale_write_authority() {
        let mut index = AttachmentIndex::default();
        let one = index.register(1, [(key(), entry(10u32))]).into_result();
        let two = index.register(2, [(key(), entry(20u32))]).into_result();
        index.register(3, [(key(), entry(30u32))]).into_result();
        let captured = query(&index);
        let tokens: Vec<_> = captured
            .iter()
            .map(|row| row.unwrap().write_token())
            .collect();
        index.set(&one, &key(), entry(11u32)).into_result();
        index.remove(&two, &key()).into_result();
        assert_eq!(values(&captured), [(1, 10), (2, 20), (3, 30)]);
        assert_eq!(values(&query(&index)), [(1, 11), (3, 30)]);
        let mut other_index = AttachmentIndex::default();
        other_index
            .register(1, [(key(), entry(100u32))])
            .into_result();
        assert_eq!(
            other_index.validate_owner(&tokens[0]),
            Err(AttachmentRowError::StaleHolder)
        );
        for (row, token) in captured.iter().zip(&tokens) {
            assert_eq!(row.unwrap().validate_write(&index), Ok(()));
            assert_eq!(index.validate_owner(token), Ok(()));
        }
        index.unload(1).into_result();
        // Even reusing the identical entry pointer creates a new registration.
        let same = index.snapshot(&key())[0].entry.clone();
        index.register(3, [(key(), same)]).into_result();
        let rows: Vec<_> = captured.iter().map(Result::unwrap).collect();
        assert_eq!(
            rows[0].validate_write(&index),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(
            rows[2].validate_write(&index),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(rows[1].validate_write(&index), Ok(()));
        assert_eq!(
            index.validate_owner(&tokens[0]),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(
            index.validate_owner(&tokens[2]),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(index.validate_owner(&tokens[1]), Ok(()));
        assert_eq!(
            AttachmentIndex::default().validate_owner(&tokens[1]),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(values(&captured), [(1, 10), (2, 20), (3, 30)]);
        assert_eq!(
            rows[1].validate_write(&AttachmentIndex::default()),
            Err(AttachmentRowError::StaleHolder)
        );
    }

    #[test]
    fn lazy_success_only_decode_and_malformed_row_is_not_skipped() {
        DECODES.store(0, Ordering::Relaxed);
        let saved = |payload| {
            Arc::new(AttachmentEntry::raw(
                [("value".into(), Tag::Compound(payload))].into(),
            ))
        };
        let good = saved([("n".into(), Tag::Int(17))].into());
        let bad = saved(Compound::new());
        let mut index = AttachmentIndex::default();
        index.register(1, [(key(), good)]).into_result();
        index.register(2, [(key(), bad.clone())]).into_result();
        index
            .register(3, [(key(), entry("wrong type"))])
            .into_result();
        let batch = query(&index);
        assert_eq!(DECODES.load(Ordering::Relaxed), 0);
        for expected in [2, 3] {
            let mut iter = batch.iter();
            assert_eq!(*iter.next().unwrap().unwrap().value, 17);
            assert!(matches!(
                iter.next(),
                Some(Err(AttachmentRowError::Value(AttachmentError::Decode)))
            ));
            assert!(matches!(
                iter.next(),
                Some(Err(AttachmentRowError::Value(
                    AttachmentError::TypeMismatch
                )))
            ));
            assert!(iter.next().is_none());
            assert_eq!(DECODES.load(Ordering::Relaxed), expected);
        }
        let super::super::Entry::Raw { reads, .. } = &bad.state else {
            panic!("raw preserved")
        };
        assert!(reads.value.get().is_none());
    }

    #[test]
    fn decoded_thousand_row_iteration_allocates_nothing() {
        let mut index = AttachmentIndex::default();
        for holder in 0..1000 {
            index
                .register(holder, [(key(), entry(holder + 1))])
                .into_result();
        }
        let batch = query(&index);
        ALLOCATIONS.with(|n| n.set(Some(0)));
        let mut total = 0;
        let mut expected_holder = 0;
        for row in batch.iter() {
            let row = row.unwrap();
            assert_eq!(row.holder, expected_holder);
            expected_holder += 1;
            total += *row.value;
        }
        let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
        assert_eq!(allocations, 0);
        assert_eq!(expected_holder, 1000);
        assert_eq!(total, 500500);
    }

    #[test]
    fn named_lookup_and_missing_index_batch_allocate_nothing() {
        let mut index = AttachmentIndex::default();
        let unicode_key = ("query😀".into(), "value水".into());
        index
            .register(1u32, [(unicode_key.clone(), entry(42u32))])
            .into_result();
        let expected = index.snapshot(&unicode_key);
        ALLOCATIONS.with(|n| n.set(Some(0)));
        let present = index.snapshot_named("query😀", "value水");
        let absent_name = index.snapshot_named("query😀", "absent");
        let absent_namespace = index.snapshot_named("absent", "value水");
        let empty = AttachmentBatch::<u32>::empty().query(&KEY).unwrap();
        let allocations = ALLOCATIONS.with(|n| n.replace(None).unwrap());
        assert_eq!(allocations, 0);
        assert!(Arc::ptr_eq(&present, &expected));
        assert!(absent_name.is_empty());
        assert!(Arc::ptr_eq(&absent_name, &absent_namespace));
        assert_eq!(empty.iter().len(), 0);
        index.unload(1).into_result();
        assert!(index.snapshot_named("query😀", "value水").is_empty());
    }

    #[test]
    fn unpaired_surrogate_lookup_preserves_stored_units() {
        let mut index = AttachmentIndex::default();
        let key = (horizon_core::nbt::JavaString(vec![0xd800]), "name".into());
        index
            .register(1u32, [(key.clone(), entry(42u32))])
            .into_result();
        assert!(index.snapshot_named("\u{fffd}", "name").is_empty());
        assert_eq!(index.snapshot(&key).len(), 1);
    }

    #[test]
    fn final_batch_drops_mod_value_outside_index_mutex() {
        struct Probe(Arc<Mutex<AttachmentIndex<u32>>>);
        impl Drop for Probe {
            fn drop(&mut self) {
                assert!(
                    self.0.try_lock().is_ok(),
                    "index locked during mod destructor"
                );
            }
        }
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let token = index
            .lock()
            .unwrap()
            .register(1, [(key(), entry(Probe(index.clone())))])
            .into_result();
        let probe_key = AttachmentType {
            namespace: "query",
            name: "value",
            default: || unreachable!(),
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let batch = AttachmentBatch::new(index.lock().unwrap().snapshot(&key()))
            .query::<Probe>(&probe_key)
            .unwrap();
        let retired = { index.lock().unwrap().unregister(&token) };
        retired.into_result();
        assert_eq!(batch.iter().count(), 1);
        drop(batch);
    }
}

#[cfg(test)]
mod existing_tests {
    use super::tests::{entry, key, query};
    use super::*;
    #[test]
    fn keyed_absent_and_unloaded_holders_return_none() {
        let mut index = AttachmentIndex::default();
        index.register(7, [(key(), entry(7u32))]).into_result();
        let retained = query(&index);
        let token = retained.iter().next().unwrap().unwrap().write_token();
        index.unload(7).into_result();
        let fresh = query(&index);
        assert_eq!(fresh.get(7), Ok(None)); // no longer live
        assert_eq!(fresh.get(11), Ok(None)); // never resident
        assert_eq!(
            index.validate_owner(&token),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(retained.get(7), Ok(Some(7))); // captured, not live
    }

    #[test]
    fn keyed_existing_values_retain_snapshots_and_validate_rebound_tokens() {
        let mut index = AttachmentIndex::default();
        let a = index.register(7, [(key(), entry(7u32))]).into_result();
        index.register(9, [(key(), entry(9u32))]).into_result();
        let old = query(&index);
        assert_eq!(old.get(7), Ok(Some(7)));
        assert_eq!(old.get(9), Ok(Some(9)));
        assert_eq!(old.get(11), Ok(None));
        index.set(&a, &key(), entry(8u32)).into_result();
        assert_eq!(query(&index).get(7), Ok(Some(8)));
        assert_eq!(old.get(7), Ok(Some(7)));
        index.remove(&a, &key()).into_result();
        assert_eq!(query(&index).get(7), Ok(None));
        index.register(7, [(key(), entry(10u32))]).into_result();
        assert_eq!(
            index.validate_owner(&a),
            Err(AttachmentRowError::StaleHolder)
        );
        assert_eq!(index.snapshot(&key()).slot_of(&a), None);
        assert_eq!(query(&index).get(7), Ok(Some(10)));
        assert_eq!(query(&AttachmentIndex::default()).get(7), Ok(None));
    }
}
