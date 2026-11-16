use std::{path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn run_refuses_code_mods_without_building() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("native-run-refusal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("machine");
    let executable = env!("CARGO_BIN_EXE_cargo-horizon-mod");
    // An empty folder as the SDK base: `new` writes the project, then names the SDK it could not
    // fetch, and nothing reaches the network.
    let no_sdk = dir.join("no-sdk");
    std::fs::create_dir_all(&no_sdk).unwrap();
    let created = Command::new(executable)
        .env("HORIZON_SDK_BASE", &no_sdk)
        .args([
            "new",
            project.to_str().unwrap(),
            "--template",
            "native",
            "--horizon",
        ])
        .arg("0123456789abcdef0123456789abcdef01234567")
        .output()
        .unwrap();
    assert!(!created.status.success(), "{created:?}");
    assert!(
        String::from_utf8_lossy(&created.stderr).contains("SHA256SUMS"),
        "{created:?}"
    );
    assert!(project.join("Cargo.toml").is_file());
    assert!(!project.join("target").exists());

    let refused = Command::new(executable)
        .current_dir(&project)
        .args(["run", "--game"])
        .arg(dir.join("must-not-start"))
        .output()
        .unwrap();
    assert!(!refused.status.success(), "{refused:?}");
    let error = String::from_utf8(refused.stderr).unwrap();
    assert!(
        error.contains("code mods run through the launcher's native build"),
        "{error}"
    );
    assert!(
        error.contains("horizon-launcher native-build . --trust-native-code"),
        "{error}"
    );
    assert!(
        error.contains("docs/modding/src/how-to/native-mods.md"),
        "{error}"
    );
    assert!(
        !project.join("target").exists(),
        "run must refuse before building"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
