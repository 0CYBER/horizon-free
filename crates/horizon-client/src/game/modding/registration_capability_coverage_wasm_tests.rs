//! Actual engine bootstrap coverage read through the compiled portable registrar.
use super::{api_values, mods, native_entity_refs::tests::run_fresh_cases};
use horizon_mod_package::{manifest::Package, selection::LaunchArguments};
use mod_api::{PackageInfo, PackageKind, Side};

#[test]
#[ignore = "requires prepared HORIZON_TEST_RESOURCES and rebuilt HORIZON_WASM_FIXTURE/HORIZON_WASM_STD"]
fn compiled_guest_reads_game_coverage_during_registration() {
    const NAME: &str = "game::registration_capability_coverage_wasm_tests::compiled_guest_reads_game_coverage_during_registration";
    if run_fresh_cases(NAME, "HORIZON_CAPABILITY_GAME_CHILD", &["coverage"]) {
        return;
    }
    horizon_guest_stack::run_on_guest_stack(|| {
        horizon_mod_wasm::copy_probe::with_load_budget(run);
    });
}

fn run() {
    let cwd = std::env::current_dir().unwrap();
    let launch = LaunchArguments::parse(std::iter::empty::<String>(), &cwd, &cwd, None).unwrap();
    crate::entry::runtime_environment::capture(&launch);
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
    let mut manifest = Package::read(&path).unwrap().manifest;
    manifest.inject.clear();
    let inventory = [
        PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            PackageKind::Code,
            Side::Both,
            vec![manifest.id.clone()],
            vec![],
        ),
        // The guest's dedicated branch asserts ITEMS/STORAGE for a chest and
        // FLUIDS for a water cauldron, using this real engine snapshot.
        PackageInfo::new(
            "capability_game_probe".into(),
            "1.0.0".into(),
            PackageKind::Code,
            Side::Both,
            vec!["capability_game_probe".into()],
            vec![],
        ),
    ];
    let mut loaded = horizon_mod_wasm::load(
        &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
        vec![horizon_mod_wasm::WasmPackage {
            manifest,
            module: Package::read_wasm(&path).unwrap(),
        }],
    );
    assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
    loaded.initialize(&inventory).unwrap();
    api_values::install_test_platform();
    mod_platform::__private::install_packages(&inventory, &[]);
    mods::install_registration_data(&root).unwrap();
    let entries = std::mem::take(&mut loaded.mods);
    mod_platform::testing::with_mods(entries.into(), || {
        mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
            .unwrap();
        let host = mod_platform::testing::FakeHost::new();
        // Written only after the guest's assertions finish. A swallowed guest trap
        // or a skipped marker branch cannot satisfy this observation.
        assert!(mod_api::block_capability_proxyable(
            &host,
            "fixture:game_coverage_completed"
        ));
    });
}
