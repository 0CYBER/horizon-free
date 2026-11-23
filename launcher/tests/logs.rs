//! `horizon-launcher logs`: the end of the game's latest.log, and its newest crash report.
use std::path::Path;
use std::process::Command;

fn logs(root: &Path, args: &[&str]) -> (bool, String, String) {
    // Every folder in scratch: startup migrates the launcher's settings before `logs` runs.
    let out = Command::new(env!("CARGO_BIN_EXE_horizon-launcher"))
        .arg("logs")
        .args(args)
        .env("HORIZON_HOME", root.join("game"))
        .env("HORIZON_LAUNCHER_HOME", root.join("launcher"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HORIZON_LAUNCHER_TRASH", root.join("trash"))
        .env("HORIZON_LAUNCHER_CLIPBOARD", root.join("clipboard"))
        .env("HORIZON_LAUNCHER_DESKTOP", root.join("desktop"))
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("HORIZON_NO_AUDIO", "1")
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn logs_prints_the_log_tail_and_the_newest_crash_report() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("game");

    let (ok, _, err) = logs(root.path(), &[]);
    assert!(!ok);
    assert!(err.contains("latest.log"), "{err}");

    std::fs::create_dir_all(home.join("logs")).unwrap();
    let text: String = (1..=150)
        .map(|i| format!("[10:00:00] [Render thread/INFO]: line {i}\n"))
        .collect();
    std::fs::write(home.join("logs/latest.log"), text).unwrap();
    let (ok, out, _) = logs(root.path(), &[]);
    assert!(ok);
    assert_eq!(out.lines().count(), 100);
    assert!(out.starts_with("[10:00:00] [Render thread/INFO]: line 51\n"));
    let (_, out, _) = logs(root.path(), &["--lines", "1"]);
    assert_eq!(out, "[10:00:00] [Render thread/INFO]: line 150\n");
    let (_, out, _) = logs(root.path(), &["--json", "--lines", "1"]);
    let json: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(json["text"], "[10:00:00] [Render thread/INFO]: line 150\n");
    assert!(json["path"].as_str().unwrap().ends_with("latest.log"));

    let (ok, _, err) = logs(root.path(), &["--crash"]);
    assert!(!ok);
    assert!(err.contains("No crash report"), "{err}");
    std::fs::create_dir_all(home.join("crash-reports")).unwrap();
    std::fs::write(
        home.join("crash-reports/crash-2026-10-13_08.00.00-client.txt"),
        "old\n",
    )
    .unwrap();
    std::fs::write(
        home.join("crash-reports/crash-2026-10-14_08.00.00-client.txt"),
        "new\n",
    )
    .unwrap();
    let (ok, out, _) = logs(root.path(), &["--crash"]);
    assert!(ok);
    assert_eq!(out, "new\n");
}
