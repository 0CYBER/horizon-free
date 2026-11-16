//! The compiled wasm listener reads the same field as the native and wire-loopback listener.
#[allow(dead_code)]
#[path = "fixture/src/block_state_notice.rs"]
mod fixture;
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[path = "support/block_state_notice.rs"]
mod support;
#[path = "../src/test_stack.rs"]
mod test_stack;
use horizon_mod_package::manifest::Package;

#[test]
#[ignore = "daily"]
fn compiled_guest_reads_predicted_like_native() {
    test_stack::on_guest_stack(|| {
        let path =
            std::env::var("HORIZON_WASM_FIXTURE").expect("build tests/fixture from this tree");
        let std = std::env::var("HORIZON_WASM_STD").expect("build this tree's shared wasm std");
        let path = std::path::Path::new(&path);
        let mut manifest = Package::read(path).unwrap().manifest;
        manifest.inject.clear();
        let info = |id: &str| {
            mod_api::PackageInfo::new(
                id.into(),
                "0.1.0".into(),
                mod_api::PackageKind::Code,
                mod_api::Side::Both,
                vec![id.into()],
                Vec::new(),
            )
        };
        let infos = [info(&manifest.id), info("block_notice_probe")];
        let loaded = horizon_mod_wasm::load(
            &std::fs::read(std).unwrap(),
            vec![horizon_mod_wasm::WasmPackage {
                manifest,
                module: Package::read_wasm(path).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        loaded.initialize(&infos).unwrap();
        mod_api::testing::with_mods(loaded.mods.into(), || {
            mod_api::__private::install_packages(&infos, &[]);
            mod_api::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::mark_game_thread();
            let world = support::World::new();
            mod_api::__private::resolve_parts(
                &std::collections::HashMap::new(),
                &std::collections::HashMap::new(),
                mod_api::__private::KnownIds {
                    block: &|id| {
                        matches!(
                            id,
                            "minecraft:stone"
                                | "minecraft:air"
                                | "minecraft:water_cauldron"
                                | "wasm_fixture:part_state"
                        )
                    },
                    item: &|id| matches!(id, "minecraft:paper" | "minecraft:snowball"),
                },
            )
            .unwrap();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            world.check(&fixture::NoticePart);
            world.read.borrow_mut().clear();
            world.dispatch(true);
            world.dispatch(false);
            assert_eq!(*world.read.borrow(), [[1, 64, 1], [0, 64, 1]]);
        });
    });
}
