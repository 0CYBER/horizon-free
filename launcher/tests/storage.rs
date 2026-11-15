//! Real CLI regressions use separate launcher/game/config roots and never touch player data.
use std::{fs, path::PathBuf, process::Command};

struct Home(tempfile::TempDir);
impl Home {
    fn new() -> Self {
        Self(tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap())
    }
    fn file(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.0.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_horizon-launcher"));
        command.args(args);
        command
            .current_dir(self.0.path())
            .env("HORIZON_HOME", self.0.path().join("game"))
            .env("HORIZON_LAUNCHER_HOME", self.0.path().join("launcher"))
            .env("XDG_DATA_HOME", self.0.path().join("data"))
            .env("XDG_CONFIG_HOME", self.0.path().join("config"))
            .env("HORIZON_LAUNCHER_TRASH", self.0.path().join("trash"))
            .env(
                "HORIZON_LAUNCHER_CLIPBOARD",
                self.0.path().join("clipboard"),
            )
            .env("HORIZON_LAUNCHER_DESKTOP", self.0.path().join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1");
        #[cfg(target_os = "linux")]
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 256,
                    rlim_max: 256,
                };
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        command.output().unwrap()
    }
    fn refuse(&self, row: &str) {
        let output = self.run(&["storage", "--clear", row]);
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("An older game build can use this cache without runtime leases."),
            "{output:?}"
        );
    }
    fn clear(&self, row: &str) -> serde_json::Value {
        let output = self.run(&["storage", "--clear", row]);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[test]
fn trust_native_code_flag_persists_install_consent() {
    let home = Home::new();
    let output = home.run(&[
        "native-build",
        "--trust-native-code",
        "--sync-internals",
        "missing-project",
    ]);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not a native mod project"),
        "{output:?}"
    );
    let settings: serde_json::Value =
        serde_json::from_slice(&fs::read(home.0.path().join("launcher/settings.json")).unwrap())
            .unwrap();
    assert!(settings["native_mods_allowed_at"]
        .as_u64()
        .is_some_and(|at| at > 0));
}

#[test]
fn build_files_refuse_without_touching_targets_sources_or_player_data() {
    let home = Home::new();
    let world = home.file("launcher/worlds/precious/level.dat", b"world");
    let game_world = home.file("game/worlds/precious/level.dat", b"world");
    let output = home.file("launcher/native/target/fast/output", &[7; 8192]);
    let source = home.file("launcher/native/source/rev/lib.rs", b"source");
    let prebuilt = home.file("launcher/native/prebuilt/rev/game", b"prebuilt");
    home.refuse("build-files");
    home.refuse("prebuilt-games");
    assert_eq!(fs::read(prebuilt).unwrap(), b"prebuilt");
    assert_eq!(fs::read(output).unwrap(), vec![7; 8192]);
    assert_eq!(fs::read(source).unwrap(), b"source");
    home.refuse("all");
    assert_eq!(fs::read(world).unwrap(), b"world");
    assert_eq!(fs::read(game_world).unwrap(), b"world");
}

#[cfg(unix)]
#[test]
fn running_unplayed_version_is_excluded_even_after_its_process_exits() {
    use std::process::Stdio;
    let home = Home::new();
    home.file("launcher/versions/v0.0.1/.installed", b"");
    home.file("launcher/settings.json", br#"{"selected":"v0.0.1"}"#);
    home.file("launcher/versions/native-played/.installed", b"");
    let played = home.file(
        "launcher/versions/native-played/game",
        b"instance-pinned native game",
    );
    home.file(
        "launcher/instances.json",
        br#"{"instances":{"horizon":{"version":"native-played"}}}"#,
    );
    home.file("launcher/versions/native-live/.installed", b"");
    let game = home.file("launcher/versions/native-live/game", b"running");
    // ps sees the version path while read waits on the retained pipe. No game data is needed.
    let mut process = Command::new("sh")
        .args(["-c", "read -r ignored", "storage-fixture"])
        .arg(&game)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    home.clear("spare-game-builds");
    assert!(game.exists());
    assert!(
        played.exists(),
        "instance-pinned native version was selected"
    );
    process.kill().unwrap();
    process.wait().unwrap();
    home.refuse("spare-game-builds");
    assert_eq!(fs::read(game).unwrap(), b"running");
    home.refuse("all");
    assert!(
        played.exists(),
        "Clear all selected an instance-pinned native version"
    );
}

impl Home {
    fn installed_version(&self) {
        self.file("launcher/versions/v0.0.1/.installed", b"");
        self.file("launcher/settings.json", br#"{"selected":"v0.0.1"}"#);
    }
}

#[cfg(unix)]
#[test]
fn instance_caches_are_preserved_while_running_and_when_idle() {
    use std::process::Stdio;
    let home = Home::new();
    home.installed_version();
    let game = home.file("launcher/versions/v0.0.1/game", b"running");
    let cache = home.file("game/.horizon-cache/held-output", b"active instance cache");
    let mut process = Command::new("sh")
        .args(["-c", "read -r ignored", "storage-instance"])
        .arg(game)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    home.clear("other-caches");
    assert_eq!(
        fs::read(&cache).unwrap(),
        b"active instance cache",
        "running instance cache was selected"
    );
    process.kill().unwrap();
    process.wait().unwrap();
    home.refuse("other-caches");
    assert_eq!(fs::read(cache).unwrap(), b"active instance cache");
}

#[test]
fn unresolved_instance_caches_are_never_selected() {
    let home = Home::new();
    home.file(
        "launcher/settings.json",
        br#"{"selected":"missing-version"}"#,
    );
    let cache = home.file("game/wasm-cache/precious", b"unresolved instance cache");
    home.clear("other-caches");
    assert_eq!(
        fs::read(cache).unwrap(),
        b"unresolved instance cache",
        "an unresolved instance cannot prove it is idle"
    );
}

#[test]
fn report_cleanup_refuses_and_retains_old_recent_and_newest_reports() {
    use std::time::{Duration, SystemTime};
    let home = Home::new();
    home.installed_version();
    for (name, half_days) in [
        ("logs/old.log", 29),
        ("logs/recent.log", 27),
        ("logs/latest.log", 0),
        ("crash-reports/old.txt", 40),
        ("crash-reports/newest.txt", 32),
    ] {
        let path = home.file(&format!("game/{name}"), b"report");
        let modified = SystemTime::now() - Duration::from_secs(half_days * 43200);
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }
    // Inventory still selects only the two old, non-newest reports.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let output = home.run(&["storage"]);
        assert!(output.status.success(), "{output:?}");
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let row = report["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["category"] == "other-caches")
            .unwrap();
        let selected_bytes: u64 = ["logs/old.log", "crash-reports/old.txt"]
            .iter()
            .map(|name| {
                fs::metadata(home.0.path().join("game").join(name))
                    .unwrap()
                    .blocks()
                    * 512
            })
            .sum();
        assert!(selected_bytes > 0);
        assert_eq!(row["bytes"], selected_bytes);
    }
    home.refuse("other-caches");
    assert!(
        home.0.path().join("game/logs/old.log").exists(),
        "14.5-day log was cleared despite refusal"
    );
    assert!(
        home.0.path().join("game/logs/recent.log").exists(),
        "13.5-day log was cleared"
    );
    assert!(
        home.0.path().join("game/logs/latest.log").exists(),
        "latest log was cleared"
    );
    assert!(
        home.0.path().join("game/crash-reports/old.txt").exists(),
        "old crash report was cleared despite refusal"
    );
    assert!(
        home.0.path().join("game/crash-reports/newest.txt").exists(),
        "newest crash report must survive even after 14 days"
    );
}

#[test]
fn all_refuses_and_retains_caches_worlds_and_mod_packages() {
    let home = Home::new();
    home.installed_version();
    let cache = home.file("game/.horizon-cache/output", b"cache");
    let world = home.file("game/worlds/precious/level.dat", b"world");
    let package = home.file("game/horizon-mods/precious.hmod", b"mod package");
    home.refuse("all");
    assert_eq!(fs::read(cache).unwrap(), b"cache");
    assert_eq!(
        fs::read(world).unwrap(),
        b"world",
        "worlds selected as instance cache"
    );
    assert_eq!(
        fs::read(package).unwrap(),
        b"mod package",
        "installed packages selected as instance cache"
    );
}

#[cfg(unix)]
#[test]
fn symlink_named_instance_is_refused_without_touching_its_outside_target() {
    let home = Home::new();
    home.installed_version();
    let outside = Home::new();
    let precious = outside.file(".horizon-cache/decoy", b"outside cache");
    fs::create_dir_all(home.0.path().join("game/instances")).unwrap();
    std::os::unix::fs::symlink(
        outside.0.path(),
        home.0.path().join("game/instances/linked"),
    )
    .unwrap();
    let output = home.run(&["storage", "--clear", "other-caches"]);
    assert_eq!(
        fs::read(precious).unwrap(),
        b"outside cache",
        "outside instance target was touched"
    );
    assert!(
        !output.status.success(),
        "symlink-named instance was accepted"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn hundreds_of_reports_and_jars_fit_the_descriptor_budget_and_refuse_repeatedly() {
    let home = Home::new();
    home.installed_version();
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(15 * 86400);
    for index in 0..400 {
        let path = home.file(&format!("game/logs/{index}.log"), b"old log");
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old))
            .unwrap();
        home.file(&format!("game/jars/{index}.jar"), b"unused jar");
    }
    let report = home.run(&["storage"]);
    assert!(
        report.status.success(),
        "read-only report exceeded 256 FDs: {report:?}"
    );
    home.refuse("other-caches");
    home.refuse("other-caches");
    assert_eq!(
        fs::read(home.0.path().join("game/jars/399.jar")).unwrap(),
        b"unused jar"
    );
    assert_eq!(
        fs::read_dir(home.0.path().join("game/logs"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "log"))
            .count(),
        400
    );
    assert_eq!(
        fs::read_dir(home.0.path().join("game/jars"))
            .unwrap()
            .count(),
        400
    );
}

#[test]
fn private_build_tools_still_clear_and_report_freed_bytes() {
    let home = Home::new();
    // Cleanup retains directory ancestry, not just the three cache roots. Establish the
    // complete empty skeleton using the same paths before measuring its allocated size.
    // Clearing zero-length placeholders also gives NTFS the same directory-entry history.
    let paths = [
        "launcher/rust/toolchain/bin/rustc",
        "launcher/cargo/registry/cache/output",
        "launcher/tmp/output",
    ];
    for path in paths {
        home.file(path, b"");
    }
    let empty = home.clear("rust-compiler");
    for path in paths {
        assert!(!home.0.path().join(path).exists(), "{path}");
    }
    let empty_row = empty["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "rust-compiler")
        .unwrap();
    let retained_bytes = empty_row["remaining_bytes"].as_u64().unwrap();
    let rust = home.file("launcher/rust/toolchain/bin/rustc", &[1; 8192]);
    let cargo = home.file("launcher/cargo/registry/cache/output", &[2; 8192]);
    let scratch = home.file("launcher/tmp/output", &[3; 8192]);
    let lock_path = home.file("launcher/cargo/.package-cache", b"");
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock().unwrap();
    let busy = home.run(&["storage", "--clear", "rust-compiler"]);
    assert!(!busy.status.success(), "{busy:?}");
    assert_eq!(fs::read(&cargo).unwrap(), vec![2; 8192]);
    drop(lock);
    // The busy attempt may clear Rust before encountering Cargo's lock. Refill it so
    // the successful clear must report freeing all three payloads, not just two.
    fs::write(&rust, [1; 8192]).unwrap();
    let report = home.clear("rust-compiler");
    for path in [rust, cargo, scratch] {
        assert!(!path.exists(), "{path:?}");
    }
    let row = report["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["category"] == "rust-compiler")
        .unwrap();
    assert!(row["freed_bytes"].as_u64().unwrap() >= 3 * 8192, "{row}");
    assert_eq!(row["remaining_bytes"], retained_bytes);
}

#[test]
fn explicit_version_removal_refuses_before_changing_install_metadata() {
    let home = Home::new();
    let marker = home.file("launcher/versions/spare/.installed", b"installed");
    let game = home.file("launcher/versions/spare/game", b"game binary");
    let output = home.run(&["remove", "spare"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("An older game build can use this cache without runtime leases."),
        "{output:?}"
    );
    assert_eq!(fs::read(marker).unwrap(), b"installed");
    assert_eq!(fs::read(game).unwrap(), b"game binary");
}
