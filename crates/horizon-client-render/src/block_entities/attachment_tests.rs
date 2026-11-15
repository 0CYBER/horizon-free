use super::*;
use horizon_mod_runtime::attachments::{
    AttachmentType, Holder, index::AttachmentIndex, observer::IndexedAttachmentObserver,
};
use std::sync::{Mutex, Weak};

type Index = Arc<Mutex<AttachmentIndex<BlockEntityIdentity>>>;
static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "be_test",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
fn observe(entities: &mut BlockEntities) -> Index {
    let index = Index::default();
    entities.observe_attachments(Arc::new(IndexedAttachmentObserver::new(&index, Some)));
    index
}
fn values(index: &Index) -> Vec<i32> {
    let batch = index
        .lock()
        .unwrap()
        .snapshot(&("be_test".into(), "value".into()));
    batch
        .iter()
        .map(|row| *row.entry.read(&VALUE).unwrap())
        .collect()
}

#[test]
fn replacement_columns_keep_surviving_ticker_slots_in_order() {
    use crate::{chunk::Chunk, world::World};
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    world.block_entities.set_states(vec![Ticking::Yes]);
    let [kept, omitted, other, added] = [(1, 64, 1), (2, 64, 1), (17, 64, 1), (3, 64, 1)];
    for pos in [kept, omitted, other] {
        world.block_entities.insert(pos, BlockEntity::Chest(vec![]));
    }
    let before = world.block_entities.tickers(|_| 0);
    assert_eq!(
        before.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
        [kept, omitted, other]
    );
    world.load_chunk_block_entities(
        (0, 0),
        vec![
            (kept, BlockEntity::Chest(vec![])),
            (added, BlockEntity::Chest(vec![])),
        ],
    );
    let after = world.block_entities.tickers(|_| 0);
    assert_eq!(
        after.iter().map(|(p, _)| *p).collect::<Vec<_>>(),
        [kept, other, added]
    );
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1], before[2]);
    assert!(!world.block_entities.contains_key(&omitted));
}

#[test]
fn scope_installation_binds_existing_maps_and_retires_the_old_scope() {
    let mut entities = BlockEntities::default();
    let p = (1, 64, 1);
    entities.insert(p, BlockEntity::Chest(vec![]));
    let mut saved = Attachments::default();
    VALUE.set(&mut saved, Holder::Level, 7).unwrap();
    entities.restore_attachments(p, saved.clone());
    let first = observe(&mut entities);
    let old = first
        .lock()
        .unwrap()
        .snapshot(&("be_test".into(), "value".into()));
    assert_eq!(old.len(), 1);
    assert!(Arc::ptr_eq(
        &old[0].entry,
        saved.entry("be_test", "value").unwrap().unwrap()
    ));
    let second = observe(&mut entities);
    assert!(values(&first).is_empty());
    assert_eq!(values(&second), [7]);
    VALUE
        .set(entities.attachments_mut(p).unwrap(), Holder::Level, 8)
        .unwrap();
    assert_eq!(values(&second), [8]);
    assert_eq!(old[0].entry.read(&VALUE), Ok(&7));
    assert_eq!(Arc::strong_count(&second), 1);
    drop(entities);
    assert!(values(&second).is_empty());
}

#[test]
fn taken_maps_restore_and_retire_without_reviving_removed_residency() {
    let mut entities = BlockEntities::default();
    let index = observe(&mut entities);
    let p = (1, 64, 1);
    entities.insert(p, BlockEntity::Chest(vec![]));
    VALUE
        .set(entities.attachments_mut(p).unwrap(), Holder::Level, 1)
        .unwrap();
    let saved = entities.attachments(p).unwrap().clone();
    let (lease, held) = entities.take(&p).unwrap();
    VALUE
        .set(entities.attachments_mut(p).unwrap(), Holder::Level, 2)
        .unwrap();
    assert_eq!(values(&index), [2]);
    entities.restore_attachments(p, saved);
    assert_eq!(values(&index), [1]);
    assert!(entities.put(lease, held));
    entities.insert(p, BlockEntity::Chest(vec![None]));
    assert_eq!(values(&index), [1]);
    entities.insert(p, BlockEntity::Jukebox(Default::default()));
    assert!(values(&index).is_empty());
    VALUE
        .set(entities.attachments_mut(p).unwrap(), Holder::Level, 3)
        .unwrap();
    let (lease, held) = entities.take(&p).unwrap();
    entities.unload(|pos| (pos.0 >> 4, pos.2 >> 4) == (0, 0));
    assert!(values(&index).is_empty());
    assert!(entities.attachments_mut(p).is_none());
    assert!(!entities.put(lease, held));
    assert!(!entities.contains_key(&p));
    assert!(values(&index).is_empty());
    entities.insert(p, BlockEntity::Chest(vec![]));
    VALUE
        .set(entities.attachments_mut(p).unwrap(), Holder::Level, 4)
        .unwrap();
    let (lease, held) = entities.take(&p).unwrap();
    entities.remove(&p);
    assert!(!entities.put(lease, held));
    assert!(entities.attachments_mut(p).is_none());
    assert!(values(&index).is_empty());
}

#[test]
fn held_replacements_retire_the_exact_identity_before_a_late_put() {
    let replacements: [fn(&mut BlockEntities); 2] = [
        |b| {
            b.insert((1, 64, 1), BlockEntity::Chest(vec![]));
        },
        |b| {
            b.get_or_insert_with((1, 64, 1), || BlockEntity::Chest(vec![]));
        },
    ];
    for replace in replacements {
        let mut entities = BlockEntities::default();
        let index = observe(&mut entities);
        let p = (1, 64, 1);
        entities.insert(p, BlockEntity::Chest(vec![]));
        VALUE
            .set(entities.attachments_mut(p).unwrap(), Holder::Level, 1)
            .unwrap();
        let key = ("be_test".into(), "value".into());
        let old = index.lock().unwrap().snapshot(&key);
        let (lease, held) = entities.take(&p).unwrap();
        replace(&mut entities);
        assert!(values(&index).is_empty());
        assert_ne!(entities.generation(&p), Some(old[0].holder.generation));
        VALUE
            .set(entities.attachments_mut(p).unwrap(), Holder::Level, 2)
            .unwrap();
        let (new_lease, new) = entities.take(&p).unwrap();
        assert!(!entities.put(lease, held));
        assert_eq!(values(&index), [2]);
        let current = index.lock().unwrap().snapshot(&key);
        assert_eq!(Some(current[0].holder.generation), entities.generation(&p));
        assert_eq!(old[0].entry.read(&VALUE), Ok(&1));
        assert!(entities.put(new_lease, new));
        assert_eq!(entities.generation(&p), Some(current[0].holder.generation));
    }
}

#[test]
fn all_retirement_paths_drop_payloads_after_the_index_guard() {
    #[derive(Clone)]
    struct Probe {
        index: Weak<Mutex<AttachmentIndex<BlockEntityIdentity>>>,
        drops: Arc<Mutex<Vec<bool>>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            let unlocked = self.index.upgrade().unwrap().try_lock().is_ok();
            self.drops.lock().unwrap().push(unlocked);
        }
    }
    static PROBE: AttachmentType<Probe> = AttachmentType {
        namespace: "be_test",
        name: "value",
        default: || unreachable!(),
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    let mut entities = BlockEntities::default();
    let index = observe(&mut entities);
    let drops = Arc::new(Mutex::new(Vec::new()));
    let payload = || {
        let mut map = Attachments::default();
        PROBE
            .set(
                &mut map,
                Holder::Level,
                Probe {
                    index: Arc::downgrade(&index),
                    drops: drops.clone(),
                },
            )
            .unwrap();
        map
    };
    let p = (1, 64, 1);
    entities.insert(p, BlockEntity::Chest(vec![]));
    entities.restore_attachments(p, payload());
    entities.set_attachments(p, None);
    entities.restore_attachments(p, payload());
    entities.restore_attachments(p, Attachments::default());
    entities.restore_attachments(p, payload());
    entities.insert(p, BlockEntity::Jukebox(Default::default()));
    entities.restore_attachments(p, payload());
    entities.load_chunk(vec![(p, BlockEntity::Chest(vec![]))]);
    entities.restore_attachments(p, payload());
    entities.unload(|_| true);
    entities.insert(p, BlockEntity::Chest(vec![]));
    entities.restore_attachments(p, payload());
    entities.remove(&p);
    entities.insert(p, BlockEntity::Chest(vec![]));
    entities.restore_attachments(p, payload());
    entities.clear();
    entities.insert(p, BlockEntity::Chest(vec![]));
    entities.restore_attachments(p, payload());
    drop(entities);
    assert_eq!(*drops.lock().unwrap(), [true; 8]);
}
