use super::*;
use mod_api::__private::game::RegistrationContextNew;
use mod_api::{ConditionError, Identifier, LoadCondition};
struct Flag;
impl LoadCondition for Flag {
    fn test(&self, args: &serde_json::Value) -> Result<bool, ConditionError> {
        args["value"].as_bool().ok_or_else(|| ConditionError {
            message: "value must be a boolean".into(),
        })
    }
}
pub(super) fn register(registrar: &mut dyn mod_api::LoadConditions) {
    registrar
        .register(Identifier::parse("wire_test:flag").unwrap(), Flag)
        .unwrap();
}
struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn register_load_conditions(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterLoadConditionsEvent<'_>,
    ) {
        register(event.conditions);
    }
}
#[test]
fn wire_loopback_load_conditions_native_and_guest_registrars_and_callbacks_agree() {
    for guest in [false, true] {
        let hooks: Box<dyn Hooks> = if guest {
            Box::new(WasmMod {
                group: 0,
                subscribed: &[Some(mod_api::HookId::RegisterLoadConditions)],
                listeners: &[],
                corrupt: false,
            })
        } else {
            Box::new(Native)
        };
        mod_api::testing::with_mods(
            vec![mod_api::ModEntry::new("wire_test", hooks)].into(),
            || {
                let mut registrar = mod_api::LoadConditionDeclarations::default();
                mod_api::dispatch_register_load_conditions(
                    &mod_api::RegistrationContext::new(),
                    &mut registrar,
                );
                let entries = registrar.finish().unwrap();
                assert_eq!(entries.len(), 1);
                let (owner, handler) = entries
                    .get(&Identifier::parse("wire_test:flag").unwrap())
                    .unwrap();
                assert_eq!(owner, "wire_test");
                assert_eq!(handler.test(&serde_json::json!({"value":true})), Ok(true));
                assert_eq!(handler.test(&serde_json::json!({"value":false})), Ok(false));
                assert_eq!(
                    handler.test(&serde_json::json!({"value":"bad"})),
                    Err(ConditionError {
                        message: "value must be a boolean".into()
                    })
                );
                assert!(!entries.contains_key(&Identifier::parse("wire_test:nope").unwrap()));
                let owned = entries
                    .iter()
                    .map(|(id, (owner, _))| (id.namespace().to_owned(), owner.clone()))
                    .collect();
                let conditions = horizon_world::conditions::LoadConditions::new(["wire_test"])
                    .with_lookup(owned, move |name, raw| {
                        let id = Identifier::parse(name).ok()?;
                        let (_, handler) = entries.get(&id)?;
                        let mut args = raw.as_object().cloned().unwrap();
                        args.remove("type");
                        Some(
                            handler
                                .test(&serde_json::Value::Object(args))
                                .map_err(|error| error.message),
                        )
                    });
                for kind in ["not", "and", "or"] {
                    for name in ["wire_test:nope", "othermod:thing"] {
                        let child = serde_json::json!({"type":name});
                        let node = if kind == "not" {
                            serde_json::json!({"type":"neoforge:not", "value":child})
                        } else {
                            serde_json::json!({"type":format!("neoforge:{kind}"), "values":[child]})
                        };
                        let file = format!("data/wire_test/recipe/{kind}_{name}.json");
                        assert!(!conditions.allows_file(
                            &serde_json::json!({"neoforge:conditions":[node]}),
                            &file
                        ));
                        if name == "wire_test:nope" {
                            let error = conditions.check_errors().unwrap_err();
                            assert!(
                                error.contains(&file)
                                    && error.contains(name)
                                    && error.contains("mod wire_test"),
                                "{error}"
                            );
                        } else {
                            assert!(
                                conditions.foreign_errors().iter().any(|warning| warning
                                    .contains(&file)
                                    && warning.contains(name))
                            );
                        }
                    }
                }
            },
        );
    }
}

pub(super) struct LoadConditionsHooks;
impl LoadConditionsHooks {
    pub(super) fn register_load_conditions(
        &self,
        _: &mod_api::RegistrationContext,
        event: &mut mod_api::RegisterLoadConditionsEvent<'_>,
    ) {
        self::register(event.conditions);
    }
}
