use super::*;
use mod_api::LoopContext;
use std::collections::{HashMap, HashSet};

#[test]
fn typed_item_membership_reaches_the_owning_loop_generation() {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    mod_api::__private::resolve_items(["minecraft:emerald", "minecraft:diamond"]).unwrap();
    let emerald = mod_api::__private::item_key("minecraft:emerald").unwrap();
    let diamond = mod_api::__private::item_key("minecraft:diamond").unwrap();
    let load = engine::LoadContext::new(
        engine::DataLoadOrigin::ClientPreload,
        Arc::new(engine::LoadInputs::new(
            Default::default(),
            Default::default(),
        )),
        [],
    );
    let mut loop_context = load.loop_context(engine::LoopSide::Client, []);
    loop_context.set_recipe_properties(Arc::new(HashMap::from([(
        "pot:input".into(),
        HashSet::from(["minecraft:emerald".into()]),
    )])));
    let adapter = LoopReader(&loop_context);
    let context = adapter.context();
    assert!(context.mod_recipe_property_contains("pot:input", emerald));
    assert!(!context.mod_recipe_property_contains("pot:input", diamond));
    assert!(!context.mod_recipe_property_contains("missing:input", emerald));
}
