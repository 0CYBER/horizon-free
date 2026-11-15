use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{EntityRef, HostTransaction};
use std::collections::HashSet;
use {
    horizon_core::nbt::{Compound, Tag},
    horizon_mod_runtime::attachments::{
        ATTACHMENTS_KEY, AttachmentCodec, AttachmentError, AttachmentType, Attachments,
    },
};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "mana",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |tag| match tag.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};
static OTHER: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "other",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

fn painting(uuid: [i32; 4], contents: Tag) -> Tag {
    Tag::compound([
        ("id", Tag::String("minecraft:painting".into())),
        ("UUID", Tag::IntArray(uuid.to_vec())),
        ("block_pos", Tag::IntArray(vec![1, 70, 3])),
        ("variant", Tag::String("minecraft:kebab".into())),
        (ATTACHMENTS_KEY, contents),
    ])
}

fn contents() -> Tag {
    Tag::compound([(
        "test",
        Tag::compound([(
            "mana",
            Tag::compound([("value", Tag::compound([("n", Tag::Int(7))]))]),
        )]),
    )])
}

#[test]
#[ignore = "daily"]
fn saved_painting_residency_precedes_reads_and_retires_at_the_sweep() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [1, 2, 3, 4];
        game.load_painting(painting(uuid, contents()), Some(uuid), (0, 0));
        let id = game.paintings[0].id;
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        let batch = index.lock().unwrap().snapshot(&key);
        let entity = game.entity(id).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].holder, AttachmentHolder::Entity(entity));
        assert_eq!(batch[0].entry.read(&VALUE), Ok(&7));
        let clone = game.paintings[0].clone();
        assert!(entity.add_tag(game, "before_removal"));
        assert!(game.entity_discard(entity));
        assert_eq!(game.paintings.len(), 1);
        assert!(game.entity(id).is_none());
        assert!(game.entity_position(entity).is_none());
        // Residency ends at the sweep, but the row's handle is already stale.
        let pending = index.lock().unwrap().snapshot(&key);
        assert_eq!(pending.len(), 1);
        let AttachmentHolder::Entity(stale) = pending[0].holder else {
            panic!("not an entity row");
        };
        assert!(!stale.add_tag(game, "after_removal"));
        game.native_removals_flush();
        assert!(game.paintings.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(batch[0].entry.read(&VALUE), Ok(&7));
        // A retained UUID map or detached entity cannot extend residency.
        VALUE
            .set(
                game.entity_attachments.get_mut(&id).unwrap(),
                Holder::Entity(id),
                9,
            )
            .unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        game.load_painting(painting(uuid, contents()), Some(uuid), (0, 0));
        let replacement = game.paintings[0].id;
        assert_ne!(replacement, id);
        drop(clone);
        let current = index.lock().unwrap().snapshot(&key);
        assert_eq!(current.len(), 1);
        assert_eq!(
            current[0].holder,
            AttachmentHolder::Entity(EntityRef::new(entity.epoch(), replacement))
        );
        assert_eq!(current[0].entry.read(&VALUE), Ok(&7));
        game.paintings.clear();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn saved_painting_membership_follows_transactions_and_chunk_unload() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [5, 6, 7, 8];
        game.load_painting(painting(uuid, contents()), Some(uuid), (0, 0));
        let holder = Holder::Entity(game.paintings[0].id);
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        let other = ("test".into(), "other".into());
        let original = index.lock().unwrap().snapshot(&key);
        let aborted: Result<(), AttachmentError> = game.transaction(|host, _| {
            VALUE.set(host, holder, 11)?;
            let inner: Result<(), AttachmentError> = host.transaction(|host, _| {
                VALUE.remove(host, holder)?;
                OTHER.set(host, holder, 23)?;
                assert!(index.lock().unwrap().snapshot(&key).is_empty());
                assert_eq!(index.lock().unwrap().snapshot(&other).len(), 1);
                Err(AttachmentError::MissingHolder)
            });
            assert!(inner.is_err());
            let restored = index.lock().unwrap().snapshot(&key);
            assert_eq!(restored[0].entry.read(&VALUE), Ok(&11));
            assert!(index.lock().unwrap().snapshot(&other).is_empty());
            Err(AttachmentError::MissingHolder)
        });
        assert!(aborted.is_err());
        assert_eq!(
            index.lock().unwrap().snapshot(&key)[0].entry.read(&VALUE),
            Ok(&7)
        );
        assert_eq!(original[0].entry.read(&VALUE), Ok(&7));
        let saved = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert!(game.paintings.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        for (key, tags) in saved {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        assert_eq!(game.paintings.len(), 1);
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(original[0].entry.read(&VALUE), Ok(&7));
    });
}

#[test]
#[ignore = "daily"]
fn saved_paintings_stay_in_their_dimension_and_empty_reads_do_not_bind() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let empty = [9, 10, 11, 12];
        game.load_painting(
            painting(empty, Tag::Compound(Compound::new())),
            Some(empty),
            (0, 0),
        );
        let id = game.paintings[0].id;
        assert_eq!(VALUE.get_existing(game, Holder::Entity(id)), Ok(None));
        assert!(!game.entity_attachments[&id].has_index_binding());
        let unknown = [13, 14, 15, 16];
        game.load_painting(
            painting(unknown, Tag::compound([("unknown", Tag::Int(1))])),
            Some(unknown),
            (0, 0),
        );
        let unknown_id = game.paintings.last().unwrap().id;
        assert!(!game.entity_attachments[&unknown_id].has_index_binding());
        let malformed = [17, 18, 19, 20];
        game.load_painting(painting(malformed, Tag::Int(1)), Some(malformed), (0, 0));
        assert!(
            !game
                .entity_attachments
                .contains_key(&game.paintings.last().unwrap().id)
        );
        let loaded = [21, 22, 23, 24];
        game.load_painting(painting(loaded, contents()), Some(loaded), (0, 0));
        let loaded_id = game.paintings.last().unwrap().id;
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.enter_level(Dim::Nether).unwrap();
        let nether_uuid = [25, 26, 27, 28];
        game.load_painting(painting(nether_uuid, contents()), Some(nether_uuid), (0, 0));
        let nether = game.attachment_indexes[&Dim::Nether].clone();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(nether.lock().unwrap().snapshot(&key).len(), 1);
        game.enter_level(Dim::Overworld).unwrap();
        game.paintings.clear();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(nether.lock().unwrap().snapshot(&key).len(), 1);
        // The saved data remains, but no detached clone becomes a live publisher.
        let detached: Attachments = game.entity_attachments[&loaded_id].clone();
        assert!(!detached.has_index_binding());
    });
}

#[test]
#[ignore = "daily"]
fn first_write_painting_publication_and_rollback() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [31, 32, 33, 34];
        game.load_painting(
            painting(uuid, Tag::Compound(Compound::new())),
            Some(uuid),
            (0, 0),
        );
        let id = game.paintings[0].id;
        let holder = Holder::Entity(id);
        let store: &mut dyn horizon_mod_runtime::attachments::AttachmentStore = game;
        assert_eq!(VALUE.has(store, holder), Ok(false));
        assert_eq!(VALUE.get_existing(store, holder), Ok(None));
        assert_eq!(VALUE.remove(store, holder), Ok(None));
        assert!(!game.entity_attachments[&id].has_index_binding());
        let bad = AttachmentType {
            namespace: "test",
            name: "bad",
            default: || 0i32,
            codec: None,
            copy_on_death: true,
            sync: false,
        };
        let store: &mut dyn horizon_mod_runtime::attachments::AttachmentStore = game;
        assert_eq!(
            bad.set(store, holder, 1),
            Err(AttachmentError::InvalidCopyPolicy)
        );
        assert!(!game.entity_attachments[&id].has_index_binding());
        let aborted: Result<(), AttachmentError> = game.transaction(|host, _| {
            let store: &mut dyn horizon_mod_runtime::attachments::AttachmentStore = host;
            assert_eq!(VALUE.get(store, holder), Ok(0));
            let index = host.attachment_indexes[&Dim::Overworld].clone();
            let key = ("test".into(), "mana".into());
            let before = index.lock().unwrap().snapshot(&key);
            assert_eq!(before[0].entry.read(&VALUE), Ok(&0));
            VALUE.set(host, holder, 10)?;
            let inner: Result<(), AttachmentError> = host.transaction(|host, _| {
                VALUE.set(host, holder, 20)?;
                Err(AttachmentError::MissingHolder)
            });
            assert!(inner.is_err());
            assert_eq!(
                index.lock().unwrap().snapshot(&key)[0].entry.read(&VALUE),
                Ok(&10)
            );
            assert_eq!(before[0].entry.read(&VALUE), Ok(&0));
            Err(AttachmentError::MissingHolder)
        });
        assert!(aborted.is_err());
        assert!(!game.entity_attachments[&id].has_index_binding());
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        let store: &mut dyn horizon_mod_runtime::attachments::AttachmentStore = game;
        VALUE.set(store, holder, 17).unwrap();
        assert_eq!(
            index.lock().unwrap().snapshot(&key)[0].entry.read(&VALUE),
            Ok(&17)
        );
        let entity = game.entity(id).unwrap();
        assert!(game.entity_discard(entity));
        let store: &mut dyn horizon_mod_runtime::attachments::AttachmentStore = game;
        assert_eq!(
            VALUE.set(store, holder, 18),
            Err(AttachmentError::MissingHolder)
        );
        game.native_removals_flush();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
    });
}
