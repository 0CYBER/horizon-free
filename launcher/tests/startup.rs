//! Full launcher startup against instrumented Windows hardware probes.
#![cfg(windows)]

mod support;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Output, Stdio},
    time::{Duration, Instant},
};

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("horizon-startup-{}", std::process::id()));
    for folder in ["game/telemetry", "launcher", "data", "config", "probes"] {
        fs::create_dir_all(root.join(folder)).unwrap();
    }
    fs::write(root.join("launcher/settings.json"), r#"{"skip_opening":true,"install_new_versions":false,"desktop_entry":false,"ping_notice_seen":true}"#).unwrap();
    let consent = serde_json::json!({"version":horizon_telemetry::CONSENT_VERSION,"choices":{}});
    fs::write(
        root.join("game/telemetry/consent.json"),
        consent.to_string(),
    )
    .unwrap();
    fs::copy(support::console_probe(), root.join("probes/powershell.exe")).unwrap();
    root
}

fn launch(root: &Path) -> Child {
    let inherited_path = std::env::var_os("PATH").unwrap();
    let paths = std::iter::once(root.join("probes")).chain(std::env::split_paths(&inherited_path));
    horizon_launcher::process::command(env!("CARGO_BIN_EXE_horizon-launcher"))
        .current_dir(root)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HORIZON_HOME", root.join("game"))
        .env("HORIZON_LAUNCHER_HOME", root.join("launcher"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .env("HORIZON_NO_AUDIO", "1")
        .env("HORIZON_TELEMETRY_URL", "off")
        .env("HORIZON_STARTUP_PROBE_LOG", root.join("probes.log"))
        .env("HORIZON_LAUNCHER_BENCH", root.join("timing.json"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_exit(mut child: Child, root: &Path) -> Output {
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("launcher startup timed out: {}", root.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

#[test]
#[ignore = "requires an interactive Windows desktop and graphics adapter"]
fn hardware_probes_run_hidden_after_the_first_frame() {
    let root = fixture();
    let output = wait_for_exit(launch(&root), &root);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let timing: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("timing.json")).unwrap()).unwrap();
    let first_frame = timing["first_frame_epoch_ms"].as_f64().unwrap().floor();
    let probes = fs::read_to_string(root.join("probes.log")).unwrap();
    // Rust resolves cmd.exe to the system directory, so instrument the seven PowerShell
    // probes, which exercise both shared helpers (the cmd probes use those same helpers).
    const POWERSHELL_PROBES: usize = 7;
    assert_eq!(probes.lines().count(), POWERSHELL_PROBES, "{probes}");
    for probe in probes.lines() {
        let fields: Vec<_> = probe.split('|').collect();
        assert!(
            fields[0].parse::<f64>().unwrap() >= first_frame,
            "probe blocked startup: {probe}"
        );
        assert_eq!(fields[1], "false", "probe opened a terminal: {probe}");
    }
}
