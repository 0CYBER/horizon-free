//! Typed index publication for lower-crate storage boundaries.
use std::{
    hash::Hash,
    sync::{Arc, Mutex, Weak},
};

use super::{AttachmentRegistration, Attachments, Holder, index::AttachmentIndex};

/// A dimension's attachment residency observer, independent of its typed handles.
/// Storage explicitly binds authoritative maps and unloads holders; map Drop and
/// detached save/COW copies must never imply residency changes.
pub trait AttachmentObserver<I = Holder>: Send + Sync {
    /// Bind the authoritative map, or leave it untouched if the holder cannot be
    /// represented or the dimension index has expired.
    fn bind(&self, holder: I, map: &mut Attachments) -> Option<AttachmentRegistration>;
    /// Retire current membership at a trusted storage boundary, without relying on
    /// a map-held token. Return false for unsupported, absent or expired holders.
    fn unload(&self, holder: I) -> bool;
}

/// A reusable observer that resolves ready-to-use typed handles only at residency
/// boundaries, never while iterating query rows. The resolver runs without an index
/// guard and may refuse identities it cannot faithfully represent. The observer
/// itself holds only a weak index reference, so it does not keep a dimension alive.
#[non_exhaustive]
pub struct IndexedAttachmentObserver<H, F> {
    index: Weak<Mutex<AttachmentIndex<H>>>,
    resolve: F,
}

impl<H, F> IndexedAttachmentObserver<H, F> {
    /// Capture the dimension's index weakly and install its cold holder resolver.
    /// No holder is registered or resolved until an explicit bind/unload call.
    pub fn new(index: &Arc<Mutex<AttachmentIndex<H>>>, resolve: F) -> Self {
        Self {
            index: Arc::downgrade(index),
            resolve,
        }
    }
}

impl<H, F, I> AttachmentObserver<I> for IndexedAttachmentObserver<H, F>
where
    H: Copy + Eq + Hash + Send + Sync + 'static,
    F: Fn(I) -> Option<H> + Send + Sync,
{
    fn bind(&self, holder: I, map: &mut Attachments) -> Option<AttachmentRegistration> {
        let index = self.index.upgrade()?;
        let holder = (self.resolve)(holder)?;
        Some(map.bind_index(&index, holder))
    }

    fn unload(&self, holder: I) -> bool {
        let Some(index) = self.index.upgrade() else {
            return false;
        };
        let Some(holder) = (self.resolve)(holder) else {
            return false;
        };
        let change = {
            let mut index = index.lock().unwrap();
            index.unload(holder)
        };
        change.into_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attachments::{AttachmentCodec, AttachmentError, AttachmentType};
    use horizon_core::nbt::Tag;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Handle {
        Chunk { dimension: u32, position: [i32; 2] },
        Level(u32),
    }
    fn resolve(holder: Holder) -> Option<Handle> {
        match holder {
            Holder::Chunk(position) => Some(Handle::Chunk {
                dimension: 7,
                position,
            }),
            Holder::Level => Some(Handle::Level(7)),
            _ => None,
        }
    }
    static DECODES: AtomicUsize = AtomicUsize::new(0);
    static VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "observer",
        name: "value",
        default: || 0,
        codec: Some(AttachmentCodec {
            encode: |n| Ok([("value".into(), Tag::Int(*n))].into()),
            decode: |form| {
                DECODES.fetch_add(1, Ordering::Relaxed);
                match form.get(&"value".into()) {
                    Some(Tag::Int(n)) => Ok(*n),
                    _ => Err(AttachmentError::Decode),
                }
            },
        }),
        copy_on_death: false,
        sync: false,
    };
    fn key() -> super::super::index::AttachmentKey {
        ("observer".into(), "value".into())
    }

    #[test]
    fn saved_before_read_bind_publishes_the_resolved_handle_without_decoding() {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let observer: Arc<dyn AttachmentObserver> =
            Arc::new(IndexedAttachmentObserver::new(&index, resolve));
        let holder = Holder::Chunk([30_000, -90_000]);
        let mut source = Attachments::default();
        VALUE.set(&mut source, holder, 42).unwrap();
        let saved = source.to_nbt();
        let mut map = Attachments::from_nbt(saved.clone());
        let registration = observer.bind(holder, &mut map).unwrap();
        let batch = index.lock().unwrap().snapshot(&key());
        assert_eq!(batch.len(), 1);
        assert_eq!(
            batch[0].holder,
            Handle::Chunk {
                dimension: 7,
                position: [30_000, -90_000],
            }
        );
        assert!(Arc::ptr_eq(
            &batch[0].entry,
            map.entry("observer", "value").unwrap().unwrap()
        ));
        assert_eq!(DECODES.load(Ordering::Relaxed), 0);
        assert_eq!(batch[0].entry.read(&VALUE), Ok(&42));
        assert_eq!(DECODES.load(Ordering::Relaxed), 1);
        assert_eq!(map.to_nbt(), saved);
        assert!(map.take_sync_changes().is_empty());
        assert!(registration.unregister());
    }

    #[test]
    fn unsupported_holders_leave_maps_standalone_and_create_no_membership() {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let observer = IndexedAttachmentObserver::new(&index, resolve);
        let holder = Holder::Player(123);
        let mut map = Attachments::default();
        VALUE.set(&mut map, holder, 7).unwrap();
        assert!(observer.bind(holder, &mut map).is_none());
        assert!(!observer.unload(holder));
        VALUE.set(&mut map, holder, 8).unwrap();
        assert_eq!(VALUE.get(&mut map, holder), Ok(8));
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
    }

    #[test]
    fn trusted_unload_survives_detached_cow_maps_and_retained_save_and_batch_arcs() {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let observer = IndexedAttachmentObserver::new(&index, resolve);
        let holder = Holder::Chunk([3, 5]);
        let mut map = Arc::new(Attachments::default());
        VALUE.set(Arc::make_mut(&mut map), holder, 7).unwrap();
        let registration = observer.bind(holder, Arc::make_mut(&mut map)).unwrap();
        let save = map.clone();
        let batch = index.lock().unwrap().snapshot(&key());
        // An unrelated payload COW detaches the map even before an attachment write.
        let detached = Arc::make_mut(&mut map);
        VALUE.set(detached, holder, 8).unwrap();
        let current = index.lock().unwrap().snapshot(&key());
        assert_eq!(current[0].entry.read(&VALUE), Ok(&7));
        assert!(observer.unload(holder));
        assert!(!observer.unload(holder));
        assert!(!registration.unregister());
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
        assert_eq!(batch[0].entry.read(&VALUE), Ok(&7));
        assert_eq!(
            save.entry("observer", "value")
                .unwrap()
                .unwrap()
                .read(&VALUE),
            Ok(&7)
        );
        VALUE.set(Arc::make_mut(&mut map), holder, 9).unwrap();
        assert!(index.lock().unwrap().snapshot(&key()).is_empty());
    }

    #[test]
    fn expired_index_refuses_work_without_resolving_or_retaining_the_dimension() {
        let index = Arc::new(Mutex::new(AttachmentIndex::<Handle>::default()));
        let weak = Arc::downgrade(&index);
        let calls = AtomicUsize::new(0);
        let observer = IndexedAttachmentObserver::new(&index, |holder| {
            calls.fetch_add(1, Ordering::Relaxed);
            resolve(holder)
        });
        drop(index);
        let mut map = Attachments::default();
        assert!(weak.upgrade().is_none());
        assert!(observer.bind(Holder::Level, &mut map).is_none());
        assert!(!observer.unload(Holder::Level));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        VALUE.set(&mut map, Holder::Level, 6).unwrap();
        assert_eq!(VALUE.get(&mut map, Holder::Level), Ok(6));
    }

    #[test]
    fn actual_factory_resolves_and_drops_payloads_without_an_index_guard() {
        #[derive(Default)]
        struct Events {
            resolutions: Mutex<Vec<bool>>,
            drops: Mutex<Vec<Option<usize>>>,
        }
        #[derive(Clone)]
        struct Probe {
            index: Weak<Mutex<AttachmentIndex<Handle>>>,
            events: Arc<Events>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let index = self.index.upgrade().unwrap();
                let state = index
                    .try_lock()
                    .ok()
                    .map(|index| index.snapshot(&key()).len());
                self.events.drops.lock().unwrap().push(state);
            }
        }
        static PROBE: AttachmentType<Probe> = AttachmentType {
            namespace: "observer",
            name: "value",
            default: || unreachable!(),
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let events = Arc::new(Events::default());
        let weak = Arc::downgrade(&index);
        let resolver_events = events.clone();
        let observer = IndexedAttachmentObserver::new(&index, move |holder| {
            let unlocked = weak.upgrade().unwrap().try_lock().is_ok();
            resolver_events.resolutions.lock().unwrap().push(unlocked);
            resolve(holder)
        });
        let payload = || {
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
            map
        };
        let mut level = payload();
        observer.bind(Holder::Level, &mut level).unwrap();
        drop(level);
        assert!(observer.unload(Holder::Level));
        let holder = Holder::Chunk([1, 2]);
        let mut chunk = payload();
        observer.bind(holder, &mut chunk).unwrap();
        drop(chunk);
        observer.bind(holder, &mut Attachments::default()).unwrap();
        assert_eq!(*events.resolutions.lock().unwrap(), [true; 4]);
        assert_eq!(*events.drops.lock().unwrap(), [Some(0), Some(0)]);
    }
}
