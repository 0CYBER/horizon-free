use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::nbt::Tag;
use mod_api::{AttachmentQueries, AttachmentQueriesMut, AttachmentType};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "mana",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

fn mob(uuid: [i32; 4]) -> Tag {
    Tag::compound([
        ("id", Tag::String("minecraft:cow".into())),
        ("UUID", Tag::IntArray(uuid.to_vec())),
        (
            "Pos",
            Tag::List(vec![
                Tag::Double(1.0f64.to_bits()),
                Tag::Double(70.0f64.to_bits()),
                Tag::Double(3.0f64.to_bits()),
            ]),
        ),
        (
            horizon_mod_runtime::attachments::ATTACHMENTS_KEY,
            Tag::compound([(
                "test",
                Tag::compound([(
                    "mana",
                    Tag::compound([("value", Tag::Compound(Default::default()))]),
                )]),
            )]),
        ),
    ])
}

#[test]
#[ignore = "daily"]
fn saved_mob_binding_survives_store_back_and_retires_at_sweep() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [17, 18, 19, 20];
        let id = game.load_mob(mob(uuid), None).unwrap();
        // Check residency before a typed read decodes the saved entry.
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        VALUE.set(game, Holder::Entity(id), 7).unwrap();
        let old = game.query_mut(&VALUE).unwrap();
        let row = old.iter().next().unwrap().unwrap();
        let work = index.lock().unwrap().work_count();
        let replacement = game.mobs[0].clone();
        horizon_client::join_seq::store_back(&mut game.mobs[0], replacement);
        assert!(game.mobs[0].attachment_lifetime.is_bound());
        let after = index.lock().unwrap().work_count();
        assert_eq!(after, work);
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        assert_eq!(VALUE.get(game, Holder::Entity(id)), Ok(7));
        assert_eq!(
            game.query(&VALUE)
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .unwrap()
                .value,
            &7
        );
        let entity = game.entity(id).unwrap();
        assert!(game.entity_discard(entity));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.native_removals_flush();
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        game.tickets.update(None);
        game.tick_creature_entities().unwrap();
        assert!(game.mobs.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(
            row.set(game, 9),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert_eq!(*row.value(), 7);
    });
}

#[test]
#[ignore = "daily"]
fn saved_mob_binding_tracks_serialized_transfer_and_parked_levels() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [21, 22, 23, 24];
        let id = game.load_mob(mob(uuid), None).unwrap();
        let source = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        assert_eq!(source.lock().unwrap().snapshot(&key).len(), 1);
        VALUE.set(game, Holder::Entity(id), 11).unwrap();
        let batch = game.query_mut(&VALUE).unwrap();
        let row = batch.iter().next().unwrap().unwrap();
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(source.lock().unwrap().snapshot(&key).len(), 1);
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(VALUE.get(game, Holder::Entity(id)), Ok(11));
        // Serialized portals discard the source owner before destination load.
        let state = game.take_transferred_entity_state(id);
        game.mobs.retain(|m| m.id != id);
        game.detach_entity(id);
        assert!(source.lock().unwrap().snapshot(&key).is_empty());
        game.enter_level(Dim::Nether).unwrap();
        let moved = game
            .load_mob_transferred(mob(uuid), None, Some(state))
            .unwrap();
        assert_ne!(moved, id); // This route allocates; retained-id vehicles have their own arrival seam.
        assert_eq!(
            game.attachment_indexes[&Dim::Nether]
                .lock()
                .unwrap()
                .snapshot(&key)
                .len(),
            1
        );
        assert_eq!(VALUE.get(game, Holder::Entity(moved)), Ok(11));
        assert_eq!(
            row.set(game, 12),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert_eq!(*row.value(), 11);
        game.mobs.clear();
        assert!(
            game.attachment_indexes[&Dim::Nether]
                .lock()
                .unwrap()
                .snapshot(&key)
                .is_empty()
        );
    });
}

#[test]
#[ignore = "daily"]
fn saved_mob_binding_rebinds_retained_id_with_a_new_generation() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let id = game.load_mob(mob([25, 26, 27, 28]), None).unwrap();
        VALUE.set(game, Holder::Entity(id), 13).unwrap();
        let batch = game.query_mut(&VALUE).unwrap();
        let row = batch.iter().next().unwrap().unwrap();
        let holder = game.entity(id).unwrap();
        let source = game.attachment_indexes[&Dim::Overworld].clone();
        let key = ("test".into(), "mana".into());
        // The retained-owner route retires before carrying the value to arrival.
        let mut owner = game.mobs.remove(0);
        owner.attachment_lifetime.retire();
        assert!(source.lock().unwrap().snapshot(&key).is_empty());
        game.enter_level(Dim::Nether).unwrap();
        game.rebind_transferred_entity(id, &mut owner);
        game.mobs.push(owner);
        let current = game.attachment_indexes[&Dim::Nether]
            .lock()
            .unwrap()
            .snapshot(&key);
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].holder, AttachmentHolder::Entity(holder));
        assert_eq!(current[0].entry.read(&VALUE), Ok(&13));
        assert_eq!(
            row.set(game, 14),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert_eq!(*row.value(), 13);
        game.mobs.clear();
        assert!(
            game.attachment_indexes[&Dim::Nether]
                .lock()
                .unwrap()
                .snapshot(&key)
                .is_empty()
        );
    });
}

#[test]
#[ignore = "daily"]
fn mob_first_store_binds_and_nested_rollback_retires_authority() {
    use mod_api::{AttachmentError, HostTransaction};
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [71, 72, 73, 74];
        let Tag::Compound(mut tag) = mob(uuid) else {
            unreachable!()
        };
        tag.remove(&horizon_mod_runtime::attachments::ATTACHMENTS_KEY.into());
        let id = game.load_mob(Tag::Compound(tag), None).unwrap();
        let holder = Holder::Entity(id);
        assert_eq!(VALUE.get_existing(game, holder), Ok(None));
        assert!(!game.mobs[0].attachment_lifetime.is_bound());
        let result: Result<(), AttachmentError> = game.transaction(|host, _| {
            VALUE.set(host, holder, 7)?;
            let index = host.attachment_indexes[&Dim::Overworld].clone();
            assert_eq!(
                index
                    .lock()
                    .unwrap()
                    .snapshot(&("test".into(), "mana".into()))
                    .len(),
                1
            );
            let inner: Result<(), AttachmentError> = host.transaction(|host, _| {
                VALUE.set(host, holder, 8)?;
                Err(AttachmentError::MissingHolder)
            });
            assert!(inner.is_err());
            assert_eq!(VALUE.get_existing(host, holder), Ok(Some(7)));
            let committed: Result<(), AttachmentError> = host.transaction(|host, _| {
                VALUE.set(host, holder, 9)?;
                Ok(())
            });
            committed?;
            Err(AttachmentError::MissingHolder)
        });
        assert!(result.is_err());
        assert!(!game.mobs[0].attachment_lifetime.is_bound());
        assert!(!game.entity_attachments[&id].has_index_binding());
        let index = game.attachment_indexes[&Dim::Overworld].clone();
        assert!(
            index
                .lock()
                .unwrap()
                .snapshot(&("test".into(), "mana".into()))
                .is_empty()
        );
        VALUE.set(game, holder, 11).unwrap();
        assert!(game.mobs[0].attachment_lifetime.is_bound());
        assert_eq!(
            game.query(&VALUE)
                .unwrap()
                .iter()
                .next()
                .unwrap()
                .unwrap()
                .value,
            &11
        );
    });
}

#[test]
#[ignore = "daily"]
fn mob_taken_store_refuses_without_writing_and_missing_store_refuses() {
    use mod_api::AttachmentError;
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [81, 82, 83, 84];
        let Tag::Compound(mut tag) = mob(uuid) else {
            unreachable!()
        };
        tag.remove(&horizon_mod_runtime::attachments::ATTACHMENTS_KEY.into());
        let id = game.load_mob(Tag::Compound(tag), None).unwrap();
        let taken = game.mobs.lift();
        assert_eq!(
            VALUE.set(game, Holder::Entity(id), 1),
            Err(AttachmentError::HolderBusy)
        );
        assert!(
            !game
                .entity_attachments
                .get(&id)
                .is_some_and(|map| map.has_index_binding())
        );
        game.mobs.restore(
            taken,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        assert_eq!(VALUE.get_existing(game, Holder::Entity(id)), Ok(None));
        for id in [0, u32::MAX] {
            assert_eq!(
                VALUE.set(game, Holder::Entity(id), 1),
                Err(AttachmentError::MissingHolder)
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn tnt_with_real_lifetime_accepts_first_store() {
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.summon_with_nbt("minecraft:tnt", [0.5, 70.0, 0.5], "{}")
            .unwrap();
        game.lan_assign_ids();
        let id = game.tnts[0].id;
        assert_eq!(VALUE.set(game, Holder::Entity(id), 7), Ok(()));
    });
}

#[test]
#[ignore = "daily"]
fn mob_inner_first_binding_commit_outer_abort_invalidates_retained_row() {
    use mod_api::{AttachmentError, HostTransaction};
    with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        let uuid = [91, 92, 93, 94];
        let Tag::Compound(mut tag) = mob(uuid) else {
            unreachable!()
        };
        tag.remove(&horizon_mod_runtime::attachments::ATTACHMENTS_KEY.into());
        let id = game.load_mob(Tag::Compound(tag), None).unwrap();
        let holder = Holder::Entity(id);
        let mut retained = None;
        let result: Result<(), AttachmentError> = game.transaction(|host, _| {
            let inner: Result<(), AttachmentError> = host.transaction(|host, _| {
                VALUE.set(host, holder, 13)?;
                retained = Some(host.query_mut(&VALUE).unwrap());
                Ok(())
            });
            inner?;
            Err(AttachmentError::MissingHolder)
        });
        assert!(result.is_err());
        assert!(!game.mobs[0].attachment_lifetime.is_bound());
        let batch = retained.unwrap();
        let row = batch.iter().next().unwrap().unwrap();
        assert_eq!(row.value(), &13);
        assert_eq!(
            row.set(game, 99),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        VALUE.set(game, holder, 17).unwrap();
        assert_eq!(
            row.set(game, 99),
            Err(mod_api::AttachmentRowError::StaleHolder)
        );
        assert_eq!(VALUE.get_existing(game, holder), Ok(Some(17)));
    });
}
