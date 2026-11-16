//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;
    #[cfg(test)]
    use crate::*;

    /// A structure type twice, one in an unowned namespace and one without
    /// a name: every registration is refused at startup, naming the mod.
    #[test]
    pub(crate) fn duplicate_and_foreign_structure_types_refuse_registration() {
        struct Tower;
        impl mod_api::StructureType for Tower {
            fn find_generation_point(
                &self,
                _: &mut dyn mod_api::GenerationContext,
            ) -> Option<mod_api::FoundStructure> {
                None
            }
            fn load_piece(
                &self,
                _: &dyn mod_api::BlockMetadata,
                _: &mod_api::Compound,
            ) -> Result<std::sync::Arc<dyn mod_api::StructurePiece>, mod_api::StructureError>
            {
                Err(mod_api::StructureError("fixture has no pieces".into()))
            }
        }
        struct Valid;
        #[mod_api::__subscriptions]
        impl Hooks for Valid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_type("tower", Tower);
            }
        }
        crate::parts::tests::install_parts_fixture(&[ModEntry::new("test", Box::new(Valid))])
            .unwrap();
        let types = crate::__private::structure_type_declarations();
        assert!(types.iter().any(|(id, _)| id == "test:tower"));
        assert_eq!(types.len(), 1);

        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.register_structure_type("tower", Tower);
                r.register_structure_type("tower", Tower);
                r.register_structure_type("other:tower", Tower);
                r.register_structure_type("", Tower);
            }
        }
        let errors = install_parts(
            &[ModEntry::new("test", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("test registered structure type test:tower twice"));
        assert!(errors[1].contains("namespace"));
        assert!(errors[2].contains("invalid identifier \"test:\""));
    }
}

impl StructuresRegistrar for PartRegistrar {
    fn add_structure_type(&mut self, name: &str, ty: Arc<dyn crate::StructureType>) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = {
            self.structure_types
                .iter()
                .any(|t| *t.id == *id)
                .then(|| format!("{} registered structure type {id} twice", self.mod_id))
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.structure_types
            .push(StructureTypeInfo { id: id.into(), ty });
    }
}
