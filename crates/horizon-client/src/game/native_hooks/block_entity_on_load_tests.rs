#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_game;
    use horizon_client::{
        chunk::Chunk,
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields},
    };
    use mod_api::{BlockEntityOnLoadEvent, BlockEntityPart, Hooks, HostSide, ModEntry, Registrar};
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(i32, i32)>>>;
    struct Loaded(Log);
    #[mod_api::horizon_part]
    impl mod_api::BlockPart for Loaded {}
    #[mod_api::horizon_part]
    impl BlockEntityPart for Loaded {
        fn block_entity_tick(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::BlockEntityTickEvent<'_>,
        ) {
            self.0.lock().unwrap().push((event.pos[0], -1));
        }

        fn block_entity_on_load(&self, host: HostSide, event: &mut BlockEntityOnLoadEvent) {
            let (pos, data) = match host {
                HostSide::Server(h) => (
                    event.entity.position(h).unwrap(),
                    event.entity.data(h).unwrap(),
                ),
                HostSide::Client(h) => (
                    event.entity.position(h).unwrap(),
                    event.entity.data(h).unwrap(),
                ),
            };
            let Some(mod_api::Tag::Int(count)) = data.get(&"count".into()) else {
                panic!("saved count missing")
            };
            self.0.lock().unwrap().push((pos[0], *count));
        }
    }
    struct User(Log);
    #[mod_api::__subscriptions]
    impl Hooks for User {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = registrar.block_entity_type("holder").build();
            registrar
                .block_entity_part(&kind, Loaded(self.0.clone()))
                .unwrap();
        }
    }
    fn holder(count: i32) -> BlockEntity {
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "test:holder".into(),
            ModBlockEntityFields {
                nbt: horizon_core::nbt::Compound::from([(
                    "count".into(),
                    horizon_core::nbt::Tag::Int(count),
                )]),
                ..Default::default()
            },
        )))
    }
    fn saved(x: i32, count: i32) -> ((i32, i32, i32), BlockEntity) {
        use horizon_core::nbt::{Compound, Tag};
        horizon_client::save::decode_block_entity(&Tag::Compound(Compound::from([
            ("id".into(), Tag::String("test:holder".into())),
            ("x".into(), Tag::Int(x)),
            ("y".into(), Tag::Int(64)),
            ("z".into(), Tag::Int(1)),
            ("count".into(), Tag::Int(count)),
        ])))
        .unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_saved_data_placement_rewrite_and_reload() {
        let log = Log::default();
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test", Box::new(User(log.clone())))].into(),
            || {
                with_game(|game| {
                    mod_api::__private::resolve_block_entity_types(["test:holder"], |_| Some(()))
                        .unwrap();
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world
                        .block_entities
                        .load_chunk(vec![saved(1, 7), saved(2, 9)]);
                    game.fire_pending_block_entity_loads();
                    assert_eq!(*log.lock().unwrap(), [(2, 9), (1, 7)]);
                    game.world
                        .block_entities
                        .set_states(vec![horizon_client::block_entities::Ticking::Yes]);
                    let tickers = game.world.block_entities.tickers(|_| 0);
                    for &(pos, _) in tickers.iter() {
                        game.tick_mod_block_entity(pos);
                    }
                    assert_eq!(*log.lock().unwrap(), [(2, 9), (1, 7), (2, -1), (1, -1)]);
                    log.lock().unwrap().clear();
                    game.fire_pending_block_entity_loads();
                    game.world.block_entities.insert((1, 64, 1), holder(11));
                    game.fire_pending_block_entity_loads();
                    assert!(log.lock().unwrap().is_empty());
                    game.world.block_entities.insert((3, 64, 1), holder(0));
                    game.fire_pending_block_entity_loads();
                    game.fire_pending_block_entity_loads();
                    assert_eq!(*log.lock().unwrap(), [(3, 0)]);
                    game.tick_mod_block_entity((3, 64, 1));
                    game.fire_pending_block_entity_loads();
                    game.tick_mod_block_entity((3, 64, 1));
                    assert_eq!(*log.lock().unwrap(), [(3, 0), (3, -1), (3, -1)]);
                    game.world.block_entities.remove(&(3, 64, 1));
                    game.world.block_entities.insert((3, 64, 1), holder(12));
                    // No pass-level drain: tick this new identity immediately.
                    game.tick_mod_block_entity((3, 64, 1));
                    assert_eq!(
                        *log.lock().unwrap(),
                        [(3, 0), (3, -1), (3, -1), (3, 12), (3, -1)]
                    );
                    game.world
                        .block_entities
                        .load_chunk(vec![((1, 64, 1), holder(7))]);
                    game.fire_pending_block_entity_loads();
                    assert_eq!(
                        *log.lock().unwrap(),
                        [(3, 0), (3, -1), (3, -1), (3, 12), (3, -1), (1, 7)]
                    );
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_client_instance_reads_received_fields() {
        use crate::game::native_entity_refs::tests::with_client_game;
        use horizon_client::level_loading::Reason;
        use horizon_protocol::packets::play::SpawnInfo;
        let log = Log::default();
        mod_platform::testing::with_mods(
            vec![ModEntry::new("test", Box::new(User(log.clone())))].into(),
            || {
                with_client_game(|game| {
                    mod_api::__private::resolve_block_entity_types(["test:holder"], |_| Some(()))
                        .unwrap();
                    game.native_lifecycle =
                        crate::game::native_lifecycle::NativeLifecycle::default();
                    let (client, _server) = horizon_protocol::connection::Pipe::pair();
                    game.connect_local(client, "OnLoadTest").unwrap();
                    let spawn = SpawnInfo {
                        dimension_type: 0,
                        dimension: "minecraft:overworld".into(),
                        seed: 0,
                        game_type: 1,
                        previous_game_type: None,
                        debug: false,
                        flat: true,
                        last_death: None,
                        portal_cooldown: 0,
                        sea_level: 63,
                    };
                    game.enter_remote_dimension(&spawn, false, Reason::Other);
                    game.remote.as_mut().unwrap().logged_in = true;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.world.block_entities.insert((1, 64, 1), holder(0));
                    let Some(BlockEntity::Mod(entity)) =
                        game.world.block_entities.get_mut(&(1, 64, 1))
                    else {
                        panic!()
                    };
                    entity.merge_update_tag(&horizon_core::nbt::Compound::from([(
                        "count".into(),
                        horizon_core::nbt::Tag::Int(7),
                    )]));
                    game.tick_remote_level_with(|game| {
                        game.fire_pending_block_entity_loads();
                        Ok(())
                    })
                    .unwrap();
                    assert_eq!(*log.lock().unwrap(), [(1, 7)]);
                    game.fire_pending_block_entity_loads();
                    assert_eq!(*log.lock().unwrap(), [(1, 7)]);
                });
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_without_listener_preserves_data_and_never_queues() {
        mod_platform::testing::with_mods(vec![].into(), || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.world
                    .block_entities
                    .load_chunk(vec![((1, 64, 1), holder(7)), ((2, 64, 1), holder(9))]);
                assert_eq!(game.world.block_entities.pending_on_load_len(), 0);
                game.fire_pending_block_entity_loads();
                assert_eq!(game.world.block_entities.next_on_load(), None);
                let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&(1, 64, 1))
                else {
                    panic!()
                };
                assert_eq!(
                    entity.nbt.get(&"count".into()),
                    Some(&horizon_core::nbt::Tag::Int(7))
                );
            })
        });
    }
}
