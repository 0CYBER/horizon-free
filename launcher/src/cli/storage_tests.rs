//! The `storage` command and the cache limit setting.
use super::*;

#[test]
fn storage_command_and_cache_limit() {
    assert!(matches!(
        Cli::try_parse_from(["horizon-launcher", "storage"])
            .unwrap()
            .command,
        Some(Cmd::Storage { clear: None })
    ));
    assert!(matches!(
        Cli::try_parse_from(["horizon-launcher", "storage", "--clear", "all"])
            .unwrap()
            .command,
        Some(Cmd::Storage {
            clear: Some(ClearStorage::All)
        })
    ));
    assert!(Cli::try_parse_from(["horizon-launcher", "storage", "--clear", "worlds"]).is_err());
    assert!(Cli::try_parse_from(["horizon-launcher", "native-cache"]).is_err());
    assert!(
        Cli::try_parse_from(["horizon-launcher", "storage", "--clear", "other-caches"]).is_ok()
    );
    let mut settings = backend::Settings::default();
    for value in ["10", "25", "50", "100", "0"] {
        set_setting(&mut settings, "cache-limit-gib", value).unwrap();
    }
    assert!(set_setting(&mut settings, "cache-limit-gib", "12").is_err());
    let old: backend::Settings = serde_json::from_str(r#"{"native_cache_gib":100}"#).unwrap();
    assert_eq!(old.cache_limit_gib, None);
    set_setting(&mut settings, "cache-limit-gib", "25").unwrap();
    let saved = serde_json::to_string(&settings).unwrap();
    let loaded: backend::Settings = serde_json::from_str(&saved).unwrap();
    assert_eq!(loaded.cache_limit_gib, Some(25));
    assert!(set_setting(&mut settings, "cache-limit-gib", "18446744073709551615").is_err());
    assert!(set_setting(&mut settings, "cache-limit-gib", "-1").is_err());
    set_setting(&mut settings, "cache-limit-gib", "0").unwrap();
    assert_eq!(settings.cache_limit_gib, Some(0));
    set_setting(&mut settings, "cache-limit-gib", "default").unwrap();
    assert_eq!(settings.cache_limit_gib, None);
}
