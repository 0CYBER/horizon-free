use super::*;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-install-lock-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn nested_guards_keep_the_lock_until_the_last_drop() {
    let home = scratch("nested");
    let first = lock_home(&home).unwrap();
    let nested = lock_home(&home.join(".")).unwrap();
    let third = try_lock_home(&home).unwrap();
    drop(first);
    drop(third);
    let (tx, rx) = channel();
    let target = home.clone();
    let worker = std::thread::spawn(move || {
        tx.send(try_lock_home(&target).is_none()).unwrap();
        let _guard = lock_home(&target).unwrap();
        tx.send(true).unwrap();
    });
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    drop(nested);
    assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    worker.join().unwrap();
    fs::remove_dir_all(home).unwrap();
}

#[test]
fn different_homes_do_not_block_each_other() {
    let home = scratch("first");
    let other = scratch("second");
    let first = lock_home(&home).unwrap();
    let target = other.clone();
    std::thread::spawn(move || {
        let _second = try_lock_home(&target).unwrap();
    })
    .join()
    .unwrap();
    drop(first);
    fs::remove_dir_all(home).unwrap();
    fs::remove_dir_all(other).unwrap();
}

#[test]
fn filesystem_errors_refuse_mutation_and_release_the_reservation() {
    let home = scratch("error");
    fs::create_dir(home.join(".mod-browser.lock")).unwrap();
    assert!(lock_home(&home).is_err());
    assert!(try_lock_home(&home).is_none());
    fs::remove_dir(home.join(".mod-browser.lock")).unwrap();
    drop(lock_home(&home).unwrap());
    fs::remove_dir_all(home).unwrap();
}

/// Invoked in a separate process by the transaction test below.
#[test]
fn process_probe() {
    let Some(home) = std::env::var_os("HORIZON_INSTALL_LOCK_TEST_HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    assert!(
        try_lock_home(&home).is_none(),
        "the parent owns the OS lock"
    );
    fs::write(home.join("ready"), b"").unwrap();
    let _guard = lock_home(&home).unwrap();
    let path = home.join("mod-browser.json");
    let mut entries: Vec<String> = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    entries.push("game".into());
    fs::write(
        home.join("manifest.part"),
        serde_json::to_vec(&entries).unwrap(),
    )
    .unwrap();
    fs::rename(home.join("manifest.part"), path).unwrap();
}

#[test]
#[ignore = "daily"]
fn processes_serialize_the_entire_manifest_transaction() {
    let home = scratch("process");
    let guard = lock_home(&home).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--include-ignored",
            "--exact",
            "install_lock::tests::process_probe",
            "--nocapture",
        ])
        .env("HORIZON_INSTALL_LOCK_TEST_HOME", &home)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !home.join("ready").exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child failed before blocking"
        );
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("child did not reach the lock");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Save after the child starts its transaction, while it must still wait.
    fs::write(home.join("mod-browser.json"), br#"["launcher"]"#).unwrap();
    drop(guard);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("child did not finish after the lock was released");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    let entries: Vec<String> =
        serde_json::from_slice(&fs::read(home.join("mod-browser.json")).unwrap()).unwrap();
    assert_eq!(entries, ["launcher", "game"]);
    fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[test]
fn symlinked_homes_share_the_reentrant_lock() {
    let root = scratch("symlink");
    let home = root.join("home");
    let alias = root.join("alias");
    fs::create_dir(&home).unwrap();
    std::os::unix::fs::symlink(&home, &alias).unwrap();
    let first = lock_home(&home).unwrap();
    let second = try_lock_home(&alias).unwrap();
    drop(first);
    drop(second);
    fs::remove_dir_all(root).unwrap();
}
