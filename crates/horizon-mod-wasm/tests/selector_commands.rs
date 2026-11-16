//! The server resolves selectors before this boundary. This harness exercises the actual
//! Wasmtime listener and wire transport, not game selector parsing or permission checks.
#[path = "support/bulk_files.rs"]
mod bulk_files;
#[path = "support/no_hooks.rs"]
mod no_hooks;

use mod_api::*;

use bulk_files::World;

#[test]
#[ignore = "requires HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built from this tree"]
fn real_wasm_command_listener_receives_ordered_selector_handles() {
    crate::test_stack::on_guest_stack(|| {
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

        let epoch = WorldEpoch::new(73);
        let first = mod_api::__private::game::player_ref(
            EntityRef::new(epoch, 91),
            0x112233445566778899aabbccddeeff00,
        );
        let second = mod_api::__private::game::player_ref(
            EntityRef::new(epoch, 7),
            0xffeeddccbbaa99887766554433221100,
        );
        let cases = [
            (
                "selector_heal",
                "targets",
                CommandArgumentValue::Players(vec![first, second]),
            ),
            (
                "selector_cow",
                "target",
                CommandArgumentValue::Entity(EntityRef::new(epoch, 42)),
            ),
            (
                "selector_herd",
                "targets",
                CommandArgumentValue::Entities(
                    (301..=600)
                        .rev()
                        .map(|id| EntityRef::new(epoch, id))
                        .collect(),
                ),
            ),
            (
                "selector_player",
                "target",
                CommandArgumentValue::Player(second),
            ),
            (
                "selector_id",
                "kind",
                CommandArgumentValue::Id(Identifier::parse("minecraft:zombie").unwrap()),
            ),
        ];
        mod_api::testing::with_mods(loaded.mods.into(), || {
            mod_api::__private::mark_game_thread();
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            let mut world = World::default();
            let source = CommandSource::Console {
                permission: PermissionLevel::All,
            };
            for (name, argument, value) in cases {
                let invocation = CommandInvocation::new(
                    vec![name.into()],
                    vec![CommandArgument::new(argument.into(), value)],
                    mod_api::DimensionKey::OVERWORLD,
                    [0.0; 3],
                );
                let mut ran = false;
                mod_api::dispatch_run_mod_command(&mut world, &source, &invocation, &mut ran);
                assert!(
                    ran,
                    "{name}: guest assertions must finish and cancellation must return over the wire"
                );
            }
            for (command, no_world) in [("wasmbulkfiles", false), ("wasmbulkfilesnoworld", true)] {
                world
                    .world
                    .no_world
                    .store(no_world, std::sync::atomic::Ordering::Relaxed);
                let mut ran = false;
                mod_api::dispatch_run_mod_command(
                    &mut world,
                    &source,
                    &CommandInvocation::new(
                        vec![command.into()],
                        vec![],
                        mod_api::DimensionKey::OVERWORLD,
                        [0.0; 3],
                    ),
                    &mut ran,
                );
                assert!(
                    ran,
                    "{command}: file capability assertions must finish inside wasm"
                );
            }
            let mut ran = false;
            mod_api::dispatch_run_mod_command(
                &mut world,
                &source,
                &CommandInvocation::new(
                    vec!["not_a_selector_fixture_command".into()],
                    vec![],
                    mod_api::DimensionKey::OVERWORLD,
                    [0.0; 3],
                ),
                &mut ran,
            );
            assert!(
                !ran,
                "an unhandled guest command must not become handled on the wire"
            );
        });
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
