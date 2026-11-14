use super::*;
use horizon_protocol::{
    buf::{Codec, Reader},
    packets::recipes::UpdateRecipes,
    registry::Ids,
};
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};
use std::sync::Arc;

struct Sets;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Sets {
    fn recipe_property_sets(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::RecipePropertySetsEvent<'_>,
    ) {
        for (key, items) in [
            (
                "pot:input",
                vec!["minecraft:emerald", "minecraft:air", "missing:item"],
            ),
            ("foreign:input", vec!["minecraft:diamond"]),
            ("minecraft:furnace_input", vec!["minecraft:diamond"]),
        ] {
            event.additions.push(mod_api::RecipePropertyAddition::new(
                key,
                items.into_iter().map(str::to_owned).collect(),
            ));
        }
    }
}
struct Extend;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Extend {
    fn recipe_property_sets(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::RecipePropertySetsEvent<'_>,
    ) {
        event.additions.push(mod_api::RecipePropertyAddition::new(
            "pot:input",
            vec!["minecraft:diamond".into()],
        ));
        event.additions.push(mod_api::RecipePropertyAddition::new(
            "pot:foreign_new",
            vec!["minecraft:diamond".into()],
        ));
    }
}

#[test]
fn custom_sets_survive_dispatch_and_sync_but_new_foreign_keys_do_not() {
    let context = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(
            Default::default(),
            "nonexistent-property-test-settings".into(),
        )),
        ["pot", "extension"],
    );
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("pot", Box::new(Sets)),
            mod_api::ModEntry::new("extension", Box::new(Extend)),
        ]
        .into(),
        || {
            let resolved = ResolvedSources::resolve(vec![], &context).unwrap();
            assert!(resolved.property_additions.get("foreign:input").is_none());
            assert!(resolved.property_additions.get("pot:foreign_new").is_none());
            assert!(
                !resolved
                    .property_additions
                    .adds("pot:input", "minecraft:air")
            );
            let mut sync = crate::lan::data::SyncData::empty();
            sync.builtin.registries.insert(
                "minecraft:item".into(),
                Ids::from_names(vec![
                    "minecraft:air".into(),
                    "minecraft:emerald".into(),
                    "minecraft:diamond".into(),
                ]),
            );
            let bytes = crate::lan::recipes::update_recipes(
                &Default::default(),
                &sync,
                &resolved.property_additions,
            );
            let packet = UpdateRecipes::read(&mut Reader::new(&bytes)).unwrap();
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "pot:input")
                    .unwrap()
                    .1,
                vec![1, 2]
            );
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "minecraft:furnace_input")
                    .unwrap()
                    .1,
                vec![2]
            );
            assert_eq!(packet.properties.len(), 10);
        },
    );
}
