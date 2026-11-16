//! Explicit account-free Play for persisted unsigned checkout lineage.
use crate::{
    developer_install,
    i18n::{tr, trf},
};
use std::process::Command;

pub const ENV: &str = "HORIZON_LAUNCHER_DEVELOPER_OFFLINE";
pub const FLAG: &str = "--developer-single-player";
pub const NOTICE: &str = "Offline developer session: single-player only; no account features.";

pub fn enabled(tag: &str) -> bool {
    if std::env::var_os(ENV).as_deref() != Some(std::ffi::OsStr::new("1")) {
        return false;
    }
    let native = crate::native_mods::about(tag);
    let base = native
        .as_ref()
        .map(|a| a.base.clone())
        .or_else(|| crate::build_import::about(tag).map(|a| a.base))
        .unwrap_or_else(|| tag.to_owned());
    eligible(
        &base,
        &crate::backend::version_dir(&base),
        native.as_ref().map(|a| a.horizon.as_str()),
    )
}

fn eligible(base: &str, directory: &std::path::Path, native_revision: Option<&str>) -> bool {
    developer_install::is_developer(base)
        && directory.join(".installed").is_file()
        && developer_install::revision_in(directory)
            .is_ok_and(|revision| native_revision.is_none_or(|expected| expected == revision))
}

pub const REBUILD: &str = "This game cannot enforce offline developer single-player. Rebuild and reinstall the developer game and its native mods.";
const START_FAILED: &str = "The game failed to start while checking support for offline developer play. Repeat your horizon-launcher developer-install command, rebuild this build’s native mods with native-build, then try Play again. If it still fails, report this message on the Horizon Discord (linked in the launcher).\nDetails: {reason}";
pub fn require_capability(tag: &str) -> Result<(), String> {
    let program = crate::backend::client_program(tag);
    let output = Command::new(&program)
        .arg("--launch-api")
        .output()
        .map_err(|e| startup_failure(&format!("{}: {e}", program.display())))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .map(str::trim)
            .find(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                trf(
                    "The game stopped without an error message ({status}).",
                    &[("status", &output.status.to_string())],
                )
            });
        return Err(startup_failure(&reason));
    }
    if supports_restriction(&output.stdout) {
        Ok(())
    } else {
        Err(tr(REBUILD).into())
    }
}

// Both spawn failures and child diagnostics cross this user-visible boundary.
fn startup_failure(reason: &str) -> String {
    let mut chars = reason.chars();
    let mut reason: String = chars.by_ref().take(500).collect();
    if chars.next().is_some() {
        reason.push('…');
    }
    trf(START_FAILED, &[("reason", &reason)])
}

fn supports_restriction(bytes: &[u8]) -> bool {
    #[derive(serde::Deserialize)]
    struct Api {
        options: Vec<OptionFlag>,
    }
    #[derive(serde::Deserialize)]
    struct OptionFlag {
        name: String,
        value: Option<String>,
    }
    serde_json::from_slice::<Api>(bytes).is_ok_and(|api| {
        api.options
            .iter()
            .any(|o| o.name == FLAG && o.value.is_none())
    })
}

pub fn mark(cmd: &mut Command) {
    cmd.arg(FLAG)
        .env_remove("HORIZON_SESSION_FILE")
        .env_remove("HORIZON_ACCOUNT_SESSIONS")
        .env_remove("HORIZON_LAUNCHER_EXE");
}

/// Validate after instance arguments and wrappers have been applied, before acquiring real leases.
pub fn validate(cmd: &Command) -> Result<(), String> {
    let offline = cmd.get_args().any(|a| a == FLAG);
    if offline
        && cmd.get_envs().any(|(key, value)| {
            (key == "HORIZON_SESSION_FILE" || key == "HORIZON_ACCOUNT_SESSIONS")
                && value.is_some_and(|v| !v.is_empty())
        })
    {
        return Err(tr(NOTICE).into());
    }
    if offline
        && cmd.get_args().any(|a| {
            let a = a.to_string_lossy();
            [
                "--server",
                "--quickPlayMultiplayer",
                "--session-file",
                "--session-stdin",
                "--dedicated",
                "--transfers",
            ]
            .iter()
            .any(|flag| a == *flag || a.starts_with(&format!("{flag}=")))
        })
    {
        return Err(tr(NOTICE).into());
    }
    Ok(())
}

pub fn selected() -> bool {
    let instance = crate::instances::selected();
    instance.java.is_none()
        && instance
            .resolve_version(None)
            .is_ok_and(|tag| enabled(&tag))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_failure_bounds_unicode_details_without_changing_short_diagnostics() {
        let short = "dyld[4321]: Library not loaded: @rpath/libhorizon_mod_host.dylib";
        assert!(startup_failure(short).ends_with(short));
        let long = "界".repeat(501);
        let message = startup_failure(&long);
        let details = message.split_once("Details: ").unwrap().1;
        assert_eq!(details.chars().count(), 501);
        assert!(details.ends_with('…'));
        assert!(!startup_failure(&"界".repeat(500)).ends_with('…'));
    }

    #[test]
    fn account_gate_is_preserved_without_opt_in_and_for_signed_lineage() {
        if let Ok(tag) = std::env::var("OFFLINE_GATE_TEST_TAG") {
            assert!(!enabled(&tag));
            let error = crate::backend::client_as(&tag, None).unwrap_err();
            assert!(
                error.contains(crate::i18n::tr(crate::backend::SIGN_IN_HINT)),
                "{error}"
            );
            return;
        }
        for (tag, opt_in) in [
            ("developer-local", None),
            ("v2026.11.10", Some("1")),
            ("nightly-abcdef", Some("1")),
            ("developer-local", Some("true")),
        ] {
            let root = tempfile::tempdir().unwrap();
            let version = root.path().join("versions").join(tag);
            std::fs::create_dir_all(&version).unwrap();
            std::fs::write(version.join(".installed"), "").unwrap();
            std::fs::write(
                version.join("developer.json"),
                serde_json::json!({"revision":"a".repeat(40)}).to_string(),
            )
            .unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "developer_offline::tests::account_gate_is_preserved_without_opt_in_and_for_signed_lineage", "--nocapture"])
                .env("OFFLINE_GATE_TEST_TAG", tag)
                .env("HORIZON_LAUNCHER_HOME", root.path())
                .env("HORIZON_HOME", root.path().join("game"))
                .env("XDG_CONFIG_HOME", root.path().join("config"))
                .env("XDG_DATA_HOME", root.path().join("data"));
            if let Some(value) = opt_in {
                child.env(ENV, value);
            } else {
                child.env_remove(ENV);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    #[test]
    fn legacy_and_malformed_capabilities_refuse() {
        for reply in [
            b"not json".as_slice(),
            br#"{}"#,
            br#"{"options":[]}"#,
            br#"{"options":[{"name":"--developer-single-player","value":"VALUE"}]}"#,
        ] {
            assert!(!supports_restriction(reply));
        }
        assert!(supports_restriction(
            br#"{"options":[{"name":"--developer-single-player","value":null}]}"#
        ));
    }
    #[test]
    fn persisted_provenance_not_just_a_prefix() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        assert!(!eligible("developer-local", dir, None));
        std::fs::write(dir.join(".installed"), "").unwrap();
        for record in ["{}", "not json", r#"{"revision":"wrong"}"#] {
            std::fs::write(dir.join("developer.json"), record).unwrap();
            assert!(!eligible("developer-local", dir, None));
        }
        std::fs::write(
            dir.join("developer.json"),
            serde_json::json!({"revision":"a".repeat(40)}).to_string(),
        )
        .unwrap();
        assert!(eligible("developer-local", dir, None));
        assert!(eligible("developer-local", dir, Some(&"a".repeat(40))));
        assert!(!eligible("developer-local", dir, Some(&"b".repeat(40))));
        for base in ["v2026.11.10", "nightly-abcdef", "native-signed"] {
            assert!(!eligible(base, dir, None));
        }
    }
    #[test]
    fn composition_cannot_reintroduce_account_state_or_clear_the_restriction() {
        let mut command = Command::new("game");
        command.env("HORIZON_SESSION_FILE", "inherited-session.json");
        command.env("HORIZON_ACCOUNT_SESSIONS", "inherited sessions");
        mark(&mut command);
        for key in [
            "HORIZON_SESSION_FILE",
            "HORIZON_ACCOUNT_SESSIONS",
            "HORIZON_LAUNCHER_EXE",
        ] {
            assert_eq!(
                command.get_envs().find(|(name, _)| *name == key).unwrap().1,
                None
            );
        }
        command.env(ENV, "0");
        assert!(command.get_args().any(|argument| argument == FLAG));
        assert!(validate(&command).is_ok());
        for key in ["HORIZON_SESSION_FILE", "HORIZON_ACCOUNT_SESSIONS"] {
            let mut command = Command::new("game");
            mark(&mut command);
            let launch = crate::launch::Launch {
                env: [(key.to_string(), "reintroduced".to_string())].into(),
                ..Default::default()
            };
            let wrapped = crate::launch::wrap(command, &launch);
            assert!(validate(&wrapped).is_err(), "{key}");
        }
    }

    #[test]
    fn final_network_and_session_intents_are_refused() {
        for flag in [
            "--server",
            "--quickPlayMultiplayer",
            "--session-file",
            "--session-stdin",
            "--dedicated",
            "--transfers",
            "--server=localhost",
        ] {
            let mut command = Command::new("game");
            mark(&mut command);
            command.arg(flag);
            assert!(validate(&command).is_err(), "{flag}");
        }
        let mut command = Command::new("game");
        mark(&mut command);
        command.args(["--world", "DeveloperProof", "--creative"]);
        assert!(validate(&command).is_ok());
        let mut normal = Command::new("game");
        normal.args(["--server", "localhost"]);
        assert!(validate(&normal).is_ok());
    }
}
