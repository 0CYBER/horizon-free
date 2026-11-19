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
fn symlinked_ancestor_in_launch_roots_publishes_and_acquires() {
    let root = tempfile::tempdir().unwrap();
    let root_path = root.path().canonicalize().unwrap();
    let var_home = root_path.join("var/home");
    let player = var_home.join("player");
    let real_data = player.join(".local/share/horizon-launcher");
    let real_install = real_data.join("versions/game");
    let real_game = player.join(".local/share/horizon");
    for dir in [&real_install, &real_game] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(real_install.join(".installed"), b"").unwrap();
    let real_executable = real_install.join("horizon-client");
    fs::write(&real_executable, b"test image").unwrap();

    // Simulate atomic Linux distros (e.g. Fedora Silverblue) where /home -> /var/home.
    let symlink_home = root_path.join("home");
    std::os::unix::fs::symlink(&var_home, &symlink_home).unwrap();
    let aliased_game = symlink_home.join("player/.local/share/horizon");
    let aliased_executable =
        symlink_home.join("player/.local/share/horizon-launcher/versions/game/horizon-client");

    let mut command = Command::new(&aliased_executable);
    command
        .current_dir(&aliased_game)
        .env("HORIZON_HOME", &aliased_game);
    let selection = crate::selection::LaunchArguments::for_command(&command).unwrap();
    let canonical_exec = aliased_executable.canonicalize().unwrap();
    let canonical_data = symlink_home
        .join("player/.local/share/horizon-launcher")
        .canonicalize()
        .unwrap();
    let trees =
        managed_lease_paths(&canonical_exec, &selection, &canonical_data, &aliased_game).unwrap();
    let program = Program {
        executable: aliased_executable.into_os_string(),
        arguments: vec![],
        environment: vec![],
        directory: aliased_game.clone(),
        raw_windows: None,
    };
    let _prepared = Prepared::new(canonical_data, trees, program, &selection.context).unwrap();

    // Also verify record::Reference::publish and acquire succeed when game_dir is canonical
    // (as from getcwd) while home still traverses the symlinked ancestor (as from $HOME).
    let registry = Registry::with_release(&real_data, crate::locks::Release::Close).unwrap();
    let install_snapshot = Snapshot::capture(&real_install, true).unwrap();
    let game_snapshot = Snapshot::capture(&real_game, false).unwrap();
    let current_exe = std::env::current_exe().unwrap();
    let image_snapshot = Snapshot::capture(current_exe.parent().unwrap(), false).unwrap();
    let entry = Entry {
        version: crate::local_control::VERSION,
        data: real_data,
        trees: vec![
            install_snapshot.tree.clone(),
            game_snapshot.tree.clone(),
            image_snapshot.tree.clone(),
        ],
    };
    for snapshot in [&install_snapshot, &game_snapshot, &image_snapshot] {
        drop(
            registry
                .lock(&snapshot.directory, LockMode::Shared)
                .unwrap(),
        );
    }
    let aliased_context = ModSelectionContext {
        game_dir: aliased_game.clone(),
        home: aliased_game.clone(),
        dev_mods: vec![],
    };
    let (reference, _file) =
        record::Reference::publish(&install_snapshot, &entry, &aliased_context).unwrap();
    let mixed_context = ModSelectionContext {
        game_dir: real_game,
        home: aliased_game,
        dev_mods: vec![],
    };
    assert!(reference.acquire(&mixed_context).is_ok());
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
