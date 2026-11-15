//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl CommandArgumentsRegistrar for PartRegistrar {
    fn add_command_argument(&mut self, name: &str, argument: Arc<dyn crate::CommandArgumentType>) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = {
            self.parts
                .command_arguments
                .iter()
                .any(|other| *other.id == id)
                .then(|| {
                    format!(
                        "{} registered command argument type {id} twice",
                        self.mod_id
                    )
                })
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.parts.command_arguments.push(CommandArgumentInfo {
            id: id.into(),
            argument,
        });
    }
}

/// Fixtures that tests of other crates use to install command argument types.
#[cfg(any(test, feature = "testing"))]
pub mod tests {

    #[test]
    pub(crate) fn registration_owns_its_namespace_and_refuses_duplicates() {
        /// A tiny argument with two valid values.
        struct Mode;

        impl crate::CommandArgumentType for Mode {
            fn kind(&self) -> crate::CommandArgumentKind {
                crate::CommandArgumentKind::Word
            }

            fn parse(&self, input: &str) -> Result<String, crate::CommandArgumentError> {
                match input {
                    "fast" | "slow" => Ok(input.to_owned()),
                    _ => Err(crate::CommandArgumentError::new(crate::Text::translate(
                        "test.commands.mode.unknown",
                        vec![crate::Text::literal(input)],
                    ))),
                }
            }

            fn suggest(
                &self,
                _: Option<&crate::CommandSender>,
                prefix: &str,
            ) -> Vec<crate::CommandSuggestion> {
                ["fast", "slow"]
                    .into_iter()
                    .filter(|mode| mode.starts_with(prefix))
                    .map(crate::CommandSuggestion::new)
                    .collect()
            }
        }

        struct Modes;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Modes {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_command_argument("mode", Mode);
            }
        }

        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_command_argument("mode", Mode);
                r.register_command_argument("mode", Mode);
                r.register_command_argument("other:mode", Mode);
                r.register_command_argument("", Mode);
            }
        }

        crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Modes),
        )])
        .unwrap();
        let installed = mod_vocab::__private::parts_installation::testing_command_arguments();
        assert_eq!(installed.len(), 1);
        assert_eq!(&*installed[0].id, "test:mode");
        assert_eq!(installed[0].argument.parse("fast"), Ok("fast".to_owned()));
        assert_eq!(
            installed[0].argument.suggest(None, "s"),
            [crate::CommandSuggestion::new("slow")]
        );

        let errors = crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Invalid),
        )])
        .unwrap_err();
        assert_eq!(errors.len(), 3);
        assert!(errors[0].contains("test registered command argument type test:mode twice"));
        assert!(errors[1].contains("namespace"));
        assert!(errors[2].contains("invalid identifier \"test:\""));
    }
}
