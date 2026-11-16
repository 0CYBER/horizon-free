use super::*;

use crate::runtime_leases::isolated_test as isolated;

fn setup() -> (tempfile::TempDir, Arc<Registry>) {
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Unlock).unwrap();
    (root, registry)
}

#[test]
#[ignore = "daily"]
fn permanent_gate_excludes_new_owners_until_cleanup_drops() {
    if isolated() {
        return;
    }
    let (root, registry) = setup();
    let before = registry.directory.metadata("gate").unwrap();
    drop(registry);
    let cleanup = Cleanup::existing(root.path()).unwrap();
    assert!(matches!(Registry::existing(root.path()), Err(Error::Busy)));
    drop(cleanup);
    let registry = Registry::existing(root.path()).unwrap();
    let after = registry.directory.metadata("gate").unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
}

#[test]
#[ignore = "daily"]
fn every_claim_owner_holds_the_gate_until_its_last_reference() {
    if isolated() {
        return;
    }
    for mode in [LockMode::Shared, LockMode::Exclusive] {
        let (root, registry) = setup();
        let tree = open_directory(root.path()).unwrap();
        let claim = registry.lock(&tree, mode).unwrap();
        let other = Registry::existing(root.path()).unwrap();
        assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
        drop(registry);
        assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
        drop(other);
        assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
        drop(claim);
        let cleanup = Cleanup::existing(root.path()).unwrap();
        // A new owner cannot get past the gate to create any tree entry.
        assert!(matches!(
            Registry::with_release(root.path(), Release::Unlock),
            Err(Error::Busy)
        ));
        drop(cleanup);
    }
}

#[test]
#[ignore = "daily"]
fn existing_never_creates_missing_namespace_or_gate() {
    if isolated() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    assert!(Cleanup::existing(root.path()).is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    fs::create_dir(root.path().join("run-leases")).unwrap();
    assert!(Cleanup::existing(root.path()).is_err());
    assert_eq!(
        fs::read_dir(root.path().join("run-leases"))
            .unwrap()
            .count(),
        0
    );
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn redirected_gate_and_namespace_are_refused() {
    if isolated() {
        return;
    }
    use std::os::unix::fs::symlink;
    let (root, registry) = setup();
    drop(registry);
    let outside = root.path().join("outside");
    fs::write(&outside, b"untouched").unwrap();
    let leases = root.path().join("run-leases");
    fs::remove_file(leases.join("gate")).unwrap();
    symlink(&outside, leases.join("gate")).unwrap();
    assert!(Cleanup::existing(root.path()).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    fs::rename(&leases, root.path().join("moved")).unwrap();
    symlink(root.path().join("moved"), &leases).unwrap();
    assert!(Cleanup::existing(root.path()).is_err());
}

#[cfg(windows)]
#[test]
fn windows_open_reader_blocks_exclusive_gate_and_collection_shares_no_gate() {
    if isolated() {
        return;
    }
    let (root, registry) = setup();
    let reader = initialize_lock(&registry.directory, "gate");
    assert!(reader.is_err()); // permanent gate is never recreated
    let mut options = OpenOptions::new();
    options.read(true);
    let reader = registry.directory.open_with("gate", &options).unwrap();
    drop(registry);
    assert!(matches!(Cleanup::existing(root.path()), Err(Error::Busy)));
    drop(reader);
    let _cleanup = Cleanup::existing(root.path()).unwrap();
    let directory = open_directory(&root.path().join("run-leases")).unwrap();
    assert!(directory.open("gate").is_err());
}

#[test]
#[ignore = "daily"]
fn published_unit_keys_survive_registry_reopen_without_creating_entries() {
    if isolated() {
        return;
    }
    let (root, registry) = setup();
    let tree = open_directory(root.path()).unwrap();
    drop(registry.lock(&tree, LockMode::Shared).unwrap());
    drop(registry);
    let registry = Registry::existing(root.path()).unwrap();
    drop(registry.lock(&tree, LockMode::Shared).unwrap());
}
