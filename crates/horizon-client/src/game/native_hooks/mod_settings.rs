use super::*;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// The typed values read so far, by the mod's data folder and the key: a read resolves the
/// setting and its file once, and a write (`set_setting`) forgets the key.
static READ: LazyLock<Mutex<HashMap<(PathBuf, String), mod_api::SettingValue>>> =
    LazyLock::new(Default::default);

/// The typed reads of the settings a mod registered (`Registrar::register_setting`), for the
/// mod whose hook or part is running (`crash::current_native_mod`): a mod reads only its own.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The calling mod's setting `key` as kept in its data folder, or its default.
    fn own_setting(&self, key: &str) -> Option<mod_api::SettingValue> {
        let mod_id = mod_api::content::crash::current_native_mod()?;
        read_typed(&self.native_data_dir(mod_id), mod_id, key)
    }
    /// The calling mod's toggle `key`: the player's choice, else its default.
    pub(in crate::game) fn setting_bool(&self, key: &str) -> Option<bool> {
        match self.own_setting(key)? {
            mod_api::SettingValue::Toggle(on) => Some(on),
            _ => None,
        }
    }
    /// The option the player picked for the calling mod's choice `key`, else its default.
    pub(in crate::game) fn setting_choice(&self, key: &str) -> Option<u32> {
        match self.own_setting(key)? {
            mod_api::SettingValue::Choice(index) => u32::try_from(index).ok(),
            _ => None,
        }
    }
    /// The calling mod's range `key`, or its default.
    pub(in crate::game) fn setting_f64(&self, key: &str) -> Option<f64> {
        match self.own_setting(key)? {
            mod_api::SettingValue::Range(value) => Some(value),
            _ => None,
        }
    }
    /// The calling mod's colour `key`, `0xRRGGBB`, or its default.
    pub(in crate::game) fn setting_color(&self, key: &str) -> Option<u32> {
        match self.own_setting(key)? {
            mod_api::SettingValue::Color(rgb) => Some(rgb),
            _ => None,
        }
    }
}

// All raw file access goes through the registered owner/key boundary.
pub(super) fn read_raw(dir: &Path, owner: &str, key: &str) -> Option<String> {
    mod_api::__private::setting(owner, key)?;
    horizon_world::native_load_context::read_setting(dir, key)
}
pub(super) fn store(dir: &Path, owner: &str, key: &str, value: &str) -> bool {
    if mod_api::__private::setting(owner, key).is_none() {
        return false;
    }
    if fs::create_dir_all(dir)
        .and_then(|()| fs::write(dir.join(key), value))
        .is_err()
    {
        return false;
    }
    if let Ok(mut read) = READ.lock() {
        read.remove(&(dir.to_path_buf(), key.to_owned()));
    }
    true
}
fn read_typed(dir: &Path, owner: &str, key: &str) -> Option<mod_api::SettingValue> {
    let at = (dir.to_path_buf(), key.to_owned());
    if let Some(value) = READ.lock().ok()?.get(&at) {
        return Some(*value);
    }
    let spec = mod_api::__private::setting(owner, key)?;
    let value = spec.read(read_raw(dir, owner, key).as_deref());
    READ.lock().ok()?.insert(at, value);
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext, SettingSpec, SettingValue};
    struct Settings;
    #[mod_api::__subscriptions]
    impl Hooks for Settings {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_setting(SettingSpec::toggle("on", "test.on", false))
                .unwrap();
        }
    }
    #[test]
    fn registered_settings_confine_files_and_cache_successful_writes() {
        let mods: mod_api::Mods = vec![ModEntry::new("a", Box::new(Settings))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let root = std::env::temp_dir().join(format!("settings-carry-{}", std::process::id()));
            fs::create_dir_all(root.join("b")).unwrap();
            fs::write(root.join("b/secret"), "secret").unwrap();
            let a = root.join("a");
            assert!(!store(&a, "a", "../b/secret", "stolen"));
            assert_eq!(read_raw(&a, "a", "../b/secret"), None);
            assert_eq!(fs::read_to_string(root.join("b/secret")).unwrap(), "secret");
            assert!(store(&a, "a", "on", "true"));
            assert_eq!(read_typed(&a, "a", "on"), Some(SettingValue::Toggle(true)));
            fs::write(a.join("on"), "false").unwrap();
            assert_eq!(read_typed(&a, "a", "on"), Some(SettingValue::Toggle(true)));
            assert_eq!(
                read_typed(&root.join("other"), "a", "on"),
                Some(SettingValue::Toggle(false))
            );
            assert!(store(&a, "a", "on", "false"));
            assert_eq!(read_typed(&a, "a", "on"), Some(SettingValue::Toggle(false)));
            fs::remove_file(a.join("on")).unwrap();
            fs::create_dir(a.join("on")).unwrap();
            assert!(!store(&a, "a", "on", "true"));
            assert_eq!(read_typed(&a, "a", "on"), Some(SettingValue::Toggle(false)));
            fs::remove_dir_all(root).unwrap();
        });
    }
}
