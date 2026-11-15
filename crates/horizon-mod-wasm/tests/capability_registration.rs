//! Production-loaded compiled guest, bracketed by native mods in registration order.
#[allow(dead_code)]
#[path = "fixture/src/capability_registration.rs"]
mod fixture;
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[path = "../src/test_stack.rs"]
mod test_stack;

use horizon_mod_package::Package;
use horizon_mod_wasm::{WasmPackage, load};
use mod_api::prelude::ItemsHost;
use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};

struct Earlier;
#[mod_api::__subscriptions]
impl Hooks for Earlier {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for index in 0..4 {
            fixture::covered(r, index, false);
        }
        fixture::provide(r, 0);
        fixture::covered(r, 0, true);
    }
}

struct Later;
#[mod_api::__subscriptions]
impl Hooks for Later {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        fixture::covered(r, 0, true);
        fixture::covered(r, 1, true);
        fixture::covered(r, 2, false);
        fixture::covered(r, 3, false);
        fixture::provide(r, 2);
        fixture::covered(r, 2, true);
        // Repeating the guest's explicit marks in another mod is still idempotent.
        r.set_capability_proxyable(&fixture::PROXY, true);
        r.set_capability_proxyable(&fixture::NON_PROXY, false);
    }
}

fn package_info(id: &str) -> mod_api::PackageInfo {
    mod_api::PackageInfo::new(
        id.into(),
        "0.1.0".into(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
        vec![id.into()],
        vec![],
    )
}

fn with_guest(conflict: Option<&str>, f: impl FnOnce() + Send) {
    test_stack::on_guest_stack(|| {
        let path = std::env::var("HORIZON_WASM_FIXTURE").expect(
            "build tests/fixture with cargo horizon-mod build --wasm --catalog points.json",
        );
        let std = std::env::var("HORIZON_WASM_STD").expect("build the fixture's shared wasm std");
        let path = std::path::Path::new(&path);
        let mut manifest = Package::read(path).unwrap().manifest;
        // No injection points: this regression exercises only the generated public API.
        manifest.inject.clear();
        let mut infos = vec![package_info("wasm_fixture"), package_info(fixture::PROBE)];
        if let Some(marker) = conflict {
            infos.push(package_info(marker));
        }
        let loaded = load(
            &std::fs::read(std).unwrap(),
            vec![WasmPackage {
                manifest,
                module: Package::read_wasm(path).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        loaded.initialize(&infos).unwrap();
        let mut mods = vec![ModEntry::new("earlier_capabilities", Box::new(Earlier))];
        mods.extend(loaded.mods);
        mods.push(ModEntry::new("later_capabilities", Box::new(Later)));
        mod_api::testing::with_mods(mods.into(), f);
    });
}

fn install() -> Result<(), Vec<String>> {
    mod_api::__private::install_parts(
        mod_api::__private::mods(),
        &mod_api::__private::part_tests::rule_schema(),
        |_| false,
    )?;
    mod_api::__private::resolve_entity_types(fixture::ENTITIES)
}

#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built by wasm-mods"]
fn compiled_guest_capability_registration_order_and_proxy_metadata() {
    with_guest(None, || {
        install().unwrap();
        mod_api::__private::resolve_block_keys(
            fixture::BLOCKS
                .into_iter()
                .chain(["minecraft:water_cauldron"]),
            |_| {
                horizon_mod_runtime::block_definitions::BlockDefinition::new(
                    &[&Default::default()],
                    0,
                )
                .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        let mut host = mod_api::testing::FakeHost::new();
        host.registries
            .resolve_items(fixture::ITEMS.map(|id| mod_api::Identifier::parse(id).unwrap()))
            .unwrap();
        for index in 0..4 {
            host.level
                .set_block([index as i32, 64, 0], fixture::BLOCKS[index]);
            host.level
                .entity(index as u32 + 1, fixture::ENTITIES[index]);
            let key = host
                .item_key(&mod_api::Identifier::parse(fixture::ITEMS[index]).unwrap())
                .unwrap();
            host.join(19, 19)
                .slots
                .insert(index as i32, mod_api::ItemStack::new(key, 1).unwrap());
        }
        mod_api::__private::mark_game_thread();
        let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        let table = mod_api::Host::capability_providers(&host);
        for (index, owner) in ["earlier_capabilities", "wasm_fixture", "later_capabilities"]
            .into_iter()
            .enumerate()
        {
            let block = mod_api::__private::block_key(fixture::BLOCKS[index]).unwrap();
            let item = host
                .item_key(&mod_api::Identifier::parse(fixture::ITEMS[index]).unwrap())
                .unwrap();
            let mut cursor = 0;
            assert_eq!(
                table
                    .block_provider(fixture::BLOCK.name, None, Some(block), &mut cursor)
                    .unwrap()
                    .1,
                owner
            );
            assert!(
                table
                    .block_provider(fixture::BLOCK.name, None, Some(block), &mut cursor)
                    .is_none()
            );
            let mut cursor = 0;
            assert_eq!(
                table
                    .item_provider(fixture::ITEM.name, item, &mut cursor)
                    .unwrap()
                    .1,
                owner
            );
            assert!(
                table
                    .item_provider(fixture::ITEM.name, item, &mut cursor)
                    .is_none()
            );
            let mut cursor = 0;
            assert_eq!(
                table
                    .entity_provider(fixture::ENTITY.name, fixture::ENTITIES[index], &mut cursor)
                    .unwrap()
                    .1,
                owner
            );
            assert!(
                table
                    .entity_provider(fixture::ENTITY.name, fixture::ENTITIES[index], &mut cursor)
                    .is_none()
            );
        }
        // These host-side reads and the guest's imported reads must see the same publication.
        let names = mod_api::block_capability_names(&host);
        assert!(names.iter().any(|name| name == fixture::BLOCK.name));
        assert!(!mod_api::block_capability_proxyable(
            &host,
            fixture::BLOCK.name
        ));
        assert!(mod_api::block_capability_proxyable(
            &host,
            fixture::PROXY.name
        ));
        assert!(!mod_api::block_capability_proxyable(
            &host,
            fixture::NON_PROXY.name
        ));
        assert_eq!(
            mod_api::block_capability_proxyable_names(&host),
            names
                .into_iter()
                .filter(|name| mod_api::block_capability_proxyable(&host, name))
                .collect::<Vec<_>>()
        );
        // Invoke the actual registered guest callback through the runtime's shared owner.
        // It currently supplies no container, yet its declaration covered ITEM_ITEMS at startup.
        assert_eq!(
            mod_api::__private::parts_installation::item_container_slots(&host, fixture::ITEMS[1]),
            None,
        );
        host.registries.loaded("capability_dynamic_slots");
        assert_eq!(
            mod_api::__private::parts_installation::item_container_slots(&host, fixture::ITEMS[1]),
            Some(3),
        );
        mod_api::dispatch_server_started(&mut host, true);
        let component = mod_api::ComponentId::new("wasm_fixture:stored_energy").unwrap();
        let stack = host.players.get(19).unwrap().slots.get(&0).unwrap();
        assert_eq!(
            mod_api::ItemResource::of(stack).component(&component),
            Some(mod_api::Tag::Int(23))
        );
        // The fixture also touched the other item providers, and its aborted extraction restored them.
        for slot in [1, 2] {
            let stack = host.players.get(19).unwrap().slots.get(&slot).unwrap();
            assert_eq!(
                mod_api::ItemResource::of(stack).component(&component),
                Some(mod_api::Tag::Int(0))
            );
        }
    });
}

fn rejects_opposite_mark(marker: &str, name: &str) {
    with_guest(Some(marker), || {
        let errors = install().unwrap_err();
        assert!(
            errors.iter().any(|error| error.contains("wasm_fixture")
                && error.contains(name)
                && error.contains("proxy")),
            "{errors:?}"
        );
        // Failed registration must not publish the attempted last value or partial names.
        let host = mod_api::testing::FakeHost::new();
        assert!(
            !mod_api::block_capability_names(&host)
                .iter()
                .any(|entry| entry == name)
        );
    });
}

#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built by wasm-mods"]
fn compiled_guest_proxy_true_then_false_refuses_registration() {
    rejects_opposite_mark("capability_proxy_conflict_false", fixture::PROXY.name);
}

#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built by wasm-mods"]
fn compiled_guest_proxy_false_then_true_refuses_registration() {
    rejects_opposite_mark("capability_proxy_conflict_true", fixture::NON_PROXY.name);
}
