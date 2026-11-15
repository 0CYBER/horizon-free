use super::*;
use std::io::{BufRead, Write};
use std::process::{Child, Stdio};

#[cfg(target_os = "macos")]
#[test]
fn spawn_errors_distinguish_missing_working_directory_and_program() {
    let root = tempfile::tempdir().unwrap();
    let missing_cwd = root.path().join("Application Support/missing-instance");
    let error = spawn_program(Command::new("/usr/bin/true").current_dir(&missing_cwd)).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains("working directory"));
    assert!(error
        .to_string()
        .contains(&missing_cwd.display().to_string()));
    let missing_program = root.path().join("Application Support/missing-helper");
    let error = spawn_program(Command::new(&missing_program).current_dir(root.path())).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert!(error.to_string().contains("execute"));
    assert!(error
        .to_string()
        .contains(&missing_program.display().to_string()));
}

#[test]
fn failed_launch_log_keeps_paths_but_not_argument_or_environment_secrets() {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(root.path().join("game"));
    command
        .args(["--token", "private-argument", "--token=private-inline"])
        .current_dir(root.path())
        .env("HORIZON_ACCOUNT_SESSIONS", "private-environment")
        .env("TMPDIR", root.path());
    let error = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "open completion marker /version/.installed: not found",
    );
    let path = write_launch_failure(&root.path().join("logs"), &command, &error).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("/version/.installed"));
    assert!(text.contains("cwd:"));
    assert!(text.contains("TMPDIR:"));
    assert!(text.contains("exists=true is-dir=true permissions="));
    assert!(text.contains("stat failed:"));
    assert!(!text.contains("private-argument"));
    assert!(!text.contains("private-inline"));
    assert!(!text.contains("private-environment"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

fn directory(path: &Path) -> Dir {
    fs::create_dir_all(path).unwrap();
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
}

struct Actor(Child);
impl Drop for Actor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killed_holder_releases_production_home_source_and_parent_leases() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let game = root.path().join("game");
    let source = root.path().join("versions/native-fixture");
    let data = root.path().join("data");
    let resources = root.path().join("base/resources");
    fs::create_dir_all(&resources).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&game).unwrap();
    fs::create_dir_all(&source).unwrap();
    #[cfg(windows)]
    fs::copy(
        std::env::current_exe().unwrap(),
        source.join(crate::backend::CLIENT),
    )
    .unwrap();
    let mut actor = Actor(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "run_leases::tests::lease_holder_actor",
                "--nocapture",
            ])
            .env("HORIZON_LEASE_ACTOR_ROOT", root.path())
            .env("HORIZON_LAUNCHER_HOME", &data)
            .env("HORIZON_HOME", &home)
            .env("XDG_DATA_HOME", root.path().join("xdg-data"))
            .env("XDG_CONFIG_HOME", root.path().join("xdg-config"))
            .env("HORIZON_LAUNCHER_TRASH", root.path().join("trash"))
            .env("HORIZON_LAUNCHER_CLIPBOARD", root.path().join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", root.path().join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1")
            .current_dir(root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let output = actor.0.stdout.take().unwrap();
    let ready = std::io::BufReader::new(output)
        .lines()
        .map_while(Result::ok)
        .any(|line| line.contains("LEASE-READY"));
    assert!(ready, "holder did not acquire its production runtime guard");
    let registry = Registry::at(&data).unwrap();
    for path in [&home, &game, &source, source.parent().unwrap(), &resources] {
        assert!(
            matches!(
                registry.lock(&directory(path), LockMode::Exclusive),
                Err(Error::Busy)
            ),
            "active tree was not protected: {}",
            path.display()
        );
    }
    actor.0.kill().unwrap(); // SIGKILL on Unix; no Drop/marker cleanup in the holder.
    actor.0.wait().unwrap();
    for path in [&home, &game, &source, source.parent().unwrap(), &resources] {
        drop(
            registry
                .lock(&directory(path), LockMode::Exclusive)
                .unwrap(),
        );
    }
}

#[test]
fn lease_holder_actor() {
    let Some(root) = std::env::var_os("HORIZON_LEASE_ACTOR_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .current_dir(root.join("game"))
        .env("HORIZON_HOME", root.join("home"))
        .arg("--resources")
        .arg(root.join("base/resources"));
    mark(&mut command, &root.join("versions/native-fixture"));
    let (_command, _guard) = crate::backend::prepare_command(command).unwrap();
    println!("LEASE-READY");
    std::io::stdout().flush().unwrap();
    let mut byte = String::new();
    std::io::stdin().read_line(&mut byte).unwrap();
}

#[cfg(unix)]
#[test]
fn routed_game_keeps_its_lease_after_launcher_sigkill() {
    routed_lifetime(false);
}
#[cfg(unix)]
#[test]
fn stop_terminates_the_routed_game_not_only_its_holder() {
    routed_lifetime(true);
}
#[cfg(unix)]
fn routed_lifetime(stop: bool) {
    crate::test_support::assert_holder_capability();
    let root = tempfile::tempdir().unwrap();
    let data = root.path().canonicalize().unwrap();
    let install = data.join("install");
    let home = data.join("instances/player");
    let resources = data.join("resources");
    fs::create_dir_all(&resources).unwrap();
    fs::create_dir_all(&install).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(install.join(".installed"), b"").unwrap();
    // Copy from a separate process, avoiding a writable image fd in libtest's concurrent forks.
    assert!(Command::new("cp")
        .arg(std::env::current_exe().unwrap())
        .arg(install.join("actor"))
        .status()
        .unwrap()
        .success());
    let mut supervisor = Actor(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "run_leases::tests::routing_actor", "--nocapture"])
            .env("ROUTING_ROOT", &data)
            .env("ROUTING_MODE", "launcher")
            .env("ROUTING_STOP", if stop { "1" } else { "0" })
            // Keep actor failures visible in CI rather than only timing out on its marker.
            .env("RUST_BACKTRACE", "1")
            .env("HORIZON_LAUNCHER_HOME", &data)
            .env("HORIZON_HOME", &home)
            .env("XDG_DATA_HOME", data.join("xdg-data"))
            .env("XDG_CONFIG_HOME", data.join("xdg-config"))
            .env("HORIZON_LAUNCHER_TRASH", data.join("trash"))
            .env("HORIZON_LAUNCHER_CLIPBOARD", data.join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", data.join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !data.join("routed-ready").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "routed game did not start"
        );
        std::thread::yield_now();
    }
    if !stop {
        supervisor.0.kill().unwrap();
        supervisor.0.wait().unwrap();
    }
    let registry = Registry::at(&data).unwrap();
    assert!(matches!(
        registry.lock(&directory(&install), LockMode::Exclusive),
        Err(Error::Busy)
    ));
    assert!(
        matches!(
            registry.lock(&directory(&resources), LockMode::Exclusive),
            Err(Error::Busy)
        ),
        "relative resources lost their lease after launcher death"
    );
    if stop {
        writeln!(supervisor.0.stdin.as_mut().unwrap(), "stop").unwrap();
        assert!(supervisor.0.wait().unwrap().success());
    } else {
        fs::write(data.join("routed-stop"), b"").unwrap();
    }
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match registry.lock(&directory(&resources), LockMode::Exclusive) {
            Ok(claim) => {
                drop(claim);
                if stop {
                    let pid: i32 = fs::read_to_string(data.join("routed-ready"))
                        .unwrap()
                        .parse()
                        .unwrap();
                    let gone = unsafe { libc::kill(pid, 0) } != 0;
                    #[cfg(target_os = "linux")]
                    let gone = gone
                        || fs::read_to_string(format!("/proc/{pid}/stat"))
                            .is_ok_and(|stat| stat.rsplit_once(") ").unwrap().1.starts_with('Z'));
                    assert!(gone, "Stop left the routed game running");
                }
                break;
            }
            Err(Error::Busy) => assert!(
                std::time::Instant::now() < end,
                "game did not release resources after exit"
            ),
            Err(error) => panic!("unexpected cleanup error: {error:?}"),
        }
        std::thread::yield_now();
    }
}

#[cfg(unix)]
#[test]
fn routing_actor() {
    horizon_mod_package::holder::enter();
    let Some(root) = std::env::var_os("ROUTING_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    if std::env::var("ROUTING_MODE").as_deref() == Ok("launcher") {
        let mut game = Command::new(root.join("install/actor"));
        game.args(["--exact", "run_leases::tests::routing_actor"])
            .env("ROUTING_MODE", "game")
            .current_dir(root.join("instances/player"));
        let mut game = crate::launch::wrap(game, &crate::launch::Launch::default());
        // A wrapper may consume engine arguments before starting libtest's receiver.
        // Preserve the declared engine command, whose cwd differs from this launcher.
        let words: Vec<std::ffi::OsString> = [
            root.join("install/actor").into_os_string(),
            "--exact".into(),
            "run_leases::tests::routing_actor".into(),
            "--resources".into(),
            "../../resources".into(),
        ]
        .into();
        game.env(super::GAME, serde_json::to_string(&words).unwrap());
        if std::env::var("ROUTING_STOP").as_deref() == Ok("1") {
            let (tx, rx) = std::sync::mpsc::channel();
            let running = crate::backend::spawn(crate::backend::Job::Client, game, tx).unwrap();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).unwrap();
            running.kill();
            loop {
                if matches!(
                    rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
                    crate::backend::Msg::Exit(_, _)
                ) {
                    break;
                }
            }
        } else {
            let (mut game, _runtime) = crate::backend::prepare_command(game).unwrap();
            let mut child = super::spawn(&mut game, false).unwrap();
            let _ = child.wait();
        }
    } else {
        horizon_mod_package::holder::enter();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(std::env::args_os().skip(1));
        let selection =
            horizon_mod_package::selection::LaunchArguments::for_command(&command).unwrap();
        let _capture = horizon_mod_package::holder::acquire(&selection);
        assert!(_capture.is_some(), "game did not enter through the holder");
        // The parent may stop the game as soon as the readiness file exists.
        let pending = root.join("routed-ready.tmp");
        fs::write(&pending, std::process::id().to_string()).unwrap();
        fs::rename(pending, root.join("routed-ready")).unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !root.join("routed-stop").exists() {
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
    }
}

#[test]
fn data_directory_leaf_never_leases_ancestors_outside_data() {
    const ENV: &str = "LEASE_BOUNDARY_ROOT";
    if let Some(root) = std::env::var_os(ENV) {
        let data = std::path::PathBuf::from(root).join("data");
        fs::create_dir_all(&data).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.current_dir(&data).env("HORIZON_HOME", &data);
        let _claims = shared(&command).unwrap();
        let registry = horizon_mod_package::runtime_leases::Registry::existing(&data).unwrap();
        let parent = directory(data.parent().unwrap());
        assert!(
            matches!(registry.lock(&parent, LockMode::Exclusive), Err(horizon_mod_package::runtime_leases::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound)
        );
        return;
    }
    let root = tempfile::tempdir().unwrap();
    assert!(Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "run_leases::tests::data_directory_leaf_never_leases_ancestors_outside_data"
        ])
        .env(ENV, root.path())
        .env("HORIZON_LAUNCHER_HOME", root.path().join("data"))
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("HORIZON_NO_AUDIO", "1")
        .status()
        .unwrap()
        .success());
}
