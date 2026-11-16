//! Self-refusal uses its own process because point initialization is once per process.
use horizon_mod_package::manifest::Package;
use horizon_mod_wasm::{WasmPackage, load};

#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD, built by wasm-mods"]
fn real_member_self_refusal() {
    crate::test_stack::on_guest_stack(|| {
        let fixture = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
        let std = std::env::var("HORIZON_WASM_STD").unwrap();
        let path = std::path::Path::new(&fixture);
        let mut manifest = Package::read(path).unwrap().manifest;
        // This case tests the guest's Parts registration refusal, independently of game points.
        manifest.inject.clear();
        let info = mod_api::PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            vec![],
        );
        let marker = mod_api::PackageInfo::new(
            "refuse_registration".into(),
            "1".into(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec!["refuse_registration".into()],
            vec![],
        );
        let loaded = load(
            &std::fs::read(std).unwrap(),
            vec![WasmPackage {
                manifest,
                module: Package::read_wasm(path).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        loaded.initialize(&[info, marker]).unwrap();
        let errors = mod_api::__private::install_parts(
            &loaded.mods,
            &mod_api::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(
            errors,
            [
                "mod wasm_fixture refused registration: guest bootstrap failed: café\nmissing recipe bridge"
            ]
        );
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
