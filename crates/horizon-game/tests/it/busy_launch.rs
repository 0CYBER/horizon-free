//! Usage refusals precede managed lifetime acquisition in the actual executable.
use horizon_mod_package::{
    locks::{LockMode, Release},
    runtime_leases::Registry,
};
use std::{fs, process::Command};

#[test]
#[ignore = "daily"]
fn invalid_options_keep_usage_status_even_when_the_managed_install_is_busy() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("data");
    let install = data.join("versions/test-game");
    let home = root.path().join("home");
    let game = root.path().join("game");
    fs::create_dir_all(&install).unwrap();
    fs::create_dir_all(&game).unwrap();
    let executable = install.join(if cfg!(windows) {
        "horizon-client.exe"
    } else {
        "horizon-client"
    });
    fs::copy(env!("CARGO_BIN_EXE_horizon-client"), &executable).unwrap();
    fs::write(install.join(".installed"), b"").unwrap();
    let registry = Registry::with_release(&data, Release::Close).unwrap();
    let directory = horizon_mod_package::runtime_leases::open_directory(&install).unwrap();
    let _busy = registry.lock(&directory, LockMode::Exclusive).unwrap();
    for (args, status, error) in [
        (vec!["--width", "wide"], 2, "--width:"),
        (vec!["--bogus", "value"], 2, "--bogus: unknown option"),
        (vec!["--frames", "1"], 78, "operation would block"),
    ] {
        let output = Command::new(&executable)
            .args(args)
            .current_dir(&game)
            .env("HORIZON_HOME", &home)
            .env("HORIZON_LAUNCHER_HOME", &data)
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("HORIZON_LAUNCHER_TRASH", root.path().join("trash"))
            .env("HORIZON_LAUNCHER_CLIPBOARD", root.path().join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", root.path().join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(error),
            "{output:?}"
        );
        assert!(
            !home.join("crash-reports").exists(),
            "usage/lease refusal wrote a crash report"
        );
    }
}
