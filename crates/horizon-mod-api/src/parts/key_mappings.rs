//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn key_mappings_are_owned_and_duplicates_refuse_startup() {
        let mut registrar = super::PartRegistrar::new("first");
        registrar.register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z"));
        registrar.register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.r"));
        registrar.register_key_mapping(KeyRegistration::new("second:zoom", "key.keyboard.z"));
        registrar.mod_id = "second";
        registrar.register_key_mapping(KeyRegistration::new("zoom", "key.keyboard.z"));
        assert_eq!(registrar.refused.len(), 2);
        let owners: Vec<_> = registrar
            .parts
            .key_mappings
            .iter()
            .map(|(owner, _)| *owner)
            .collect();
        assert_eq!(owners, ["first", "second"]);
        assert_eq!(registrar.parts.key_mappings[0].1.key, "key.keyboard.z");
    }
}

impl KeyMappingsRegistrar for PartRegistrar {
    fn register_key_mapping(&mut self, mapping: crate::KeyRegistration) {
        if mapping.id.is_empty()
            || !mapping
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
            || self
                .parts
                .key_mappings
                .iter()
                .any(|(owner, prior)| *owner == self.mod_id && prior.id == mapping.id)
        {
            self.refused.push(format!(
                "mod {}: duplicate or invalid key mapping {:?}",
                self.mod_id, mapping.id
            ));
            return;
        }
        self.parts.key_mappings.push((self.mod_id, mapping));
    }
}
