//! Real wasm commands through the game's shared selector resolver and Wasmtime listener.
use super::*;
use horizon_client::mobs::Mob;
use horizon_core::geometry::Vec3;

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built from this tree"]
fn wasm_selectors_resolve_in_game_and_refuse_before_guest_dispatch() {
    // Enters the guest: libtest's thread is smaller than the guest stack policy requires. The
    // load budget: this checks behaviour, not speed, and a loaded runner can pass 100 ms.
    horizon_guest_stack::run_on_guest_stack(|| {
        horizon_mod_wasm::copy_probe::with_load_budget(
            wasm_selectors_resolve_in_game_and_refuse_before_guest_dispatch_on_guest_stack,
        )
    });
}

fn wasm_selectors_resolve_in_game_and_refuse_before_guest_dispatch_on_guest_stack() {
    use horizon_mod_package::manifest::Package;
    use horizon_mod_wasm::{WasmPackage, load};
    let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
    let manifest = Package::read(std::path::Path::new(&path)).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
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
    mod_platform::testing::with_mods(loaded.mods.into(), || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            crate::game::mods::register_cauldron_fluid_contents(game.pack.assets.registry);
            game.register_native_commands();
            let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            for (id, x) in [(101, 12.0), (103, 4.0), (102, 7.0)] {
                game.mobs
                    .push(Mob::new(id, cow.clone(), Vec3::new(x, 64.0, 5.0), 0.0));
            }
            game.as_guest(12, |game| {
                for line in [
                    "wasmcauldron",
                    "wasmheal @a",
                    "wasmentity @e[type=minecraft:cow,limit=1,sort=nearest]",
                    "wasmentities @e[type=minecraft:cow,sort=nearest]",
                ] {
                    game.chat_capture = Some(Vec::new());
                    assert!(game.perform_native_command(12, 4, line), "{line}");
                    assert!(
                        game.chat_capture.take().unwrap().is_empty(),
                        "{line}: guest must complete its handle/order assertions and return handled"
                    );
                }
                for (line, key) in [
                    (
                        "wasmheal @a[name=missing]",
                        "argument.entity.notfound.player",
                    ),
                    (
                        "wasmentities @e[type=minecraft:cow,name=missing]",
                        "argument.entity.notfound.entity",
                    ),
                    (
                        "wasmentity @e[type=minecraft:cow,name=missing,limit=1]",
                        "argument.entity.notfound.entity",
                    ),
                    (
                        "wasmentity @e[type=minecraft:cow]",
                        "argument.entity.toomany",
                    ),
                    ("wasmplayer @a", "argument.player.toomany"),
                ] {
                    game.chat_capture = Some(Vec::new());
                    assert!(game.perform_native_command(12, 4, line), "{line}");
                    let output = game.chat_capture.take().unwrap();
                    assert_eq!(
                        output[0].plain(),
                        Text::translate(key, &[]).plain(),
                        "{line}"
                    );
                }
                let random = game.world.random.state();
                for line in [
                    "wasmheal @a",
                    "wasmentity @r",
                    "wasmentities @e",
                    "wasmplayer @s",
                ] {
                    game.chat_capture = Some(Vec::new());
                    assert!(game.perform_native_command(12, 0, line), "{line}");
                    let output = game.chat_capture.take().unwrap();
                    assert_eq!(
                        output[0].plain(),
                        Text::translate("argument.entity.selector.not_allowed", &[]).plain(),
                        "{line}"
                    );
                }
                assert_eq!(game.world.random.state(), random);
                // This guest call asserts its ordinal is exactly four, proving no failed
                // selection or denied selector entered the wasm listener in between.
                game.chat_capture = Some(Vec::new());
                assert!(game.perform_native_command(12, 0, "wasmplayer guest13"));
                assert!(
                    game.chat_capture.take().unwrap().is_empty(),
                    "guest call-count assertion and single-player decoding must finish"
                );
            })
            .unwrap();
        });
    });
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built from this tree"]
fn wasm_server_tick_messages_reach_dedicated_game() {
    // Enters the guest: libtest's thread is smaller than the guest stack policy requires. The
    // load budget: this checks behaviour, not speed, and a loaded runner can pass 100 ms.
    horizon_guest_stack::run_on_guest_stack(|| {
        horizon_mod_wasm::copy_probe::with_load_budget(
            wasm_server_tick_messages_reach_dedicated_game_on_guest_stack,
        )
    });
}

fn wasm_server_tick_messages_reach_dedicated_game_on_guest_stack() {
    use horizon_mod_package::manifest::Package;
    use horizon_mod_wasm::{WasmPackage, load};
    let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
    let manifest = Package::read(std::path::Path::new(&path)).unwrap().manifest;
    let info = mod_api::PackageInfo::new(
        manifest.id.clone(),
        manifest.version.clone(),
        mod_api::PackageKind::Code,
        mod_api::Side::Both,
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

    use std::sync::{Arc, Mutex};
    struct Heard(Arc<Mutex<Vec<mod_api::Text>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Heard {
        fn broadcast_message(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::BroadcastMessageEvent<'_>,
        ) -> mod_api::Flow {
            self.0.lock().unwrap().push(event.line.clone());
            mod_api::Flow::Continue
        }
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut mods = loaded.mods;
    mods.push(mod_api::ModEntry::new(
        "message_observer",
        Box::new(Heard(heard.clone())),
    ));
    mod_platform::testing::with_mods(mods.into(), || {
        super::super::native_guest_turn_tests::with_guests(|game| {
            game.options.execution_role =
                crate::game::execution_role::ExecutionRole::DedicatedServer;
            game.options.dedicated = Some(Vec::new());
            let (tx, rx) = std::sync::mpsc::channel();
            let host = game.lan.as_mut().unwrap();
            host.hub.guests.update(0, |guest| {
                guest.sender = horizon_client::lan::PlayerSender::new(tx, Arc::default())
            });
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            let context =
                mod_api::LifecycleContext::from(mod_api::__private::built::LifecycleContext {
                    authority: mod_api::Authority::DedicatedServer,
                    epoch: mod_api::WorldEpoch::new(73),
                    tick: mod_api::TickId::new(1),
                });
            game.native_server_tick(
                &context,
                mod_api::ServerTickPhase::ServerTickStart {
                    world_running: true,
                },
            );
            assert_eq!(
                *heard.lock().unwrap(),
                [mod_api::Text::literal("wasm tick phase regression")]
            );
            assert!(
                rx.try_recv().is_ok(),
                "broadcast must reach the placed player's connection"
            );
        });
    });
}
