#[path = "../../packaging/executable_link.rs"]
mod executable_link;
// The benchmark includes the same fixture root natively, so it needs the identical module list.
#[allow(dead_code)]
#[path = "tests/fixture/build.rs"]
mod fixture_build;

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/executable_link.rs");
    println!("cargo:rerun-if-changed=../horizon-guest-stack/src/guest_stack_constants.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    for arg in executable_link::arguments(&os, &env) {
        println!("cargo:rustc-link-arg-tests={arg}");
    }
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    if std::env::var_os("CARGO_FEATURE_LISTENER_FIXTURE").is_some() {
        fixture_build::generate(&root.join("crates/horizon-mod-wasm/tests/fixture"), &out);
    }
    println!("cargo:rerun-if-changed=../horizon-mod-api/src");
    println!("cargo:rerun-if-changed=../horizon-mod-vocab/src");
    println!("cargo:rerun-if-changed=../horizon-mod-api-gen/src");
    let code = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || horizon_mod_api_gen::generate_wire_host(&root))
        .expect("starting host generator")
        .join()
        .expect("host generator panicked")
        .expect("generating host boundary");
    horizon_mod_api_gen::write_if_changed(out.join("wire_host_generated.rs"), code)
        .expect("writing host boundary");
}
