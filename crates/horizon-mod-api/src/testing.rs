//! Test support with API-owned registration and vocabulary-owned fakes.
pub use mod_vocab::testing::*;

/// Run with these mods and their controller and pose registrations on this thread.
/// Nested scopes and unwinding restore the preceding immutable lookup tables.
pub fn with_mods<R>(mods: crate::Mods, f: impl FnOnce() -> R) -> R {
    crate::__private::initialize().expect("test registration requires the mod API runtime");
    let (controllers, poses) = crate::parts::registered_controllers(&mods);
    mod_vocab::testing::with_controller_tables(controllers, poses, || {
        mod_vocab::testing::with_dispatch(mods, f)
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
