//! Trust status and runtime privacy follow a developer base through imported and native games.
use horizon_launcher::{backend, developer_install};
use std::fs;

#[test]
fn instance_status_and_runtime_keep_developer_provenance() {
    let home = tempfile::tempdir().unwrap();
    for name in [
        "HORIZON_HOME",
        "HORIZON_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "HORIZON_LAUNCHER_TRASH",
        "HORIZON_LAUNCHER_CLIPBOARD",
        "HORIZON_LAUNCHER_DESKTOP",
    ] {
        std::env::set_var(name, home.path().join(name));
    }
    std::env::set_var("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1");
    std::env::set_var("HORIZON_NO_AUDIO", "1");
    let revision = "0123456789abcdef0123456789abcdef01234567";
    let developer = "developer-012345";
    let imported = "imported-2026-11-10-012345";
    let native = "native-0123456-0123456789ab";
    for (tag, record, value) in [
        (
            developer,
            "developer.json",
            serde_json::json!({"revision": revision}),
        ),
        (
            imported,
            "imported.json",
            serde_json::json!({"base": developer, "file": "horizon-client"}),
        ),
        (
            native,
            "native-mods.json",
            serde_json::json!({"base": developer, "horizon": revision, "target": "client", "left_out": []}),
        ),
    ] {
        let folder = backend::version_dir(tag);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(record), value.to_string()).unwrap();
        assert!(!backend::select_after_update(Some(tag), &[]));
        for prepared in [false, true] {
            let status = developer_install::instance_status(tag, prepared).unwrap();
            assert!(
                status.contains("Local unsigned developer build"),
                "{status}"
            );
            assert!(status.contains(revision), "{status}");
            assert!(
                status.contains(if prepared { "Ready" } else { "Needs preparing" }),
                "{status}"
            );
        }
        for command in [
            backend::client_command(tag),
            backend::server_command(tag, home.path()),
        ] {
            assert!(command
                .get_envs()
                .any(|(key, value)| key == "HORIZON_TELEMETRY_URL"
                    && value == Some(std::ffi::OsStr::new("off"))));
        }
    }
    assert!(developer_install::instance_status("v2026.11.10", true).is_none());
    assert!(!backend::select_after_update(Some(developer), &[]));
}
