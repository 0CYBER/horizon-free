use super::*;
use std::{
    ffi::OsString,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
};
use windows_sys::Win32::Foundation::{
    GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT,
};

fn wait_for(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "actor rendezvous timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn inherited_exclusion_exact_handles_arguments_and_failure_release() {
    if std::env::var_os("HORIZON_WINDOWS_PRIVATE_TEST").is_none() {
        let status = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process::leases::tests::windows_tests::inherited_exclusion_exact_handles_arguments_and_failure_release", "--nocapture"])
            .env("HORIZON_WINDOWS_PRIVATE_TEST", "1").status().unwrap();
        assert!(status.success());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let trees = [directory(&root.path().join("tree"))];
    let ready = root.path().join("ready");
    let stop = root.path().join("stop");
    let extra_path = root.path().join("unrelated");
    let extra = File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&extra_path)
        .unwrap();
    // SAFETY: only this isolated test holder marks its own unrelated handle inheritable.
    assert_ne!(
        unsafe {
            SetHandleInformation(
                extra.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        },
        0
    );
    let args: Vec<OsString> = [
        "--exact",
        "process::leases::tests::windows_tests::actor",
        "--nocapture",
        "--",
        "",
        "a b",
        "quote\"and\\",
        "trailing\\\\",
        "snowman ☃",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let mut environment: Vec<_> = std::env::vars_os().collect();
    environment.extend([
        ("HORIZON_WINDOWS_LEASE_ACTOR".into(), "1".into()),
        ("HORIZON_WINDOWS_READY".into(), ready.as_os_str().to_owned()),
        ("HORIZON_WINDOWS_STOP".into(), stop.as_os_str().to_owned()),
        ("HORIZON_WINDOWS_VALUE".into(), "☃ = value\nline".into()),
        ("horizon_runtime_lease_handles".into(), "bogus".into()),
        ("horizon_runtime_data_root".into(), "bogus".into()),
    ]);
    let program = std::env::current_exe().unwrap();
    let child = ChildLeases::acquire(&data, &trees)
        .unwrap()
        .spawn_windows(&program, &args, &environment, false)
        .unwrap();
    drop(extra);
    wait_for(&ready);
    let received: Vec<Vec<u16>> = serde_json::from_slice(&std::fs::read(&ready).unwrap()).unwrap();
    let expected: Vec<Vec<u16>> = std::iter::once(program.as_os_str())
        .chain(args.iter().map(OsString::as_os_str))
        .map(|s| s.encode_wide().collect())
        .collect();
    assert_eq!(received, expected);
    // A leaked unrelated handle would retain deny-sharing even after the creator closes it.
    let unrelated = File::options()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&extra_path);
    assert!(
        unrelated.is_ok(),
        "HANDLE_LIST inherited an unselected file object"
    );
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    assert!(matches!(
        registry.lock(&trees[0], LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(registry);
    let gate = data.join("run-leases/gate");
    assert!(File::options()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&gate)
        .is_err());
    std::fs::write(&stop, b"stop").unwrap();
    assert_eq!(child.wait().unwrap().code(), Some(37));
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    assert!(registry.lock(&trees[0], LockMode::Exclusive).is_ok());
    drop(registry);
    assert!(File::options()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&gate)
        .is_ok());
    let failed = ChildLeases::acquire(&data, &trees).unwrap().spawn_windows(
        &root.path().join("missing.exe"),
        &[],
        &environment,
        false,
    );
    assert!(failed.is_err());
    let bad_args = [OsString::from("bad\0argument")];
    let mut bad_value = environment.clone();
    bad_value.push(("INVALID".into(), "bad\0value".into()));
    let mut bad_key = environment.clone();
    bad_key.push(("BAD=KEY".into(), "value".into()));
    for (args, env) in [
        (&bad_args[..], &environment[..]),
        (&args[..0], &bad_value[..]),
        (&args[..0], &bad_key[..]),
    ] {
        let error = ChildLeases::acquire(&data, &trees)
            .unwrap()
            .spawn_windows(&program, args, env, false)
            .err()
            .expect("invalid FFI input launched a child");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    assert!(registry.lock(&trees[0], LockMode::Exclusive).is_ok());
    drop(registry);
    assert!(File::options()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&gate)
        .is_ok());
}

#[test]
fn actor() {
    if std::env::var_os("HORIZON_WINDOWS_LEASE_ACTOR").is_none() {
        return;
    }
    let numbers = std::env::var("HORIZON_RUNTIME_LEASE_HANDLES").unwrap();
    assert!(!numbers.is_empty());
    for number in numbers.split(',') {
        let handle = number.parse::<usize>().unwrap() as *mut std::ffi::c_void;
        let mut flags = 0;
        // SAFETY: GetHandleInformation only queries the candidate handle, never dereferences it.
        assert_ne!(unsafe { GetHandleInformation(handle, &mut flags) }, 0);
        assert_ne!(flags & HANDLE_FLAG_INHERIT, 0);
    }
    assert_ne!(std::env::var("HORIZON_RUNTIME_DATA_ROOT").unwrap(), "bogus");
    assert_eq!(
        std::env::var("HORIZON_WINDOWS_VALUE").unwrap(),
        "☃ = value\nline"
    );
    let values: Vec<Vec<u16>> = std::env::args_os()
        .map(|s| s.encode_wide().collect())
        .collect();
    let ready = std::path::PathBuf::from(std::env::var_os("HORIZON_WINDOWS_READY").unwrap());
    let staging = ready.with_extension("staging");
    std::fs::write(&staging, serde_json::to_vec(&values).unwrap()).unwrap();
    std::fs::rename(staging, ready).unwrap();
    wait_for(Path::new(
        &std::env::var_os("HORIZON_WINDOWS_STOP").unwrap(),
    ));
    std::process::exit(37);
}
