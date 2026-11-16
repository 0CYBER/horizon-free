use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{chunk::Chunk, containers::BlockEntity};
use horizon_core::nbt::Tag;
use horizon_mod_runtime::attachments::{
    AttachmentCodec, AttachmentError, AttachmentType, Attachments,
};
use mod_api::HostTransaction;

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "be_test",
    name: "value",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |form| match form.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};
fn saved(value: i32) -> Attachments {
    let mut map = Attachments::default();
    VALUE.set(&mut map, Holder::Level, value).unwrap();
    Attachments::from_nbt(map.to_nbt())
}

#[test]
#[ignore = "daily"]
fn block_entity_rows_follow_load_restore_replacement_and_chunk_unload_in_their_dimension() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let level = game.level(Dim::Overworld.id()).unwrap();
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("be_test".into(), "value".into());
        let p = (1, 64, 1);
        let q = (2, 64, 1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        for pos in [p, q] {
            game.world.set_stored(pos.0, pos.1, pos.2, chest);
        }
        game.world.load_chunk_block_entities(
            (0, 0),
            vec![
                (p, BlockEntity::Chest(vec![None; 27])),
                (q, BlockEntity::Chest(vec![None; 27])),
            ],
        );
        for pos in [p, q] {
            game.world.block_entities.restore_attachments(pos, saved(7));
        }
        let old = index.lock().unwrap().snapshot(&key);
        assert_eq!(old.len(), 2);
        let handle = game.block_entity(level, [1, 64, 1]).unwrap();
        assert_eq!(old[0].holder, AttachmentHolder::BlockEntity(handle));
        assert!(Arc::ptr_eq(
            &old[0].entry,
            game.world
                .block_entities
                .attachments(p)
                .unwrap()
                .entry("be_test", "value")
                .unwrap()
                .unwrap()
        ));
        let keep = game.world.block_entities.chunk_attachments(0, 0);
        let aborted: Result<(), AttachmentError> = game.transaction(|host, _| {
            VALUE.set(host, Holder::BlockEntity([1, 64, 1]), 9)?;
            let tentative = index.lock().unwrap().snapshot(&key);
            assert_eq!(tentative[0].entry.read(&VALUE), Ok(&9));
            Err(AttachmentError::MissingHolder)
        });
        assert!(aborted.is_err());
        let restored = index.lock().unwrap().snapshot(&key);
        assert_eq!(restored[0].entry.read(&VALUE), Ok(&7));
        game.enter_level(Dim::Nether).unwrap();
        let nether = game.attachment_indexes[&Dim::Nether].clone();
        let nether_level = game.level(Dim::Nether.id()).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.set_stored(p.0, p.1, p.2, chest);
        game.world
            .load_chunk_block_entities((0, 0), vec![(p, BlockEntity::Chest(vec![]))]);
        game.world.block_entities.restore_attachments(p, saved(7));
        let other = nether.lock().unwrap().snapshot(&key);
        assert_eq!(
            other[0].holder,
            AttachmentHolder::BlockEntity(game.block_entity(nether_level, [1, 64, 1]).unwrap())
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 2);
        // A replacement column omits q and installs a fresh instance at p.
        let (stale, removed) = game.world.block_entities.take(&p).unwrap();
        game.world
            .load_chunk_block_entities((0, 0), vec![(p, BlockEntity::Chest(vec![]))]);
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert!(!game.world.block_entities.contains_key(&q));
        game.world.block_entities.restore_attachments(p, saved(11));
        let (current, replacement) = game.world.block_entities.take(&p).unwrap();
        assert!(!game.world.block_entities.put(stale, removed));
        assert!(game.world.block_entities.put(current, replacement));
        assert_eq!(
            index.lock().unwrap().snapshot(&key)[0].entry.read(&VALUE),
            Ok(&11)
        );
        let (stale, removed) = game.world.block_entities.take(&p).unwrap();
        let column = game.world.chunks.remove(&(0, 0)).unwrap();
        // Match Game's existing ordering: chunk unload callbacks precede BE unload.
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.world
            .block_entities
            .unload(|pos| (pos.0 >> 4, pos.2 >> 4) == (0, 0));
        assert!(!game.world.block_entities.put(stale, removed));
        assert!(!game.world.block_entities.contains_key(&p));
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(nether.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(old[0].entry.read(&VALUE), Ok(&7));
        for (_, map) in keep {
            assert_eq!(
                map.entry("be_test", "value").unwrap().unwrap().read(&VALUE),
                Ok(&7)
            );
        }
        drop(column);
    });
}
