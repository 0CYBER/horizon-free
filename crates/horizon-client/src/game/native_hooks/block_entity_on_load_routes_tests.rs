#[cfg(test)]
mod tests {
    use crate::game::{
        Game,
        native_entity_refs::tests::{run_fresh_cases, with_game_role},
    };
    use horizon_client::{chunk::Chunk, containers::BlockEntity};
    use mod_api::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    type Log = Arc<Mutex<Vec<(&'static str, i32)>>>;
    struct Listener {
        air: BlockKey,
        replacement: BlockKey,
        log: Log,
        replace: Arc<AtomicBool>,
    }
    #[mod_api::horizon_part]
    impl BlockPart for Listener {}
    #[mod_api::horizon_part]
    impl BlockEntityPart for Listener {
        fn block_entity(&self, _: LevelSide<'_>, _: &mut BlockEntityEvent<'_>) -> Option<Compound> {
            Some(Compound::from([("count".into(), Tag::Int(7))]))
        }
        fn block_entity_ticks(&self, _: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
            event.ticks = true;
        }
        fn block_entity_tick(&self, _: &mut dyn ServerHost, _: &mut BlockEntityTickEvent<'_>) {
            self.log.lock().unwrap().push(("tick", -1));
        }
        fn block_entity_on_load(&self, host: HostSide, event: &mut BlockEntityOnLoadEvent) {
            let (label, data) = match &host {
                HostSide::Server(h) => ("load", event.entity.data(&**h).unwrap()),
                HostSide::Client(h) => ("client_load", event.entity.data(&**h).unwrap()),
            };
            let Some(Tag::Int(count)) = data.get(&"count".into()) else {
                panic!("missing saved count")
            };
            self.log.lock().unwrap().push((label, *count));
            if self.replace.load(Ordering::SeqCst) {
                let HostSide::Server(host) = host else {
                    panic!()
                };
                let pos = event.entity.position(host).unwrap();
                let level = event.entity.level(host).unwrap();
                let state = BlockState::of(&host.block_states(), self.replacement, &[]).unwrap();
                let air = BlockState::of(&host.block_states(), self.air, &[]).unwrap();
                assert_eq!(
                    level.set_block(host, pos, air, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
                assert_eq!(
                    level.set_block(host, pos, state, BlockUpdateFlags::KNOWN_SHAPE),
                    Ok(true)
                );
            }
        }
    }
    struct User {
        log: Log,
        replace: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for User {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let key = r
                .block_entity_type("holder")
                .blocks(&["test:a", "test:b", "test:box_shulker_box"])
                .build();
            let air = r.block_key("minecraft:air");
            let replacement = r.block_key("test:b");
            r.block_entity_part(
                &key,
                Listener {
                    air,
                    replacement,
                    log: self.log.clone(),
                    replace: self.replace.clone(),
                },
            )
            .unwrap();
        }
        fn items(&self, _: &RegistrationContext, e: &mut ItemsEvent<'_>) {
            let mut item = Item::new("test:box_shulker_box", "minecraft:purple_shulker_box");
            item.block = Some("test:box_shulker_box".into());
            e.items.push(item);
        }
        fn blocks(&self, _: &RegistrationContext, e: &mut BlocksEvent<'_>) {
            // Its item is the shulker box item above, not a block item of its own.
            e.blocks
                .push(Block::new("test:box_shulker_box", "minecraft:barrel").with_item(false));
            e.blocks
                .extend(["test:a", "test:b"].map(|id| Block::new(id, "minecraft:white_banner")));
        }
    }
    fn with_fixture(replace: bool, f: impl FnOnce(&mut Game<'_>, Log, Arc<AtomicBool>)) {
        with_fixture_role(
            replace,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            f,
        );
    }
    fn with_fixture_role(
        replace: bool,
        role: crate::game::execution_role::ExecutionRole,
        f: impl FnOnce(&mut Game<'_>, Log, Arc<AtomicBool>),
    ) {
        let log = Log::default();
        let replace = Arc::new(AtomicBool::new(replace));
        let mods: Mods = vec![ModEntry::new(
            "test",
            Box::new(User {
                log: log.clone(),
                replace: replace.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_role(root.prepared().to_path_buf(), role, |game| {
            if game.is_logical_server() {
                game.options.dedicated = Some(vec![]);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.level_set_block(level, [1, 63, 1], stone, BlockUpdateFlags::NONE)
                    .unwrap();
            }
            f(game, log, replace);
        });
    }
    fn place(game: &mut Game<'_>) {
        let level = game.level("minecraft:overworld").unwrap();
        let state = game.pack.assets.registry.default_state("test:a").unwrap();
        assert_eq!(
            game.level_set_block(level, [1, 64, 1], state, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert!(matches!(
            game.world.block_entities.get(&(1, 64, 1)),
            Some(BlockEntity::Mod(_))
        ));
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_reentrant_replacement_is_bounded() {
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_reentrant_replacement_is_bounded";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_REENTRANT", &["run"]) {
            return;
        }
        with_fixture(true, |game, log, replace| {
            place(game);
            game.fire_pending_block_entity_loads();
            assert_eq!(*log.lock().unwrap(), [("load", 7)]);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(*log.lock().unwrap(), [("load", 7), ("load", 7)]);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
            replace.store(false, Ordering::SeqCst);
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(
                *log.lock().unwrap(),
                [("load", 7), ("load", 7), ("load", 7), ("tick", -1)]
            );
            assert_eq!(game.world.block_entities.pending_on_load_len(), 0);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_shared_type_replacement_has_fresh_handle() {
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_shared_type_replacement_has_fresh_handle";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_SHARED_TYPE", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            place(game);
            let level = game.level("minecraft:overworld").unwrap();
            let old = game.block_entity(level, [1, 64, 1]).unwrap();
            let generation = game.world.block_entities.generation(&(1, 64, 1)).unwrap();
            game.fire_pending_block_entity_loads();
            let a = game.pack.assets.registry.default_state("test:a").unwrap();
            let b = game.pack.assets.registry.default_state("test:b").unwrap();
            game.world.set(1, 64, 1, b).unwrap();
            game.sync_block_entity((1, 64, 1), Some(a), b);
            let new = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(old, new);
            assert_ne!(
                game.world.block_entities.generation(&(1, 64, 1)),
                Some(generation)
            );
            assert_eq!(old.data(game), None);
            assert_eq!(
                new.data(game).unwrap().get(&"count".into()),
                Some(&Tag::Int(7))
            );
            assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
            game.tick_mod_block_entity((1, 64, 1));
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(
                *log.lock().unwrap(),
                [("load", 7), ("load", 7), ("tick", -1), ("tick", -1)]
            );
        });
    }
    fn assert_first_two_ticks(game: &mut Game<'_>, log: &Log, count: i32) {
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
        game.tick_mod_block_entity((1, 64, 1));
        game.tick_mod_block_entity((1, 64, 1));
        assert_eq!(
            *log.lock().unwrap(),
            [("load", count), ("tick", -1), ("tick", -1)]
        );
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_command_placement() {
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_command_placement";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_COMMAND", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("setblock 1 64 1 test:a"));
            assert_first_two_ticks(game, &log, 7);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_dispenser_placement() {
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_dispenser_placement";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_DISPENSER", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            let left = game
                .dispense_item(
                    (0, 64, 1),
                    horizon_core::geometry::Direction::East,
                    horizon_client::items::ItemStack::new("test:box_shulker_box", 1),
                )
                .unwrap();
            assert!(left.is_none());
            assert!(matches!(
                game.world.block_entities.get(&(1, 64, 1)),
                Some(BlockEntity::Mod(_))
            ));
            assert_first_two_ticks(game, &log, 7);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_falling_block_placement() {
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_falling_block_placement";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_FALLING", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            let state = game.pack.assets.registry.default_state("test:a").unwrap();
            assert!(game.spawn_falling_block(
                "minecraft:overworld",
                state,
                [1.5, 66.0, 1.5],
                [0.0; 3],
                Falling::default()
            ));
            let join = game.falling[0].join;
            let mut block = game.falling.extract(join).unwrap();
            let id = block.id;
            block.saved = Some(Box::new((
                [1, 2, 3, 4],
                Tag::Compound(Compound::from([(
                    "TileEntityData".into(),
                    Tag::Compound(Compound::from([("count".into(), Tag::Int(29))])),
                )])),
            )));
            game.finish_falling_block(block, [1, 64, 1]);
            game.falling.retire_extracted(id, join);
            assert_first_two_ticks(game, &log, 29);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_stale_queued_identity_is_skipped() {
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_stale_queued_identity_is_skipped";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_STALE", &["run"]) {
            return;
        }
        with_fixture(false, |game, log, _| {
            place(game);
            let a = game.pack.assets.registry.default_state("test:a").unwrap();
            let b = game.pack.assets.registry.default_state("test:b").unwrap();
            game.world.set(1, 64, 1, b).unwrap();
            game.sync_block_entity((1, 64, 1), Some(a), b);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 2);
            game.fire_pending_block_entity_loads();
            assert_eq!(*log.lock().unwrap(), [("load", 7)]);
            assert_eq!(game.world.block_entities.pending_on_load_len(), 0);
            game.tick_mod_block_entity((1, 64, 1));
            assert_eq!(*log.lock().unwrap(), [("load", 7), ("tick", -1)]);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn block_entity_on_load_real_client_chunk_receive() {
        use horizon_protocol::{
            chunk::{BlockEntityInfo, ChunkData, LightData, Paletted, Section, Strategy},
            packets::play::{LevelChunkWithLight, SpawnInfo},
            registry::Builtin,
        };
        const NAME: &str = "game::native_hooks::block_entity_on_load_routes_tests::tests::block_entity_on_load_real_client_chunk_receive";
        if run_fresh_cases(NAME, "HORIZON_ON_LOAD_CLIENT", &["run"]) {
            return;
        }
        with_fixture_role(
            false,
            crate::game::execution_role::ExecutionRole::Client,
            |game, log, _| {
                game.native_lifecycle = crate::game::native_lifecycle::NativeLifecycle::default();
                let (client, _server) = horizon_protocol::connection::Pipe::pair();
                game.connect_local(client, "OnLoadChunk").unwrap();
                game.enter_remote_dimension(
                    &SpawnInfo {
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
                    },
                    false,
                    horizon_client::level_loading::Reason::Other,
                );
                let remote = game.remote.as_mut().unwrap();
                remote.logged_in = true;
                let mut builtin = (*remote.builtin).clone();
                builtin.registries.extend(Builtin::parse(r#"{"minecraft:block_entity_type":{"entries":{"test:holder":{"protocol_id":0}}}}"#).unwrap().registries);
                remote.builtin = Arc::new(builtin);
                let air = game.res.facts().air.0 as i32;
                let state = game.pack.assets.registry.default_state("test:a").unwrap().0 as i32;
                let sections: Vec<_> = (0..24)
                    .map(|i| {
                        let mut states = vec![air; 4096];
                        if i == 8 {
                            states[17] = state;
                        }
                        Section {
                            non_empty: if i == 8 { 1 } else { 0 },
                            fluids: 0,
                            states: Paletted::from_values(&states, Strategy::Blocks, 16),
                            biomes: Paletted::from_values(&[0; 64], Strategy::Biomes, 2),
                        }
                    })
                    .collect();
                let packet = LevelChunkWithLight {
                    x: 0,
                    z: 0,
                    chunk: ChunkData {
                        heightmaps: vec![],
                        buffer: ChunkData::from_sections(&sections),
                        block_entities: vec![BlockEntityInfo {
                            packed_xz: 0x11,
                            y: 64,
                            kind: 0,
                            tag: Tag::Compound(Compound::from([("count".into(), Tag::Int(23))])),
                        }],
                    },
                    light: LightData {
                        sky_mask: vec![],
                        block_mask: vec![],
                        empty_sky_mask: vec![],
                        empty_block_mask: vec![],
                        sky: vec![],
                        block: vec![],
                    },
                };
                game.remote_chunk(&packet, None);
                assert!(log.lock().unwrap().is_empty());
                assert_eq!(game.world.block_entities.pending_on_load_len(), 1);
                let level = game.level("minecraft:overworld").unwrap();
                let first = game.block_entity(level, [1, 64, 1]).unwrap();
                game.fire_pending_block_entity_loads();
                assert_eq!(*log.lock().unwrap(), [("client_load", 23)]);
                game.remote_chunk(&packet, None);
                assert_ne!(game.block_entity(level, [1, 64, 1]).unwrap(), first);
                game.fire_pending_block_entity_loads();
                game.fire_pending_block_entity_loads();
                assert_eq!(
                    *log.lock().unwrap(),
                    [("client_load", 23), ("client_load", 23)]
                );
            },
        );
    }
}
