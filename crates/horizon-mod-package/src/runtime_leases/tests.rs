use super::*;
#[cfg(windows)]
#[test]
fn an_open_initializer_allows_readers_but_excludes_cleanup() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Unlock).unwrap();
    let initializing = initialize_lock(&registry.directory, "initializing").unwrap();
    let acquire = |mode| {
        open_lock(
            &registry.directory,
            "initializing",
            mode,
            Release::Unlock,
            true,
        )
    };
    let reader = acquire(LockMode::Shared)
        .expect("an entry creator must not spuriously refuse a concurrent shared launch");
    drop(reader);
    assert!(matches!(acquire(LockMode::Exclusive), Err(Error::Busy)));
    drop(initializing);
    drop(acquire(LockMode::Exclusive).unwrap());
    // The old initializer's write access alone reproduces the sharing refusal.
    let writable_initializer = registry.directory.create("initializing").unwrap();
    assert!(acquire(LockMode::Shared).is_err());
    drop(writable_initializer);
    drop(acquire(LockMode::Shared).unwrap());
}

fn directory(path: &Path) -> Dir {
    fs::create_dir_all(path).unwrap();
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
}

#[test]
#[ignore = "daily"]
fn shared_readers_exclude_cleanup_but_not_an_unrelated_tree() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(&root.path().join("data"), Release::Unlock).unwrap();
    let first = directory(&root.path().join("first"));
    let other = directory(&root.path().join("other"));
    let a = registry.lock(&first, LockMode::Shared).unwrap();
    let b = registry.lock(&first, LockMode::Shared).unwrap();
    assert!(matches!(
        registry.lock(&first, LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(registry.lock(&other, LockMode::Exclusive).unwrap());
    drop(a);
    assert!(matches!(
        registry.lock(&first, LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(b);
    drop(registry.lock(&first, LockMode::Exclusive).unwrap());
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_moved_tree_keeps_its_identity_and_redirected_registries_are_refused() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    let path = root.path().join("tree");
    let tree = directory(&path);
    let held = registry.lock(&tree, LockMode::Shared).unwrap();
    let moved = root.path().join("moved");
    fs::rename(&path, &moved).unwrap();
    let alias = directory(&moved);
    assert!(matches!(
        registry.lock(&alias, LockMode::Exclusive),
        Err(Error::Busy)
    ));
    let replacement = directory(&path);
    drop(registry.lock(&replacement, LockMode::Exclusive).unwrap());
    drop(held);
    drop(registry);
    fs::remove_dir_all(data.join("run-leases")).unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, data.join("run-leases")).unwrap();
    assert!(Registry::with_release(&data, Release::Unlock).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn a_lock_file_cannot_redirect_access_outside_the_registry() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    let tree = directory(&root.path().join("tree"));
    drop(registry.lock(&tree, LockMode::Shared).unwrap());
    // The registry also holds its gate; directory order is the filesystem's, so pick the tree's file by name.
    let files: Vec<_> = fs::read_dir(data.join("run-leases"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.file_name().unwrap() != "gate")
        .collect();
    let [file] = <[_; 1]>::try_from(files).unwrap();
    fs::remove_file(&file).unwrap();
    let precious = root.path().join("precious");
    fs::write(&precious, b"keep").unwrap();
    std::os::unix::fs::symlink(&precious, &file).unwrap();
    assert!(registry.lock(&tree, LockMode::Exclusive).is_err());
    // Capabilities also confine followed links; an in-registry alias must still be refused.
    fs::remove_file(&file).unwrap();
    fs::write(data.join("run-leases/sibling"), b"keep").unwrap();
    std::os::unix::fs::symlink("sibling", &file).unwrap();
    assert!(registry.lock(&tree, LockMode::Exclusive).is_err());
    assert_eq!(fs::read(precious).unwrap(), b"keep");
}

#[test]
#[ignore = "daily"]
fn gate_excludes_collection_mid_acquire_and_until_last_claim_drops() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Unlock).unwrap();
    let held = registry.directory.try_clone().unwrap();
    assert!(matches!(
        open_lock(&held, "gate", LockMode::Exclusive, Release::Unlock, false),
        Err(Error::Busy)
    ));
    let claim = registry
        .lock(&directory(&root.path().join("tree")), LockMode::Shared)
        .unwrap();
    drop(registry);
    assert!(matches!(
        open_lock(&held, "gate", LockMode::Exclusive, Release::Unlock, false),
        Err(Error::Busy)
    ));
    drop(claim);
    let exclusive = open_lock(&held, "gate", LockMode::Exclusive, Release::Unlock, false).unwrap();
    assert!(matches!(
        Registry::with_release(root.path(), Release::Unlock),
        Err(Error::Busy)
    ));
    drop(exclusive);
    assert!(Registry::with_release(root.path(), Release::Unlock).is_ok());
}

#[test]
#[ignore = "daily"]
fn entry_is_noncreating_and_owns_independent_exclusion() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    assert!(Registry::existing(&path).is_err());
    assert!(!path.join("run-leases").exists());
    let bootstrap = Registry::with_release(&path, Release::Unlock).unwrap();
    let tree = directory(&path.join("tree"));
    let before = bootstrap.directory.entries().unwrap().count();
    let entry = Registry::existing(&path).unwrap();
    assert!(entry.lock(&tree, LockMode::Shared).is_err());
    assert_eq!(bootstrap.directory.entries().unwrap().count(), before);
    drop(bootstrap.lock(&tree, LockMode::Shared).unwrap());
    let held = entry.lock(&tree, LockMode::Shared).unwrap();
    drop(bootstrap);
    assert!(matches!(
        entry.lock(&tree, LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(held);
    assert!(entry.lock(&tree, LockMode::Exclusive).is_ok());
    drop(entry);
    fs::remove_file(path.join("run-leases/gate")).unwrap();
    assert!(Registry::existing(&path).is_err());
    assert!(!path.join("run-leases/gate").exists());
}

#[cfg(windows)]
#[test]
fn entry_refuses_junction_components() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("real/inner")).unwrap();
    let status = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(root.path().join("junction"))
        .arg(root.path().join("real"))
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let path = root.path().canonicalize().unwrap().join("junction/inner");
    assert!(open_directory(&path).is_err());
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn entry_does_not_follow_redirected_components() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    fs::create_dir(path.join("real")).unwrap();
    fs::create_dir(path.join("real/inner")).unwrap();
    std::os::unix::fs::symlink("real", path.join("redirect")).unwrap();
    assert!(open_directory(&path.join("redirect/inner")).is_err());
    assert!(open_directory(Path::new("relative")).is_err());
    assert!(open_directory(&path.join("real/../real")).is_err());
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "daily"]
fn unsupported_volume_refuses_registry_and_tree_without_creating_entries() {
    if isolated_test() {
        return;
    }
    assert!(matches!(
        Registry::with_release(Path::new("/proc"), Release::Unlock),
        Err(Error::UnsupportedFilesystem)
    ));
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Unlock).unwrap();
    let before = registry.directory.entries().unwrap().count();
    let proc = Dir::open_ambient_dir("/proc", cap_std::ambient_authority()).unwrap();
    assert!(matches!(
        registry.lock(&proc, LockMode::Shared),
        Err(Error::UnsupportedFilesystem)
    ));
    assert_eq!(registry.directory.entries().unwrap().count(), before);
}

#[test]
#[ignore = "daily"]
fn retiring_an_exclusive_unit_key_does_not_retire_other_units_or_the_gate() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Close).unwrap();
    let first = root.path().join("first");
    let other = root.path().join("other");
    fs::create_dir(&first).unwrap();
    fs::create_dir(&other).unwrap();
    let first = open_directory(&first).unwrap();
    let other = open_directory(&other).unwrap();
    drop(registry.lock(&other, LockMode::Shared).unwrap());
    registry
        .lock(&first, LockMode::Exclusive)
        .unwrap()
        .retire()
        .unwrap();
    let existing = Registry::existing(root.path()).unwrap();
    assert!(
        matches!(existing.lock(&first, LockMode::Shared), Err(Error::Io(error)) if error.kind() == io::ErrorKind::NotFound)
    );
    drop(existing.lock(&other, LockMode::Shared).unwrap());
    assert!(
        matches!(registry.lock(&other, LockMode::Shared).unwrap().retire(), Err(Error::Io(error)) if error.kind() == io::ErrorKind::InvalidInput)
    );
}

#[test]
#[ignore = "daily"]
fn publication_snapshot_excludes_new_builds_without_blocking_game_tree_claims() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Close).unwrap();
    let tree = open_directory(root.path()).unwrap();
    let first_install = registry.publication(LockMode::Shared).unwrap();
    let other_install = registry.publication(LockMode::Shared).unwrap();
    assert!(matches!(
        registry.publication(LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(first_install);
    drop(other_install);
    let cleanup_snapshot = registry.publication(LockMode::Exclusive).unwrap();
    assert!(matches!(
        registry.publication(LockMode::Shared),
        Err(Error::Busy)
    ));
    drop(registry.lock(&tree, LockMode::Shared).unwrap());
    drop(cleanup_snapshot);
    drop(registry.publication(LockMode::Shared).unwrap());
}

#[test]
#[ignore = "daily"]
fn unit_removal_excludes_new_installs_and_games_through_atomic_detachment() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Close).unwrap();
    let unit = root.path().join("unit");
    fs::create_dir(&unit).unwrap();
    fs::write(unit.join(".installed"), b"").unwrap();
    fs::write(unit.join("resource"), b"complete resource").unwrap();
    let install = registry.publication(LockMode::Shared).unwrap();
    assert!(matches!(
        registry.remove_unit::<Error>(&unit, |_, _| Ok(())),
        Err(Error::Busy)
    ));
    assert!(unit.join(".installed").exists());
    drop(install);
    registry
        .remove_unit::<Error>(&unit, |directory, is_directory| {
            assert!(is_directory);
            assert_eq!(directory.read("resource").unwrap(), b"complete resource");
            assert!(matches!(
                registry.publication(LockMode::Shared),
                Err(Error::Busy)
            ));
            assert!(matches!(
                registry.lock(directory, LockMode::Shared),
                Err(Error::Busy)
            ));
            Ok(())
        })
        .unwrap();
    assert!(!unit.exists());
    assert!(!fs::read_dir(root.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".trash-")
    }));
    drop(registry.publication(LockMode::Shared).unwrap());
}

#[cfg(windows)]
#[test]
fn windows_unit_removal_never_restores_completion_after_a_rename_refusal() {
    if isolated_test() {
        return;
    }
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Close).unwrap();
    let unit = root.path().join("unit");
    fs::create_dir(&unit).unwrap();
    fs::write(unit.join(".installed"), b"").unwrap();
    fs::write(unit.join("game.exe"), b"mapped executable").unwrap();
    let running = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(unit.join("game.exe"))
        .unwrap();
    assert!(matches!(
        registry.remove_unit::<Error>(&unit, |_, _| Ok(())),
        Err(Error::Busy)
    ));
    assert!(unit.is_dir());
    assert!(unit.join("game.exe").is_file());
    assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".trash-")
    }));
    assert!(!unit.join(".installed").exists());
    drop(running);
    registry.remove_unit::<Error>(&unit, |_, _| Ok(())).unwrap();
    assert!(!unit.exists());
}

#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn unit_removal_refuses_a_redirected_final_component() {
    if isolated_test() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let registry = Registry::with_release(root.path(), Release::Close).unwrap();
    let target = root.path().join("target-unit");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("resource"), b"precious").unwrap();
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    assert!(matches!(
        registry.remove_unit::<Error>(&alias, |_, _| Ok(())),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(fs::read(target.join("resource")).unwrap(), b"precious");
}
