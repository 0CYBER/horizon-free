//! Shared revision identities for engine block entities and their API snapshots, and the
//! one-shot client update fields queued under them.
use horizon_core::nbt::Compound;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

static MADE: AtomicU64 = AtomicU64::new(0);
/// The batches waiting for a block change, by identity. Kept here so that the plain-data
/// entity crosses to wasm and is saved, cloned and compared without them.
static UPDATE_FIELDS: Mutex<BTreeMap<u64, Compound>> = Mutex::new(BTreeMap::new());
/// How many batches wait: with none, as nearly always, no call takes the lock.
static WAITING: AtomicUsize = AtomicUsize::new(0);

/// Allocate the identity portion of a newly constructed mod block entity's revision.
/// Snapshots copy their source identity and must not call this allocator.
pub fn new_identity() -> u64 {
    MADE.fetch_add(1, Ordering::Relaxed)
}

fn update_fields() -> MutexGuard<'static, BTreeMap<u64, Compound>> {
    UPDATE_FIELDS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Replace the batch waiting under `identity`; an empty one clears it.
pub fn set_client_update_fields(identity: u64, fields: Compound) {
    let mut waiting = update_fields();
    if fields.is_empty() {
        waiting.remove(&identity);
    } else {
        waiting.insert(identity, fields);
    }
    WAITING.store(waiting.len(), Ordering::Relaxed);
}

/// Take the batch waiting under `identity`: the next call finds none.
pub fn take_client_update_fields(identity: u64) -> Option<Compound> {
    if WAITING.load(Ordering::Relaxed) == 0 {
        return None;
    }
    let mut waiting = update_fields();
    let fields = waiting.remove(&identity);
    WAITING.store(waiting.len(), Ordering::Relaxed);
    fields
}

/// Whether a batch waits under `identity`.
pub fn has_client_update_fields(identity: u64) -> bool {
    WAITING.load(Ordering::Relaxed) != 0 && update_fields().contains_key(&identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::nbt::Tag;

    #[test]
    fn a_batch_is_replaced_cleared_by_an_empty_one_and_taken_once() {
        let (one, other) = (new_identity(), new_identity());
        let batch = |n| Compound::from([("effect".into(), Tag::Int(n))]);
        set_client_update_fields(one, batch(1));
        set_client_update_fields(one, batch(2));
        set_client_update_fields(other, batch(3));
        assert!(has_client_update_fields(one));
        assert_eq!(take_client_update_fields(one), Some(batch(2)));
        assert_eq!(take_client_update_fields(one), None);
        assert!(!has_client_update_fields(one));
        set_client_update_fields(other, Compound::new());
        assert_eq!(take_client_update_fields(other), None);
    }
}
