fn main() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../horizon-client/src/crafting/ingredient/builtin_kinds_tests/families");
    println!("cargo:rerun-if-changed={}", tests.display());
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        out.join("builtin_ingredient_test_modules.rs"),
        horizon_game_discovery::folder_modules(&tests).expect("discovering ingredient tests"),
    )
    .expect("writing ingredient test modules");
}
