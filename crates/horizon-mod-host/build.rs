use std::process::Command;
fn main() {
    println!("cargo:rerun-if-env-changed=HORIZON_GAME_BUILD_ID");
    let commit = std::env::var("HORIZON_GAME_BUILD_ID").unwrap_or_else(|_| {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git build identity");
        assert!(
            output.status.success(),
            "git rev-parse HEAD failed; set HORIZON_GAME_BUILD_ID for staged builds"
        );
        String::from_utf8(output.stdout)
            .expect("git utf8")
            .trim()
            .to_owned()
    });
    assert!(
        !commit.trim().is_empty(),
        "game build identity must not be empty"
    );
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
