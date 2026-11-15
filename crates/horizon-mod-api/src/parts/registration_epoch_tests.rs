#[cfg(test)]
use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Hooks;
    impl crate::Hooks for Hooks {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }

    #[test]
    fn identities_follow_real_mod_lists_and_parts_epochs() {
        let first: crate::Mods = Arc::from([crate::ModEntry::new("test:first", Box::new(Hooks))]);
        let second: crate::Mods = Arc::from([crate::ModEntry::new("test:second", Box::new(Hooks))]);
        crate::testing::with_mods(first.clone(), || {
            assert!(registry_installation_identity().1.is_none());
            crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
            let initial = registry_installation_identity();
            assert!(initial.1.is_some());
            crate::testing::with_mods(first.clone(), || {
                assert_eq!(registry_installation_identity(), initial);
            });
            crate::testing::with_mods(second, || {
                assert_ne!(registry_installation_identity().0, initial.0);
            });
            crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
            let replaced = registry_installation_identity();
            assert_eq!(initial.0, replaced.0);
            assert_ne!(initial.1, replaced.1);
            let active = crate::parts::ActiveRegistration::enter();
            let previous = crate::parts::REGISTERING_KEYS.with_borrow_mut(|keys| {
                keys.replace(std::rc::Rc::new(
                    std::cell::RefCell::new(Default::default()),
                ))
            });
            assert!(registry_installation_identity().1.is_none());
            crate::parts::REGISTERING_KEYS.with_borrow_mut(|keys| *keys = previous);
            drop(active);
            assert_eq!(registry_installation_identity(), replaced);
        });
    }
}
