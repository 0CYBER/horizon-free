//! One deferred native-code action. Declining never invokes its installer or builder.
use horizon_launcher::{
    backend::Settings,
    browser::{installed::Entry, java, Project, SourceId},
    github_mods::Resolved,
};
use std::path::Path;

pub(super) enum PendingConsent {
    Build(Vec<String>),
    Toggle(Entry),
    Install(SourceId, String, Option<Project>),
    Github(Resolved, Option<String>),
    Local(String),
}

impl PendingConsent {
    pub(super) fn name(&self) -> String {
        match self {
            Self::Build(ids) => ids.join(", "),
            Self::Toggle(entry) => entry.title.clone(),
            Self::Install(_, id, _) | Self::Local(id) => id.clone(),
            Self::Github(found, _) => found.label(),
        }
    }
}

/// Consume exactly one request. Permission is persisted before returning an action to execute.
/// Cancel and Escape both use `allow = false`, and cannot return an executable action.
pub(super) fn resolve(
    pending: &mut Option<PendingConsent>,
    allow: bool,
    supported: bool,
    settings: &mut Settings,
    home: &Path,
) -> Result<Option<PendingConsent>, String> {
    if pending.is_none() {
        return Ok(None);
    }
    if allow {
        if !supported {
            return Err(crate::i18n::tr("Unavailable on this platform").into());
        }
        settings.allow_native_mods()?;
        return Ok(pending.take());
    }
    if let Some(action) = pending.take() {
        match action {
            PendingConsent::Build(ids) => {
                // One rewrite: a failure can never leave some declined mods on.
                let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
                java::set_all_enabled(home, &ids, false)?;
            }
            PendingConsent::Toggle(entry) => {
                if let Some(id) = horizon_launcher::browser::horizon::mod_id(&entry) {
                    java::set_enabled(home, id, false)?;
                }
            }
            PendingConsent::Local(id) => java::set_enabled(home, &id, false)?,
            PendingConsent::Install(..) | PendingConsent::Github(..) => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_escape_and_allow_consume_only_the_pending_toggle() {
        // Settings is process-global: exercise this transition in its own isolated process.
        if std::env::var_os("HORIZON_CONSENT_TEST_CHILD").is_none() {
            let home = tempfile::tempdir().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_consent::tests::cancel_escape_and_allow_consume_only_the_pending_toggle"])
                .env("HORIZON_CONSENT_TEST_CHILD", "1")
                .env("HORIZON_LAUNCHER_HOME", home.path())
                .env("HORIZON_HOME", home.path())
                .status().unwrap();
            assert!(status.success());
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        // Decline returns no dispatchable action: no installer or builder can run.
        // The build layer separately tests its HTTP backstop with a mock proxy.
        for _ in ["Cancel", "Escape"] {
            let mut pending = Some(PendingConsent::Local("sample".into()));
            assert!(
                resolve(&mut pending, false, true, &mut settings, home.path())
                    .unwrap()
                    .is_none()
            );
            assert!(pending.is_none());
            assert!(!java::is_enabled(home.path(), "sample"));
            assert!(settings.native_mods_allowed_at.is_none());
        }
        java::set_enabled(home.path(), "sample", true).unwrap();
        java::set_enabled(home.path(), "second", true).unwrap();
        java::set_enabled(home.path(), "unrelated", true).unwrap();
        let mut pending = Some(PendingConsent::Build(vec![
            "sample".into(),
            "second".into(),
        ]));
        resolve(&mut pending, false, true, &mut settings, home.path()).unwrap();
        assert!(
            !java::is_enabled(home.path(), "sample") && !java::is_enabled(home.path(), "second"),
            "NoConsent cancellation turns every one of the build's mods off"
        );
        assert!(java::is_enabled(home.path(), "unrelated"));
        let mut unsupported = Some(PendingConsent::Local("sample".into()));
        assert!(resolve(&mut unsupported, true, false, &mut settings, home.path()).is_err());
        assert!(unsupported.is_some());
        assert!(Settings::load().native_mods_allowed_at.is_none());
        if horizon_launcher::rust_toolchain::host().is_some() {
            let mut pending = Some(PendingConsent::Local("sample".into()));
            let action = resolve(&mut pending, true, true, &mut settings, home.path())
                .unwrap()
                .unwrap();
            assert!(Settings::load().native_mods_allowed_at.is_some());
            match action {
                PendingConsent::Local(id) => java::set_enabled(home.path(), &id, true).unwrap(),
                _ => panic!("wrong pending action"),
            }
            assert!(java::is_enabled(home.path(), "sample"));
            assert!(
                resolve(&mut pending, true, true, &mut settings, home.path())
                    .unwrap()
                    .is_none(),
                "the action cannot execute twice"
            );
        }
    }
}
