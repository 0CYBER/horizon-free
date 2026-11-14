//! Real launcher startup: constrained entry stacks and instrumented Windows hardware probes.
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

/// Constrain the product's entry thread, not the test runner or the startup worker.
/// PE optional headers use the same stack-field offset in PE32 and PE32+.
fn one_mib_entry_stack(source: &Path, directory: &Path) -> PathBuf {
    use std::io::{Read, Seek, SeekFrom, Write};
    let executable = directory.join(source.file_name().unwrap());
    fs::copy(source, &executable).unwrap();
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&executable)
        .unwrap();
    file.seek(SeekFrom::Start(0x3c)).unwrap();
    let mut offset = [0; 4];
    file.read_exact(&mut offset).unwrap();
    let pe = u64::from(u32::from_le_bytes(offset));
    file.seek(SeekFrom::Start(pe)).unwrap();
    let mut signature = [0; 4];
    file.read_exact(&mut signature).unwrap();
    assert_eq!(&signature, b"PE\0\0");
    file.seek(SeekFrom::Start(pe + 24)).unwrap();
    let mut magic = [0; 2];
    file.read_exact(&mut magic).unwrap();
    let field = pe + 24 + 72;
    let reserve: Vec<u8> = match u16::from_le_bytes(magic) {
        0x10b => 1_048_576_u32.to_le_bytes().to_vec(),
        0x20b => 1_048_576_u64.to_le_bytes().to_vec(),
        magic => panic!("unsupported PE optional header: {magic:#x}"),
    };
    // Refuse a truncated file: a write past its end would extend the copy, not patch it.
    assert!(
        field + reserve.len() as u64 <= file.metadata().unwrap().len(),
        "PE optional header is shorter than its stack reserve field"
    );
    file.seek(SeekFrom::Start(field)).unwrap();
    file.write_all(&reserve).unwrap();
    executable
}

#[test]
#[ignore = "daily"]
fn cli_settings_work_with_one_mib_process_stack() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    let executable = one_mib_entry_stack(Path::new(env!("CARGO_BIN_EXE_horizon-launcher")), root);
    fs::create_dir(root.join("launcher")).unwrap();
    fs::write(
        root.join("launcher/settings.json"),
        br#"{"language":"en","install_new_versions":false,"desktop_entry":false}"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        horizon_launcher::process::command(&executable)
            .args(args)
            .current_dir(root)
            .env("HORIZON_HOME", root.join("game"))
            .env("HORIZON_LAUNCHER_HOME", root.join("launcher"))
            .env("XDG_DATA_HOME", root.join("data"))
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("HORIZON_LAUNCHER_TRASH", root.join("trash"))
            .env("HORIZON_LAUNCHER_CLIPBOARD", root.join("clipboard"))
            .env("HORIZON_LAUNCHER_DESKTOP", root.join("desktop"))
            .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
            .env("HORIZON_NO_AUDIO", "1")
            .env("HORIZON_NO_KEYCHAIN", "1")
            .env("HORIZON_TELEMETRY_URL", "off")
            .output()
            .unwrap()
    };
    let output = run(&["--json", "settings", "opening-animation", "off"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let settings: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(settings["opening_animation"], false);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("launcher/settings.json")).unwrap()).unwrap();
    assert_eq!(saved["skip_opening"], true);
    // Command failures still leave the process with their original exit code.
    let refused = run(&["--json", "settings", "opening-animation", "invalid"]);
    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(refused.stdout.is_empty(), "{refused:?}");
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("launcher/settings.json")).unwrap()).unwrap();
    assert_eq!(saved["skip_opening"], true);
}
