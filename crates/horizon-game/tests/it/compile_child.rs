//! No resources or ordinary startup are needed for malformed compiler requests.
#[test]
#[ignore = "daily"]
fn real_executable_refuses_malformed_hidden_requests() {
    for args in [
        vec!["--compile-wasm-module"],
        vec!["--compile-wasm-module", "missing", "key"],
        vec!["--compile-wasm-module", "missing", "key", "extra"],
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(env!("CARGO_BIN_EXE_horizon-client"))
            .args(args)
            .current_dir(scratch.path())
            .env_clear()
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(1));
        assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
    }
}
