//! World-preset editor registration.

/// A client screen used to customize a data-pack world preset.
#[derive(horizon_mod_api_macros::Wire, Clone, Debug, PartialEq, Eq)]
// exhaustive: registration is plain data shared by native and wasm mods
pub struct PresetEditor {
    /// World-preset registry id, such as `example:islands`.
    pub preset: String,
    /// Namespaced editor id, available through `ClientHost::preset_editor`.
    pub screen: String,
}

#[cfg(test)]
mod tests {
    use crate::*;
    struct EditorMod;
    #[crate::__subscriptions]
    impl Hooks for EditorMod {
        fn register_preset_editors(
            &self,
            _: &RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            event.editors.push(PresetEditor {
                preset: "example:islands".into(),
                screen: "example:editor".into(),
            });
        }
    }
    #[test]
    fn preset_editor_registration_uses_the_generated_dispatch() {
        testing::with_dispatch(
            vec![ModEntry::new("example", Box::new(EditorMod))].into(),
            || {
                let mut editors = Vec::new();
                dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
                assert_eq!(
                    editors,
                    [PresetEditor {
                        preset: "example:islands".into(),
                        screen: "example:editor".into()
                    }]
                );
            },
        );
        testing::with_dispatch(Vec::new().into(), || {
            let mut editors = Vec::new();
            dispatch_register_preset_editors(&RegistrationContext::new(), &mut editors);
            assert!(editors.is_empty());
        });
    }
}
