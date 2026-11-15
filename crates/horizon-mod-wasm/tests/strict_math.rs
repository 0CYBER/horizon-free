//! Execute independent StrictMath vectors inside the compiled fixture, not a
//! host-side echo. Quiet deliberately implements no math or other host read.
#[path = "support/no_hooks.rs"]
mod no_hooks;
#[path = "../src/test_stack.rs"]
mod test_stack;

struct Quiet;
impl mod_api::Host for Quiet {}
mod_api::impl_host_areas!(Quiet);
impl mod_api::ClientHost for Quiet {}
mod_api::impl_client_host_areas!(Quiet);

#[test]
#[ignore = "requires HORIZON_WASM_FIXTURE and HORIZON_WASM_STD built from this tree"]
fn compiled_guest_strict_math_matches_literal_jdk_vectors() {
    test_stack::on_guest_stack(|| {
        use horizon_mod_package::manifest::Package;
        use horizon_mod_wasm::{WasmPackage, load};
        let fixture = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
        let path = std::path::Path::new(&fixture);
        let mut manifest = Package::read(path).unwrap().manifest;
        // Only ordinary generated hook transport is needed for this check.
        manifest.inject.clear();
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
        mod_api::testing::with_mods(loaded.mods.into(), || {
            let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            // A distinct fixture request keeps the host-profile probe independent.
            let mut checked = -2.0;
            // The guest runs all literal assertions before writing its completed
            // row count. A guest panic/trap cannot masquerade as successful math.
            mod_api::dispatch_tick_target_millis(&Quiet, &mut checked);
            assert_eq!(checked, 4381.0, "guest did not complete all reference rows");
        });
    });
}
