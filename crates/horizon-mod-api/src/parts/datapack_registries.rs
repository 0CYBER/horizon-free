//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl DatapackRegistriesRegistrar for PartRegistrar {
    fn declare_datapack_registry(
        &mut self,
        path: &str,
        sync: DatapackRegistrySync,
        mut fields: Vec<DatapackField>,
    ) -> Result<(), DatapackRegistrationError> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::loaded_mods()
                .get(self.mod_id)
                .is_some_and(|package| package.side() != crate::Side::Both)
            {
                return Err(DatapackRegistrationError::WrongSide);
            }
            let (namespace, name) = path.split_once(':').unwrap_or((self.mod_id, path));
            let key = format!("{namespace}:{name}");
            if namespace != self.mod_id
                || !mod_vocab::__private::parts_installation::valid_datapack_id(&key)
            {
                return Err(DatapackRegistrationError::InvalidKey);
            }
            if self
                .datapack_registries
                .iter()
                .any(|declared| declared.key == key)
            {
                return Err(DatapackRegistrationError::Duplicate);
            }
            for field in &mut fields {
                if matches!(
                    field.kind,
                    crate::DatapackFieldKind::Identifier
                        | crate::DatapackFieldKind::Reference { .. }
                ) && let Some(crate::DatapackValue::Text(value)) = &mut field.default
                {
                    *value = crate::Identifier::parse(value)
                        .map_err(|_| DatapackRegistrationError::InvalidDefault)?
                        .to_string();
                }
            }
            mod_vocab::__private::parts_installation::validate_datapack_schema(&fields)?;
            self.datapack_registries.push(DatapackRegistryDeclaration {
                mod_id: self.mod_id.to_owned(),
                key,
                fields,
                sync,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} datapack registry {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DatapackFieldKind;

    fn power() -> Vec<DatapackField> {
        vec![DatapackField::required(
            "power",
            DatapackFieldKind::Int { min: 0, max: 100 },
        )]
    }

    /// A key in the mod's namespace stands; foreign, vanilla and malformed keys, a
    /// duplicate and a bad schema refuse, naming the mod.
    #[test]
    pub(crate) fn datapack_registry_declarations_validate() {
        use DatapackRegistrationError as E;
        let mut registrar = PartRegistrar::new("example");
        let sync = DatapackRegistrySync::Required;
        registrar
            .declare_datapack_registry("spell", sync, power())
            .unwrap();
        assert_eq!(
            registrar.datapack_registries[0].key, "example:spell",
            "a bare path is in the declaring mod's namespace"
        );
        assert_eq!(
            registrar.declare_datapack_registry("example:spell", sync, power()),
            Err(E::Duplicate)
        );
        assert_eq!(
            registrar.declare_datapack_registry("minecraft:spell", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("addon:spell", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("Not A Key", sync, power()),
            Err(E::InvalidKey)
        );
        assert_eq!(
            registrar.declare_datapack_registry("", sync, power()),
            Err(E::InvalidKey)
        );
        let mut bad = power();
        bad.push(bad[0].clone());
        assert_eq!(
            registrar.declare_datapack_registry("other", sync, bad),
            Err(E::InvalidFieldName)
        );
        // A refused schema leaves no half-declared registry behind.
        assert_eq!(registrar.datapack_registries.len(), 1);
        assert_eq!(registrar.refused.len(), 6);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod example"))
        );
    }

    #[test]
    fn datapack_declarations_round_trip_the_wire() {
        use crate::wire::{Reader, Wire, Writer};
        let declaration = DatapackRegistryDeclaration {
            mod_id: "example".into(),
            key: "example:spell".into(),
            fields: power(),
            sync: DatapackRegistrySync::Optional,
        };
        let mut writer = Writer::new();
        declaration.put(&mut writer);
        let bytes = writer.into_bytes();
        assert_eq!(
            DatapackRegistryDeclaration::take(&mut Reader::new(&bytes)).unwrap(),
            declaration
        );
    }
}

#[cfg(test)]
mod default_tests {
    use super::*;
    #[test]
    fn registered_identifier_defaults_match_explicit_values() {
        for kind in [
            crate::DatapackFieldKind::Identifier,
            crate::DatapackFieldKind::Reference {
                registry: "minecraft:item".into(),
            },
        ] {
            let mut registrar = PartRegistrar::new("defaults");
            registrar
                .declare_datapack_registry(
                    "entry",
                    DatapackRegistrySync::default(),
                    vec![DatapackField::with_default(
                        "id",
                        kind,
                        crate::DatapackValue::Text("ice".into()),
                    )],
                )
                .unwrap();
            let declaration = &registrar.datapack_registries[0];
            assert_eq!(
                declaration.decode_entry("{}").unwrap(),
                declaration.decode_entry(r#"{"id":"ice"}"#).unwrap()
            );
        }
    }
}
