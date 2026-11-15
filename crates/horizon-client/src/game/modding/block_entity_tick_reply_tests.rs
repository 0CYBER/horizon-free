//! Shared Parts run natively and as compiled wasm guests against real Game storage.
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity},
};
use mod_api::{Hooks, Registrar};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
#[path = "../../../../horizon-mod-wasm/tests/fixture/src/tick_reply.rs"]
mod fixture;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(registrar);
    }
}

struct Effects(Arc<[AtomicU32; 2]>);
#[mod_api::horizon_part]
impl mod_api::BlockPart for Effects {
    fn contents_changed(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::ContentsChangedEvent,
    ) {
        if event.pos == horizon_core::position::BlockPos::new(1, 64, 1) {
            self.0[0].fetch_add(1, Ordering::Relaxed);
        }
    }

    fn on_neighbor_change(
        &self,
        _: &dyn mod_api::LevelHost,
        event: &mut mod_api::OnNeighborChangeEvent,
    ) {
        if event.pos == horizon_core::position::BlockPos::new(1, 64, 2)
            && event.neighbor == horizon_core::position::BlockPos::new(1, 64, 1)
        {
            self.0[1].fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Effects {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let stone = registrar.block_key("minecraft:stone");
        registrar.block_part(stone, Effects(self.0.clone()));
        let glass = registrar.block_key("minecraft:glass");
        registrar.block_part(glass, Effects(self.0.clone()));
    }
}

pub(super) fn wasm_mods() -> Vec<mod_api::ModEntry> {
    use horizon_mod_package::manifest::Package;
    use horizon_mod_wasm::{WasmPackage, load};
    let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
    let manifest = Package::read(&path).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![manifest.id.clone()],
        vec![],
    );
    let loaded = load(
        &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![WasmPackage {
            manifest,
            module: Package::read_wasm(&path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    assert_eq!(loaded.mods.len(), 1);
    loaded.initialize(&[info]).unwrap();
    loaded.mods
}

#[test]
#[ignore = "needs prepared resources and compiled HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn native_and_compiled_wasm_tick_reply_matrix() {
    const NAME: &str =
        "game::block_entity_tick_reply_tests::native_and_compiled_wasm_tick_reply_matrix";
    if super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "TICK_REPLY_TARGET",
        &["native", "wasm"],
    ) {
        return;
    }
    horizon_guest_stack::run_on_guest_stack(|| {
        let mut entries = if std::env::var("TICK_REPLY_TARGET").unwrap() == "wasm" {
            wasm_mods()
        } else {
            vec![mod_api::ModEntry::new("wasm_fixture", Box::new(Native))]
        };
        let effects = Arc::new([AtomicU32::new(0), AtomicU32::new(0)]);
        entries.push(mod_api::ModEntry::new(
            "tick_effects",
            Box::new(Effects(effects.clone())),
        ));
        let mods: mod_api::Mods = entries.into();
        mod_platform::testing::with_mods(mods.clone(), || {
            super::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
            let root = super::mods::content_root(&base, &Default::default()).unwrap();
            super::native_entity_refs::tests::with_game_at(root.prepared().to_owned(), |game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in [1, 2] {
                    game.world.set(x, 64, 1, stone).unwrap();
                }
                let glass = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:glass")
                    .unwrap();
                game.world.set(1, 64, 2, glass).unwrap();
                let state = game.world.block(1, 64, 1).0;
                game.world.block_entities.set_mod_containers(
                    [(
                        state,
                        Arc::new(horizon_client::containers::ModContainer {
                            slots: 1,
                            ..Default::default()
                        }),
                    )]
                    .into_iter()
                    .collect(),
                );
                game.spawn_item(
                    horizon_client::items::ItemStack::new("minecraft:stone", 5),
                    [0.0; 3],
                    [0.0; 3],
                    0,
                );
                let item_id = game.dropped[0].id;
                let entity = |mode| {
                    ModBlockEntity::new(
                        "wasm_fixture:tick_reply".into(),
                        horizon_client::containers::ModBlockEntityFields {
                            data: vec![mode, 0, 0, item_id as i32],
                            ..Default::default()
                        },
                    )
                };
                game.world
                    .block_entities
                    .insert((2, 64, 1), BlockEntity::Mod(Box::new(entity(0))));
                let mut expected_item_count = 5;
                for mode in [0, 8, 1, 4, 5, 9, 6, 7, 10, 3, 2] {
                    if mode == 7 {
                        expected_item_count = 2;
                    }
                    let original = entity(mode);
                    let revision = original.revision();
                    game.world
                        .block_entities
                        .insert((1, 64, 1), BlockEntity::Mod(Box::new(original)));
                    game.world.modified.clear();
                    let rounds = if matches!(mode, 0 | 8) { 64 } else { 1 };
                    for _ in 0..rounds {
                        for counter in effects.iter() {
                            counter.store(0, Ordering::Relaxed);
                        }
                        game.tick_mod_block_entity((1, 64, 1));
                        if mode == 2 {
                            assert!(matches!(
                                game.world.block_entities.get(&(1, 64, 1)),
                                Some(BlockEntity::Chest(_))
                            ));
                            assert!(game.world.modified.contains(&(0, 0)));
                            continue;
                        }
                        let Some(BlockEntity::Mod(current)) =
                            game.world.block_entities.get(&(1, 64, 1))
                        else {
                            panic!("entity lost")
                        };
                        let changed = matches!(mode, 1 | 4 | 5 | 9 | 10);
                        assert_eq!(
                            current.data,
                            [
                                mode,
                                match mode {
                                    1 => 1,
                                    5 => 7,
                                    _ => 0,
                                },
                                if matches!(mode, 4 | 9) { 42 } else { 0 },
                                item_id as i32,
                            ]
                        );
                        assert_eq!(
                            current.nbt,
                            if mode == 5 {
                                mod_api::Compound::from([("first".into(), mod_api::Tag::Int(7))])
                            } else {
                                Default::default()
                            }
                        );
                        assert_eq!(current.items.len(), usize::from(changed));
                        if mode == 10 {
                            assert_eq!(current.items[0].as_ref().unwrap().count, 2);
                        } else {
                            assert!(current.items.iter().all(Option::is_none));
                        }
                        assert_eq!(current.revision().identity(), revision.identity());
                        if changed {
                            assert!(current.revision().changes() > revision.changes());
                        } else {
                            assert_eq!(current.revision().changes(), revision.changes());
                        }
                        assert_eq!(game.world.modified.contains(&(0, 0)), changed || mode == 3);
                        for counter in effects.iter() {
                            assert_eq!(counter.load(Ordering::Relaxed), u32::from(mode == 10));
                        }
                        assert_eq!(game.dropped[0].stack.count, expected_item_count);
                    }
                    if mode == 3 {
                        let Some(BlockEntity::Mod(sink)) =
                            game.world.block_entities.get(&(2, 64, 1))
                        else {
                            panic!("sink lost")
                        };
                        assert_eq!(sink.data, [0, 1, 0, item_id as i32]);
                        assert_eq!(sink.revision().changes(), 1);
                    }
                }
            });
        });
    });
}
