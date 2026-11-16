// Included in persist::tests to keep existing test paths stable.
#[test]
#[ignore = "daily"]
fn deferred_position_records_do_not_resurrect_a_rejected_duplicate_uuid() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let source = (32, 48);
        game.world.chunks.clear();
        game.world
            .chunks
            .insert(source, Arc::new(Chunk::empty(source.0, source.1)));
        let uuid = [2469, 0, 0, 2];
        let live = Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            (
                "Pos",
                Tag::List(
                    [512.0f64, 200.0, 768.0]
                        .into_iter()
                        .map(|value| Tag::Double(value.to_bits()))
                        .collect(),
                ),
            ),
        ]);
        let id = game.load_mob(live, None).unwrap();
        let duplicate = Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("UUID", Tag::IntArray(uuid.to_vec())),
            ("Pos", Tag::String("invalid".into())),
        ]);
        game.world.queue_entity_nbt(source, vec![duplicate]);
        game.spawn_loaded_entities();
        // Remove the original after disk admission has rejected the duplicate.
        game.mobs.retain(|mob| mob.id != id);
        game.forget_entity(id);
        let unloaded = game.entity_chunks(&HashSet::from([source]), true);
        assert!(
            unloaded.iter().all(|(_, tags)| tags.is_empty()),
            "a rejected UUID must not be saved for a later retry"
        );
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for (key, tags) in unloaded {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        assert!(game.target_with_uuid(uuid).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn malformed_position_from_a_distant_chunk_survives_save_and_unload() {
    super::super::native_entity_refs::tests::with_game(|game| {
        let source = (32, 48);
        game.world.chunks.clear();
        game.world
            .chunks
            .insert(source, Arc::new(Chunk::empty(source.0, source.1)));
        let raw = Tag::compound([
            ("id", Tag::String("minecraft:pig".into())),
            ("Pos", Tag::String("invalid".into())),
            ("UUID", Tag::IntArray(vec![2469, 0, 0, 1])),
            ("marker", Tag::Int(37)),
        ]);
        game.world.queue_entity_nbt(source, vec![raw.clone()]);
        game.spawn_loaded_entities();
        let keys = HashSet::from([source]);
        let saved = game.entity_chunks(&keys, false);
        assert_eq!(saved, vec![(source, vec![raw.clone()])]);
        assert!(
            saved.iter().all(|(key, _)| *key != (0, 0)),
            "never overwrite an unread origin chunk"
        );
        let unloaded = game.entity_chunks(&keys, true);
        assert_eq!(unloaded, vec![(source, vec![raw])]);
    });
}

#[test]
#[ignore = "daily"]
fn neutral_mob_load_notifies_target_even_without_saved_anger() {
    use std::sync::{Arc, Mutex};

    struct Observe(Arc<Mutex<Vec<(u32, bool)>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Observe {
        fn living_change_target(
            &self,
            host: &dyn mod_api::Host,
            event: &mut mod_api::LivingChangeTargetEvent<'_>,
        ) -> mod_api::Flow {
            assert!(event.mob.living().health(host).is_some());
            self.0
                .lock()
                .unwrap()
                .push((event.mob.entity().id(), event.original_target.is_none()));
            mod_api::Flow::Continue
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "observe",
        Box::new(Observe(observed.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            let decoded = entity_nbt::mob_from_nbt(
                &entity("minecraft:wolf"),
                game.res.mob_types(),
                300,
                None,
            )
            .unwrap();
            assert_eq!(decoded.position, Vec3::ZERO);
            let wolf = game.load_mob(entity("minecraft:wolf"), None).unwrap();
            let golem = game.load_mob(entity("minecraft:iron_golem"), None).unwrap();
            let pig = game.load_mob(entity("minecraft:pig"), None).unwrap();
            for id in [wolf, golem, pig] {
                assert_eq!(
                    game.mobs.iter().find(|mob| mob.id == id).unwrap().position,
                    Vec3::ZERO
                );
            }
            assert_eq!(*observed.lock().unwrap(), [(wolf, true), (golem, true)]);
            // The same codec default applies when Pos cannot decode.
            for pos in [
                Tag::String("invalid".into()),
                Tag::List(vec![Tag::Double(1.0f64.to_bits())]),
            ] {
                let raw =
                    Tag::compound([("id", Tag::String("minecraft:pig".into())), ("Pos", pos)]);
                let id = game.load_mob(raw, None).unwrap();
                assert_eq!(
                    game.mobs.iter().find(|mob| mob.id == id).unwrap().position,
                    Vec3::ZERO
                );
            }
        });
    });
}
