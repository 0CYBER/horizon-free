use super::*;
fn holder() -> BlockEntity {
    BlockEntity::Mod(Box::new(crate::containers::ModBlockEntity::new(
        "test:holder".into(),
        Default::default(),
    )))
}
#[test]
fn on_load_tracks_identity_and_skips_vanilla_and_retired_instances() {
    let mut entities = BlockEntities::default();
    entities.set_on_load_enabled(true);
    let pos = (1, 64, 1);
    entities.insert(pos, holder());
    let generation = entities.generation(&pos).unwrap();
    assert_eq!(entities.next_on_load(), Some((pos, generation)));
    entities.insert(pos, holder());
    assert_eq!(entities.next_on_load(), None);
    let (lease, entity) = entities.take(&pos).unwrap();
    assert!(entities.put(lease, entity));
    assert_eq!(entities.next_on_load(), None);
    entities.insert((2, 64, 1), holder());
    entities.remove(&(2, 64, 1));
    entities.insert((3, 64, 1), BlockEntity::Chest(vec![]));
    assert_eq!(entities.next_on_load(), None);
    entities.load_chunk(vec![(pos, holder())]);
    let replacement = entities.generation(&pos).unwrap();
    assert_ne!(generation, replacement);
    assert_eq!(entities.next_on_load(), Some((pos, replacement)));
    assert_eq!(entities.next_on_load(), None);
}

#[test]
fn on_load_without_listener_never_queues_mod_chunk_entities() {
    let mut entities = BlockEntities::default();
    entities.load_chunk(vec![((1, 64, 1), holder()), ((2, 64, 1), holder())]);
    assert_eq!(entities.pending_on_load_len(), 0);
    entities.insert((3, 64, 1), holder());
    entities.replace((1, 64, 1), holder());
    entities.get_or_insert_with((4, 64, 1), holder);
    assert_eq!(entities.pending_on_load_len(), 0);
}

#[test]
fn on_load_batch_counts_stale_entries_and_defers_new_work() {
    let mut entities = BlockEntities::default();
    entities.set_on_load_enabled(true);
    let pos = (1, 64, 1);
    entities.insert(pos, holder());
    entities.replace(pos, holder());
    assert_eq!(entities.pending_on_load_len(), 2);
    let mut remaining = entities.pending_on_load_len();
    assert_eq!(
        entities.next_on_load_in_batch(&mut remaining),
        Some((pos, entities.generation(&pos).unwrap()))
    );
    assert_eq!(remaining, 0);
    entities.insert((2, 64, 1), holder());
    assert_eq!(entities.next_on_load_in_batch(&mut remaining), None);
    assert_eq!(entities.pending_on_load_len(), 1);
}
