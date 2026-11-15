//! Validation for the shared load-owned custom statistic declarations.
use super::*;

impl CustomStatsRegistrar for PartRegistrar {
    fn register_custom_stat(&mut self, id: &str) -> Result<(), String> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen".to_owned());
            }
            let id = if id.contains(':') {
                id.to_owned()
            } else {
                format!("{}:{id}", self.mod_id)
            };
            let identifier = crate::Identifier::parse_namespaced(&id)
                .ok_or("lowercase resource-id namespace and path required")?;
            if self.custom_stats.iter().any(|declared| declared == &id) {
                return Err("duplicate custom statistic".into());
            }
            if identifier.namespace() == "minecraft"
                || !owns_namespace(self.mod_id, identifier.namespace())
            {
                return Err("namespace must be owned by the registering mod, not minecraft".into());
            }
            self.custom_stats.push(id);
            Ok(())
        })();
        result.map_err(|reason: String| {
            let message = format!("mod {} custom statistic {id:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_stat_registration_qualifies_owned_ids_and_refuses_invalid_entries() {
        let mut registrar = PartRegistrar::new("demo");
        registrar.register_custom_stat("steps").unwrap();
        assert_eq!(registrar.custom_stats, ["demo:steps"]);
        assert!(registrar.register_custom_stat("demo:steps").is_err());
        for id in [
            "demo:",
            ":steps",
            "demo:Steps",
            "minecraft:steps",
            "other:steps",
            "demo:a:b",
        ] {
            assert!(registrar.register_custom_stat(id).is_err(), "{id}");
        }
        assert_eq!(registrar.custom_stats, ["demo:steps"]);
    }

    #[test]
    fn custom_stat_registration_refuses_reuse_across_mods() {
        let mut registrar = PartRegistrar::new("first");
        registrar.register_custom_stat("steps").unwrap();
        registrar.mod_id = "second";
        assert!(registrar.register_custom_stat("first:steps").is_err());
        assert!(registrar.register_custom_stat("first:other").is_err());
        registrar.register_custom_stat("steps").unwrap();
        assert_eq!(registrar.custom_stats, ["first:steps", "second:steps"]);
    }
}
