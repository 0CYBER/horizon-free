//! Test support with API-owned registration and vocabulary-owned fakes.
pub use mod_vocab::testing::*;

/// Run with these mods and their controller and pose registrations on this thread.
/// Nested scopes and unwinding restore the preceding immutable lookup tables and Parts snapshot.
pub fn with_mods<R>(mods: crate::Mods, f: impl FnOnce() -> R) -> R {
    crate::__private::initialize().expect("test registration requires the mod API runtime");
    let (controllers, poses) = crate::parts::registered_controllers(&mods);
    mod_vocab::testing::with_parts_scope(|| {
        mod_vocab::testing::with_controller_tables(controllers, poses, || {
            mod_vocab::testing::with_dispatch(mods, f)
        })
    })
}

/// Install explicit raw serializer/type pairs for resource-loader fixtures.
/// Each listed ID is declared in both independent registries by its namespace owner.
pub fn with_recipe_types<R>(ids: &[&'static str], f: impl FnOnce() -> R) -> R {
    struct RawTypes(Vec<&'static str>);
    #[mod_api::__subscriptions]
    impl crate::Hooks for RawTypes {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            for id in &self.0 {
                r.register_recipe_type(id);
                r.register_raw_recipe_serializer(id, id);
            }
        }
    }
    let entries = || {
        let mut groups = std::collections::BTreeMap::<_, Vec<_>>::new();
        for id in ids {
            groups
                .entry(id.split_once(':').unwrap().0)
                .or_default()
                .push(*id);
        }
        groups
            .into_iter()
            .map(|(ns, ids)| crate::ModEntry::new(ns, Box::new(RawTypes(ids))))
            .collect::<Vec<_>>()
    };
    with_mods(entries().into(), || {
        crate::__private::part_tests::install_parts_fixture(&entries()).unwrap();
        f()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CustomStat(&'static str);
    #[mod_api::__subscriptions]
    impl crate::Hooks for CustomStat {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            r.register_custom_stat(self.0).unwrap();
        }
    }

    #[test]
    fn with_mods_restores_parts_on_exit_and_unwind() {
        use mod_vocab::__private::custom_stat_declarations;

        let make = |ns: &'static str, stat: &'static str| -> Vec<crate::ModEntry> {
            vec![crate::ModEntry::new(ns, Box::new(CustomStat(stat)))]
        };
        assert!(custom_stat_declarations().is_empty());

        // Normal exit restores TEST_PARTS so a subsequent session on the same thread starts clean.
        with_mods(make("first", "first:jumps").into(), || {
            crate::__private::part_tests::install_parts_fixture(&make("first", "first:jumps"))
                .unwrap();
            assert_eq!(custom_stat_declarations(), ["first:jumps"]);

            // A nested session that replaces parts via rebuild_parts restores the outer snapshot.
            with_mods(std::sync::Arc::from([]), || {
                let res: Result<(), ()> = mod_vocab::testing::rebuild_parts(|| {
                    crate::__private::part_tests::install_parts_fixture(&make(
                        "inner",
                        "inner:hops",
                    ))
                    .unwrap();
                    Ok(())
                });
                assert_eq!(res, Ok(()));
                assert_eq!(custom_stat_declarations(), ["inner:hops"]);
            });
            assert_eq!(custom_stat_declarations(), ["first:jumps"]);
        });
        assert!(custom_stat_declarations().is_empty());

        // Unwinding from a session also restores TEST_PARTS so later registrations are not refused.
        let unwound = std::panic::catch_unwind(|| {
            with_mods(make("panic_mod", "panic_mod:falls").into(), || {
                crate::__private::part_tests::install_parts_fixture(&make(
                    "panic_mod",
                    "panic_mod:falls",
                ))
                .unwrap();
                assert_eq!(custom_stat_declarations(), ["panic_mod:falls"]);
                panic!("simulated test failure");
            });
        });
        assert!(unwound.is_err());
        assert!(custom_stat_declarations().is_empty());

        with_mods(make("after", "after:ok").into(), || {
            crate::__private::part_tests::install_parts_fixture(&make("after", "after:ok"))
                .unwrap();
            assert_eq!(custom_stat_declarations(), ["after:ok"]);
        });
        assert!(custom_stat_declarations().is_empty());
    }
}
