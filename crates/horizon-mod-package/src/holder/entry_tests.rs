use super::*;
use crate::selection::ModSelectionContext;
use std::fs;
fn fixture() -> (
    tempfile::TempDir,
    record::Reference,
    File,
    ModSelectionContext,
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    fs::write(path.join(".installed"), b"").unwrap();
    let registry =
        Registry::with_release(&path.join("data"), crate::locks::Release::Close).unwrap();
    let snapshot = Snapshot::capture(&path, true).unwrap();
    let executable = std::env::current_exe().unwrap();
    let image = Snapshot::capture(executable.parent().unwrap(), false).unwrap();
    let entry = Entry {
        version: crate::local_control::VERSION,
        data: path.join("data"),
        trees: vec![snapshot.tree.clone(), image.tree],
    };
    drop(
        registry
            .lock(&snapshot.directory, LockMode::Shared)
            .unwrap(),
    );
    drop(registry.lock(&image.directory, LockMode::Shared).unwrap());
    let context = ModSelectionContext {
        game_dir: path.clone(),
        home: path,
        dev_mods: vec![],
    };
    let (reference, file) = record::Reference::publish(&snapshot, &entry, &context).unwrap();
    (root, reference, file, context)
}
#[test]
fn bounded_record_authentication_roots_and_file_replacement_are_enforced() {
    for change in 0..6 {
        let (root, reference, file, mut context) = fixture();
        let encoded = reference.encode().unwrap();
        if change == 0 {
            assert!(reference.acquire(&context).is_ok());
        } else if change == 1 {
            let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
            value["key"][0] = (value["key"][0].as_u64().unwrap() ^ 1).into();
            let forged = record::Reference::decode(&value.to_string()).unwrap();
            assert!(forged.acquire(&context).is_err());
        } else if change == 2 {
            context.home = context.home.join("data");
            assert!(reference.acquire(&context).is_err());
        } else {
            let path = root.path().join("launch.json");
            if change == 3 {
                fs::rename(&path, root.path().join("old")).unwrap();
                fs::write(&path, fs::read(root.path().join("old")).unwrap()).unwrap();
            } else if change == 4 {
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend(vec![b' '; 65537]);
                fs::write(&path, bytes).unwrap();
            } else {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                value["version"] = 99.into();
                fs::write(&path, value.to_string()).unwrap();
            }
            assert!(reference.acquire(&context).is_err());
        }
        drop(file);
        assert!(record::Reference::decode(&format!("{encoded}{}", " ".repeat(4097))).is_err());
        let mut value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        value["version"] = 99.into();
        assert!(record::Reference::decode(&value.to_string()).is_err());
        value["version"] = crate::local_control::VERSION.into();
        let mut expanded_path = context.home.clone();
        expanded_path.push("a".repeat(5000));
        value["tree"]["path"] = serde_json::to_value(expanded_path.into_os_string()).unwrap();
        let expanded: record::Reference = serde_json::from_value(value).unwrap();
        assert!(expanded.encode().is_err());
    }
}
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn every_fresh_lease_and_retained_metadata_fd_survives_closed_stdio() {
    use std::os::fd::AsRawFd;
    const MASK: &str = "HORIZON_ENTRY_TEST_CLOSED_STDIO";
    if let Ok(mask) = std::env::var(MASK) {
        let (_root, reference, _file, context) = fixture();
        for target in 0..3 {
            if mask.parse::<u8>().unwrap() & (1 << target) != 0 {
                // SAFETY: this isolated test process owns its standard descriptors.
                unsafe {
                    libc::close(target);
                }
            }
        }
        let acquired = reference.acquire(&context).unwrap();
        let check = |fd| {
            assert!(fd > 2, "fresh retained descriptor aliases stdio");
            // SAFETY: the owning object remains live throughout this query.
            assert_ne!(
                unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC,
                0
            );
        };
        for claim in &acquired._claims {
            for file in claim.files() {
                check(file.as_raw_fd());
            }
        }
        for snapshot in &acquired._snapshots {
            check(snapshot.directory.as_raw_fd());
            if let Some(file) = &snapshot._complete {
                check(file.as_raw_fd());
            }
        }
        check(acquired._record.as_ref().unwrap().as_raw_fd());
        check(acquired._capture.as_ref().unwrap().as_raw_fd());
        return;
    }
    for mask in 1..8 {
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "holder::entry_tests::every_fresh_lease_and_retained_metadata_fd_survives_closed_stdio"])
            .env(MASK, mask.to_string()).status().unwrap().success());
    }
}
