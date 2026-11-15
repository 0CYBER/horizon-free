#[cfg(test)]
mod block_tests {
    use super::*;
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            assert_eq!(
                mod_vocab::__private::parts_installation::block_key_slot(
                    registrar.block_key("stone")
                ),
                0
            );
        }
    }

    #[test]
    fn completed_blocks_preserve_slots_and_bind_unrequested_states() {
        crate::__private::part_tests::install_parts_fixture(&[ModEntry::new(
            "test",
            Box::new(Requests),
        )])
        .unwrap();
        let definition = |_: &str| {
            horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&Default::default()], 0)
                .map_err(|error| format!("{error:?}"))
        };
        assert!(resolve_block_keys(["minecraft:air"], definition).is_err());
        assert!(block_key("minecraft:air").is_none());
        assert!(
            resolve_block_keys(["minecraft:stone", "minecraft:air"], |_| Err(
                "missing definition".into()
            ))
            .is_err()
        );
        assert!(block_key("minecraft:air").is_none());
        let names = [
            "test:machine",
            "minecraft:zombie_head",
            "minecraft:stone",
            "minecraft:air",
            "minecraft:birch_log",
            "minecraft:air",
        ];
        resolve_block_keys(names, definition).unwrap();
        let stone = block_key("minecraft:stone").unwrap();
        let air = block_key("minecraft:air").unwrap();
        assert_eq!(
            stone,
            mod_vocab::__private::parts_installation::block_key(0)
        );
        assert_eq!(air, mod_vocab::__private::parts_installation::block_key(1));
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_block_key(
                mod_vocab::__private::parts_installation::block_key_slot(air)
            ),
            Ok(air)
        );
        assert_eq!(
            block_key("minecraft:birch_log"),
            Some(mod_vocab::__private::parts_installation::block_key(2))
        );
        assert_eq!(
            block_key("minecraft:zombie_head"),
            Some(mod_vocab::__private::parts_installation::block_key(3))
        );
        assert_eq!(
            block_key("test:machine"),
            Some(mod_vocab::__private::parts_installation::block_key(4))
        );
        assert!(mod_vocab::__private::parts_installation::validate_block_key(5).is_err());
        for name in [
            "",
            "missing:block",
            "minecraft:不存在",
            &"x".repeat(100_000),
        ] {
            assert_eq!(block_key(name), None);
        }
        assert_eq!(block_id(air), Some("minecraft:air"));
        let ordered: Vec<_> = block_keys()
            .into_iter()
            .filter_map(|key| block_id(key))
            .collect();
        assert_eq!(
            ordered,
            [
                "test:machine",
                "minecraft:zombie_head",
                "minecraft:stone",
                "minecraft:air",
                "minecraft:birch_log",
            ]
        );
        // Reversing the registry's enumeration cannot change any completed key.
        crate::__private::part_tests::install_parts_fixture(&[ModEntry::new(
            "test",
            Box::new(Requests),
        )])
        .unwrap();
        resolve_block_keys(names.into_iter().rev(), definition).unwrap();
        for (name, slot) in [
            ("minecraft:stone", 0),
            ("minecraft:air", 1),
            ("minecraft:birch_log", 2),
            ("minecraft:zombie_head", 3),
            ("test:machine", 4),
        ] {
            assert_eq!(
                block_key(name),
                Some(mod_vocab::__private::parts_installation::block_key(slot))
            );
        }
        // Replacing Parts replaces the load-owned identities, not a second global registry.
        crate::__private::install_parts(&[], &crate::__private::part_tests::rule_schema(), |_| {
            false
        })
        .unwrap();
        assert!(block_key("minecraft:air").is_none());
        assert!(
            mod_vocab::__private::parts_installation::validate_block_key(
                mod_vocab::__private::parts_installation::block_key_slot(air)
            )
            .is_err()
        );
    }
}
#[cfg(test)]
mod key_order_tests {
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            let _ = registrar.block_key("minecraft:stone");
        }
    }

    #[test]
    fn registry_order_puts_vanilla_first_and_mods_after_them() {
        crate::__private::part_tests::install_parts_fixture(&[ModEntry::new(
            "test",
            Box::new(Requests),
        )])
        .unwrap();
        super::resolve_block_keys(
            [
                "minecraft:air",
                "minecraft:stone",
                "minecraft:granite",
                "test:machine",
            ],
            |_| {
                horizon_mod_runtime::block_definitions::BlockDefinition::new(
                    &[&Default::default()],
                    0,
                )
                .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        let ordered: Vec<_> = super::block_keys()
            .into_iter()
            .filter_map(|key| mod_vocab::__private::parts_installation::block_id(key))
            .collect();
        assert_eq!(
            ordered,
            [
                "minecraft:air",
                "minecraft:stone",
                "minecraft:granite",
                "test:machine",
            ]
        );
    }
}
#[cfg(test)]
mod ordered_key_tests {
    #[test]
    fn built_in_registry_order_lists_vanilla_first_and_mods_after_them() {
        crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
        super::resolve_entity_types(["minecraft:pig", "minecraft:zombie", "test:probe"]).unwrap();
        // The registry's own order, not sorted: `pig` registers before `zombie`.
        let ordered: Vec<_> = super::entity_type_keys()
            .into_iter()
            .map(|key| super::entity_type_key_id(key).unwrap())
            .collect();
        assert_eq!(ordered, ["minecraft:pig", "minecraft:zombie", "test:probe"]);
        let first = super::entity_type_keys()[0];
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_entity_type_key(
                mod_vocab::__private::parts_installation::entity_type_key_slot(first)
            ),
            Ok(first)
        );
        assert!(
            mod_vocab::__private::parts_installation::validate_entity_type_key(u32::MAX).is_err()
        );
        // A second load of the same registry keeps every key and the order.
        super::resolve_entity_types(["minecraft:pig", "minecraft:zombie", "test:probe"]).unwrap();
        assert_eq!(super::entity_type_keys(), super::entity_type_keys());
        assert_eq!(
            super::entity_type_key_id(first).as_deref(),
            Some("minecraft:pig")
        );
    }

    #[test]
    fn a_reload_keeps_retained_enchantment_slots_and_lists_the_new_order() {
        super::resolve_enchantments(["minecraft:sharpness", "minecraft:mending"]).unwrap();
        let sharpness = super::enchantment_keys()[0];
        super::resolve_enchantments(["minecraft:mending", "minecraft:sharpness"]).unwrap();
        // The retained slot still names sharpness; the listed order follows the reload.
        assert_eq!(
            super::enchantment_key_id(sharpness).as_deref(),
            Some("minecraft:sharpness")
        );
        let ordered: Vec<_> = super::enchantment_keys()
            .into_iter()
            .map(|key| super::enchantment_key_id(key).unwrap())
            .collect();
        assert_eq!(ordered, ["minecraft:mending", "minecraft:sharpness"]);
    }
}
#[cfg(test)]
mod item_tests {
    use super::*;
    use crate::{Hooks, ModEntry, Registrar, RegistrationContext};

    struct Requests;
    #[crate::__subscriptions]
    impl Hooks for Requests {
        fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
            assert_eq!(
                mod_vocab::__private::parts_installation::item_key_slot(
                    registrar.item_key("stick")
                ),
                0
            );
        }
    }

    #[test]
    fn completed_items_preserve_forward_slots_and_refuse_unresolved_requests() {
        crate::__private::install_parts(
            &[ModEntry::new("test", Box::new(Requests))],
            &crate::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        assert!(resolve_items(["minecraft:stone"]).is_err());
        assert!(item_key("minecraft:stone").is_none());
        resolve_items(["minecraft:stone", "minecraft:stick", "minecraft:apple"]).unwrap();
        assert_eq!(
            item_key("minecraft:stick"),
            Some(mod_vocab::__private::parts_installation::item_key(0))
        );
        let apple = item_key("minecraft:apple").unwrap();
        assert_ne!(apple, mod_vocab::__private::parts_installation::item_key(0));
        assert_eq!(
            mod_vocab::__private::parts_installation::validate_item_key(
                mod_vocab::__private::parts_installation::item_key_slot(apple)
            ),
            Ok(apple)
        );
        assert_eq!(item_id(apple), Some("minecraft:apple"));
        assert!(mod_vocab::__private::parts_installation::validate_item_key(3).is_err());
    }
}
#[cfg(test)]
mod hot_call_key_tests {
    use crate::*;
    #[test]
    fn hot_call_signatures_and_unknown_slots_are_typed() {
        let _: fn(&dyn Host, ItemKey) -> Option<i32> = |h, k| h.item_burn_time(k);
        let _: fn(&dyn Host, ItemKey) -> Option<f32> = |h, k| h.item_compost_chance(k);
        let _: fn(&mut dyn ClientHost, ItemKey, u32) -> Result<(), String> = |h, k, n| h.give(k, n);
        let _: fn(&dyn ClientHost, &str, ItemKey) -> bool = |h, e, k| h.enchantment_supports(e, k);
        crate::testing::with_mods(Vec::new().into(), || {
            assert!(mod_vocab::__private::parts_installation::validate_item_key(u32::MAX).is_err());
            assert_eq!(
                __private::item_key_id(mod_vocab::__private::parts_installation::item_key(
                    u32::MAX
                )),
                None
            );
            assert_eq!(
                __private::block_key_id(mod_vocab::__private::parts_installation::block_key(
                    u32::MAX
                )),
                None
            );
        });
    }
}
#[cfg(test)]
mod fluid_mapping_tests {
    #[test]
    fn world_fluid_order_is_supplied_by_engine_not_key_order() {
        crate::__private::part_tests::install_parts_fixture(&[]).unwrap();
        super::resolve_fluids(
            &["minecraft:lava", "minecraft:empty", "minecraft:water"],
            &["minecraft:empty", "minecraft:water", "minecraft:lava"],
        )
        .unwrap();
        for (index, expected) in ["minecraft:empty", "minecraft:water", "minecraft:lava"]
            .iter()
            .enumerate()
        {
            let key = super::world_fluid_key(index).unwrap();
            assert_eq!(super::fluid_id(key), Some(*expected));
        }
        assert!(super::world_fluid_key(3).is_none());
    }
}

#[cfg(test)]
use super::*;

#[cfg(test)]
use mod_vocab::__private::parts_installation::{block_id, item_id};
