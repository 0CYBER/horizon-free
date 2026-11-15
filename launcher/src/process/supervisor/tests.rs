use super::*;
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
static PREVIOUS: AtomicU32 = AtomicU32::new(0);
static MEMBER: AtomicU32 = AtomicU32::new(0);
extern "C" fn previous(_: i32) {
    PREVIOUS.fetch_add(1, Ordering::SeqCst);
}
extern "C" fn member_notice(_: i32) {
    MEMBER.store(1, Ordering::SeqCst);
}
fn wait(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(Instant::now() < deadline, "supervisor rendezvous timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn command(root: &Path, role: &str, case: &str, number: i32) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "process::supervisor::tests::actor",
            "--nocapture",
        ])
        .env("HORIZON_SIGNAL_ROOT", root)
        .env("HORIZON_SIGNAL_ROLE", role)
        .env("HORIZON_SIGNAL_CASE", case)
        .env("HORIZON_SIGNAL_NUMBER", number.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}
#[test]
fn owned_children_forward_exact_signals_preserve_codes_and_restore_actions() {
    for (case, number, expected) in [
        ("normal", 0, 0),
        ("normal", 37, 37),
        ("signal", libc::SIGINT, 130),
        ("signal", libc::SIGTERM, 143),
        ("signal", libc::SIGHUP, 129),
        ("early", libc::SIGTERM, 143),
        ("group", libc::SIGTERM, 143),
        ("reaped", libc::SIGTERM, 37),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut child = command(root.path(), "supervisor", case, number)
            .spawn()
            .unwrap();
        if case == "early" {
            wait(&root.path().join("installed"));
            assert_eq!(unsafe { libc::kill(child.id() as i32, number) }, 0);
            std::fs::write(root.path().join("go"), b"go").unwrap();
        } else if case != "reaped" {
            wait(&root.path().join("ready"));
            if case == "normal" {
                std::fs::write(root.path().join("stop"), b"stop").unwrap();
            } else {
                if case == "group" {
                    wait(&root.path().join("member-ready"));
                }
                assert_eq!(unsafe { libc::kill(child.id() as i32, number) }, 0);
            }
        }
        assert_eq!(child.wait().unwrap().code(), Some(expected), "case {case}");
    }
}
#[test]
fn actor() {
    let Ok(role) = std::env::var("HORIZON_SIGNAL_ROLE") else {
        return;
    };
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_SIGNAL_ROOT").unwrap());
    let case = std::env::var("HORIZON_SIGNAL_CASE").unwrap();
    let number: i32 = std::env::var("HORIZON_SIGNAL_NUMBER")
        .unwrap()
        .parse()
        .unwrap();
    if role == "leader" {
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            unsafe {
                libc::signal(signal, libc::SIG_DFL);
            }
        }
        std::fs::write(root.join("ready"), b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if root.join("stop").exists() {
                std::process::exit(if case == "normal" { number } else { 37 });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        std::process::exit(71); // A lost forward fails by outcome, not an indefinitely hung test.
    }
    if role == "member" {
        unsafe {
            libc::signal(libc::SIGTERM, member_notice as *const () as usize);
        }
        std::fs::write(root.join("member-ready"), b"ready").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !root.join("member-stop").exists()
            && MEMBER.load(Ordering::SeqCst) == 0
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let received = MEMBER.load(Ordering::SeqCst) != 0;
        std::process::exit(if received == (case == "group") { 0 } else { 70 });
    }
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe {
            libc::signal(signal, previous as *const () as usize);
        }
    }
    let forward = Forward::install().unwrap();
    std::fs::write(root.join("installed"), b"installed").unwrap();
    if case == "early" {
        wait(&root.join("go"));
    }
    let mut leader = command(&root, "leader", &case, number)
        .process_group(0)
        .spawn()
        .unwrap();
    let mut member = if case == "group" || case == "reaped" {
        wait(&root.join("ready"));
        let child = command(&root, "member", &case, number)
            .process_group(leader.id() as i32)
            .spawn()
            .unwrap();
        wait(&root.join("member-ready"));
        Some(child)
    } else {
        None
    };
    if case == "reaped" {
        std::fs::write(root.join("stop"), b"stop").unwrap();
        assert_eq!(leader.wait().unwrap().code(), Some(37));
        unsafe {
            libc::raise(libc::SIGTERM);
        }
    }
    let code = forward.wait(&mut leader, true).unwrap();
    if let Some(member) = &mut member {
        std::fs::write(root.join("member-stop"), b"stop").unwrap();
        assert_eq!(
            member.wait().unwrap().code(),
            Some(0),
            "wrong group or post-reap delivery"
        );
    }
    drop(forward);
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe {
            libc::raise(signal);
        }
    }
    assert_eq!(
        PREVIOUS.load(Ordering::SeqCst),
        3,
        "prior signal actions not restored"
    );
    std::process::exit(code);
}
