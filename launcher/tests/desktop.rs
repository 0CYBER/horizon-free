//! Distributor wrappers must survive disabling and re-enabling menu integration.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::Command;

fn desktop_entry(home: &Path, wrapper: Option<&str>, args: &[&str]) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_horizon-launcher"));
    command
        .arg("desktop-entry")
        .args(args)
        .env("HORIZON_LAUNCHER_HOME", home.join("launcher"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1")
        .env_remove("HORIZON_LAUNCHER_DESKTOP_EXEC");
    if let Some(wrapper) = wrapper {
        command.env("HORIZON_LAUNCHER_DESKTOP_EXEC", wrapper);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn menu_registration_preserves_the_package_wrapper() {
    let home = std::env::temp_dir().join(format!("horizon-desktop-wrapper-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let entry = home.join("data/applications/horizon-launcher.desktop");

    // Migrate a registration that pointed at the private executable.
    desktop_entry(&home, None, &[]);
    let original = std::fs::read_to_string(&entry).unwrap();
    assert!(original.contains(&format!(
        "Exec=\"{}\"\n",
        env!("CARGO_BIN_EXE_horizon-launcher")
    )));

    let wrapper = "/usr/bin/horizon-launcher";
    for _ in 0..2 {
        desktop_entry(&home, Some(wrapper), &[]);
        let text = std::fs::read_to_string(&entry).unwrap();
        assert!(text.contains(&format!("Exec=\"{wrapper}\"\n")));
        assert!(text.contains(&format!("TryExec={wrapper}\n")));
        desktop_entry(&home, Some(wrapper), &["--remove"]);
        assert!(!entry.exists());
    }

    // An unset or empty override keeps standalone installations working.
    desktop_entry(&home, Some(""), &[]);
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), original);
    std::fs::remove_dir_all(&home).unwrap();
}
