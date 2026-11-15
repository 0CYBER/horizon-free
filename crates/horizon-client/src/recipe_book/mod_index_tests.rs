use super::*;
struct Resources;
impl DisplayResources for Resources {
    fn book(&self, key: &mod_api::item::recipe_book::RecipeBookTypeKey) -> bool {
        matches!(key.as_str(), "demo:pot" | "demo:oven")
    }
    fn item(&self, id: &str) -> bool {
        id == "minecraft:stone"
    }
    fn stack(&self, _: &mod_api::ItemStack) -> bool {
        false
    }
    fn tag(&self, id: &str) -> bool {
        id == "minecraft:logs"
    }
    fn component(&self, id: &str) -> bool {
        id == "minecraft:damage"
    }
    fn trim_pattern(&self, id: &str) -> bool {
        id == "minecraft:sentry"
    }
    fn menu(&self, id: &str) -> bool {
        matches!(id, "demo:pot" | "demo:oven")
    }
    fn recipe_type(&self, kind: &str) -> bool {
        kind == "demo:cooking"
    }
    fn recipe(&self, id: &str, kind: &str) -> bool {
        id == "demo:soup" && kind == "demo:cooking"
    }
}
fn category(book: &str, id: &str) -> RecipeBookCategory {
    RecipeBookCategory {
        id: id.into(),
        recipe_type: "demo:cooking".into(),
        icon: "minecraft:stone".into(),
        target_menu: book.into(),
        book: {
            // The key is opaque outside registration. Decode its owned id as a fixture;
            // Resources models the frozen registry consulted by the load boundary.
            let mut wire = mod_api::wire::Writer::new();
            wire.put(&book.to_owned());
            mod_api::wire::Reader::decode(wire.bytes()).unwrap()
        },
        search_group: format!("{book}_search"),
        displays: Vec::new(),
    }
}
fn item() -> Slot {
    Slot::Item("minecraft:stone".into())
}
fn shaped(ingredients: Vec<Slot>) -> Display {
    Display::Shaped {
        width: 1,
        height: 1,
        ingredients,
        result: item(),
        station: item(),
    }
}
#[test]
fn each_search_contains_exactly_its_books_categories() {
    let index = ModBookIndex::build(
        vec![
            category("demo:pot", "demo:soups"),
            category("demo:oven", "demo:breads"),
            category("demo:pot", "demo:stews"),
        ],
        &Resources,
    )
    .unwrap();
    assert_eq!(
        index.search_categories("demo:pot"),
        ["demo:soups", "demo:stews"]
    );
    assert_eq!(index.search_categories("demo:oven"), ["demo:breads"]);
    assert!(index.search_categories("minecraft:crafting").is_empty());
    assert!(index.search_categories("demo:absent").is_empty());
    assert_eq!(
        index.category("demo:soups").unwrap().target_menu,
        "demo:pot"
    );
}
#[test]
fn invalid_declarations_refuse_the_whole_generation() {
    let valid = category("demo:pot", "demo:soups");
    for id in [
        "bare",
        "demo:",
        ":soups",
        "demo:Bad",
        "demo:bad:id",
        "minecraft:crafting_misc",
    ] {
        let mut invalid = valid.clone();
        invalid.id = id.into();
        assert!(matches!(
            ModBookIndex::build(vec![valid.clone(), invalid], &Resources),
            Err(DisplayLoadError::InvalidId(_))
        ));
    }
    for icon in ["", "minecraft:air", "demo:absent"] {
        let mut invalid = valid.clone();
        invalid.id = "demo:stews".into();
        invalid.icon = icon.into();
        assert!(matches!(
            ModBookIndex::build(vec![valid.clone(), invalid], &Resources),
            Err(DisplayLoadError::UnknownResource { kind: "item", .. })
        ));
    }
    let mut second = valid.clone();
    second.id = "demo:stews".into();
    let index = ModBookIndex::build(vec![valid.clone(), second], &Resources).unwrap();
    assert_eq!(
        index
            .panel("demo:pot", &valid.book)
            .unwrap()
            .categories()
            .count(),
        2
    );
}
#[test]
fn panels_reject_wrong_menu_and_stale_tabs() {
    let pot = category("demo:pot", "demo:soups");
    let oven = category("demo:oven", "demo:breads");
    let index = ModBookIndex::build(vec![pot.clone(), oven.clone()], &Resources).unwrap();
    let panel = index.panel("demo:pot", &pot.book).unwrap();
    assert_eq!(panel.search_group(), "demo:pot_search");
    assert_eq!(
        panel
            .categories()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["demo:soups"]
    );
    assert!(panel.category("demo:breads").is_none());
    assert_eq!(
        panel.category("demo:soups").unwrap().icon,
        "minecraft:stone"
    );
    assert!(index.panel("demo:oven", &pot.book).is_none());
    assert!(index.panel("demo:pot", &oven.book).is_none());
    let mut invalid = pot;
    invalid.recipe_type = "demo:absent".into();
    assert!(matches!(
        ModBookIndex::build(vec![invalid], &Resources),
        Err(DisplayLoadError::UnknownResource {
            kind: "recipe type",
            ..
        })
    ));
}
#[test]
fn shared_search_or_mismatched_menu_cannot_merge_books() {
    let a = category("demo:pot", "demo:soups");
    let mut b = category("demo:oven", "demo:breads");
    b.search_group = a.search_group.clone();
    assert!(matches!(
        ModBookIndex::build(vec![a.clone(), b], &Resources),
        Err(DisplayLoadError::ConflictingBook(_))
    ));
    let mut b = category("demo:pot", "demo:stews");
    b.target_menu = "demo:oven".into();
    assert!(matches!(
        ModBookIndex::build(vec![a.clone(), b], &Resources),
        Err(DisplayLoadError::ConflictingBook(_))
    ));
    assert!(matches!(
        ModBookIndex::build(vec![a.clone(), a], &Resources),
        Err(DisplayLoadError::DuplicateCategory(_))
    ));
}
#[test]
fn all_five_display_kinds_validate_and_unknown_nested_items_are_typed_errors() {
    let displays = vec![
        shaped(vec![item()]),
        Display::Shapeless {
            ingredients: vec![Slot::Tag("minecraft:logs".into())],
            result: item(),
            station: item(),
        },
        Display::Furnace {
            ingredient: item(),
            fuel: Slot::AnyFuel,
            result: item(),
            station: item(),
            duration: 200,
            experience: 0.7,
        },
        Display::Stonecutter {
            input: item(),
            result: item(),
            station: item(),
        },
        Display::Smithing {
            template: item(),
            base: item(),
            addition: item(),
            result: item(),
            station: item(),
        },
    ];
    let mut a = category("demo:pot", "demo:soups");
    a.displays.push(mod_api::RecipeBookDisplayEntry {
        recipe: "demo:soup".into(),
        displays,
        placement: None,
    });
    assert!(ModBookIndex::build(vec![a.clone()], &Resources).is_ok());
    a.displays[0]
        .displays
        .push(shaped(vec![Slot::WithRemainder {
            input: Box::new(item()),
            remainder: Box::new(Slot::Composite(vec![Slot::Item("demo:missing".into())])),
        }]));
    assert!(
        matches!(ModBookIndex::build(vec![a], &Resources), Err(DisplayLoadError::UnknownResource { kind: "item", id }) if id == "demo:missing")
    );
}
#[test]
fn malformed_layouts_and_wrong_recipe_ownership_are_refused() {
    assert!(matches!(
        validate_display(&shaped(Vec::new()), "demo:soup", &Resources),
        Err(DisplayLoadError::InvalidLayout(_))
    ));
    let mut nested = item();
    for _ in 0..34 {
        nested = Slot::WithAnyPotion(Box::new(nested));
    }
    assert!(matches!(
        validate_display(&shaped(vec![nested]), "demo:soup", &Resources),
        Err(DisplayLoadError::InvalidLayout(_))
    ));
    let mut a = category("demo:pot", "demo:soups");
    a.displays.push(mod_api::RecipeBookDisplayEntry {
        recipe: "demo:wrong_type".into(),
        displays: vec![shaped(vec![item()])],
        placement: None,
    });
    assert!(matches!(
        ModBookIndex::build(vec![a], &Resources),
        Err(DisplayLoadError::UnknownResource { kind: "recipe", .. })
    ));
}
