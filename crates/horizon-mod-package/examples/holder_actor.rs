//! Standalone receiver actor; no libtest threads before private bootstrap.
use horizon_mod_package::holder;
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
#[path = "holder_actor/scene.rs"]
mod scene;
#[cfg(unix)]
extern "C" fn delayed_exit() {
    std::fs::write(std::env::var_os("EXIT_READY").unwrap(), b"atexit").unwrap();
    let stop = PathBuf::from(std::env::var_os("EXIT_STOP").unwrap());
    let end = Instant::now() + Duration::from_secs(5);
    while !stop.is_file() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn main() {
    #[cfg(unix)]
    if std::env::var_os("EXIT_READY").is_some() {
        assert_eq!(unsafe { libc::atexit(delayed_exit) }, 0);
    }
    match std::env::var("HOLDER_TEST_MODE").as_deref() {
        Ok("direct") => {
            let _ = bootstrap();
            let ready = PathBuf::from(std::env::var_os("READY").unwrap());
            std::fs::write(&ready, b"direct ready").unwrap();
            let stop = PathBuf::from(std::env::var_os("STOP").unwrap());
            scene::until(|| stop.is_file());
            return;
        }
        Ok("copy-image") => {
            let destination = std::env::var_os("HOLDER_TEST_IMAGE").unwrap();
            std::fs::copy(std::env::current_exe().unwrap(), destination).unwrap();
            return;
        }
        Ok("gui-capture") => {
            let root = PathBuf::from(std::env::var_os("HOLDER_TEST_ROOT").unwrap());
            let holder::Started::Holder { child } = scene::start(&root, "capture") else {
                panic!("missing capability")
            };
            std::fs::write(root.join("holder-pid"), child.id().to_string()).unwrap();
            let _ = std::io::stdin().read_exact(&mut [0]);
            return;
        }
        Ok("capture") => {
            #[cfg(unix)]
            if let Ok(mask) = std::env::var("HOLDER_TEST_CLOSED_MASK") {
                for fd in 0..3 {
                    if mask.parse::<u8>().unwrap() & (1 << fd) != 0 {
                        // SAFETY: isolated test actor closes its own stdio before any bootstrap IO.
                        unsafe {
                            libc::close(fd);
                        }
                    }
                }
            }
            let (_capture, launch) = bootstrap();
            let ready = PathBuf::from(std::env::var_os("READY").unwrap());
            let root = ready.parent().unwrap();
            std::env::set_current_dir(root).unwrap(); // Emulate a native hook changing cwd after bootstrap.
            assert_eq!(launch.context.game_dir, root.join("world"));
            assert_eq!(launch.context.home, root.join("world"));
            std::fs::write(&ready, b"capture ready").unwrap();
            scene::until(|| {
                if root.join("emit").is_file() && !root.join("emitted").is_file() {
                    println!("stdout after both supervisors died");
                    eprintln!("stderr after both supervisors died");
                    std::io::stdout().flush().unwrap();
                    std::io::stderr().flush().unwrap();
                    std::fs::write(root.join("emitted"), b"done").unwrap();
                }
                root.join("stop").is_file()
            });
            return;
        }
        Ok("gui") => {
            let root = PathBuf::from(std::env::var_os("HOLDER_TEST_ROOT").unwrap());
            drop(scene::start(&root, "file-stop"));
            return;
        }
        #[cfg(target_os = "linux")]
        Ok("closefrom") => {
            use std::os::unix::process::CommandExt;
            // SAFETY: isolated wrapper owns its descriptors; no library worker has been started.
            assert_eq!(
                unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0) },
                0
            );
            let error = std::process::Command::new(std::env::current_exe().unwrap())
                .args(std::env::args_os().skip(1))
                .env_remove("HOLDER_TEST_MODE")
                .exec();
            panic!("closefrom wrapper exec: {error}");
        }
        Ok("unreapable") => {
            std::io::stdin().read_exact(&mut [0]).unwrap();
            return;
        }
        Ok("late") => std::thread::sleep(Duration::from_secs(4)),
        Ok("wrong-ack" | "stale-ack") => {
            use horizon_mod_package::local_control::{Channel, Marker, connect};
            let marker: Marker =
                serde_json::from_str(&std::env::var(holder::ENV).unwrap()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            let channel = Channel::authenticate(
                connect(&marker, deadline).unwrap(),
                &marker,
                false,
                deadline,
            )
            .unwrap();
            let (channel, request) = channel.receive::<serde_json::Value>().unwrap();
            if std::env::var("HOLDER_TEST_MODE").unwrap() == "stale-ack" {
                let entry: holder::Entry =
                    serde_json::from_value(request["entry"].clone()).unwrap();
                let _claims = entry.acquire().unwrap();
                let complete = std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join(".installed");
                std::fs::rename(&complete, complete.with_extension("old")).unwrap();
                std::fs::write(&complete, b"").unwrap();
                let _ = channel
                    .send(&"Acquired")
                    .unwrap()
                    .receive::<serde_json::Value>();
            } else {
                drop(channel.send(&"Commit").unwrap());
            }
            return;
        }
        _ => {}
    }
    holder::enter();
    std::hint::black_box(holder::CAPABILITY);
    assert!(std::env::var_os(holder::ENV).is_none());
    let entry: serde_json::Value =
        serde_json::from_str(&std::env::var(holder::ENTRY).unwrap()).unwrap();
    assert!(entry.get("program").is_none());
    let context = horizon_mod_package::selection::ModSelectionContext {
        game_dir: std::env::current_dir().unwrap(),
        home: std::env::current_dir().unwrap(),
        dev_mods: vec![],
    };
    Box::leak(Box::new(holder::acquire_entry(&context).unwrap().unwrap()));
    assert!(std::env::var_os(holder::ENTRY).is_none());
    let ready = PathBuf::from(std::env::var_os("READY").unwrap());
    let partial = ready.with_extension("partial");
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    std::fs::write(&partial, serde_json::to_vec(&arguments).unwrap()).unwrap();
    std::fs::rename(partial, ready).unwrap();
    if std::env::var("HOLDER_TEST_MODE").as_deref() == Ok("file-stop") {
        let stop = PathBuf::from(std::env::var_os("STOP").unwrap());
        scene::until(|| stop.is_file());
    } else {
        let _ = std::io::stdin().read_exact(&mut [0]);
    }
    std::io::stdout().flush().unwrap();
}

fn bootstrap() -> (
    Option<horizon_mod_package::output::Capture>,
    horizon_mod_package::selection::LaunchArguments,
) {
    holder::enter();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args(std::env::args_os().skip(1));
    let selection = horizon_mod_package::selection::LaunchArguments::for_command(&command).unwrap();
    (holder::acquire(&selection), selection)
}
