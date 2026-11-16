//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn settings_are_owned_and_bad_or_duplicate_ones_are_refused() {
        let mut registrar = super::PartRegistrar::new("first");
        assert_eq!(
            registrar.register_setting(SettingSpec::toggle("on", "first.on", true)),
            Ok(())
        );
        assert_eq!(
            registrar.register_setting(SettingSpec::toggle("on", "first.on", false)),
            Err(SettingError::Duplicate("on".into()))
        );
        assert_eq!(
            registrar.register_setting(SettingSpec::range(
                "speed",
                "first.speed",
                1.0,
                0.0,
                0.1,
                0.5
            )),
            Err(SettingError::EmptyRange)
        );
        registrar.mod_id = "second";
        assert_eq!(
            registrar.register_setting(SettingSpec::toggle("on", "second.on", false)),
            Ok(())
        );
        let owners: Vec<_> = registrar
            .parts
            .settings
            .iter()
            .map(|(owner, s)| (*owner, s.key.as_str()))
            .collect();
        assert_eq!(owners, [("first", "on"), ("second", "on")]);
        assert!(
            registrar.refused.is_empty(),
            "a refused setting is the mod's error, not startup's"
        );
    }
}

impl SettingsRegistrar for PartRegistrar {
    fn register_setting(&mut self, setting: crate::SettingSpec) -> Result<(), crate::SettingError> {
        setting.check()?;
        if self
            .parts
            .settings
            .iter()
            .any(|(owner, prior)| *owner == self.mod_id && prior.key == setting.key)
        {
            return Err(crate::SettingError::Duplicate(setting.key));
        }
        self.parts.settings.push((self.mod_id, setting));
        Ok(())
    }
}
