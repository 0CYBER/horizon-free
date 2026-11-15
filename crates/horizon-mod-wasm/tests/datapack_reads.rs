//! Run the declaring guest through Wasmtime and the generated Host imports.
#[path = "support/no_hooks.rs"]
mod no_hooks;
use mod_api::__private::game::BackendRefOf;
use mod_api::*;
struct World(
    DatapackRegistrySet,
    std::cell::Cell<[u32; 3]>,
    std::cell::Cell<bool>,
);
impl Host for World {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
impl Backend for World {}
mod_api::impl_backend_areas!(World, except(levels));
mod_api::impl_host_areas!(World, except(levels, keys));
impl mod_api::prelude::LevelsHost for World {
    fn is_loaded(&self, id: &str) -> bool {
        if id == "datapack_fixture_completed" {
            self.2.set(true);
        }
        id == "datapack_fixture" || id == "datapack_fixture_completed"
    }
}
impl mod_api::prelude::KeysHost for World {
    fn registry_entry(&self, registry: &str, id: &str) -> Option<RegistryEntry> {
        self.0.contains_entry(registry, id).then(|| {
            RegistryEntry::new(
                Identifier::parse(registry).unwrap(),
                Identifier::parse(id).unwrap(),
            )
        })
    }
    fn registry_entries(&self, registry: &str) -> Option<Vec<Identifier>> {
        self.0
            .entries(registry)
            .ok()?
            .into_iter()
            .map(|id| Identifier::parse(id).ok())
            .collect()
    }
    fn registry_entry_in_tag(&self, entry: &RegistryEntry, tag: &str) -> Option<bool> {
        self.0
            .tag_contains(&entry.registry.to_string(), &entry.id.to_string(), tag)
    }
}
impl mod_api::prelude::LevelsBackend for World {
    fn level_datapack_entries(&self, _: LevelRef, registry: &str) -> Option<Vec<String>> {
        let mut calls = self.1.get();
        calls[0] += 1;
        self.1.set(calls);
        Some(
            self.0
                .entries(registry)
                .ok()?
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
    }
    fn level_datapack_handle(
        &self,
        _: LevelRef,
        registry: &str,
        entry: &str,
    ) -> Result<DatapackHandle, DatapackReadError> {
        let mut calls = self.1.get();
        calls[1] += 1;
        self.1.set(calls);
        self.0.handle(registry, entry)
    }
    fn level_datapack_read(
        &self,
        _: LevelRef,
        handle: &DatapackHandle,
    ) -> Result<Vec<(String, DatapackValue)>, DatapackReadError> {
        let mut calls = self.1.get();
        calls[2] += 1;
        self.1.set(calls);
        Ok(self.0.get(handle)?.values.clone())
    }
}
impl ServerHost for World {}
mod_api::impl_server_host_areas!(World);
#[test]
#[ignore = "requires HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built from this tree"]
fn real_wasm_datapack_reads_preserve_values_and_ordering() {
    crate::test_stack::on_guest_stack(|| {
        struct Logger;
        impl log::Log for Logger {
            fn enabled(&self, _: &log::Metadata<'_>) -> bool {
                true
            }
            fn log(&self, record: &log::Record<'_>) {
                eprintln!("{}", record.args());
            }
            fn flush(&self) {}
        }
        let _ = log::set_logger(&Logger);
        log::set_max_level(log::LevelFilter::Warn);
        use horizon_mod_package::manifest::Package;
        use horizon_mod_wasm::{WasmPackage, load};
        let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
        let mut manifest = Package::read(std::path::Path::new(&path)).unwrap().manifest;
        // This fixture variant exercises ordinary hooks and Parts, not game points.
        manifest.inject.clear();
        let info = PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            PackageKind::Code,
            Side::Both,
            vec![manifest.id.clone()],
            vec![],
        );
        let loaded = load(
            &std::fs::read(std::env::var("HORIZON_WASM_STD").unwrap()).unwrap(),
            vec![WasmPackage {
                manifest,
                module: Package::read_wasm(std::path::Path::new(&path)).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        loaded.initialize(&[info]).unwrap();
        let declaration = DatapackRegistryDeclaration {
            mod_id: "wasm_fixture".into(),
            key: "wasm_fixture:spell".into(),
            fields: vec![DatapackField::required(
                "power",
                DatapackFieldKind::Int { min: 0, max: 100 },
            )],
            sync: DatapackRegistrySync::ServerOnly,
        };
        let mut candidate = DatapackCandidate::build(
            &[declaration],
            &[
                (
                    "data/wasm_fixture/wasm_fixture/spell/fire.json".into(),
                    vec![("pack".to_owned(), br#"{"power":7}"#.to_vec())],
                ),
                (
                    "data/addon/wasm_fixture/spell/ice.json".into(),
                    vec![("pack".to_owned(), br#"{"power":3}"#.to_vec())],
                ),
            ],
        )
        .unwrap();
        candidate
            .bind_tags(
                "wasm_fixture:spell",
                [("wasm_fixture:hot".into(), vec!["wasm_fixture:fire".into()])].into(),
            )
            .unwrap();
        let mut world = World(
            DatapackRegistrySet::first(candidate, 8),
            std::cell::Cell::new([0; 3]),
            std::cell::Cell::new(false),
        );
        mod_api::testing::with_mods(loaded.mods.into(), || {
            mod_api::__private::mark_game_thread();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            mod_api::dispatch_server_started(&mut world, true);
            assert!(
                world.2.get(),
                "all assertions in the guest must finish; calls: {:?}",
                world.1.get()
            );
            assert_eq!(
                world.1.get(),
                [2, 2, 1],
                "the real guest must execute all three host imports"
            );
        });
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
