//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use crate::*;

    #[test]
    pub(crate) fn entity_attribute_declarations_validate_and_refuse_duplicates() {
        let mut registrar = super::PartRegistrar::new("ruby");
        registrar
            .declare_entity_attributes(
                "ghoul",
                "minecraft:zombie",
                vec![("minecraft:max_health".to_string(), 55.0)],
            )
            .unwrap();
        // The same type twice is a duplicate, whatever declares it.
        assert_eq!(
            registrar.declare_entity_attributes("ghoul", "minecraft:zombie", vec![]),
            Err(EntityAttributeError::Duplicate)
        );
        // The path stays in the declaring mod's namespace.
        assert_eq!(
            registrar.declare_entity_attributes("foreign:ghoul", "minecraft:zombie", vec![]),
            Err(EntityAttributeError::InvalidId)
        );
        // A set validates itself: a duplicate attribute, a non-finite base.
        assert_eq!(
            registrar.declare_entity_attributes(
                "wraith",
                "minecraft:zombie",
                vec![
                    ("minecraft:max_health".to_string(), 1.0),
                    ("minecraft:max_health".to_string(), 2.0),
                ]
            ),
            Err(EntityAttributeError::DuplicateAttribute)
        );
        assert_eq!(
            registrar.declare_entity_attributes(
                "wraith",
                "minecraft:zombie",
                vec![("minecraft:max_health".to_string(), f64::NAN)]
            ),
            Err(EntityAttributeError::NotFinite)
        );
        // The refusals name the mod; the good declaration stands.
        assert_eq!(registrar.entity_attributes.len(), 1);
        assert_eq!(
            registrar.entity_attributes[0].attributes,
            [("minecraft:max_health".to_string(), 55.0)]
        );
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }

    /// Additions to existing types (`EntityAttributeModificationEvent`)
    /// reach any namespace, may repeat a type, and validate each call's set.
    #[test]
    pub(crate) fn entity_attribute_modifications_validate_each_set() {
        let mut registrar = super::PartRegistrar::new("ruby");
        // A vanilla type, by bare name or full id, and the same type again.
        registrar
            .modify_entity_attributes("zombie", vec![("minecraft:luck".to_string(), Some(2.0))])
            .unwrap();
        registrar
            .modify_entity_attributes(
                "minecraft:zombie",
                vec![("minecraft:follow_range".to_string(), None)],
            )
            .unwrap();
        // Another mod's type is fine too.
        registrar
            .modify_entity_attributes("other:ghoul", vec![])
            .unwrap();
        assert_eq!(
            registrar.modify_entity_attributes("Not An Id", vec![]),
            Err(EntityAttributeError::InvalidId)
        );
        assert_eq!(
            registrar.modify_entity_attributes(
                "minecraft:skeleton",
                vec![
                    ("minecraft:luck".to_string(), None),
                    ("minecraft:luck".to_string(), Some(1.0)),
                ]
            ),
            Err(EntityAttributeError::DuplicateAttribute)
        );
        assert_eq!(
            registrar.modify_entity_attributes(
                "minecraft:skeleton",
                vec![("minecraft:luck".to_string(), Some(f64::INFINITY))]
            ),
            Err(EntityAttributeError::NotFinite)
        );
        let entities: Vec<_> = registrar
            .entity_attribute_modifications
            .iter()
            .map(|m| m.entity.as_str())
            .collect();
        assert_eq!(
            entities,
            ["minecraft:zombie", "minecraft:zombie", "other:ghoul"]
        );
        assert_eq!(
            registrar.entity_attribute_modifications[1].attributes,
            [("minecraft:follow_range".to_string(), None)]
        );
        assert_eq!(registrar.refused.len(), 3);
        assert!(
            registrar
                .refused
                .iter()
                .all(|refusal| refusal.contains("mod ruby"))
        );
    }
}

impl EntityAttributesRegistrar for PartRegistrar {
    fn declare_entity_attributes(
        &mut self,
        path: &str,
        template: &str,
        attributes: Vec<(String, f64)>,
    ) -> Result<(), crate::EntityAttributeError> {
        let id = match self.owned_declaration_id(path) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return Err(crate::EntityAttributeError::InvalidId);
            }
        };
        let result = (|| {
            let (template_ns, template_name) =
                template.split_once(':').unwrap_or(("minecraft", template));
            if template_ns.is_empty() || template_name.is_empty() {
                return Err(crate::EntityAttributeError::InvalidId);
            }
            if self.entity_attributes.iter().any(|d| d.id == id) {
                return Err(crate::EntityAttributeError::Duplicate);
            }
            check_attribute_set(&attributes, |base| Some(*base))?;
            self.entity_attributes.push(EntityAttributeDeclaration {
                id,
                template: format!("{template_ns}:{template_name}"),
                attributes,
            });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} entity attributes {path:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }

    fn modify_entity_attributes(
        &mut self,
        entity: &str,
        attributes: Vec<(String, Option<f64>)>,
    ) -> Result<(), crate::EntityAttributeError> {
        let result = (|| {
            let id = if entity.contains(':') {
                entity.to_owned()
            } else {
                format!("minecraft:{entity}")
            };
            if crate::Identifier::parse_namespaced(&id).is_none() {
                return Err(crate::EntityAttributeError::InvalidId);
            }
            check_attribute_set(&attributes, |base| *base)?;
            self.entity_attribute_modifications
                .push(EntityAttributeModification {
                    entity: id,
                    attributes,
                });
            Ok(())
        })();
        if let Err(error) = &result {
            self.refused.push(format!(
                "mod {} entity attribute additions {entity:?}: {error:?}",
                self.mod_id
            ));
        }
        result
    }
}

/// Refuses a set that names an attribute twice or has a non-finite base.
fn check_attribute_set<B>(
    attributes: &[(String, B)],
    base: impl Fn(&B) -> Option<f64>,
) -> Result<(), crate::EntityAttributeError> {
    for (index, (attribute, value)) in attributes.iter().enumerate() {
        if base(value).is_some_and(|base| !base.is_finite()) {
            return Err(crate::EntityAttributeError::NotFinite);
        }
        if attributes[..index]
            .iter()
            .any(|(other, _)| other == attribute)
        {
            return Err(crate::EntityAttributeError::DuplicateAttribute);
        }
    }
    Ok(())
}
