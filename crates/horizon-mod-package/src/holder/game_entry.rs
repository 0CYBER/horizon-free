use super::*;
use crate::{
    output::Capture,
    selection::{LaunchArguments, ModSelectionContext},
};
/// Earliest-main only: acquire before logging, mod loading or worker creation.
pub fn acquire_entry(context: &ModSelectionContext) -> io::Result<Option<Acquired>> {
    let Some(encoded) = std::env::var_os(ENTRY) else {
        return Ok(None);
    };
    let reference = record::Reference::decode(encoded.to_str().ok_or(io::ErrorKind::InvalidData)?)?;
    // SAFETY: this entry point runs before application threads and consumes private bootstrap state.
    unsafe {
        std::env::remove_var(ENTRY);
    }
    reference.acquire(context).map(Some)
}
/// Acquire only after the game has validated its complete Options. Call enter() first.
pub fn acquire(selection: &LaunchArguments) -> Option<Capture> {
    if std::env::var_os(ENTRY).is_none() {
        let result = (|| -> io::Result<()> {
            let executable = std::env::current_exe()?;
            // Help and capability replies never open game caches or load mods.
            if selection.is_information_request() {
                return Ok(());
            }
            if let Some(ownership) = acquire_standalone(&executable, selection)? {
                Box::leak(Box::new(ownership));
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Horizon: cannot acquire launcher cache leases: {error}");
            std::process::exit(78);
        }
        return None;
    }
    let result = (|| -> io::Result<Capture> {
        let ownership = acquire_entry(&selection.context)?.ok_or(io::ErrorKind::InvalidData)?;
        // Native exit callbacks run after main's locals drop; only kernel exit may release claims.
        let ownership = Box::leak(Box::new(ownership));
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, IntoRawFd};
            for target in 0..3 {
                // SAFETY: earliest-main owns its standard slots; no concurrent descriptor creation.
                if unsafe { libc::fcntl(target, libc::F_GETFD) } < 0 {
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open("/dev/null")?;
                    // SAFETY: both slots are valid and exclusively owned at this point.
                    if unsafe { libc::dup2(file.as_raw_fd(), target) } < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if file.as_raw_fd() == target {
                        let _ = file.into_raw_fd();
                    }
                }
            }
        }
        let capture = Capture::install(ownership._capture.as_ref().unwrap())?;
        Ok(capture)
    })();
    match result {
        Ok(capture) => Some(capture),
        Err(error) => {
            eprintln!("Horizon: {error}");
            std::process::exit(78);
        }
    }
}

/// Direct and Windows launches participate without creating launcher state.
fn acquire_standalone(
    executable: &Path,
    selection: &LaunchArguments,
) -> io::Result<Option<Acquired>> {
    let install = executable.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let mut found = None;
    for data in install.ancestors() {
        // Do not probe filesystem capabilities on unrelated development ancestors.
        if std::fs::symlink_metadata(data.join("run-leases")).is_err() {
            continue;
        }
        // Only ancestors of the data root may be aliases. Keep the root leaf and
        // registry descendants no-follow, as when the launcher creates the registry.
        crate::runtime_leases::checked(data, true).map_err(lease_error)?;
        let data = data.canonicalize()?;
        match Registry::existing(&data) {
            Ok(registry) => {
                found = Some((data, registry));
                break;
            }
            Err(error) => return Err(lease_error(error)),
        }
    }
    let Some((data, registry)) = found else {
        return Ok(None);
    };
    let paths = lease_paths(executable, selection, &data, &std::env::current_dir()?)?;
    let snapshots = paths
        .iter()
        .map(|(path, complete)| Snapshot::capture(path, *complete))
        .collect::<io::Result<Vec<_>>>()?;
    let claims = acquire_snapshots(&registry, &snapshots)?;
    Ok(Some(Acquired {
        _record: None,
        _capture: None,
        _claims: claims,
        _snapshots: snapshots,
    }))
}

/// Resolve aliases before a missing selected suffix, so deletion cannot make a
/// managed home appear to be outside its canonical launcher data directory.
fn canonical_candidate(path: &Path) -> io::Result<PathBuf> {
    let mut ancestor = path;
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                if ancestor != path {
                    resolved.push(path.strip_prefix(ancestor).expect("selected path ancestor"));
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or(error)?;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Only the launcher's macOS game bundle relocates the image below its installation.
/// Snapshot capture requires the returned installation's `.installed` marker.
#[cfg(target_os = "macos")]
fn installation(folder: &Path) -> &Path {
    if folder.ends_with("Horizon.app/Contents/MacOS") {
        folder.ancestors().nth(3).unwrap_or(folder)
    } else {
        folder
    }
}

/// Canonical managed units and their enclosing units, shared by launcher and game.
/// Cleanup locks unit identities, so a child must also protect deletable ancestors.
pub fn lease_paths(
    executable: &Path,
    selection: &LaunchArguments,
    data: &Path,
    directory: &Path,
) -> io::Result<Vec<(PathBuf, bool)>> {
    // Normalize both sides, including Windows' extended-path prefix and macOS /tmp aliases.
    let data = data.canonicalize()?;
    let data = data.as_path();
    let program = executable
        .parent()
        .ok_or(io::ErrorKind::InvalidInput)?
        .canonicalize()?;
    #[cfg(target_os = "macos")]
    let install = installation(&program);
    #[cfg(not(target_os = "macos"))]
    let install = program.as_path();
    if install.ancestors().any(|path| {
        path.file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with(".trash-"))
    }) {
        return Err(io::Error::other("launcher cache is being removed"));
    }
    let complete = !install.strip_prefix(data).is_ok_and(|relative| {
        !relative.starts_with("versions")
            && relative
                .components()
                .any(|part| part.as_os_str() == "target")
    });
    let mut paths = vec![(install.to_path_buf(), complete)];
    // The holder checks that the program's own folder is leased (`Entry::check_image`).
    if program != install {
        paths.push((program.clone(), false));
    }
    let mut candidates = vec![
        selection.context.game_dir.clone(),
        selection.context.home.clone(),
    ];
    for argument in &selection.rest {
        if let crate::selection::Argument::Value(name, value) = argument
            && name == "--resources"
        {
            candidates.push(directory.join(value));
        }
    }
    for path in candidates {
        let path = canonical_candidate(&path)?;
        if path.starts_with(data) && !paths.iter().any(|(existing, _)| *existing == path) {
            paths.push((path, false));
        }
    }
    let leaves = paths.clone();
    for (path, _) in leaves {
        for parent in path.ancestors().skip(1).take_while(|p| *p != data) {
            if !parent.starts_with(data) {
                break;
            }
            if !paths.iter().any(|(existing, _)| existing == parent) {
                paths.push((parent.to_owned(), false));
            }
        }
    }
    Ok(paths)
}

/// Managed launches publish real context-root claims before creating the immutable record.
/// Standalone entry uses `lease_paths` instead: it can only acquire already registered cache units.
pub fn managed_lease_paths(
    executable: &Path,
    selection: &LaunchArguments,
    data: &Path,
    directory: &Path,
) -> io::Result<Vec<(PathBuf, bool)>> {
    let mut paths = lease_paths(executable, selection, data, directory)?;
    for root in [&selection.context.game_dir, &selection.context.home] {
        let root = root.canonicalize()?;
        if !paths.iter().any(|(path, _)| *path == root) {
            paths.push((root, false));
        }
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locks::Release;

    #[test]
    #[ignore = "fixture writer subprocess only"]
    fn write_missing_registration_actor() {
        if let Some(destination) = std::env::var_os("HORIZON_MISSING_LEASE_COPY") {
            let destination = std::path::PathBuf::from(destination);
            std::fs::copy(std::env::current_exe().unwrap(), &destination).unwrap();
            std::fs::OpenOptions::new()
                .write(true)
                .open(destination)
                .unwrap()
                .sync_all()
                .unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn missing_registration_is_displayed_and_information_does_not_register() {
        const ACTOR: &str = "HORIZON_MISSING_LEASE_ACTOR";
        if let Some(mode) = std::env::var_os(ACTOR) {
            let mut command = Command::new(std::env::current_exe().unwrap());
            if mode == "help" {
                command.arg("--help");
            }
            let selection = LaunchArguments::for_command(&command).unwrap();
            acquire(&selection);
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let install = data.join("target/fast");
        std::fs::create_dir_all(&install).unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        drop(Registry::with_release(&data, Release::Close).unwrap());
        let executable = install.join(format!("actor{}", std::env::consts::EXE_SUFFIX));
        // Isolate the writable image from unrelated test threads' fork/exec windows.
        // CLOEXEC alone only closes inherited writable descriptors at exec, not fork.
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "holder::game_entry::tests::write_missing_registration_actor",
                "--ignored",
            ])
            .env("HORIZON_MISSING_LEASE_COPY", &executable)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .any(|line| {
                    line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; ")
                })
        );
        let run = |mode| {
            Command::new(&executable)
                .args([
                    "--include-ignored",
                    "--exact",
                    "holder::game_entry::tests::missing_registration_is_displayed_and_information_does_not_register",
                    "--nocapture",
                ])
                .env(ACTOR, mode)
                .env("HORIZON_HOME", &home)
                .env_remove(ENTRY)
                .current_dir(root.path())
                .output()
                .unwrap()
        };
        assert!(run("help").status.success());
        let registry = Registry::existing(&data).unwrap();
        assert!(matches!(
            registry.lock(&open_directory(&install).unwrap(), LockMode::Shared),
            Err(crate::runtime_leases::Error::Io(error)) if error.kind() == io::ErrorKind::NotFound
        ));
        drop(registry);
        let output = run("game");
        assert_eq!(output.status.code(), Some(78));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("unregistered"), "{stderr}");
        assert!(
            stderr.contains("this managed installation has unregistered cache directories; use Horizon Launcher Play from the same launcher data directory with the intended instance and installed version (horizon-launcher play --instance <instance> --version <tag>)"),
            "{stderr}"
        );
        assert!(!stderr.contains("start the launcher once"), "{stderr}");
    }

    #[test]
    fn managed_launch_binds_external_instance_and_log_roots() {
        for separate in [false, true] {
            let root = tempfile::tempdir().unwrap();
            // Compare canonical identities on every platform, including Windows' extended prefix.
            let root_path = root.path().canonicalize().unwrap();
            let data = root_path.join("launcher");
            let install = data.join("versions/game");
            let game = root_path.join("instance");
            let home = if separate {
                root_path.join("logs")
            } else {
                game.clone()
            };
            for directory in [&install, &game, &home] {
                std::fs::create_dir_all(directory).unwrap();
            }
            std::fs::write(install.join(".installed"), "").unwrap();
            let executable = install.join("game");
            std::fs::write(&executable, "test image").unwrap();
            let mut command = Command::new(&executable);
            command.current_dir(&game).env("HORIZON_HOME", &home);
            let selection = LaunchArguments::for_command(&command).unwrap();
            let trees = managed_lease_paths(&executable, &selection, &data, &game).unwrap();
            for context in [&game, &home] {
                let matching: Vec<_> = trees.iter().filter(|(path, _)| path == context).collect();
                assert_eq!(matching.len(), 1);
                assert!(
                    !matching[0].1,
                    "external roots have no installed completion marker"
                );
                assert!(!context.join(".installed").exists());
            }
            assert!(!trees.iter().any(|(path, _)| path == &root_path));
            assert!(
                trees
                    .iter()
                    .any(|(path, complete)| *path == install && *complete)
            );
            assert!(trees.iter().any(|(path, _)| *path == data.join("versions")));
            let program = Program {
                executable: executable.into_os_string(),
                arguments: vec![],
                environment: vec![],
                directory: game,
                raw_windows: None,
            };
            let _prepared = Prepared::new(data, trees, program, &selection.context).unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn managed_launch_from_a_macos_app_bundle_leases_the_installation() {
        // The launcher starts the game on macOS from <version>/Horizon.app/Contents/MacOS, whose
        // programs are hard links to the version's: the marker stays in the version folder (arismoko/horizon#3812).
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let data = root_path.join("launcher");
        let install = data.join("versions/game");
        let programs = install.join("Horizon.app/Contents/MacOS");
        let game = root_path.join("instance");
        for directory in [&programs, &game] {
            std::fs::create_dir_all(directory).unwrap();
        }
        std::fs::write(install.join(".installed"), "").unwrap();
        let executable = programs.join("game");
        std::fs::write(&executable, "test image").unwrap();
        let mut command = Command::new(&executable);
        command.current_dir(&game).env("HORIZON_HOME", &game);
        let selection = LaunchArguments::for_command(&command).unwrap();
        let trees = managed_lease_paths(&executable, &selection, &data, &game).unwrap();
        let complete: Vec<_> = trees.iter().filter(|(_, complete)| *complete).collect();
        assert_eq!(complete, [&(install.clone(), true)]);
        // The holder's image check asks for the program's own folder.
        assert!(trees.contains(&(programs.clone(), false)));
        let program = Program {
            executable: executable.into_os_string(),
            arguments: vec![],
            environment: vec![],
            directory: game,
            raw_windows: None,
        };
        let _prepared = Prepared::new(data, trees, program, &selection.context).unwrap();
    }

    #[test]
    fn flat_and_lookalike_layouts_require_completion_beside_the_image() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().canonicalize().unwrap();
        let mut layouts = vec![
            "flat",
            "Other.app/Contents/MacOS",
            "Horizon.app/Contents/extra/MacOS",
        ];
        if !cfg!(target_os = "macos") {
            layouts.push("Horizon.app/Contents/MacOS");
        }
        for layout in layouts {
            let version = data.join("versions").join(layout.replace('/', "-"));
            let folder = version.join(layout);
            let game = data.join("instance");
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::create_dir_all(&game).unwrap();
            // An unrelated enclosing marker must not make this image complete.
            std::fs::write(version.join(".installed"), "").unwrap();
            let executable = folder.join("game");
            std::fs::write(&executable, "test image").unwrap();
            let mut command = Command::new(&executable);
            command.current_dir(&game).env("HORIZON_HOME", &game);
            let selection = LaunchArguments::for_command(&command).unwrap();
            let trees = managed_lease_paths(&executable, &selection, &data, &game).unwrap();
            assert_eq!(trees[0], (folder.clone(), true), "{layout}");
            let program = || Program {
                executable: executable.clone().into_os_string(),
                arguments: vec![],
                environment: vec![],
                directory: game.clone(),
                raw_windows: None,
            };
            let error = Prepared::new(data.clone(), trees.clone(), program(), &selection.context)
                .err()
                .unwrap();
            assert_eq!(error.kind(), io::ErrorKind::NotFound, "{layout}");
            std::fs::write(folder.join(".installed"), "").unwrap();
            assert_eq!(
                managed_lease_paths(&executable, &selection, &data, &game).unwrap(),
                trees
            );
            Prepared::new(data.clone(), trees, program(), &selection.context).unwrap();
        }
    }

    #[test]
    #[ignore = "daily"]
    fn direct_entry_blocks_cleanup_and_refuses_busy_or_deleted_install() {
        if crate::runtime_leases::isolated_test() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("versions/game");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join(".installed"), b"").unwrap();
        let registry = Registry::with_release(root.path(), Release::Close).unwrap();
        let directory = open_directory(&install).unwrap();
        // The installer publishes the tree's lock before completing installation.
        drop(registry.lock(&directory, LockMode::Shared).unwrap());
        drop(
            registry
                .lock(
                    &open_directory(install.parent().unwrap()).unwrap(),
                    LockMode::Shared,
                )
                .unwrap(),
        );
        let executable = install.join("game");
        let command = Command::new(&executable);
        let selection = LaunchArguments::for_command(&command).unwrap();
        let ownership = acquire_standalone(&executable, &selection)
            .unwrap()
            .unwrap();
        assert!(matches!(
            registry.lock(&directory, LockMode::Exclusive),
            Err(crate::runtime_leases::Error::Busy)
        ));
        drop(ownership);
        let cleanup = registry.lock(&directory, LockMode::Exclusive).unwrap();
        assert_eq!(
            acquire_standalone(&executable, &selection)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        std::fs::remove_file(install.join(".installed")).unwrap();
        drop(cleanup);
        assert!(acquire_standalone(&executable, &selection).is_err());
        drop(directory);
        std::fs::remove_dir(&install).unwrap();
        assert!(acquire_standalone(&executable, &selection).is_err());
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn standalone_entry_resolves_data_root_ancestors_but_rejects_a_linked_root() {
        if crate::runtime_leases::isolated_test() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let real = root_path.join("real");
        let data = real.join("data");
        let versions = data.join("versions");
        let install = versions.join("game");
        let home = data.join("home");
        for path in [&install, &home] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(install.join(".installed"), b"").unwrap();
        let alias = root_path.join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let aliased_data = alias.join("data");
        let registry = Registry::with_release(&aliased_data, Release::Close).unwrap();
        let directories: Vec<_> = [&install, &versions, &home]
            .into_iter()
            .map(|path| open_directory(path).unwrap())
            .collect();
        for directory in &directories {
            drop(registry.lock(directory, LockMode::Shared).unwrap());
        }
        let executable = aliased_data.join("versions/game/game");
        let mut command = Command::new(&executable);
        command
            .current_dir(aliased_data.join("home"))
            .env("HORIZON_HOME", aliased_data.join("home"));
        let selection = LaunchArguments::for_command(&command).unwrap();
        let ownership = acquire_standalone(&executable, &selection)
            .unwrap()
            .unwrap();
        for directory in &directories {
            assert!(matches!(
                registry.lock(directory, LockMode::Exclusive),
                Err(crate::runtime_leases::Error::Busy)
            ));
        }
        drop(ownership);
        let cleanup = registry.lock(&directories[0], LockMode::Exclusive).unwrap();
        assert_eq!(
            acquire_standalone(&executable, &selection)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(cleanup);
        let linked_data = root_path.join("linked-data");
        std::os::unix::fs::symlink(&data, &linked_data).unwrap();
        let executable = linked_data.join("versions/game/game");
        assert!(acquire_standalone(&executable, &selection).is_err());
        std::fs::rename(data.join("run-leases"), data.join("held-registry")).unwrap();
        std::os::unix::fs::symlink("held-registry", data.join("run-leases")).unwrap();
        let executable = aliased_data.join("versions/game/game");
        assert!(acquire_standalone(&executable, &selection).is_err());
    }

    #[test]
    #[ignore = "daily"]
    fn unpublished_enclosing_unit_refuses_and_target_build_needs_no_install_marker() {
        if crate::runtime_leases::isolated_test() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let data = root.path().canonicalize().unwrap();
        let install = data.join("target/fast");
        std::fs::create_dir_all(&install).unwrap();
        let registry = Registry::with_release(&data, Release::Close).unwrap();
        drop(
            registry
                .lock(&open_directory(&install).unwrap(), LockMode::Shared)
                .unwrap(),
        );
        let executable = install.join("game");
        let selection = LaunchArguments::for_command(&Command::new(&executable)).unwrap();
        assert!(acquire_standalone(&executable, &selection).is_err());
        drop(
            registry
                .lock(
                    &open_directory(&data.join("target")).unwrap(),
                    LockMode::Shared,
                )
                .unwrap(),
        );
        let ownership = acquire_standalone(&executable, &selection)
            .unwrap()
            .unwrap();
        assert!(matches!(
            registry.lock(
                &open_directory(&data.join("target")).unwrap(),
                LockMode::Exclusive
            ),
            Err(crate::runtime_leases::Error::Busy)
        ));
        drop(ownership);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn symlinked_resources_are_leased_by_canonical_identity() {
        if crate::runtime_leases::isolated_test() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let data = root.path().canonicalize().unwrap();
        let install = data.join("install");
        let resources = data.join("resources");
        std::fs::create_dir(&install).unwrap();
        std::fs::create_dir(&resources).unwrap();
        std::fs::write(install.join(".installed"), b"").unwrap();
        let alias = data.join("alias");
        std::os::unix::fs::symlink(&data, &alias).unwrap();
        let registry = Registry::with_release(&data, Release::Close).unwrap();
        for path in [&install, &resources] {
            drop(
                registry
                    .lock(&open_directory(path).unwrap(), LockMode::Shared)
                    .unwrap(),
            );
        }
        let executable = install.join("game");
        let mut command = Command::new(&executable);
        command.arg("--resources").arg(alias.join("resources"));
        let selection = LaunchArguments::for_command(&command).unwrap();
        let _ownership = acquire_standalone(&executable, &selection)
            .unwrap()
            .unwrap();
        assert!(matches!(
            registry.lock(&open_directory(&resources).unwrap(), LockMode::Exclusive),
            Err(crate::runtime_leases::Error::Busy)
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn acquired_snapshot_rechecks_completion_and_directory_identity() {
        if crate::runtime_leases::isolated_test() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let install = root.path().join("install");
        std::fs::create_dir(&install).unwrap();
        std::fs::write(install.join(".installed"), b"").unwrap();
        let registry = Registry::with_release(root.path(), Release::Close).unwrap();
        let snapshot = Snapshot::capture(&install, true).unwrap();
        drop(
            registry
                .lock(&snapshot.directory, LockMode::Shared)
                .unwrap(),
        );
        std::fs::rename(install.join(".installed"), install.join("old-marker")).unwrap();
        std::fs::write(install.join(".installed"), b"").unwrap();
        assert!(acquire_snapshots(&registry, &[snapshot]).is_err());
        let snapshot = Snapshot::capture(&install, true).unwrap();
        #[cfg(windows)]
        {
            // Retained Windows directory handles exclude replacement itself.
            assert!(std::fs::rename(&install, root.path().join("old-install")).is_err());
            assert!(acquire_snapshots(&registry, &[snapshot]).is_ok());
        }
        #[cfg(unix)]
        {
            std::fs::rename(&install, root.path().join("old-install")).unwrap();
            std::fs::create_dir(&install).unwrap();
            std::fs::write(install.join(".installed"), b"").unwrap();
            assert!(acquire_snapshots(&registry, &[snapshot]).is_err());
        }
    }

    #[test]
    fn resources_resolve_against_the_game_command_directory_not_the_launcher() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().canonicalize().unwrap();
        let install = data.join("install");
        let game = data.join("instances/player");
        let resources = data.join("resources");
        for path in [&install, &game, &resources] {
            std::fs::create_dir_all(path).unwrap();
        }
        let executable = install.join("game");
        let mut command = Command::new(&executable);
        command
            .current_dir(&game)
            .args(["--resources", "../../resources"]);
        let selection = LaunchArguments::for_command(&command).unwrap();
        let paths = lease_paths(&executable, &selection, &data, &game).unwrap();
        assert!(paths.iter().any(|(path, _)| *path == resources));
    }

    #[test]
    fn unmanaged_entry_does_not_create_registry() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("game");
        let selection = LaunchArguments::for_command(&Command::new(&executable)).unwrap();
        assert!(
            acquire_standalone(&executable, &selection)
                .unwrap()
                .is_none()
        );
        assert!(!root.path().join("run-leases").exists());
    }
}
