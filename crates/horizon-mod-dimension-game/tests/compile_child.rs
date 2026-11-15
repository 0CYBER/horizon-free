#![cfg(unix)]
use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn run(args: &[&std::ffi::OsStr], home: &std::path::Path) -> std::process::ExitStatus {
    let mut child = Command::new(env!("CARGO_BIN_EXE_horizon-mod-dimension-game"))
        .args(args)
        .current_dir(home)
        .env_clear()
        .env("HORIZON_HOME", home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if start.elapsed() > Duration::from_secs(10) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("compiler child exceeded 10 seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "daily"]
fn private_compile_exits_zero_and_writes_one_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".horizon-cache/wasm");
    fs::create_dir_all(&root).unwrap();
    let input = root.join(format!(".input-{}-0.tmp", std::process::id()));
    let bytes =
        wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))").unwrap();
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&input)
        .unwrap()
        .write_all(&bytes)
        .unwrap();
    let key = horizon_mod_wasm::compile_child_key(&bytes).unwrap();
    assert_eq!(
        run(
            &[
                "--compile-wasm-module".as_ref(),
                input.as_os_str(),
                key.as_ref()
            ],
            dir.path()
        )
        .code(),
        Some(0)
    );
    let entries: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| name.to_string_lossy().ends_with(".cwasm"))
        .collect();
    assert_eq!(
        entries,
        vec![std::ffi::OsString::from(format!("{key}.cwasm"))]
    );
}

#[test]
#[ignore = "daily"]
fn missing_key_exits_one_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        run(
            &["--compile-wasm-module".as_ref(), "missing".as_ref()],
            dir.path()
        )
        .code(),
        Some(1)
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

fn cache_loads(parents: usize) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".horizon-cache/wasm");
    let bytes =
        wat::parse_str("(module (func (export \"answer\") (result i32) i32.const 42))").unwrap();
    let barrier = std::sync::Barrier::new(parents);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..parents)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    horizon_mod_wasm::fixture::cached_answer(
                        &root,
                        std::path::Path::new(env!("CARGO_BIN_EXE_horizon-mod-dimension-game")),
                        &bytes,
                    )
                    .unwrap()
                })
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), (42, true));
        }
    });
    let names: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let key = horizon_mod_wasm::compile_child_key(&bytes).unwrap();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from(format!("{key}.cwasm"))]
    );
}

#[test]
#[ignore = "daily"]
fn production_parent_uses_child_artifact() {
    cache_loads(1);
}

#[test]
#[ignore = "daily"]
fn concurrent_production_parents_share_one_clean_artifact() {
    cache_loads(4);
}
