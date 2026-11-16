//! Consent revocation persists even when native package cleanup cannot complete.
use horizon_launcher::backend::Settings;

#[test]
fn revocation_survives_reloading_settings_and_preserves_limit() {
    let scratch = tempfile::tempdir().unwrap();
    // This integration test has its own process and a single test; no shared player home.
    std::env::set_var("HORIZON_LAUNCHER_HOME", scratch.path());
    let mut settings = Settings {
        native_mods_allowed_at: Some(12345),
        cache_limit_gib: Some(50),
        ..Settings::default()
    };
    settings.save().unwrap();
    settings.revoke_native_mods().unwrap();
    assert!(settings.native_mods_allowed_at.is_none());
    let reloaded = Settings::load();
    assert!(reloaded.native_mods_allowed_at.is_none());
    assert_eq!(reloaded.cache_limit_gib, Some(50));
}
