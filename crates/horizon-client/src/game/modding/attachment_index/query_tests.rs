//! Observable host query semantics through real holder writes and transaction journals.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::chunk::Chunk;
use mod_api::{AttachmentQueries, AttachmentQueriesMut, AttachmentType, HostTransaction};

static VALUE: AttachmentType<u32> = AttachmentType {
    namespace: "query_test",
    name: "value",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};
fn values(game: &Game<'_>) -> Vec<u32> {
    let host: &dyn mod_api::Host = game;
    host.query(&VALUE)
        .unwrap()
        .iter()
        .map(|row| *row.unwrap().value)
        .collect()
}

#[test]
#[ignore = "daily"]
fn query_snapshot_dimension_and_stale_writes() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        assert!(values(game).is_empty());
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        VALUE.set(game, Holder::Chunk([2, 3]), 10).unwrap();
        let batch = game.query_mut(&VALUE).unwrap();
        let row = batch.iter().next().unwrap().unwrap();
        VALUE.set(game, Holder::Chunk([2, 3]), 11).unwrap();
        assert_eq!(*row.value(), 10);
        assert_eq!(values(game), [11]);
        game.enter_level(Dim::Nether).unwrap();
        assert!(values(game).is_empty());
        assert_eq!(
            row.set(game, 12),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        VALUE.set(game, Holder::Chunk([2, 3]), 20).unwrap();
        assert_eq!(values(game), [20]);
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(values(game), [11]);
        row.set(game, 12).unwrap();
        assert_eq!(values(game), [12]);
        let retained = game.world.chunks.remove(&(2, 3)).unwrap();
        assert!(values(game).is_empty());
        assert_eq!(*row.value(), 10);
        assert_eq!(
            row.remove(game),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        game.world.chunks.insert((2, 3), retained);
        assert_eq!(
            row.set(game, 13),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        let fresh = game.query_mut(&VALUE).unwrap();
        let fresh_row = fresh.iter().next().unwrap().unwrap();
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        assert_eq!(
            fresh_row.set(game, 14),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert!(values(game).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn query_writes_join_transaction_journal_without_implicit_commit() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((2, 3), Arc::new(Chunk::empty(2, 3)));
        VALUE.set(game, Holder::Chunk([2, 3]), 10).unwrap();
        let batch = game.query_mut(&VALUE).unwrap();
        let row = batch.iter().next().unwrap().unwrap();
        let aborted: Result<(), ()> = game.transaction(|host, _| {
            row.set(host, 11).unwrap();
            assert_eq!(values(host), [11]);
            let current = host.query_mut(&VALUE).unwrap();
            current
                .iter()
                .next()
                .unwrap()
                .unwrap()
                .remove(host)
                .unwrap();
            assert!(values(host).is_empty());
            Err(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(values(game), [10]);
        assert_eq!(*row.value(), 10);
        let committed: Result<(), ()> = game.transaction(|host, _| {
            let current = host.query_mut(&VALUE).unwrap();
            current
                .iter()
                .next()
                .unwrap()
                .unwrap()
                .set(host, 15)
                .unwrap();
            Ok(())
        });
        assert_eq!(committed, Ok(()));
        drop(batch);
        assert_eq!(values(game), [15]);
    });
}
