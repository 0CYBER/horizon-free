fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let catalog = horizon_mod_api_gen::neoforge_catalog(&root).expect("reading hook counterparts");
    std::fs::write(out_dir.join("hook-counterparts.json"), catalog)
        .expect("writing hook counterparts");
    println!(
        "cargo:rerun-if-changed={}",
        root.join(horizon_mod_api_gen::DECLARATIONS).display()
    );
}
