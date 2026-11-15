//! Native behavioral coverage of the explicit default-provider domain cutover.
#[cfg(test)]
mod tests {
    use crate::{
        ENERGY, FLUIDS, Hooks, Host, ITEM_FLUIDS, ITEM_ITEMS, ITEMS, ModEntry, Registrar,
        RegistrationContext, STORAGE,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    };

    struct Defaults {
        slots: Arc<AtomicU32>,
        calls: Arc<AtomicUsize>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Defaults {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let machine = r.block_key("defaults:machine");
            let backpack = r.item_key("defaults:backpack");
            let bucket = r.item_key("minecraft:water_bucket");
            let later = r.block_key("later:machine");
            assert!(!r.is_block_capability_registered(&ITEMS, &later));
            assert!(!r.is_block_capability_registered(&ITEMS, &machine));
            assert!(!r.is_block_capability_registered(&STORAGE, &machine));
            assert!(!r.is_block_capability_registered(&FLUIDS, &machine));
            assert!(!r.is_block_capability_registered(&ENERGY, &machine));
            assert!(!r.is_item_capability_registered(&ITEM_ITEMS, &backpack));
            assert!(r.is_item_capability_registered(&ITEM_FLUIDS, &bucket));
            // A registered provider can currently answer no slots/tanks/energy. Its type
            // domain is still registered, so a fallback must not replace it.
            r.block_container(&machine, Default::default());
            r.block_fluids(&machine, Default::default());
            r.block_energy(&machine, Default::default());
            assert!(r.is_block_capability_registered(&ITEMS, &machine));
            assert!(r.is_block_capability_registered(&STORAGE, &machine));
            assert!(r.is_block_capability_registered(&FLUIDS, &machine));
            assert!(r.is_block_capability_registered(&ENERGY, &machine));
            let slots = self.slots.clone();
            let calls = self.calls.clone();
            r.item_container(&backpack, move |_: &dyn Host| {
                calls.fetch_add(1, Ordering::SeqCst);
                let current = slots.load(Ordering::SeqCst);
                (current > 0).then_some(current)
            });
            assert!(r.is_item_capability_registered(&ITEM_ITEMS, &backpack));
            assert_eq!(self.calls.load(Ordering::SeqCst), 0);
        }
    }

    struct LaterDefaults;
    #[mod_api::__subscriptions]
    impl Hooks for LaterDefaults {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let earlier = r.block_key("defaults:machine");
            let later = r.block_key("later:machine");
            assert!(r.is_block_capability_registered(&ITEMS, &earlier));
            assert!(!r.is_block_capability_registered(&ITEMS, &later));
            let replacement = crate::ModContainer {
                slots: 9,
                ..Default::default()
            };
            r.block_container(&earlier, replacement.clone());
            r.block_container(&later, replacement);
            assert!(r.is_block_capability_registered(&ITEMS, &later));
        }
    }

    #[test]
    fn native_default_domains_do_not_evaluate_dynamic_item_providers() {
        let slots = Arc::new(AtomicU32::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        crate::__private::part_tests::install_parts_fixture(&[
            ModEntry::new(
                "defaults",
                Box::new(Defaults {
                    slots: slots.clone(),
                    calls: calls.clone(),
                }),
            ),
            ModEntry::new("later", Box::new(LaterDefaults)),
        ])
        .unwrap();
        let host = crate::testing::FakeHost::new();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            mod_vocab::__private::parts_installation::item_container_slots(
                &host,
                "defaults:backpack"
            ),
            None
        );
        slots.store(7, Ordering::SeqCst);
        assert_eq!(
            mod_vocab::__private::parts_installation::item_container_slots(
                &host,
                "defaults:backpack"
            ),
            Some(7)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            mod_vocab::__private::parts_installation::block_container("defaults:machine")
                .unwrap()
                .slots,
            0
        );
        assert_eq!(
            mod_vocab::__private::parts_installation::block_container("later:machine")
                .unwrap()
                .slots,
            9
        );
        assert!(
            mod_vocab::__private::parts_installation::block_fluids("defaults:machine").is_some()
        );
        assert!(
            mod_vocab::__private::parts_installation::block_energy("defaults:machine").is_some()
        );
    }

    #[test]
    fn requested_entity_type_slots_resolve_without_renumbering_and_refuse_unknown_ids() {
        let mut registrar = super::super::PartRegistrar::new("requester");
        let registrar_api = &mut registrar as &mut dyn Registrar;
        let zombie = registrar_api.entity_type_key("minecraft:zombie");
        let pig = registrar_api.entity_type_key("minecraft:pig");
        assert_ne!(zombie, pig);
        super::super::install_registrar(registrar, &crate::__private::part_tests::rule_schema())
            .unwrap();
        mod_vocab::__private::resolve_entity_types(["minecraft:pig", "minecraft:zombie"]).unwrap();
        assert_eq!(
            mod_vocab::__private::entity_type_key_id(zombie).as_deref(),
            Some("minecraft:zombie")
        );
        assert_eq!(
            mod_vocab::__private::entity_type_key_id(pig).as_deref(),
            Some("minecraft:pig")
        );
        assert_eq!(mod_vocab::__private::entity_type_keys(), [pig, zombie]);
        let errors = mod_vocab::__private::resolve_entity_types(["minecraft:pig"]).unwrap_err();
        assert_eq!(
            errors,
            ["mod requester requested unknown entity type minecraft:zombie"]
        );
        assert_eq!(
            mod_vocab::__private::entity_type_key_id(zombie).as_deref(),
            Some("minecraft:zombie")
        );
    }
}
