#[test]
#[ignore = "daily"]
fn stateful_creation_uses_actual_state_and_preserves_existing_entity() {
    use crate::game::native_entity_refs::tests::{record_key, run_fresh_cases, with_game_at};
    use horizon_client::{chunk::Chunk, containers::BlockEntity};
    use mod_api::{self, *};
    use std::sync::{Arc, Mutex, OnceLock};

    if run_fresh_cases(
        "game::stateful_block_entity_tests::stateful_creation_uses_actual_state_and_preserves_existing_entity",
        "HORIZON_STATEFUL_CREATION_GAME_CHILD",
        &["valid"],
    ) {
        return;
    }

    const BLOCK: &str = "state_factory_test:banner";
    const KIND: &str = "state_factory_test:machine";
    #[derive(Default)]
    struct Observations {
        populated: OnceLock<BlockState>,
        calls: Mutex<Vec<BlockState>>,
        kind: OnceLock<BlockEntityTypeKey>,
    }
    #[derive(Clone)]
    struct Factory(Arc<Observations>);
    #[mod_api::__subscriptions]
    impl Hooks for Factory {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = registrar
                .block_entity_type("machine")
                .blocks(&[BLOCK])
                .build();
            registrar.block_entity_part(&kind, self.clone()).unwrap();
            record_key(&self.0.kind, kind);
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
            event
                .blocks
                .push(mod_api::Block::new(BLOCK, "minecraft:white_banner"));
        }
    }
    #[mod_api::horizon_part]
    impl BlockPart for Factory {}
    #[mod_api::horizon_part]
    impl BlockEntityPart for Factory {
        fn stateful_block_entity(&self, _: &dyn Host, event: &mut StatefulBlockEntityEvent<'_>) {
            if event.block == BLOCK {
                event.stateful = true;
            }
        }
        fn block_entity(
            &self,
            _: LevelSide<'_>,
            event: &mut BlockEntityEvent<'_>,
        ) -> Option<Compound> {
            if event.block != BLOCK {
                return None;
            }
            self.0.calls.lock().unwrap().push(event.state);
            (event.state == *self.0.populated.get().unwrap())
                .then(|| Compound::from([("progress".into(), Tag::Int(37))]))
        }
    }

    let observed = Arc::new(Observations::default());
    let mods: Mods = vec![ModEntry::new(
        "state_factory_test",
        Box::new(Factory(observed.clone())),
    )]
    .into();
    mod_platform::__private::install(mods.clone());
    crate::game::api_values::install_parts(&mods).unwrap();
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let root = crate::game::mods::content_root(&base, &Default::default()).unwrap();
    with_game_at(root.prepared().to_path_buf(), |game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let registry = game.pack.assets.registry;
        let default = registry.default_state(BLOCK).unwrap();
        let populated =
            crate::game::native_hooks::with_property(registry, default, "rotation", "1").unwrap();
        let empty =
            crate::game::native_hooks::with_property(registry, default, "rotation", "2").unwrap();
        assert_ne!(
            populated, default,
            "the factory must see a non-default state"
        );
        assert_ne!(empty, populated);
        use crate::game::api_values::TryIntoApi;
        let populated_api = populated.try_into_api().unwrap();
        let empty_api = empty.try_into_api().unwrap();
        observed.populated.set(populated_api).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        for x in [1, 2, 3] {
            assert_eq!(
                game.level_set_block(level, [x, 63, 0], stone, BlockUpdateFlags::NONE),
                Ok(true)
            );
        }
        assert!(observed.calls.lock().unwrap().is_empty());

        // The whole-block Host write must create inline, not call the factory
        // again after LevelChunk.setBlockState has already installed its entity.
        assert_eq!(
            game.level_set_block(level, [1, 64, 0], populated, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(*observed.calls.lock().unwrap(), [populated_api]);
        let handle = game
            .block_entity(level, [1, 64, 0])
            .expect("factory entity");
        assert_eq!(handle.kind(game).as_ref(), observed.kind.get());
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&(1, 64, 0)) else {
            panic!("registered type must override its vanilla banner template");
        };
        assert_eq!(entity.id(), KIND);
        assert_eq!(entity.nbt.get(&"progress".into()), Some(&Tag::Int(37)));
        entity.nbt.insert("progress".into(), Tag::Int(91));

        // This state would answer None on a fresh placement, but a property
        // change must preserve both the live payload and the handle generation.
        assert_eq!(
            game.level_set_block(level, [1, 64, 0], empty, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(*observed.calls.lock().unwrap(), [populated_api]);
        assert_eq!(handle.kind(game).as_ref(), observed.kind.get());
        let saved = game
            .block_entity_data(handle)
            .expect("original handle remains live");
        assert_eq!(saved.get(&"progress".into()), Some(&Tag::Int(91)));

        // A fresh unanswered stateful placement is absent, not an empty declared
        // mod entity or the template's built-in banner entity.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], empty, BlockUpdateFlags::NONE),
            Ok(true)
        );
        assert_eq!(*observed.calls.lock().unwrap(), [populated_api, empty_api]);
        assert!(game.world.block_entities.get(&(2, 64, 0)).is_none());
        assert!(game.block_entity(level, [2, 64, 0]).is_none());

        // A hook's LevelWriteHost write takes the GameLevel path without the
        // outer Game write wrapper and still creates exactly one instance.
        let written = game.with_level(|_, level, ticks| {
            LevelWriteHost::set_block(
                &mut crate::game::mod_api_host::LevelTicks { level, ticks },
                [3, 64, 0],
                populated_api,
                BlockUpdateFlags::NONE,
            )
        });
        assert!(written);
        assert_eq!(
            *observed.calls.lock().unwrap(),
            [populated_api, empty_api, populated_api]
        );
        let second = game
            .block_entity(level, [3, 64, 0])
            .expect("level write entity");
        assert_eq!(second.kind(game).as_ref(), observed.kind.get());
        assert_eq!(
            game.block_entity_data(second)
                .unwrap()
                .get(&"progress".into()),
            Some(&Tag::Int(37))
        );
    });
}
