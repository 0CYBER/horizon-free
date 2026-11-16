//! Load a real point-bearing guest into the plain engine and require a named refusal.
use horizon_mod_package::manifest::Package;
use horizon_mod_wasm::{WasmPackage, load};

#[test]
#[ignore = "needs HORIZON_WASM_FIXTURE and HORIZON_WASM_STD, built by wasm-mods"]
fn real_guest_point_is_refused_without_staging() {
    crate::test_stack::on_guest_stack(|| {
        if mod_points::STAGED {
            return;
        }
        let fixture = std::env::var("HORIZON_WASM_FIXTURE").unwrap();
        let path = std::path::Path::new(&fixture);
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
        let refused = loaded
            .initialize(&[info])
            .expect_err("plain engine refuses game points");
        assert_eq!(refused.len(), 1);
        let (id, reason) = &refused[0];
        assert_eq!(id, "wasm_fixture");
        let message = reason.to_string();
        assert!(
            message.contains("wasm_fixture")
                && message.contains("::tick")
                && message.contains("no point sites"),
            "{message}"
        );
    })
}

#[path = "../src/test_stack.rs"]
mod test_stack;
