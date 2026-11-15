use super::*;
impl Game<'_> {
    /// The block states mods add to vanilla POI types, once the block registry is resolved.
    pub(in crate::game) fn native_extend_poi_types(
        context: &mod_api::RegistrationContext,
        extensions: &mut dyn PoiTypeExtensions,
    ) {
        Self::fire_extend_poi_types(context, extensions);
    }
}

#[cfg(test)]
mod tests {
    use horizon_world::{chunk::BlockState, poi::PoiExtension, registry::BlockRegistry};
    use mod_api::{
        ExtendPoiTypesEvent, Hooks, Identifier, ModEntry, RegistrationContext, StateConstraint,
        StatePredicate,
    };
    use mod_platform::testing::with_mods;

    struct Tags;
    impl mod_api::Tags for Tags {}

    struct TestMod;
    #[mod_api::__subscriptions]
    impl Hooks for TestMod {
        fn extend_poi_types(&self, _: &RegistrationContext, event: &mut ExtendPoiTypesEvent<'_>) {
            let key = |id: &str| mod_api::__private::block_key(id).unwrap();
            let metadata = event.extensions.metadata();
            let additions = [
                (
                    "minecraft:home",
                    StatePredicate::block(metadata, key("minecraft:hay_block"), &[]).unwrap(),
                ),
                (
                    "minecraft:home",
                    StatePredicate::block(metadata, key("minecraft:bell"), &[]).unwrap(),
                ),
                (
                    "minecraft:meeting",
                    StatePredicate::block(
                        metadata,
                        key("minecraft:red_bed"),
                        &[StateConstraint::new("part", "foot")],
                    )
                    .unwrap(),
                ),
                (
                    "minecraft:nowhere",
                    StatePredicate::block(metadata, key("minecraft:stone"), &[]).unwrap(),
                ),
                (
                    "minecraft:armorer",
                    StatePredicate::tag(
                        metadata,
                        &Tags,
                        mod_api::__private::tag_key(&Identifier::parse("minecraft:beds").unwrap())
                            .unwrap(),
                        &[],
                    )
                    .unwrap(),
                ),
            ];
            for (poi, states) in additions {
                event
                    .extensions
                    .extend(Identifier::parse(poi).unwrap(), states);
            }
        }
    }

    fn state(name: &str, properties: &[(&str, &str)]) -> BlockState {
        BlockState {
            name: name.into(),
            properties: properties
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn a_mods_predicates_become_the_complete_states_it_adds() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:hay_block":3,"minecraft:bell":5,"minecraft:red_bed":6},"states":[
            ["a",true,false,"","minecraft:air"],
            ["s",false,false,"","minecraft:stone"],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"x"}}],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"y"}}],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"z"}}],
            ["b",false,false,"",{"id":"minecraft:bell","properties":{"attachment":"floor"}}],
            ["r",false,false,"",{"id":"minecraft:red_bed","properties":{"part":"head"}}],
            ["r",false,false,"",{"id":"minecraft:red_bed","properties":{"part":"foot"}}]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        crate::game::api_values::resolve_block_definitions(&registry).unwrap();
        with_mods(
            vec![ModEntry::new("poi_test", Box::new(TestMod))].into(),
            || {
                let (additions, refusals) = crate::game::mods::collect_poi_extensions(&registry);
                let addition = |name: &str, properties: &[(&str, &str)], poi: &str| PoiExtension {
                    state: state(name, properties),
                    poi: poi.into(),
                };
                assert_eq!(
                    additions,
                    [
                        addition("minecraft:hay_block", &[("axis", "x")], "minecraft:home"),
                        addition("minecraft:hay_block", &[("axis", "y")], "minecraft:home"),
                        addition("minecraft:hay_block", &[("axis", "z")], "minecraft:home"),
                        addition(
                            "minecraft:bell",
                            &[("attachment", "floor")],
                            "minecraft:home"
                        ),
                        addition(
                            "minecraft:red_bed",
                            &[("part", "foot")],
                            "minecraft:meeting"
                        ),
                        addition("minecraft:stone", &[], "minecraft:nowhere"),
                    ]
                );
                // The tag predicate is the one refusal the engine makes before the table does.
                assert_eq!(refusals.len(), 1, "{refusals:?}");
                assert!(
                    refusals[0].contains("minecraft:armorer") && refusals[0].contains("tag"),
                    "{}",
                    refusals[0]
                );
            },
        );
    }
}
