use super::*;
use std::{fs, sync::Mutex};
static SERIAL: Mutex<()> = Mutex::new(());
fn scratch() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "horizon-modprofile-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
#[test]
#[ignore = "daily"]
fn modprofile_state_and_report_side_effects() {
    // Other game tests may enter process-wide mod guards. Run this service test alone.
    if std::env::var_os("HORIZON_MODPROFILE_TEST_CHILD").is_none() {
        assert!(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "game::commands::modprofile::tests::modprofile_state_and_report_side_effects"
                ])
                .env("HORIZON_MODPROFILE_TEST_CHILD", "1")
                .status()
                .unwrap()
                .success()
        );
        return;
    }
    let dir = scratch();
    assert_eq!(
        run(Action::Stop, &dir).unwrap_err().key,
        "horizon.modprofile.notRunning"
    );
    assert_eq!(
        run(Action::Report, &dir).unwrap_err().key,
        "horizon.modprofile.noReport"
    );
    assert!(!dir.join("profiles").exists());
    // This guard predates profiling, so it holds no active profiler reservation.
    // Omitting the reentrant check then fails an assertion, rather than hanging
    // while a stop waits for its own profiled guard.
    let unprofiled_guard =
        mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
            id: "preexisting_call",
            hook: "tick",
            part: None,
        });
    assert_eq!(
        run(Action::Start, &dir).unwrap()[0].translation_key(),
        Some("horizon.modprofile.started")
    );
    assert!(profiler::is_enabled());
    let reentrant = run(Action::Stop, &dir);
    assert!(
        reentrant.is_err(),
        "stop must be refused inside a native callback"
    );
    assert_eq!(reentrant.unwrap_err().key, "horizon.modprofile.inHook");
    drop(unprofiled_guard);
    assert!(profiler::is_enabled());
    assert_eq!(
        run(Action::Start, &dir).unwrap_err().key,
        "horizon.modprofile.alreadyRunning"
    );
    assert!(run(Action::Report, &dir).is_err());
    assert!(profiler::is_enabled());
    assert!(!dir.join("profiles").exists());
    let guard = mod_platform::content::crash::enter_native_mod(mod_api::content::crash::ModCall {
        id: "profile_test",
        hook: "tick",
        part: None,
    });
    assert_eq!(
        run(Action::Stop, &dir).unwrap_err().key,
        "horizon.modprofile.inHook"
    );
    drop(guard);
    assert!(run(Action::Stop, &dir).is_ok());
    assert!(!profiler::is_enabled());
    let paths: Vec<_> = fs::read_dir(dir.join("profiles"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 1);
    let bytes = fs::read(&paths[0]).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["meta"]["version"], 37);
    assert_eq!(
        json["threads"][0]["markers"]["data"][0][5]["mod"],
        "profile_test"
    );
    let report = run(Action::Report, &dir).unwrap();
    assert!(report.iter().any(|t| t.plain().contains("profile_test")));
    assert_eq!(fs::read_dir(dir.join("profiles")).unwrap().count(), 1);
    assert_eq!(fs::read(&paths[0]).unwrap(), bytes);
    assert_eq!(
        run(Action::Stop, &dir).unwrap_err().key,
        "horizon.modprofile.notRunning"
    );
    assert!(run(Action::Start, &dir).is_ok());
    assert!(run(Action::Report, &dir).is_ok());
    assert!(profiler::is_enabled());
    assert_eq!(fs::read_dir(dir.join("profiles")).unwrap().count(), 1);
    // A write failure must be reported, while leaving the stopped report available.
    fs::rename(dir.join("profiles"), dir.join("saved")).unwrap();
    fs::write(dir.join("profiles"), b"not a directory").unwrap();
    assert_eq!(
        run(Action::Stop, &dir).unwrap_err().key,
        "horizon.modprofile.writeFailed"
    );
    assert!(!profiler::is_enabled());
    assert!(run(Action::Report, &dir).is_ok());
    fs::remove_dir_all(dir).unwrap();
}
#[test]
fn modprofile_atomic_file_and_fixed_bytes() {
    let _serial = SERIAL.lock().unwrap();
    let dir = scratch();
    let bytes = br#"{"meta":{"version":37},"threads":[]}"#;
    let first = write_profile(&dir, 123, bytes).unwrap();
    assert_eq!(fs::read(dir.join(&first)).unwrap(), bytes);
    let parsed: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join(&first)).unwrap()).unwrap();
    assert_eq!(parsed["meta"]["version"], 37);
    let second = write_profile(&dir, 123, bytes).unwrap();
    assert_ne!(
        first, second,
        "sessions at one timestamp must not overwrite"
    );
    // Force failure before rename: durable writes must leave an existing target intact.
    let next = NEXT.load(Ordering::Relaxed);
    let target = dir
        .join("profiles")
        .join(format!("modprofile-456-{next}.json"));
    fs::write(&target, b"old complete profile").unwrap();
    fs::create_dir(target.with_file_name(format!("modprofile-456-{next}.json_new"))).unwrap();
    assert!(write_profile(&dir, 456, bytes).is_err());
    assert_eq!(fs::read(target).unwrap(), b"old complete profile");
    fs::remove_dir_all(dir).unwrap();
}
