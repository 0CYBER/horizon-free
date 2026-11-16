use super::*;
struct Registry;
impl DisplayResources for Registry {
    fn book(&self, _: &mod_api::RecipeBookTypeKey) -> bool {
        true
    }
    fn item(&self, id: &str) -> bool {
        id == "minecraft:stone"
    }
    fn stack(&self, _: &mod_api::ItemStack) -> bool {
        false
    }
    fn tag(&self, _: &str) -> bool {
        false
    }
    fn component(&self, _: &str) -> bool {
        false
    }
    fn trim_pattern(&self, _: &str) -> bool {
        false
    }
    fn menu(&self, _: &str) -> bool {
        true
    }
    fn recipe_type(&self, _: &str) -> bool {
        true
    }
    fn recipe(&self, _: &str, _: &str) -> bool {
        true
    }
}
fn batch(info: Option<PlacementInfo>) -> Vec<RecipeBookCategory> {
    let mut wire = mod_api::wire::Writer::new();
    wire.put(&"demo:book".to_owned());
    vec![RecipeBookCategory {
        id: "demo:category".into(),
        recipe_type: "demo:cooking".into(),
        icon: "minecraft:stone".into(),
        target_menu: "demo:pot".into(),
        book: mod_api::wire::Reader::decode(wire.bytes()).unwrap(),
        search_group: "demo:search".into(),
        displays: vec![RecipeBookDisplayEntry {
            recipe: "demo:soup".into(),
            displays: vec![],
            placement: info,
        }],
    }]
}
fn info() -> PlacementInfo {
    PlacementInfo {
        cells: vec![Some(mod_api::Ingredient::of(vec![
            "minecraft:stone".into(),
        ]))],
        shape: None,
    }
}
fn layout() -> crate::recipe_book::slot_layout::CheckedSlotLayout {
    crate::recipe_book::menu_layout::compile(
        mod_api::RecipeBookLayout::Furnace {
            input: 0,
            fuel: 1,
            result: 2,
        },
        crate::recipe_book::menu_layout::MenuLayoutMetadata {
            slots: 3,
            crafting: None,
        },
    )
    .unwrap()
}
#[test]
fn explicit_not_placeable_is_not_missing_authoritative_information() {
    let layout = layout();
    let missing = batch(None);
    let error = compile("demo", "demo:pot", &missing[0].displays[0], &layout).unwrap_err();
    assert!(
        matches!(error, DisplayLoadError::InvalidPlacement(reason) if reason.contains("mod demo") && reason.contains("PlacementInfo"))
    );
    let mut impossible = batch(Some(PlacementInfo::NOT_PLACEABLE));
    validate_batch(&mut impossible, &Registry).unwrap();
    assert!(
        compile("demo", "demo:pot", &impossible[0].displays[0], &layout)
            .unwrap()
            .is_none()
    );
    let mut available = batch(Some(info()));
    validate_batch(&mut available, &Registry).unwrap();
    assert!(
        compile("demo", "demo:pot", &available[0].displays[0], &layout)
            .unwrap()
            .is_some()
    );
}
#[test]
fn item_identity_holder_order_and_shape_are_checked_at_load() {
    let mut unknown = info();
    unknown.cells[0]
        .as_mut()
        .unwrap()
        .items
        .insert("demo:missing".into());
    assert!(validate_batch(&mut batch(Some(unknown)), &Registry).is_err());
    let mut order = info();
    order.cells[0].as_mut().unwrap().order.clear();
    assert!(validate_batch(&mut batch(Some(order)), &Registry).is_err());
    let mut shape = info();
    shape.shape = Some([2, 1]);
    assert!(validate_batch(&mut batch(Some(shape)), &Registry).is_err());
    let mut shape = info();
    shape.shape = Some([1, 1]);
    let mut values = batch(Some(shape));
    validate_batch(&mut values, &Registry).unwrap();
    assert!(compile("demo", "demo:pot", &values[0].displays[0], &layout()).is_err());
}
#[test]
fn rejected_native_ingredient_tree_is_disposed_without_recursive_drop() {
    let mut ingredient = mod_api::Ingredient::of(vec![]);
    for _ in 0..100_000 {
        let mut parent = mod_api::Ingredient::of(vec![]);
        parent.children = Some((mod_api::item::crafting::Shape::Compound, vec![ingredient]));
        ingredient = parent;
    }
    let values = batch(Some(PlacementInfo {
        cells: vec![Some(ingredient)],
        shape: None,
    }));
    assert!(matches!(
        super::super::ModBookIndex::build(values, &Registry),
        Err(DisplayLoadError::InvalidPlacement(_))
    ));
}
#[test]
fn rejected_native_custom_data_is_disposed_without_recursive_drop() {
    let mut tag = horizon_core::nbt::Tag::Int(1);
    for _ in 0..100_000 {
        tag = horizon_core::nbt::Tag::List(vec![tag]);
    }
    let mut value = info();
    value.cells[0].as_mut().unwrap().custom = Some((
        "demo:exact".into(),
        [("data".into(), tag)].into_iter().collect(),
    ));
    assert!(matches!(
        super::super::ModBookIndex::build(batch(Some(value)), &Registry),
        Err(DisplayLoadError::InvalidPlacement(_))
    ));
}
#[test]
fn native_custom_data_keeps_the_independent_vanilla_nbt_depth() {
    for (depth, expected) in [(512, true), (513, false)] {
        let mut tag = horizon_core::nbt::Tag::Int(1);
        for _ in 0..depth {
            tag = horizon_core::nbt::Tag::List(vec![tag]);
        }
        let mut value = info();
        value.cells[0].as_mut().unwrap().custom = Some((
            "demo:exact".into(),
            [("data".into(), tag)].into_iter().collect(),
        ));
        let mut values = batch(Some(value));
        let mut wire = mod_api::wire::Writer::new();
        wire.put(&values);
        let transport =
            mod_api::wire::Reader::decode::<Vec<RecipeBookCategory>>(wire.bytes()).is_ok();
        assert_eq!(transport, expected, "NBT depth {depth}");
        assert_eq!(
            validate_batch(&mut values, &Registry).is_ok(),
            expected,
            "native NBT depth {depth}"
        );
    }
}

#[test]
fn native_depth_acceptance_matches_the_actual_category_wire_envelope() {
    let limit = usize::from(mod_api::wire::MAX_DEPTH);
    for (depth, expected) in [(limit - 5, true), (limit - 4, true), (limit - 3, false)] {
        let mut ingredient = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
        for _ in 0..depth {
            let mut parent = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
            parent.children = Some((mod_api::item::crafting::Shape::Compound, vec![ingredient]));
            ingredient = parent;
        }
        let mut values = batch(Some(PlacementInfo {
            cells: vec![Some(ingredient)],
            shape: None,
        }));
        let mut bytes = mod_api::wire::Writer::new();
        bytes.put(&values);
        let transport =
            mod_api::wire::Reader::decode::<Vec<RecipeBookCategory>>(bytes.bytes()).is_ok();
        assert_eq!(transport, expected, "transport at depth {depth}");
        assert_eq!(
            validate_batch(&mut values, &Registry).is_ok(),
            expected,
            "depth {depth}"
        );
    }
}

#[test]
fn non_simple_placement_accepts_empty_display_order_with_candidates() {
    let mut value = info();
    let ingredient = value.cells[0].as_mut().unwrap();
    ingredient.order.clear();
    ingredient.rule = Some(Box::new(
        mod_api::item::crafting::IngredientRule::Components {
            patch: [("minecraft:damage".into(), horizon_core::nbt::Tag::Int(5))].into(),
            removed: Vec::new(),
            strict: false,
        },
    ));
    validate_batch(&mut batch(Some(value)), &Registry).unwrap();
}
