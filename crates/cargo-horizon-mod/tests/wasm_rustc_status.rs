#![cfg(unix)]

use std::{fs, os::unix::fs::symlink, process::Command};

#[test]
#[ignore = "daily"]
fn compiler_wrapper_preserves_diagnostics_exit_code_and_signal() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../target/wasm-rustc-status-{}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let compiler = root.join("diagnostic-rustc");
    symlink("/bin/sh", &compiler).unwrap();
    let mut missing = Vec::new();
    for (script, status) in [
        ("echo compiler-detail >&2; exit 42", "exit status: 42"),
        ("kill -KILL $$", "signal: 9"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
            .args([compiler.to_str().unwrap(), "-c", script])
            .env("HORIZON_GROUP_WRAPPER", "{}")
            .env("HORIZON_BUILD_ROOT", &root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8(output.stderr).unwrap();
        if !stderr.contains(status) {
            missing.push(format!("missing {status}: {stderr}"));
        }
        if script.starts_with("echo") {
            assert!(stderr.contains("compiler-detail"), "{stderr}");
        }
    }
    fs::remove_dir_all(root).unwrap();
    assert!(missing.is_empty(), "{missing:?}");
}
