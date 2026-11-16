use super::*;
use crate::{local_control::Endpoint, locks::Release};
use std::{
    fs,
    io::Write,
    process::{Child, Stdio},
};
struct Scene {
    _root: tempfile::TempDir,
    marker: Marker,
    request: Request,
    claims: Vec<Claim>,
    snapshots: Vec<Snapshot>,
    _record: Option<File>,
}
/// How long a test waits for a spawned actor to connect or reach a phase. Spawning and starting a process on a loaded
/// CI runner can take seconds, and only a failing test waits this long, so it is a ceiling, not a timing check. The
/// one test of a launch timeout passes its own short deadline.
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(60)
}
pub(super) fn actor() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join(format!("holder_actor{}", std::env::consts::EXE_SUFFIX))
}
impl Scene {
    fn new(native: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        let name = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(vec![110, 255])
        };
        #[cfg(windows)]
        let name = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[110, 0xd800])
        };
        let path = root.path().canonicalize().unwrap().join(&name);
        #[cfg(unix)]
        let path = path.join("long-launcher-home-".repeat(8));
        fs::create_dir_all(&path).unwrap();
        let data = path.join("data");
        let registry = Registry::with_release(&data, Release::Close).unwrap();
        let snapshots: Vec<_> = [("image", !native), ("world", false), ("sdk", true)]
            .into_iter()
            .map(|(name, complete)| {
                let directory = path.join(name);
                fs::create_dir(&directory).unwrap();
                if complete {
                    fs::write(directory.join(".installed"), b"").unwrap();
                }
                Snapshot::capture(&directory, complete).unwrap()
            })
            .collect();
        let image = path.join("image").join(actor().file_name().unwrap());
        // A concurrent fork can retain fs::copy's writable destination descriptor until
        // exec, making our immediate exec fail with ETXTBSY even after copy returns.
        // Keep that descriptor in a single-threaded actor, never in the libtest process.
        assert!(
            Command::new(actor())
                .env("HOLDER_TEST_MODE", "copy-image")
                .env("HOLDER_TEST_IMAGE", &image)
                .status()
                .unwrap()
                .success()
        );
        let claims = snapshots
            .iter()
            .map(|snapshot| registry.lock(&snapshot.directory, LockMode::Shared))
            .collect::<Result<_, _>>()
            .unwrap();
        let entry = Entry {
            version: crate::local_control::VERSION,
            data,
            trees: snapshots
                .iter()
                .map(|snapshot| snapshot.tree.clone())
                .collect(),
        };
        let program = Program {
            executable: image.into_os_string(),
            arguments: vec!["".into(), "alpha beta".into(), name],
            environment: vec![
                (ENV.into(), "must-not-recurse".into()),
                ("READY".into(), path.join("ready").into_os_string()),
            ],
            directory: path.join("world"),
            raw_windows: None,
        };
        let selection = crate::selection::ModSelectionContext {
            game_dir: program.directory.clone(),
            home: program.directory.clone(),
            dev_mods: vec![],
        };
        let (record, file) = record::Reference::publish(&snapshots[1], &entry, &selection).unwrap();
        Self {
            _root: root,
            marker: {
                #[cfg(unix)]
                {
                    Marker::runtime().unwrap()
                }
                #[cfg(windows)]
                {
                    Marker::new(path.join("endpoint")).unwrap()
                }
            },
            request: Request {
                entry,
                program,
                record,
            },
            claims,
            snapshots,
            _record: Some(file),
        }
    }
    fn ready(&self) -> PathBuf {
        PathBuf::from(&self.request.program.environment[1].1)
    }
    fn begin(&self, global: bool, delay_exit: bool) -> (Endpoint, Child, io::Result<Channel>) {
        let mut endpoint = Endpoint::bind(&self.marker).unwrap();
        let image = if global {
            actor().into_os_string()
        } else {
            self.request.program.executable.clone()
        };
        let mut command = Command::new(image);
        command
            .env(ENV, serde_json::to_string(&self.marker).unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if delay_exit {
            command
                .env("EXIT_READY", self.ready().with_extension("exit"))
                .env("EXIT_STOP", self.ready().with_extension("stop"));
        }
        let child = command.spawn().unwrap();
        let result = endpoint
            .accept(deadline())
            .and_then(|stream| Channel::authenticate(stream, &self.marker, true, deadline()))
            .and_then(|channel| channel.send(&self.request))
            .and_then(|channel| {
                let (channel, reply) = channel.receive::<Reply>()?;
                assert!(matches!(reply, Reply::Acquired));
                Ok(channel)
            });
        (endpoint, child, result)
    }
    fn release(&mut self) {
        self.claims.clear();
        self.snapshots.clear();
    }
}
fn ready(path: &Path) {
    let end = deadline();
    while !path.is_file() {
        assert!(Instant::now() < end, "actor did not reach expected phase");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn prepared(scene: &Scene) -> Prepared {
    let selection = crate::selection::ModSelectionContext {
        game_dir: scene.request.program.directory.clone(),
        home: scene.request.program.directory.clone(),
        dev_mods: vec![],
    };
    Prepared::new(
        scene.request.entry.data.clone(),
        scene
            .request
            .entry
            .trees
            .iter()
            .map(|tree| (tree.path.clone(), tree.complete.is_some()))
            .collect(),
        scene.request.program.clone(),
        &selection,
    )
    .unwrap()
}
#[cfg(windows)]
#[test]
fn windows_controller_quietly_declines_managed_mode_without_spawning() {
    let scene = Scene::new(false);
    let prepared = prepared(&scene);
    let Started::Legacy(command) = launch(
        Command::new("must-not-spawn"),
        &scene.marker,
        &prepared,
        deadline(),
    )
    .unwrap() else {
        panic!("Windows managed mode was armed")
    };
    assert_eq!(command.get_program(), &scene.request.program.executable);
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        scene
            .request
            .program
            .arguments
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<_>>()
    );
    assert!(
        !scene
            .request
            .program
            .directory
            .parent()
            .unwrap()
            .join("endpoint")
            .exists()
    );
    assert!(!scene.ready().exists());
}
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn controller_checks_capability_before_spawn_and_refuses_stale_or_wrong_ack() {
    for mode in ["legacy", "wrong-ack", "stale-ack", "late"] {
        let scene = Scene::new(false);
        let prepared = prepared(&scene);
        let image = if mode == "legacy" {
            let image = scene.request.program.directory.join("legacy");
            fs::write(&image, b"not executable, no runtime capability").unwrap();
            image
        } else {
            scene.request.program.executable.clone().into()
        };
        let mut command = Command::new(image);
        command
            .env("HOLDER_TEST_MODE", mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let started = Instant::now();
        let result = launch(
            command,
            &scene.marker,
            &prepared,
            started + Duration::from_secs(1),
        );
        if mode == "legacy" {
            assert!(matches!(result, Ok(Started::Legacy(_))));
            assert!(
                !scene
                    .request
                    .program
                    .directory
                    .parent()
                    .unwrap()
                    .join("endpoint")
                    .exists()
            );
        } else {
            assert!(
                matches!(result, Err(error) if error.kind() == if mode == "late" { io::ErrorKind::TimedOut } else { io::ErrorKind::InvalidData })
            );
            assert!(started.elapsed() < Duration::from_millis(2500));
            assert!(!scene.ready().exists());
        }
    }
}
#[test]
#[ignore = "daily"]
fn failed_kill_hands_unreaped_child_off_without_exceeding_launch_budget() {
    use std::sync::mpsc;
    for budget in [Duration::ZERO, Duration::from_millis(40)] {
        let mut child = Command::new(actor())
            .env("HOLDER_TEST_MODE", "unreapable")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut release = child.stdin.take().unwrap();
        assert!(child.try_wait().unwrap().is_none());
        let (returned, prompt) = mpsc::channel();
        let (owned, background) = mpsc::channel();
        let (reaped, finished) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            controller::retire(
                child,
                Instant::now() + budget,
                |_| Err(io::ErrorKind::PermissionDenied.into()),
                move |mut child| {
                    owned.send(()).unwrap();
                    let status = child.wait().unwrap();
                    reaped.send(status.success()).unwrap();
                },
            );
            returned.send(()).unwrap();
        });
        let bounded = prompt.recv_timeout(Duration::from_millis(300)).is_ok();
        background.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            finished.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        // Release even when the deadline assertion will fail, so mutants do not strand actors.
        release.write_all(b"x").unwrap();
        assert!(finished.recv_timeout(Duration::from_secs(3)).unwrap());
        caller.join().unwrap();
        assert!(bounded, "unreaped child blocked launch after its deadline");
    }
}
#[test]
#[ignore = "daily"]
fn failed_reaper_spawn_retains_ownership_without_waiting_or_panicking() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Owner(Arc<AtomicBool>);
    impl Drop for Owner {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(AtomicBool::new(false));
    let waited = Arc::new(AtomicBool::new(false));
    let observed = waited.clone();
    let start = Instant::now();
    controller::handoff(
        Owner(dropped.clone()),
        move |_| {
            observed.store(true, Ordering::SeqCst);
        },
        |job| {
            drop(job); // Builder::spawn consumes/drops the closure even when creation fails.
            Err(io::ErrorKind::WouldBlock.into())
        },
    );
    assert!(start.elapsed() < Duration::from_millis(300));
    assert!(
        !waited.load(Ordering::SeqCst),
        "failed spawn fell back to inline wait"
    );
    assert!(
        !dropped.load(Ordering::SeqCst),
        "unreaped owner was silently dropped"
    );
}
/// Used by the Unix-only holder tests below.
#[cfg(unix)]
fn assert_record_owned(data: &Path, expected: bool) {
    let record = fs::read_dir(data.join("run-records"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let payload: serde_json::Value =
        serde_json::from_slice(&fs::read(record.join("launch.json")).unwrap()).unwrap();
    let entry: Entry = serde_json::from_value(payload["entry"].clone()).unwrap();
    let registry = Registry::existing(data).unwrap();
    for tree in entry.trees {
        assert_eq!(
            matches!(
                registry.lock(&tree.recheck().unwrap(), LockMode::Exclusive),
                Err(crate::runtime_leases::Error::Busy)
            ),
            expected,
            "a selected tree or record lost ownership"
        );
    }
}
/// Used by the Unix-only holder tests below.
#[cfg(unix)]
fn released_record(data: &Path) {
    let end = deadline();
    loop {
        let record = fs::read_dir(data.join("run-records"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let registry = Registry::existing(data).unwrap();
        if registry
            .lock(&open_directory(&record).unwrap(), LockMode::Exclusive)
            .is_ok()
        {
            return;
        }
        assert!(Instant::now() < end, "game retained record after exit");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(unix)]
#[test]
#[ignore = "daily"]
fn committed_game_retains_independent_record_after_gui_process_exit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap();
    assert!(
        Command::new(actor())
            .env("HOLDER_TEST_MODE", "gui")
            .env("HOLDER_TEST_ROOT", &path)
            .status()
            .unwrap()
            .success()
    );
    ready(&path.join("ready"));
    assert_record_owned(&path.join("data"), true);
    fs::write(path.join("stop"), b"stop").unwrap();
    released_record(&path.join("data"));
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "daily"]
fn independent_game_retains_record_after_holder_death_even_through_closefrom_wrapper() {
    for mode in ["stdin", "closefrom"] {
        let mut scene = Scene::new(false);
        scene
            .request
            .program
            .environment
            .push(("HOLDER_TEST_MODE".into(), mode.into()));
        let prepared = prepared(&scene);
        let mut command = Command::new(&scene.request.program.executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let Started::Holder { mut child, .. } =
            launch(command, &scene.marker, &prepared, deadline()).unwrap()
        else {
            panic!("missing capability")
        };
        drop(prepared);
        scene.release();
        ready(&scene.ready());
        let mut input = child.stdin.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert_record_owned(&scene.request.entry.data, true);
        input.write_all(&[1]).unwrap();
        drop(input);
        released_record(&scene.request.entry.data);
    }
}
#[cfg(target_os = "linux")]
#[test]
#[ignore = "daily"]
fn game_owned_record_capture_writes_after_both_supervisors_die() {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::fs::PermissionsExt;
    for mask in 0..8 {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let mut gui = Command::new(actor())
            .env("HOLDER_TEST_MODE", "gui-capture")
            .env("HOLDER_TEST_CLOSED_MASK", mask.to_string())
            .env("HOLDER_TEST_ROOT", &path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        ready(&path.join("ready"));
        ready(&path.join("holder-pid"));
        let pid: i32 = fs::read_to_string(path.join("holder-pid"))
            .unwrap()
            .parse()
            .unwrap();
        // SAFETY: this controlled driver published its live holder; pidfd pins identity before GUI death.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
        assert!(fd >= 0);
        // SAFETY: successful pidfd_open returned a new exclusively owned descriptor.
        let holder = unsafe { OwnedFd::from_raw_fd(fd) };
        gui.kill().unwrap();
        gui.wait().unwrap();
        // SAFETY: owned pidfd targets only this controlled holder, even across PID reuse.
        assert_eq!(
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    holder.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            },
            0
        );
        let data = path.join("data");
        assert_record_owned(&data, true);
        let record = fs::read_dir(data.join("run-records"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::metadata(&record).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(record.join("launch.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::write(path.join("emit"), b"emit").unwrap();
        for name in ["stdout.log", "stderr.log"] {
            let output = record.join(name);
            let end = deadline();
            while !fs::read_to_string(&output)
                .unwrap_or_default()
                .contains("after both supervisors died")
            {
                assert!(
                    Instant::now() < end,
                    "game-owned output stopped after supervisor death"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                fs::metadata(output).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(path.join("stop"), b"stop").unwrap();
        released_record(&data);
    }
}
#[test]
#[ignore = "daily"]
fn private_version_is_refused_before_normal_startup() {
    let status = Command::new(actor())
        .env(ENV, r#"{"version":99}"#)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(78));
}
#[test]
#[ignore = "daily"]
fn malformed_entry_is_noncreating_and_versioned() {
    let scene = Scene::new(false);
    let mut entry = scene.request.entry.clone();
    entry.data = scene._root.path().join("absent");
    assert!(entry.acquire().is_err());
    assert!(!entry.data.exists());
    let mut value = serde_json::to_value(entry).unwrap();
    for (key, value_bad) in [
        ("version", serde_json::json!(99)),
        ("unexpected", serde_json::json!(true)),
    ] {
        let mut bad = value.clone();
        bad[key] = value_bad;
        assert!(serde_json::from_value::<Entry>(bad).is_err());
    }
    value["trees"] = serde_json::json!([]);
    assert!(
        serde_json::from_value::<Entry>(value)
            .unwrap()
            .acquire()
            .is_err()
    );
}
#[test]
#[ignore = "daily"]
fn replaced_tree_completion_or_unreserved_image_never_starts_game() {
    for change in 0..3 {
        let mut scene = Scene::new(false);
        scene.snapshots.clear(); // Model stale GUI descriptions without directory delete-denial.
        if change == 0 {
            drop(scene._record.take()); // Also release the retained child file's Windows delete denial.
            let world = &scene.request.entry.trees[1].path;
            fs::rename(world, world.with_extension("old")).unwrap();
            fs::create_dir(world).unwrap();
            let registry =
                Registry::with_release(&scene.request.entry.data, Release::Close).unwrap();
            drop(
                registry
                    .lock(&open_directory(world).unwrap(), LockMode::Shared)
                    .unwrap(),
            );
        }
        if change == 1 {
            // Keep the old marker alive beside its replacement: a deleted inode may be reused at once
            // (ext4), and these cleared snapshots no longer hold it as the GUI's retained ones do.
            let complete = scene.request.entry.trees[0].path.join(".installed");
            fs::rename(&complete, complete.with_extension("old")).unwrap();
            fs::write(complete, b"").unwrap();
        }
        let (_endpoint, mut child, result) = scene.begin(change == 2, false);
        assert!(result.is_err());
        drop(child.stdin.take());
        assert_eq!(child.wait().unwrap().code(), Some(78));
        assert!(!scene.ready().exists());
    }
}
#[test]
#[ignore = "daily"]
fn eof_wrong_commit_and_postcommit_replacement_refuse_even_wrappers() {
    for change in 0..4 {
        let mut scene = Scene::new(false);
        if change == 3 {
            scene.request.program.raw_windows = Some("must not execute".into());
        }
        let (_endpoint, mut child, result) = scene.begin(false, false);
        let channel = result.unwrap();
        if change == 2 {
            let complete = scene.request.entry.trees[0].path.join(".installed");
            fs::remove_file(&complete).unwrap();
            fs::write(complete, b"").unwrap();
        }
        match change {
            0 => drop(channel),
            1 => drop(channel.send(&Reply::Acquired).unwrap()),
            _ => drop(channel.send(&Reply::Commit).unwrap()),
        }
        drop(child.stdin.take());
        assert_eq!(child.wait().unwrap().code(), Some(78));
        assert!(!scene.ready().exists());
    }
}
#[test]
#[ignore = "daily"]
fn native_paths_arguments_and_scrubbed_metadata_survive_committed_launch() {
    for native in [false, true] {
        let mut scene = Scene::new(native);
        let (_endpoint, mut child, result) = scene.begin(false, false);
        drop(result.unwrap().send(&Reply::Commit).unwrap());
        scene.release();
        ready(&scene.ready());
        let args: Vec<OsString> =
            serde_json::from_slice(&fs::read(scene.ready()).unwrap()).unwrap();
        assert_eq!(args, scene.request.program.arguments);
        child.stdin.take().unwrap().write_all(&[1]).unwrap();
        assert!(child.wait().unwrap().success());
    }
}
#[test]
#[ignore = "daily"]
#[cfg(target_os = "linux")]
fn independently_retained_objects_and_claims_survive_native_exit_callbacks() {
    use std::os::unix::fs::MetadataExt as Unix;
    let mut scene = Scene::new(false);
    let directory = fs::metadata(&scene.request.entry.trees[0].path).unwrap();
    let complete = fs::metadata(scene.request.entry.trees[0].path.join(".installed")).unwrap();
    let (_endpoint, mut child, result) = scene.begin(false, true);
    drop(result.unwrap().send(&Reply::Commit).unwrap());
    scene.release();
    ready(&scene.ready());
    let held: Vec<_> = fs::read_dir(format!("/proc/{}/fd", child.id()))
        .unwrap()
        .filter_map(|entry| fs::metadata(entry.ok()?.path()).ok())
        .map(|meta| (Unix::dev(&meta), Unix::ino(&meta)))
        .collect();
    child.stdin.take().unwrap().write_all(&[1]).unwrap();
    ready(&scene.ready().with_extension("exit"));
    let registry = Registry::existing(&scene.request.entry.data).unwrap();
    let protected = scene.request.entry.trees.iter().all(|tree| {
        matches!(
            registry.lock(&tree.recheck().unwrap(), LockMode::Exclusive),
            Err(crate::runtime_leases::Error::Busy)
        )
    });
    fs::write(scene.ready().with_extension("stop"), b"stop").unwrap();
    assert!(child.wait().unwrap().success());
    assert!(protected, "claims ended before native exit callbacks");
    assert!(held.contains(&(Unix::dev(&directory), Unix::ino(&directory))));
    assert!(held.contains(&(Unix::dev(&complete), Unix::ino(&complete))));
}

#[test]
#[ignore = "daily"]
fn direct_game_owns_install_resources_and_home_until_process_exit() {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().canonicalize().unwrap();
    let install = data.join("install");
    let resources = data.join("resources");
    let home = data.join("home");
    for path in [&install, &resources, &home] {
        fs::create_dir(path).unwrap();
    }
    fs::write(install.join(".installed"), b"").unwrap();
    let registry = Registry::with_release(&data, Release::Unlock).unwrap();
    let directories: Vec<_> = [&install, &resources, &home]
        .into_iter()
        .map(|p| open_directory(p).unwrap())
        .collect();
    for directory in &directories {
        drop(registry.lock(directory, LockMode::Shared).unwrap());
    }
    let image = install.join(actor().file_name().unwrap());
    assert!(
        Command::new(actor())
            .env("HOLDER_TEST_MODE", "copy-image")
            .env("HOLDER_TEST_IMAGE", &image)
            .status()
            .unwrap()
            .success()
    );
    let ready = data.join("ready");
    let stop = data.join("stop");
    let command = || {
        let mut command = Command::new(&image);
        command
            .env("HOLDER_TEST_MODE", "direct")
            .env("READY", &ready)
            .env("STOP", &stop)
            .env("HORIZON_HOME", &home)
            .env_remove(ENTRY)
            .env_remove(ENV)
            .args(["--resources"])
            .arg(&resources)
            .arg("--game-dir")
            .arg(&home)
            .current_dir(&home);
        command
    };
    let exclusive = registry.lock(&directories[0], LockMode::Exclusive).unwrap();
    assert_eq!(command().status().unwrap().code(), Some(78));
    assert!(!ready.exists());
    drop(exclusive);
    let mut child = command().spawn().unwrap();
    let end = deadline();
    while !ready.is_file() {
        // An actor that exits before signalling fails at once, without waiting out the ceiling.
        assert_eq!(
            child.try_wait().unwrap(),
            None,
            "actor exited before it was ready"
        );
        assert!(Instant::now() < end, "actor did not become ready");
        std::thread::sleep(Duration::from_millis(5));
    }
    for directory in &directories {
        assert!(matches!(
            registry.lock(directory, LockMode::Exclusive),
            Err(crate::runtime_leases::Error::Busy)
        ));
    }
    fs::write(&stop, b"").unwrap();
    assert!(child.wait().unwrap().success());
    for directory in &directories {
        drop(registry.lock(directory, LockMode::Exclusive).unwrap());
    }
    fs::remove_file(&ready).unwrap();
    fs::remove_file(install.join(".installed")).unwrap();
    assert_eq!(command().status().unwrap().code(), Some(78));
    assert!(!ready.exists());
}
