mod build_identity;

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=HORIZON_GAME_BUILD_ID");
    let manifest_dir =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let env_id = std::env::var("HORIZON_GAME_BUILD_ID").ok();
    let resolved = build_identity::resolve(env_id.as_deref(), &manifest_dir, OsStr::new("git"))
        .unwrap_or_else(|error| panic!("{error}"));
    for path in &resolved.watch_paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let commit = resolved.commit;
    let rustc = Command::new(std::env::var_os("RUSTC").expect("RUSTC"))
        .arg("-vV")
        .output()
        .expect("rustc");
    let version = String::from_utf8(rustc.stdout).expect("rustc utf8");
    let release = version
        .lines()
        .find_map(|line| line.strip_prefix("release: "))
        .expect("release");
    println!("cargo:rustc-env=HORIZON_GAME_BUILD_ID={commit}");
    println!("cargo:rustc-env=HORIZON_RUSTC_VERSION={release}");
}
