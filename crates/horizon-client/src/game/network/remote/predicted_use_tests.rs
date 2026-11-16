//! The real control hand loop, with identical native and compiled-wasm listeners.
use super::*;
#[path = "../../../../../horizon-mod-wasm/tests/fixture/src/predicted_use.rs"]
mod fixture;

struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        fixture::register(registrar);
    }
    fn entity_interact(
        &self,
        host: HostSide<'_>,
        event: &mut mod_api::EntityInteractEvent<'_>,
    ) -> Option<Answer> {
        fixture::entity_interact(host, event)
    }
    fn client_packet_send(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ClientPacketSendEvent<'_>,
    ) -> Flow {
        fixture::packet(event);
        Flow::Continue
    }
}

fn matrix(mods: mod_api::Mods) {
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let root = super::super::super::mods::content_root(&base, &Default::default()).unwrap();
        with_interaction_game_at(true, root.prepared().to_owned(), |game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let air = game.res.facts().air;
            // Literal packet [UseItemOn, UseItem, Interact], placement and swing outcomes.
            for path in 1..=4 {
                for (answer, consumes, success_swing) in [
                    (1, true, 1),
                    (2, true, 0),
                    (3, true, 0),
                    (4, false, 0),
                    (5, false, 0),
                    (6, false, 0),
                ] {
                    for server_only in [false, true] {
                        game.set_mode_id(0);
                        game.inventory[game.selected] = Some(ItemStack::new("minecraft:paper", 1));
                        let target_state = if path >= 3 { air } else { stone };
                        game.world.set(0, 64, 0, target_state).unwrap();
                        game.offhand = Some(ItemStack::new("minecraft:torch", 2));
                        game.world.set(0, 65, 0, air).unwrap();
                        game.forced_hit = (path == 1).then(hit);
                        game.tick_hit.set(None);
                        game.tick_mob = None;
                        game.mobs.clear();
                        game.keys.clear();
                        game.yaw = 0.0;
                        game.pitch = 0.0;
                        let player_z = if path >= 3 { 0.5 } else { 3.5 };
                        game.player = Some(
                            PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, player_z)).unwrap(),
                        );
                        if path >= 3 {
                            let kind = game
                                .res
                                .mob_types()
                                .get("minecraft:zombie")
                                .unwrap()
                                .clone();
                            game.mobs.push(horizon_client::mobs::Mob::new(
                                42,
                                kind,
                                Vec3::new(0.5, 64.0, 2.5),
                                0.0,
                            ));
                        }
                        game.swing = Default::default();
                        let mut control = mod_api::ModBlockEntity::new(
                            "wasm_fixture:prediction_control".into(),
                            Default::default(),
                        );
                        control.data = vec![path, answer, i32::from(server_only)];
                        game.native_block_entity_tick(
                            &mut mod_api::BlockEntityTickReply::Changed,
                            [0, 64, 0],
                            &mut control,
                        );
                        assert!(control.data.is_empty());
                        let swings = game.swings_started;
                        horizon_client::interaction_input::use_on_target(
                            &mut super::super::RemoteUseWorld(
                                game,
                                mod_api::InteractionAttempt::Press,
                            ),
                            path >= 3,
                        );
                        game.native_block_entity_tick(
                            &mut mod_api::BlockEntityTickReply::Changed,
                            [0, 64, 0],
                            &mut control,
                        );
                        let stops = !server_only && (consumes || (path == 1 && answer == 4));
                        let packets = match (path, stops) {
                            (1, true) => [1, 0, 0],
                            (1, false) => [2, 1, 0],
                            (2, true) => [0, 1, 0],
                            (2, false) => [0, 2, 0],
                            (_, true) => [0, 0, 1],
                            (_, false) => [0, 2, 2],
                        };
                        assert_eq!(
                            control.data, packets,
                            "path={path} answer={answer} server_only={server_only}"
                        );
                        let placed = path == 1 && !stops;
                        assert_eq!(game.world.block(0, 65, 0) != air, placed);
                        if placed {
                            assert_eq!(&*game.block_name([0, 65, 0]), "minecraft:torch");
                        }
                        assert_eq!(
                            game.offhand.as_ref().unwrap().count,
                            if placed { 1 } else { 2 }
                        );
                        assert_eq!(game.inventory[game.selected].as_ref().unwrap().count, 1);
                        assert_eq!(game.world.block(0, 64, 0), target_state);
                        assert!(game.mobs.iter().all(|mob| !mob.removed && mob.alive()));
                        assert_eq!(
                            game.swings_started - swings,
                            if server_only {
                                u32::from(placed)
                            } else if consumes {
                                success_swing
                            } else {
                                u32::from(placed)
                            }
                        );
                    }
                }
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn native_client_prediction_owns_the_hand_loop() {
    crate::game::in_own_process(
        "game::remote::interaction_event_tests::predicted_use_tests::native_client_prediction_owns_the_hand_loop",
        || matrix(vec![mod_api::ModEntry::new("wasm_fixture", Box::new(Native))].into()),
    );
}

#[test]
#[ignore = "daily"]
fn wasm_client_prediction_owns_the_hand_loop() {
    crate::game::in_own_process(
        "game::remote::interaction_event_tests::predicted_use_tests::wasm_client_prediction_owns_the_hand_loop",
        || {
            horizon_guest_stack::run_on_guest_stack(|| {
                horizon_mod_wasm::copy_probe::with_load_budget(|| {
                    use horizon_mod_package::manifest::Package;
                    use horizon_mod_wasm::{WasmPackage, load};
                    let path =
                        std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
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
                    loaded.initialize(&[info]).unwrap();
                    matrix(loaded.mods.into());
                })
            })
        },
    );
}
