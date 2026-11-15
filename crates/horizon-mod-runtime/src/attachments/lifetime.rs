//! Entity-owned attachment registration lifetime, independent of map and snapshot ownership.
use std::fmt;

use super::AttachmentRegistration;

/// Optional authority to retire an entity's attachment registration on destruction.
///
/// Keep this on the authoritative entity, never on its map or index rows: retained
/// UUID maps and query/save snapshots must not prolong a live registration. Moving
/// the slot preserves ownership; cloning it always produces an empty, detached slot.
/// Default has no allocation, reference counting or per-tick work. Only construction
/// from a registration allocates a box, keeping the empty slot one pointer wide.
///
/// This is lifecycle metadata, not gameplay data: equality and Debug ignore it.
#[derive(Default)]
#[non_exhaustive]
pub struct AttachmentLifetime {
    registration: Option<Box<AttachmentRegistration>>,
}

/// An authoritative entity that can own an attachment registration. There is no
/// default accessor: every supported kind must provide its actual owning slot.
pub trait HasAttachmentLifetime {
    /// The entity's lifecycle slot, never a detached map or snapshot's slot.
    fn attachment_lifetime(&mut self) -> &mut AttachmentLifetime;
}

/// Implement the internal lifecycle accessor for entities with an owning slot.
#[doc(hidden)]
#[macro_export]
macro_rules! impl_attachment_lifetime {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::attachments::HasAttachmentLifetime for $ty {
            fn attachment_lifetime(&mut self) -> &mut $crate::attachments::AttachmentLifetime {
                &mut self.attachment_lifetime
            }
        }
    )*};
}

impl AttachmentLifetime {
    /// Bind an authoritative entity's map to its already resolved holder identity.
    pub fn bind<H: Copy + Eq + std::hash::Hash + Send + Sync + 'static>(
        &mut self,
        map: &mut super::Attachments,
        index: &std::sync::Arc<std::sync::Mutex<super::index::AttachmentIndex<H>>>,
        holder: H,
    ) {
        *self = AttachmentLifetime::new(map.bind_index(index, holder));
    }

    /// Whether this entity owns a registration.
    pub fn is_bound(&self) -> bool {
        self.registration.is_some()
    }

    /// Copy the registration handle without taking lifetime ownership.
    #[doc(hidden)]
    pub fn registration(&self) -> Option<AttachmentRegistration> {
        self.registration.as_deref().cloned()
    }

    /// Take lifetime ownership of an authoritative map's registration. Rebinding
    /// the map creates a new generation before replacing an old slot, so destruction
    /// of that old slot cannot retire the replacement.
    pub fn new(registration: AttachmentRegistration) -> Self {
        Self {
            registration: Some(Box::new(registration)),
        }
    }

    /// Retire this generation before physical destruction, for logical removal or
    /// transfer. Returns true only on its first successful retirement. Future writes
    /// through the retired map cannot republish it; a stale slot cannot retire a new
    /// generation. Retain ownership metadata until destruction, even after retirement.
    pub fn retire(&self) -> bool {
        self.registration
            .as_ref()
            .is_some_and(|registration| registration.unregister())
    }
}

impl Drop for AttachmentLifetime {
    fn drop(&mut self) {
        self.retire();
    }
}

impl Clone for AttachmentLifetime {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl fmt::Debug for AttachmentLifetime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AttachmentLifetime")
    }
}

impl PartialEq for AttachmentLifetime {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for AttachmentLifetime {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{AttachmentType, Attachments, Holder, index::AttachmentIndex};
    use std::sync::{Arc, Mutex};

    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "lifetime",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };

    fn values(index: &Arc<Mutex<AttachmentIndex<u32>>>) -> Vec<(u32, i32)> {
        index
            .lock()
            .unwrap()
            .snapshot(&("test".into(), "lifetime".into()))
            .iter()
            .map(|row| (row.holder, *row.entry.read(&VALUE).unwrap()))
            .collect()
    }

    #[test]
    fn move_preserves_publication_then_drop_retires_despite_retained_map_and_batch() {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Level, 10).unwrap();
        let lifetime = AttachmentLifetime::new(map.bind_index(&index, 1));
        let before = index
            .lock()
            .unwrap()
            .snapshot(&("test".into(), "lifetime".into()));
        let moved = vec![lifetime];
        VALUE.set(&mut map, Holder::Level, 20).unwrap();
        assert_eq!(values(&index), [(1, 20)]);
        drop(moved);
        assert!(values(&index).is_empty());
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].holder, 1);
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        VALUE.set(&mut map, Holder::Level, 30).unwrap();
        assert!(values(&index).is_empty());
        assert_eq!(VALUE.get(&mut map, Holder::Level), Ok(30));
    }

    #[test]
    fn entity_derived_clone_is_detached_and_metadata_does_not_affect_equality_or_debug() {
        #[derive(Clone, Debug, PartialEq)]
        struct Entity {
            health: u32,
            lifetime: AttachmentLifetime,
        }
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Level, 10).unwrap();
        let entity = Entity {
            health: 5,
            lifetime: AttachmentLifetime::new(map.bind_index(&index, 1)),
        };
        // Neither cloning nor dropping a detached copy may try to lock the index.
        let guard = index.lock().unwrap();
        let clone = entity.clone();
        assert_eq!(entity, clone);
        assert_eq!(format!("{entity:?}"), format!("{clone:?}"));
        assert!(!clone.lifetime.retire());
        drop(entity.clone());
        drop(guard);
        assert_eq!(values(&index), [(1, 10)]);
        drop(entity);
        assert!(values(&index).is_empty());
        drop(clone);
        assert!(values(&index).is_empty());
    }

    #[test]
    fn early_retirement_is_immediate_idempotent_and_disables_map_publication() {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut map = Attachments::default();
        VALUE.set(&mut map, Holder::Level, 10).unwrap();
        let lifetime = AttachmentLifetime::new(map.bind_index(&index, 1));
        assert!(lifetime.retire());
        assert!(values(&index).is_empty());
        assert!(!lifetime.retire());
        VALUE.set(&mut map, Holder::Level, 20).unwrap();
        VALUE.remove(&mut map, Holder::Level).unwrap();
        VALUE.get(&mut map, Holder::Level).unwrap();
        assert!(values(&index).is_empty());
        drop(lifetime);
        assert!(values(&index).is_empty());
    }

    #[test]
    fn stale_slot_retirement_and_drop_cannot_revoke_a_replacement_generation() {
        for retire_first in [false, true] {
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut old_map = Attachments::default();
            VALUE.set(&mut old_map, Holder::Level, 10).unwrap();
            let old = AttachmentLifetime::new(old_map.bind_index(&index, 1));
            if retire_first {
                assert!(old.retire());
            }
            let mut map = old_map.clone();
            VALUE.set(&mut map, Holder::Level, 20).unwrap();
            let current = AttachmentLifetime::new(map.bind_index(&index, 1));
            assert!(!old.retire());
            drop(old);
            VALUE.set(&mut old_map, Holder::Level, 99).unwrap();
            VALUE.remove(&mut old_map, Holder::Level).unwrap();
            assert_eq!(values(&index), [(1, 20)]);
            VALUE.set(&mut map, Holder::Level, 30).unwrap();
            assert_eq!(values(&index), [(1, 30)]);
            drop(current);
            assert!(values(&index).is_empty());
        }
    }

    #[test]
    fn owner_drop_destroys_index_only_payloads_outside_the_index_lock() {
        use std::sync::atomic::{AtomicBool, Ordering};

        #[derive(Clone)]
        struct Probe {
            index: std::sync::Weak<Mutex<AttachmentIndex<u32>>>,
            dropped: Arc<AtomicBool>,
            unlocked: Arc<AtomicBool>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let index = self.index.upgrade().unwrap();
                self.unlocked
                    .fetch_and(index.try_lock().is_ok(), Ordering::Relaxed);
                self.dropped.store(true, Ordering::Relaxed);
            }
        }
        let key = AttachmentType {
            namespace: "test",
            name: "drop_probe",
            default: || panic!("no default"),
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let dropped = Arc::new(AtomicBool::new(false));
        let unlocked = Arc::new(AtomicBool::new(true));
        let mut map = Attachments::default();
        key.set(
            &mut map,
            Holder::Level,
            Probe {
                index: Arc::downgrade(&index),
                dropped: dropped.clone(),
                unlocked: unlocked.clone(),
            },
        )
        .unwrap();
        let lifetime = AttachmentLifetime::new(map.bind_index(&index, 1));
        drop(map);
        assert!(!dropped.load(Ordering::Relaxed));
        drop(lifetime);
        assert!(dropped.load(Ordering::Relaxed));
        assert!(unlocked.load(Ordering::Relaxed));
        assert!(
            index
                .lock()
                .unwrap()
                .snapshot(&("test".into(), "drop_probe".into()))
                .is_empty()
        );
    }

    #[test]
    fn empty_slots_need_no_index_access_and_expired_index_destruction_is_inert() {
        let index = Arc::new(Mutex::new(AttachmentIndex::<u32>::default()));
        let guard = index.lock().unwrap();
        let empty = AttachmentLifetime::default();
        assert!(!empty.retire());
        drop(empty.clone());
        drop(empty);
        drop(guard);
        let mut map = Attachments::default();
        let lifetime = AttachmentLifetime::new(map.bind_index(&index, 1));
        drop(index);
        assert!(!lifetime.retire());
        drop(lifetime);
        eprintln!(
            "slot = {} bytes; bound box payload = {} bytes",
            size_of::<AttachmentLifetime>(),
            size_of::<AttachmentRegistration>()
        );
    }
}
