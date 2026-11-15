use mod_api::{Host, MathProfile};

fn assert_platform_profile(actual: MathProfile) {
    // Independent literal expectations, including Windows ARM's precedence.
    #[cfg(target_arch = "aarch64")]
    assert_eq!(actual, MathProfile::Aarch64Fma);
    #[cfg(all(windows, not(target_arch = "aarch64")))]
    assert_eq!(actual, MathProfile::CorrectlyRounded);
    #[cfg(all(not(windows), not(target_arch = "aarch64")))]
    assert_eq!(actual, MathProfile::LibmRounding);
}

#[test]
fn math_profile_selection_matches_platform_and_is_stable() {
    let first = super::selected_profile();
    assert_platform_profile(first);
    for _ in 0..128 {
        assert_eq!(super::selected_profile(), first);
    }
}

#[test]
#[ignore = "daily"]
fn game_host_math_profile_matches_platform_and_is_stable() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let host: &dyn Host = game;
        let first = host.math_profile();
        assert_platform_profile(first);
        for _ in 0..128 {
            assert_eq!(host.math_profile(), first);
        }
    });
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built from this tree"]
fn compiled_wasm_math_profile_import_matches_actual_game_host() {
    horizon_guest_stack::run_on_guest_stack(compiled_wasm_loopback);
}

fn compiled_wasm_loopback() {
    use horizon_mod_package::manifest::Package;
    use horizon_mod_wasm::{WasmPackage, load};
    let path = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
    let path = std::path::Path::new(&path);
    let manifest = Package::read(path).unwrap().manifest;
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
            module: Package::read_wasm(path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&[info]).unwrap();
    mod_platform::testing::with_mods(loaded.mods.into(), || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let native = (game as &dyn Host).math_profile();
            assert_platform_profile(native);
            game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
            let expected = match native {
                MathProfile::LibmRounding => 1.0,
                MathProfile::CorrectlyRounded => 2.0,
                MathProfile::Aarch64Fma => 3.0,
                MathProfile::StrictMath => 4.0,
                _ => panic!("test needs updating for the new math profile"),
            };
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            for _ in 0..2 {
                let mut received = -1.0;
                game.native_tick_target_millis(&mut received);
                assert_eq!(received, expected);
            }
        });
    });
}
