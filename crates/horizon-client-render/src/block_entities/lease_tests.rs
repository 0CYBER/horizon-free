use super::*;
use horizon_mod_runtime::attachments::{AttachmentType, Holder};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "lease_test",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

const P: Pos = (1, 64, 1);
const Q: Pos = (17, 64, 1);
fn chest(size: usize) -> BlockEntity {
    BlockEntity::Chest(vec![None; size])
}
fn level() -> BlockEntities {
    let mut entities = BlockEntities::default();
    entities.set_states(vec![Ticking::Yes, Ticking::Idle]);
    entities
}

#[test]
fn retired_leases_never_restore_removed_or_unloaded_instances() {
    let retirements: [fn(&mut BlockEntities); 4] = [
        |b| {
            b.remove(&P);
        },
        |b| b.unload(|p| *p == P),
        BlockEntities::clear,
        |b| {
            b.tickers(|_| 0);
        },
    ];
    for retire in retirements {
        let mut entities = level();
        entities.insert(P, chest(1));
        VALUE
            .set(entities.attachments_mut(P).unwrap(), Holder::Level, 7)
            .unwrap();
        let slots = entities.tickers(|_| 0);
        let (mut lease, taken) = entities.take(&P).unwrap();
        assert!(entities.rekey(&P, &mut lease));
        retire(&mut entities);
        assert!(!entities.rekey(&P, &mut lease));
        assert!(!entities.put(lease, taken));
        assert!(entities.get(&P).is_none());
        assert!(entities.attachments_mut(P).is_none());
        assert!(entities.attachments(P).is_none());
        assert!(!entities.is_live(slots[0]));
        assert!(entities.tickers(|_| 0).is_empty());
    }
}

#[test]
fn replacement_taken_before_old_put_rejects_aba_without_changing_tick_order() {
    let replacements: [fn(&mut BlockEntities); 3] = [
        |b| {
            b.insert(P, chest(2));
        },
        |b| b.load_chunk(vec![(P, chest(2))]),
        |b| {
            b.get_or_insert_with(P, || chest(2));
        },
    ];
    for replace in replacements {
        let mut entities = level();
        entities.insert(P, BlockEntity::Mod(Box::default()));
        entities.insert(Q, chest(3));
        let before = entities.tickers(|_| 0);
        assert_eq!(before.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [P, Q]);
        let (old_lease, old) = entities.take(&P).unwrap();
        replace(&mut entities);
        VALUE
            .set(entities.attachments_mut(P).unwrap(), Holder::Level, 9)
            .unwrap();
        assert!(entities.mods_near(0.0, 0.0, 64.0).next().is_none());
        assert!(!entities.put(old_lease, old));
        assert!(matches!(entities.get(&P), Some(BlockEntity::Chest(items)) if items.len() == 2));
        let (mut old_lease, old) = entities.take(&P).unwrap();
        replace(&mut entities);
        VALUE
            .set(entities.attachments_mut(P).unwrap(), Holder::Level, 9)
            .unwrap();
        assert!(!entities.rekey(&P, &mut old_lease));
        let (new_lease, new) = entities.take(&P).unwrap();
        assert!(!entities.put(old_lease, old));
        assert!(entities.get(&P).is_none());
        assert!(entities.put(new_lease, new));
        assert_eq!(
            VALUE.get(entities.attachments_mut(P).unwrap(), Holder::Level),
            Ok(9)
        );
        assert!(matches!(entities.get(&P), Some(BlockEntity::Chest(items)) if items.len() == 2));
        assert_eq!(*entities.tickers(|_| 0), *before);
        let (lease, entity) = entities.take(&P).unwrap();
        assert!(entities.put(lease, entity));
        assert_eq!(*entities.tickers(|_| 0), *before);
    }
}

#[test]
fn a_loaded_replacement_can_stop_ticking_without_restoring_the_taken_instance() {
    let mut entities = level();
    entities.insert(P, chest(1));
    entities.insert(Q, chest(3));
    let before = entities.tickers(|_| 0);
    let (lease, entity) = entities.take(&P).unwrap();
    entities.load_chunk(vec![(P, chest(2))]);
    let after = entities.tickers(|p| u32::from(p == P));
    assert_eq!(*after, [before[1]]);
    assert!(!entities.put(lease, entity));
    assert!(matches!(entities.get(&P), Some(BlockEntity::Chest(items)) if items.len() == 2));
}

#[test]
fn remote_column_retirement_revokes_taken_instances_including_omitted_replacements() {
    for replacement in [false, true] {
        let mut entities = level();
        entities.insert(P, chest(1));
        entities.insert(Q, chest(2));
        let (lease, entity) = entities.take(&P).unwrap();
        entities.unload(|p| chunk_of(*p) == (0, 0));
        if replacement {
            entities.insert((2, 64, 1), chest(3));
        }
        assert!(!entities.put(lease, entity));
        assert!(entities.get(&P).is_none());
        assert!(matches!(entities.get(&Q), Some(BlockEntity::Chest(items)) if items.len() == 2));
    }
}

#[test]
fn a_lease_from_another_store_cannot_consume_the_current_lease() {
    let mut first = level();
    let mut second = level();
    first.insert(P, chest(1));
    second.insert(P, chest(2));
    let (mut foreign, old) = first.take(&P).unwrap();
    let (current, new) = second.take(&P).unwrap();
    assert!(!second.rekey(&P, &mut foreign));
    assert!(!second.put(foreign, old));
    assert!(second.put(current, new));
    assert!(matches!(second.get(&P), Some(BlockEntity::Chest(items)) if items.len() == 2));
}
