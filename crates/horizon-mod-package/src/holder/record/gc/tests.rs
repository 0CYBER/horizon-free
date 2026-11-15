use super::*;
use crate::runtime_leases::{Error, Registry};
// A module-local mutex cannot exclude forks from other libtest modules. Run
// gate-lifetime assertions without unrelated threads temporarily inheriting locks.
fn isolated() -> bool {
    if std::env::var_os("RECORD_GC_ISOLATED").is_some() {
        return false;
    }
    let test = std::thread::current().name().unwrap().to_owned();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--include-ignored", "--exact", &test, "--nocapture"])
        .env("RECORD_GC_ISOLATED", "1")
        .output()
        .unwrap();
    // The child must run this one test: a success that ran nothing (an ignored or filtered name) checks nothing.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed;"),
        "the isolated rerun of {test} did not pass exactly one test:\n{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    drop(Registry::with_release(root.path(), crate::locks::Release::Close).unwrap());
    let path = root.path().join("run-records").join("a".repeat(64));
    std::fs::create_dir_all(&path).unwrap();
    let dir = open_directory(&path).unwrap();
    let record = Record {
        version: crate::local_control::VERSION,
        key: [0; 16],
        roots: [identity(&dir).unwrap(); 2],
        entry: Entry {
            version: crate::local_control::VERSION,
            data: root.path().to_owned(),
            trees: vec![Tree {
                path: path.clone(),
                identity: identity(&dir).unwrap(),
                complete: Some([0; 2]),
            }],
        },
    };
    std::fs::write(
        path.join("launch.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    for name in ["stdout.log", "stdout.log.1", "stderr.log", "stderr.log.1"] {
        std::fs::write(path.join(name), b"raw capture").unwrap();
    }
    (root, path)
}

fn assert_malformed_roots_retained(invalid: &[usize]) {
    let (root, path) = fixture();
    let payload = path.join("launch.json");
    let mut record: Record = serde_json::from_slice(&std::fs::read(&payload).unwrap()).unwrap();
    for &index in invalid {
        record.roots[index] = [u64::MAX; 2];
    }
    let bytes = serde_json::to_vec(&record).unwrap();
    std::fs::write(&payload, &bytes).unwrap();
    assert_eq!(
        Cleanup::existing(root.path())
            .unwrap()
            .collect_records()
            .unwrap(),
        0
    );
    assert_eq!(std::fs::read(&payload).unwrap(), bytes);
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 5);
    for name in ["stdout.log", "stdout.log.1", "stderr.log", "stderr.log.1"] {
        assert_eq!(std::fs::read(path.join(name)).unwrap(), b"raw capture");
    }
}

#[test]
#[ignore = "daily"]
fn both_malformed_roots_retain_entire_group() {
    if isolated() {
        return;
    }
    assert_malformed_roots_retained(&[0, 1]);
}

#[test]
#[ignore = "daily"]
fn either_malformed_root_retains_entire_group() {
    if isolated() {
        return;
    }
    for index in 0..2 {
        assert_malformed_roots_retained(&[index]);
    }
}

#[cfg(unix)]
#[test]
fn closed_stdio_actor() {
    use std::os::fd::AsRawFd;
    let Some(root) = std::env::var_os("RECORD_GC_CLOSED_ROOT") else {
        return;
    };
    // SAFETY: isolated child process, no further standard IO is required.
    unsafe {
        for fd in 0..3 {
            libc::close(fd);
        }
    }
    let cleanup = Cleanup::existing(Path::new(&root)).unwrap();
    let dir = retained_dir(cleanup.data.open_dir_nofollow("run-records").unwrap()).unwrap();
    assert!(dir.as_raw_fd() > 2);
    assert_eq!(cleanup.collect_records().unwrap(), 1);
    std::process::exit(0);
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn collection_with_closed_stdio() {
    if isolated() {
        return;
    }
    let (root, path) = fixture();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--include-ignored",
            "--exact",
            "holder::record::gc::tests::closed_stdio_actor",
        ])
        .env("RECORD_GC_CLOSED_ROOT", root.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!path.exists());
}

#[test]
fn independent_owner_actor() {
    let Some(root) = std::env::var_os("RECORD_GC_OWNER_ROOT") else {
        return;
    };
    let _owner = Registry::existing(Path::new(&root)).unwrap();
    println!("OWNER_READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
}

#[test]
#[ignore = "daily"]
fn final_process_exit_authorizes_collection() {
    if isolated() {
        return;
    }
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let (root, path) = fixture();
    let spawn = || {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "holder::record::gc::tests::independent_owner_actor",
                "--nocapture",
            ])
            .env("RECORD_GC_OWNER_ROOT", root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line.contains("OWNER_READY") {
                break;
            }
            line.clear();
        }
        child.stdout = Some(reader.into_inner());
        child
    };
    let mut first = spawn();
    let mut last = spawn();
    assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
    first.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(first.wait().unwrap().success());
    assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
    assert!(path.join("launch.json").exists());
    last.stdin.take().unwrap().write_all(b"x").unwrap();
    assert!(last.wait().unwrap().success());
    assert_eq!(
        Cleanup::existing(root.path())
            .unwrap()
            .collect_records()
            .unwrap(),
        1
    );
}

#[test]
#[ignore = "daily"]
fn collective_owners_and_complete_group() {
    if isolated() {
        return;
    }
    let (root, path) = fixture();
    let first = Registry::existing(root.path()).unwrap();
    let last = Registry::existing(root.path()).unwrap();
    assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
    drop(first);
    assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
    assert!(path.join("stdout.log").exists());
    drop(last);
    let cleanup = Cleanup::existing(root.path()).unwrap();
    assert_eq!(cleanup.collect_records().unwrap(), 1);
    assert!(!path.exists());
    assert!(root.path().join("run-leases/gate").exists());
    assert!(matches!(Registry::existing(root.path()), Err(Error::Busy)));
}

#[test]
#[ignore = "daily"]
fn malformed_unknown_and_wrong_identity_retain_entire_group() {
    if isolated() {
        return;
    }
    for variant in 0..10 {
        let (root, path) = fixture();
        let payload = path.join("launch.json");
        match variant {
            0 => std::fs::write(&payload, b"{").unwrap(),
            1 => {
                std::fs::write(path.join("unknown"), b"").unwrap();
            }
            2 => {
                std::fs::create_dir(path.join("subdir")).unwrap();
            }
            3 => {
                std::fs::write(&payload, vec![b' '; LIMIT as usize + 1]).unwrap();
            }
            _ => {
                let mut record: Record =
                    serde_json::from_slice(&std::fs::read(&payload).unwrap()).unwrap();
                match variant {
                    4 => record.entry.trees[0].identity = [0; 2],
                    5 => record.version = 255,
                    6 => record.entry.trees.clear(),
                    7 => record.entry.data = path.clone(),
                    8 => record.entry.trees[0].path = root.path().to_owned(),
                    _ => {
                        std::fs::OpenOptions::new()
                            .write(true)
                            .open(path.join("stdout.log"))
                            .unwrap()
                            .set_len(crate::output::LIMIT + 1)
                            .unwrap();
                    }
                }
                std::fs::write(&payload, serde_json::to_vec(&record).unwrap()).unwrap();
            }
        }
        assert_eq!(
            Cleanup::existing(root.path())
                .unwrap()
                .collect_records()
                .unwrap(),
            0
        );
        assert!(payload.exists());
        assert!(path.join("stdout.log").exists());
    }
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn links_are_not_followed_or_unlinked() {
    if isolated() {
        return;
    }
    for hard in [false, true] {
        let (root, path) = fixture();
        let outside = root.path().join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        std::fs::remove_file(path.join("stderr.log")).unwrap();
        if hard {
            std::fs::hard_link(&outside, path.join("stderr.log")).unwrap();
        } else {
            std::os::unix::fs::symlink(&outside, path.join("stderr.log")).unwrap();
        }
        assert_eq!(
            Cleanup::existing(root.path())
                .unwrap()
                .collect_records()
                .unwrap(),
            0
        );
        assert!(path.join("launch.json").exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"keep");
    }
    let (root, path) = fixture();
    let moved = root.path().join("outside");
    std::fs::rename(&path, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &path).unwrap();
    assert_eq!(
        Cleanup::existing(root.path())
            .unwrap()
            .collect_records()
            .unwrap(),
        0
    );
    assert!(moved.join("launch.json").exists());
}
