//! A wasm package that still declares the removed `[[inject]]` is refused when it is read, with
//! the retirement's diagnostic, before any module is loaded.
use horizon_mod_package::manifest::{INJECT_REMOVED, MANIFEST, Package};

#[test]
fn a_package_with_inject_is_refused_as_removed() {
    let package = tempfile::tempdir().unwrap();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixture/horizon-mod.toml"
    );
    let manifest = std::fs::read_to_string(fixture).unwrap()
        + "\n[[inject]]\nid = 'tick'\nat = 'horizon_client::game::Game::tick'\nkind = 'head'\nwith = 'on_tick'\n";
    std::fs::write(package.path().join(MANIFEST), manifest).unwrap();
    std::fs::write(package.path().join("mod.wasm"), b"\0asm\x01\0\0\0").unwrap();
    let error = Package::read(package.path()).err().expect("refused");
    assert_eq!(error, format!("{MANIFEST}: {INJECT_REMOVED}"));
}
