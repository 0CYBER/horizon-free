use super::*;

#[cfg(all(test, unix))]
#[test]
fn fresh_native_install_starts_directly_without_a_launcher_launch() {
    crate::test_support::assert_holder_capability();
    use std::process::Command;
    let Some(root) = std::env::var_os("DIRECT_INSTALL_ROOT") else {
        let root = tempfile::tempdir().unwrap();
        let mut actor = Command::new(std::env::current_exe().unwrap());
        actor
            .args([
                "--exact",
                "native_mods::runtime_leases_tests::fresh_native_install_starts_directly_without_a_launcher_launch",
            ])
            .env("DIRECT_INSTALL_ROOT", root.path())
            .env("HORIZON_LAUNCHER_HOME", root.path())
            .env("HORIZON_HOME", root.path().join("home"))
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("HORIZON_LAUNCHER_TRASH", root.path().join("trash"))
            .env("HORIZON_LAUNCHER_CLIPBOARD", root.path().join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", root.path().join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1");
        assert!(actor.status().unwrap().success());
        return;
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(root.join("home")).unwrap();
    if std::env::var_os("DIRECT_INSTALLED_GAME").is_some() {
        horizon_mod_package::holder::enter();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(std::env::args_os().skip(1));
        let selection =
            horizon_mod_package::selection::LaunchArguments::for_command(&command).unwrap();
        let _capture = horizon_mod_package::holder::acquire(&selection);
        let registry = crate::run_leases::Registry::open().unwrap();
        let directory = cap_std::fs::Dir::open_ambient_dir(
            backend::version_dir("fresh"),
            cap_std::ambient_authority(),
        )
        .unwrap();
        assert!(matches!(
            registry.lock(&directory, crate::native_cache::LockMode::Exclusive),
            Err(crate::native_cache::Error::Busy)
        ));
        return;
    }
    fs::create_dir_all(backend::version_dir("base")).unwrap();
    // The production installer copies this image in a separate process, so libtest's
    // concurrent forks cannot retain a writable descriptor to the installed executable.
    install(
        "base",
        "fresh",
        Target::Client,
        &std::env::current_exe().unwrap(),
        &[],
        &serde_json::json!({}),
        &[],
    )
    .unwrap();
    crate::run_leases::publish(&root.join("home")).unwrap();
    let cache = root.join("cache");
    fs::create_dir(&cache).unwrap();
    crate::cache_cleanup::delete_game_unit_observed(&cache, || {
        let refused = install(
            "base",
            "concurrent",
            Target::Client,
            &std::env::current_exe().unwrap(),
            &[],
            &serde_json::json!({}),
            &[],
        )
        .unwrap_err();
        assert_eq!(
            refused,
            cache_error(crate::native_cache::Error::Busy),
            "publication must exclude an install between classification and rename"
        );
        assert!(!backend::version_dir("concurrent").exists());
    })
    .unwrap();
    install(
        "base",
        "concurrent",
        Target::Client,
        &std::env::current_exe().unwrap(),
        &[],
        &serde_json::json!({}),
        &[],
    )
    .unwrap();
    assert!(
        Command::new(backend::version_dir("fresh").join(backend::CLIENT))
            .args([
                "--exact",
                "native_mods::runtime_leases_tests::fresh_native_install_starts_directly_without_a_launcher_launch"
            ])
            .env("DIRECT_INSTALLED_GAME", "1")
            .current_dir(root.join("home"))
            .status()
            .unwrap()
            .success()
    );
}
