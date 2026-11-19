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
    horizon_platform::mod_package::holder::enter();
    let Some(root) = std::env::var_os("ROUTING_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    if std::env::var("ROUTING_MODE").as_deref() == Ok("launcher") {
        let mut game = Command::new(root.join("install/actor"));
        game.args(["--exact", "run_leases::tests::routing_actor"])
            .env("ROUTING_MODE", "game")
            .current_dir(root.join("instances/player"));
        let mut game = crate::launch::wrap(game, &crate::launch::Launch::default()).unwrap();
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
        horizon_platform::mod_package::holder::enter();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(std::env::args_os().skip(1));
        let selection =
            horizon_platform::mod_package::selection::LaunchArguments::for_command(&command)
                .unwrap();
        let _capture = horizon_platform::mod_package::holder::acquire(&selection);
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
        // Registry entry consumes an authoritative path; shared() resolves the same root.
        let data = data.canonicalize().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.current_dir(&data).env("HORIZON_HOME", &data);
        let _claims = shared(&command).unwrap();
        let registry =
            horizon_platform::mod_package::runtime_leases::Registry::existing(&data).unwrap();
        let parent = directory(data.parent().unwrap());
        assert!(
            matches!(registry.lock(&parent, LockMode::Exclusive), Err(horizon_platform::mod_package::runtime_leases::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound)
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

#[test]
fn a_java_editions_command_is_not_read_as_horizons() {
    // Minecraft: Java Edition's command, as `backend::java_game` gives it (#17950): Horizon's
    // option grammar pairs it up and finds `release` with no value.
    let root = tempfile::tempdir().unwrap();
    let mut java = Command::new("java");
    java.args([
        "-Xmx2G",
        "-Djava.library.path=natives",
        "-cp",
        "client.jar",
        "net.minecraft.client.main.Main",
        "--username",
        "Steve",
        "--versionType",
        "release",
    ])
    .current_dir(root.path())
    .env("HORIZON_HOME", root.path().join("logs"));
    let java = crate::launch::wrap(java, &crate::launch::Launch::default()).unwrap();
    shared(&java)
        .map_err(crate::native_mods::cache_error)
        .unwrap();
    assert!(override_value(&java, GAME).is_none());
    // Horizon's own game keeps the command its leases are read from.
    let mut game = Command::new(root.path().join("horizon-client"));
    game.current_dir(root.path());
    mark(&mut game, root.path());
    let game = crate::launch::wrap(game, &crate::launch::Launch::default()).unwrap();
    assert!(override_value(&game, GAME).is_some());
}

/// A Java Edition command started from `java`, in the instance folder `home`.
fn java_edition(java: &Path, home: &Path, logs: &Path) -> Command {
    let mut command = Command::new(java);
    command
        .args(["-cp", "client.jar", "net.minecraft.client.main.Main"])
        .current_dir(home)
        .env("HORIZON_HOME", logs);
    crate::launch::wrap(command, &crate::launch::Launch::default()).unwrap()
}

#[test]
fn a_java_installed_outside_the_launcher_is_left_unmarked() {
    // A Java the player installed (Program Files, /usr/lib/jvm) refuses the launcher's stamp:
    // Play failed with "Could not access the native build cache: Access is denied" (#18018).
    let root = tempfile::tempdir().unwrap();
    let java_home = root.path().join("jdk");
    let java = java_home
        .join("bin")
        .join(if cfg!(windows) { "javaw.exe" } else { "java" });
    fs::create_dir_all(java.parent().unwrap()).unwrap();
    fs::write(&java, b"").unwrap();
    let home = root.path().join("instance");
    fs::create_dir_all(&home).unwrap();
    let _claims = shared(&java_edition(&java, &home, &root.path().join("logs")))
        .map_err(crate::native_mods::cache_error)
        .unwrap();
    for folder in [&java_home, &java_home.join("bin")] {
        assert!(
            !folder.join(".last-used").exists(),
            "stamped {}",
            folder.display()
        );
    }
}

#[test]
fn a_java_edition_launch_starts_when_its_folders_cannot_be_marked() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("instance");
    // Opening a folder as the stamp file is refused on every system, as an administrator too.
    fs::create_dir_all(home.join(".last-used")).unwrap();
    let logs = root.path().join("logs");
    let (_command, guard) =
        crate::backend::prepare_command(java_edition(Path::new("java"), &home, &logs)).unwrap();
    let shown = crate::backend::plain(home.canonicalize().unwrap())
        .display()
        .to_string();
    let warning = guard.warning.as_deref().unwrap();
    assert!(warning.contains(&shown), "{warning}");
    // Horizon's own game still stops on it, and the message names the folder.
    let source = root.path().join("game");
    fs::create_dir_all(&source).unwrap();
    let mut game = Command::new(source.join("horizon-client"));
    game.current_dir(&home).env("HORIZON_HOME", &logs);
    mark(&mut game, &source);
    let Err(error) = shared(&game).map_err(crate::native_mods::cache_error) else {
        panic!("Horizon's game started with its folder unmarked");
    };
    assert!(error.contains(&shown), "{error}");
}

/// Horizon's game command as the launcher builds it: `--resources` and the session file last.
fn horizon_game(root: &Path) -> Command {
    let mut game = Command::new(root.join("game"));
    game.arg("--resources")
        .arg(root.join("resources"))
        .arg("--session-file")
        .arg(root.join("sessions/session-1.json"))
        .current_dir(root);
    mark(&mut game, &root.join("versions/native-fixture"));
    game
}

/// `game` run through `wrapper`'s words, as [`crate::launch::wrap`] builds a direct plan.
fn wrapped(mut game: Command, wrapper: &[&str]) -> Command {
    snapshot(&mut game);
    let mut out = Command::new(wrapper[0]);
    out.args(&wrapper[1..])
        .arg(game.get_program())
        .args(game.get_args())
        .current_dir(game.get_current_dir().unwrap());
    for (key, value) in game.get_envs() {
        out.env(key, value.unwrap());
    }
    out
}

#[test]
fn wrapped_game_holds_its_resources_with_any_number_of_wrapper_words() {
    let root = tempfile::tempdir().unwrap();
    for wrapper in [&["gamemoderun"][..], &["env", "A=1"], &["nice", "-n", "5"]] {
        let paths = argument_paths(&wrapped(horizon_game(root.path()), wrapper))
            .unwrap_or_else(|error| panic!("{wrapper:?}: {error}"));
        assert!(
            paths.contains(&root.path().join("resources")),
            "{wrapper:?}"
        );
    }
}

#[test]
fn unwrapped_game_holds_its_resources() {
    let root = tempfile::tempdir().unwrap();
    let mut game = horizon_game(root.path());
    snapshot(&mut game);
    let paths = argument_paths(&game).unwrap();
    assert!(paths.contains(&root.path().join("resources")));
    assert!(paths.contains(&std::path::absolute(root.path()).unwrap()));
}

#[test]
fn command_without_snapshot_reads_resources_from_its_own_words() {
    let root = tempfile::tempdir().unwrap();
    let mut native = Command::new(root.path().join("client"));
    native
        .arg("--resources")
        .arg(root.path().join("resources"))
        .current_dir(root.path());
    assert_eq!(
        argument_paths(&native).unwrap(),
        [root.path().join("resources")]
    );
    let mut tool = Command::new(root.path().join("tool"));
    tool.args(["--odd"]).current_dir(root.path());
    assert!(argument_paths(&tool).unwrap().is_empty());
}

#[test]
fn wrapped_game_starts_and_holds_the_wrappers_resources() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("instance");
    let source = root.path().join("versions/a");
    let wrapper_data = root.path().join("versions/b/resources");
    for folder in [
        &home,
        &source,
        &root.path().join("resources"),
        &wrapper_data,
    ] {
        fs::create_dir_all(folder).unwrap();
    }
    let game = || {
        let mut game = horizon_game(root.path());
        game.current_dir(&home)
            .env("HORIZON_HOME", root.path().join("logs"));
        mark(&mut game, &source);
        game
    };
    // One wrapper word leaves the session file without a value when the whole command is read
    // as the game's; the second wrapper names version B's data by a path relative to the instance.
    let odd = wrapped(game(), &["gamemoderun"]);
    drop(shared(&odd).unwrap());
    let relative = Path::new("..").join("versions/b/resources");
    let own = wrapped(
        game(),
        &["wrapper", "--resources", relative.to_str().unwrap(), "--"],
    );
    let claims = shared(&own).unwrap();
    let registry = Registry::open().unwrap();
    assert!(matches!(
        registry.lock(&directory(&wrapper_data), LockMode::Exclusive),
        Err(Error::Busy)
    ));
    drop(claims);
}
