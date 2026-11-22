//! Independently built library, observer and dependent packages load through the production loader.
//! Its own test binary: the game's points registry is process-global and sealed once.
// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
#[path = "support/no_hooks.rs"]
mod no_hooks;
use horizon_mod_package::manifest::Package;
use horizon_mod_wasm::{WasmPackage, load};

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl mod_api::ServerHost for Quiet {}
mod_api::impl_server_host_areas!(Quiet);

#[test]
fn real_library_dependency() {
    crate::test_stack::on_guest_stack(|| {
        let (Ok(library), Ok(dependent), Ok(observer), Ok(std)) = (
            std::env::var("HORIZON_WASM_LIBRARY"),
            std::env::var("HORIZON_WASM_DEPENDENT"),
            std::env::var("HORIZON_WASM_OBSERVER"),
            std::env::var("HORIZON_WASM_STD"),
        ) else {
            eprintln!("skipping: inter-mod wasm fixtures are not set");
            return;
        };
        let mut packages = Vec::new();
        let mut infos = Vec::new();
        for path in [&library, &dependent, &observer] {
            let path = std::path::Path::new(path);
            let manifest = Package::read(path).unwrap().manifest;
            infos.push(mod_api::PackageInfo::new(
                manifest.id.clone(),
                manifest.version.clone(),
                mod_api::PackageKind::Code,
                mod_api::Side::Both,
                vec![manifest.id.clone()],
                Vec::new(),
            ));
            packages.push(WasmPackage {
                manifest,
                module: Package::read_wasm(path).unwrap(),
            });
        }
        let mut replaced: Vec<_> = packages
            .iter()
            .map(|p| WasmPackage {
                manifest: p.manifest.clone(),
                module: p.module.clone(),
            })
            .collect();
        // Another valid library member, but not the build these dependents compiled against.
        replaced[0].module.extend_from_slice(&[0, 2, 1, b'x']);
        let mut manifest =
            serde_json::to_value(replaced[0].manifest.wasm.as_ref().unwrap()).unwrap();
        manifest["sha512"] = horizon_mod_package::manifest::sha512_of(&replaced[0].module).into();
        replaced[0].manifest.wasm = Some(serde_json::from_value(manifest).unwrap());
        let refused = load(&std::fs::read(&std).unwrap(), replaced);
        assert!(refused.refused.iter().any(|(_, r)| matches!(r, horizon_mod_wasm::Refusal::LinkedAgainst { dependency } if dependency == "wasm_test_library")));
        let loaded = load(&std::fs::read(std).unwrap(), packages);
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 3);
        loaded.initialize(&infos).unwrap();
        mod_api::testing::with_mods(loaded.mods.into(), || {
            mod_api::__private::install_parts(
                mod_api::__private::mods(),
                &mod_api::__private::part_tests::rule_schema(),
                |_| false,
            )
            .unwrap();
            mod_api::__private::mark_game_thread();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            let mut entity = mod_api::ModBlockEntity::new(
                "wasm_test_dependent:dependent".into(),
                Default::default(),
            );
            mod_api::dispatch_block_entity_tick(
                &mut Quiet,
                &mut mod_api::BlockEntityTickReply::Changed,
                [0, 64, 0],
                &mut entity,
            );
            assert_eq!(&entity.data[..2], [37, 1], "library call and owned Drop");
            let address = entity.data[2];
            let mut observer = mod_api::ModBlockEntity::new(
                "wasm_test_observer:observer".into(),
                Default::default(),
            );
            mod_api::dispatch_block_entity_tick(
                &mut Quiet,
                &mut mod_api::BlockEntityTickReply::Changed,
                [0, 64, 0],
                &mut observer,
            );
            assert_eq!(
                observer.data,
                [73, address],
                "one static, including its address"
            );
        });
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
