fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // Include fragments at the original lexical scope: public fixture paths and
    // test names stay stable. A new method group adds only its own file.
    for area in ["blocks", "shared"] {
        let directory = root.join(format!("crates/horizon-mod-api/src/parts/{area}_fixtures"));
        println!("cargo:rerun-if-changed={}", directory.display());
        let mut paths: Vec<_> = std::fs::read_dir(&directory)
            .expect("reading Part fixture directory")
            .map(|entry| entry.expect("reading Part fixture entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "empty Part fixture directory: {area}");
        let mut includes = String::new();
        for path in paths {
            println!("cargo:rerun-if-changed={}", path.display());
            includes.push_str(&format!("include!({:?});\n", path));
        }
        std::fs::write(out_dir.join(format!("{area}_fixtures.rs")), includes)
            .expect("writing Part fixture includes");
    }
    horizon_mod_api_gen::generate(&root, &out_dir, horizon_mod_api_gen::Target::Api)
        .expect("generating the mod API");
}
