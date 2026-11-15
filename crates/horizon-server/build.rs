#[path = "../../packaging/executable_link.rs"]
mod executable_link;
fn main() {
    println!("cargo:rerun-if-changed=../../packaging/executable_link.rs");
    println!("cargo:rerun-if-changed=../horizon-guest-stack/src/guest_stack_constants.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    for arg in executable_link::arguments(&os, &env) {
        println!("cargo:rustc-link-arg-bins={arg}");
    }
}
