use super::*;
use crate::storage::tests::Home;

#[cfg(unix)]
#[test]
fn actor_starting_after_the_last_process_observation_cannot_authorize_cleanup() {
    use std::{
        io::{Read, Write},
        process::{Command, Stdio},
    };
    let home = Home::new();
    let cache = home.seed("cache/output");
    let storage = Storage::at(&home.0).unwrap();
    let entries = vec![Entry::fixed(
        &Root::open(&home.0).unwrap(),
        Path::new("cache"),
        Category::OtherCaches,
    )
    .unwrap()];
    let tag = format!(
        "last-scan-{}",
        home.0.file_name().unwrap().to_string_lossy()
    );
    // This is the LAST process observation: the actor starts only after its result exists.
    assert!(!backend::in_use(&tag));
    let mut actor = Command::new("sh")
        .args([
            "-c",
            "exec 3<>\"$1\"; printf ready; read -r stop",
            "late-cache-actor",
        ])
        .arg(&cache)
        .arg(backend::version_dir(&tag).join(backend::CLIENT))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut ready = [0; 5];
    actor.stdout.take().unwrap().read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    let result = storage.apply(entries, Some(Category::OtherCaches), false);
    let retained = cache.exists();
    actor.stdin.take().unwrap().write_all(b"stop\n").unwrap();
    assert!(actor.wait().unwrap().success());
    assert!(matches!(result, Err(Error::UnprovenExclusion)));
    assert!(
        retained,
        "actor started after the final scan lost its cache"
    );
    // A later absence observation is not mutual exclusion either.
    assert!(!backend::in_use(&tag));
    let entry = Entry::fixed(
        &Root::open(&home.0).unwrap(),
        Path::new("cache"),
        Category::OtherCaches,
    )
    .unwrap();
    assert!(matches!(entry.clear(), Err(Error::UnprovenExclusion)));
    assert!(cache.exists());
}

#[cfg(unix)]
#[test]
fn entry_uses_held_directory_after_root_path_is_redirected() {
    let home = Home::new();
    let outside = Home::new();
    home.seed("cache/output");
    let precious = outside.seed("cache/precious");
    let storage = Storage::at(&home.0).unwrap();
    let entry = Entry::fixed(
        &Root::open(&home.0).unwrap(),
        Path::new("cache"),
        Category::RustCompiler,
    )
    .unwrap();
    let moved = home.0.with_extension("original");
    fs::rename(&home.0, &moved).unwrap();
    std::os::unix::fs::symlink(&outside.0, &home.0).unwrap();
    assert!(matches!(Root::configured(&home.0), Err(Error::Unsafe(_))));
    storage
        .apply(vec![entry], Some(Category::RustCompiler), false)
        .unwrap();
    assert!(precious.exists());
    assert!(!moved.join("cache/output").exists());
    fs::remove_file(&home.0).unwrap();
    fs::rename(moved, &home.0).unwrap();
}

#[cfg(unix)]
#[test]
fn hard_linked_outputs_across_entries_count_once() {
    use std::os::unix::fs::MetadataExt;
    let home = Home::new();
    let output = home.seed("native/source/first");
    fs::create_dir_all(home.0.join("native/patched")).unwrap();
    fs::hard_link(&output, home.0.join("native/patched/second")).unwrap();
    let expected = [
        output,
        home.0.join("native/source"),
        home.0.join("native/patched"),
    ]
    .into_iter()
    .map(|path| fs::metadata(path).unwrap().blocks() * 512)
    .sum::<u64>();
    let storage = Storage::at(&home.0).unwrap();
    let entries = ["native/source", "native/patched"]
        .into_iter()
        .map(|path| {
            Entry::fixed(
                &Root::open(&home.0).unwrap(),
                Path::new(path),
                Category::BuildFiles,
            )
            .unwrap()
        })
        .collect();
    let report = storage.apply(entries, None, false).unwrap();
    let row = report
        .rows
        .iter()
        .find(|row| row.category == Category::BuildFiles)
        .unwrap();
    assert_eq!(row.bytes, expected);
}

#[test]
fn referenced_jar_and_uninstrumented_unused_jar_are_kept() {
    use horizon_platform::mod_package::{manifest::JarRef, Package};
    use sha2::{Digest, Sha512};
    use std::io::Write;
    let home = Home::new();
    fs::create_dir_all(home.0.join("jars")).unwrap();
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file(
            "assets/example/lang/en_us.json",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"{}").unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    let jar = JarRef::new(
        "original".into(),
        "1.0.0".into(),
        format!("{:x}", Sha512::digest(&bytes)),
    )
    .unwrap();
    let referenced = jar.cache_path(&home.0.join("jars"));
    fs::write(&referenced, bytes).unwrap();
    let manifest = home.seed("horizon-mods/example/horizon-mod.toml");
    fs::write(&manifest, format!("id='example'\nname='Example'\nversion='1.0.0'\napi='0.2'\ncapabilities=['registries']\n[[jar]]\nid='original'\nversion='1.0.0'\nsha512='{}'\n", jar.sha512())).unwrap();
    Package::read(manifest.parent().unwrap()).unwrap();
    let unused = home.seed("jars/unused.jar");
    let storage = Storage::at(&home.0).unwrap();
    let mut entries = Vec::new();
    unused_jars(&mut entries, &Root::open(&home.0).unwrap()).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, unused);
    assert!(matches!(
        storage.apply(entries, Some(Category::OtherCaches), false),
        Err(Error::UnprovenExclusion)
    ));
    assert!(referenced.exists());
    assert!(unused.exists());
}

#[cfg(unix)]
#[test]
fn capable_units_delete_only_after_the_last_game_lease_and_mixed_builds_stay_residual() {
    crate::test_support::assert_holder_capability();
    use std::process::Command;
    let Some(root) = std::env::var_os("CLASSIFICATION_ROOT") else {
        let root = tempfile::tempdir().unwrap();
        // Exercise a configured home behind an ancestor alias on every Unix host, not only
        // macOS's /var and /tmp. Managed descendants must still be walked without links.
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor.args(["--include-ignored", "--exact", "storage::inventory::regressions::capable_units_delete_only_after_the_last_game_lease_and_mixed_builds_stay_residual"])
            .env("CLASSIFICATION_ROOT", &alias).env("HORIZON_LAUNCHER_HOME", alias.join("data"))
            .env("HORIZON_HOME", alias.join("data/cache"))
            .env("XDG_DATA_HOME", alias.join("xdg-data")).env("XDG_CONFIG_HOME", alias.join("xdg-config"))
            .env("HORIZON_LAUNCHER_TRASH", alias.join("trash")).env("HORIZON_LAUNCHER_CLIPBOARD", alias.join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", alias.join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1").env("HORIZON_NO_AUDIO", "1");
        assert!(actor.status().unwrap().success());
        return;
    };
    let root = PathBuf::from(root);
    if std::env::var_os("CLASSIFICATION_GAME").is_some() {
        horizon_platform::mod_package::holder::enter();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(std::env::args_os().skip(1));
        let selection =
            horizon_platform::mod_package::selection::LaunchArguments::for_command(&command)
                .unwrap();
        let _capture = horizon_platform::mod_package::holder::acquire(&selection);
        fs::write(root.join("ready"), b"").unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !root.join("stop").exists() {
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
        return;
    }
    let cache = root.join("data/cache");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("resource"), b"whole resource").unwrap();
    fs::create_dir(root.join("work")).unwrap();
    let install = backend::version_dir("modern");
    fs::create_dir_all(&install).unwrap();
    let image = install.join(backend::CLIENT);
    assert!(Command::new("cp")
        .arg(std::env::current_exe().unwrap())
        .arg(&image)
        .status()
        .unwrap()
        .success());
    fs::write(install.join(".installed"), b"").unwrap();
    crate::run_leases::publish(&install).unwrap();
    crate::run_leases::publish(&cache).unwrap();
    assert!(matches!(
        Access::for_unit(&cache).unwrap(),
        Access::LeasedGame
    ));
    let old = backend::version_dir("legacy");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join(".installed"), b"").unwrap();
    fs::write(old.join(backend::CLIENT), b"uninstrumented actor").unwrap();
    assert!(matches!(
        Access::for_unit(&cache).unwrap(),
        Access::UninstrumentedGame
    ));
    assert!(matches!(
        Access::for_unit(&install).unwrap(),
        Access::LeasedGame
    ));
    assert!(matches!(
        crate::cache_cleanup::delete_game_unit(&cache),
        Err(Error::UnprovenExclusion)
    ));
    assert_eq!(fs::read(cache.join("resource")).unwrap(), b"whole resource");
    let outside = root.join("outside");
    fs::create_dir_all(outside.join("cache")).unwrap();
    fs::write(outside.join("cache/world"), b"player data").unwrap();
    let redirected = root.join("data/redirected");
    std::os::unix::fs::symlink(&outside, &redirected).unwrap();
    assert!(crate::cache_cleanup::delete_game_unit(&redirected.join("cache")).is_err());
    assert_eq!(
        fs::read(outside.join("cache/world")).unwrap(),
        b"player data"
    );
    let linked = root.join("data/linked");
    std::os::unix::fs::symlink(&cache, &linked).unwrap();
    assert!(matches!(
        crate::cache_cleanup::delete_game_unit(&linked),
        Err(Error::Unsafe(_))
    ));
    assert_eq!(fs::read(cache.join("resource")).unwrap(), b"whole resource");
    fs::remove_dir_all(&old).unwrap();
    let command = || {
        let mut game = Command::new(&image);
        // reexec_include_ignored: skip, the child reads these as launch arguments (`--key value` pairs), where a
        // lone `--include-ignored` would leave the test name without a value
        game.args(["--exact", "storage::inventory::regressions::capable_units_delete_only_after_the_last_game_lease_and_mixed_builds_stay_residual"])
            .env("CLASSIFICATION_GAME", "1")
            .env("RUST_TEST_NOCAPTURE", "1")
            .current_dir(root.join("work"));
        game
    };
    let mut game = command().spawn().unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !root.join("ready").exists() {
        assert!(
            std::time::Instant::now() < end,
            "game never acknowledged its leases; child status: {:?}",
            game.try_wait().unwrap()
        );
        std::thread::yield_now();
    }
    assert!(matches!(
        crate::cache_cleanup::delete_game_unit(&cache),
        Err(Error::Busy)
    ));
    fs::write(root.join("stop"), b"").unwrap();
    assert!(game.wait().unwrap().success());
    crate::cache_cleanup::delete_game_unit(&cache).unwrap();
    assert!(!cache.exists());
    assert_eq!(command().status().unwrap().code(), Some(78));
    fs::remove_file(install.join(".installed")).unwrap();
    let install = install.canonicalize().unwrap();
    let storage = Storage::at(&root.join("data")).unwrap();
    assert!(
        storage
            .entries()
            .unwrap()
            .iter()
            .any(|entry| entry.path == install),
        "incomplete installs must stay visible for residual reporting and retry"
    );
}

#[test]
fn storage_protects_a_held_selection_and_a_native_build_pinned_after_inventory() {
    use std::process::Command;
    if std::env::var_os("HORIZON_TEST_STORAGE_PIN").is_none() {
        let root = tempfile::tempdir().unwrap();
        assert!(Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", "storage::inventory::regressions::storage_protects_a_held_selection_and_a_native_build_pinned_after_inventory"])
            .env("HORIZON_TEST_STORAGE_PIN", "1")
            .env("HORIZON_LAUNCHER_HOME", root.path().join("launcher"))
            .env("HORIZON_HOME", root.path().join("game"))
            .env("HORIZON_LAUNCHER_LOCATION_FILE", root.path().join("location.json"))
            .status().unwrap().success());
        return;
    }
    let native = "native-storage-race";
    let latest = "nightly-2026-11-22-aaaaaaa";
    for tag in [native, latest] {
        let dir = backend::version_dir(tag);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(backend::COMPLETE), "").unwrap();
    }
    let native_dir = backend::version_dir(native);
    fs::write(
        native_dir.join(backend::CLIENT),
        horizon_platform::mod_package::holder::CAPABILITY,
    )
    .unwrap();
    crate::run_leases::publish(&native_dir).unwrap();
    let storage = Storage::open().unwrap();
    let listed = || {
        storage
            .entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.path == native_dir.canonicalize().unwrap())
            .unwrap()
    };
    let mut settings = backend::Settings {
        selected: Some(native.into()),
        ..backend::Settings::default()
    };
    settings.save().unwrap();
    assert!(matches!(listed().clear(), Err(Error::InstanceVersion(_))));
    assert!(backend::remove_version(native)
        .unwrap_err()
        .contains("Horizon"));
    assert!(native_dir.join(backend::COMPLETE).is_file());

    settings.instance_versions_migrated = true;
    settings.save().unwrap();
    let entry = listed();
    assert_eq!(entry.category, Category::SpareGameBuilds);

    instances::use_version_for(instances::DEFAULT, native).unwrap();
    let report = storage
        .apply_retaining(vec![entry], Some(Category::SpareGameBuilds), false, true)
        .unwrap();
    let row = report
        .rows
        .into_iter()
        .find(|row| row.category == Category::SpareGameBuilds)
        .unwrap();
    assert!(row.kept.iter().any(|kept| kept.reason.contains("Horizon")));
    assert!(native_dir.join(backend::COMPLETE).is_file());

    instances::use_version_for(instances::DEFAULT, latest).unwrap();
    listed().clear().unwrap();
    assert!(!native_dir.exists());
}

#[test]
fn a_survey_is_summed_again_without_walking_the_folder_again() {
    let home = Home::new();
    home.seed("a/one");
    home.seed("a/deep/two");
    home.seed("b/three");
    let root = Root::open(&home.0).unwrap();
    let entries = vec![
        Entry::fixed(&root, Path::new("a"), Category::OtherCaches).unwrap(),
        Entry::fixed(&root, Path::new("b"), Category::OtherCaches).unwrap(),
    ];
    crate::storage::SURVEYS.with(|s| s.borrow_mut().clear());
    let surveyed: Vec<_> = entries
        .iter()
        .map(Entry::survey)
        .collect::<Result<_, _>>()
        .unwrap();
    // The row sum, the per-unit sum and the per-unit "alone" sum, as the cleanup pass takes them.
    let row = allocated_of(&entries, &surveyed, Category::OtherCaches);
    let mut seen = HashSet::new();
    let units: u64 = surveyed.iter().map(|items| sum_new(items, &mut seen)).sum();
    let alone: u64 = surveyed
        .iter()
        .map(|items| sum_new(items, &mut HashSet::new()))
        .sum();
    let walked = crate::storage::SURVEYS.with(|s| s.borrow().clone());
    assert_eq!(walked.len(), entries.len(), "{walked:?}");
    assert_eq!(row, units);
    assert_eq!(row, alone);
    assert_eq!(row, allocated(&entries, Category::OtherCaches).unwrap());
    assert!(row > 0);
}
