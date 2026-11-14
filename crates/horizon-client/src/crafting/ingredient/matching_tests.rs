use super::*;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
use std::cell::{Cell, RefCell};
thread_local! { static LIVE: Cell<bool> = const { Cell::new(true) }; static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
fn matching(
    _: &horizon_world::native_load_context::LoopContext,
    kind: &str,
    _: &horizon_core::nbt::Compound,
    stack: &ItemStack,
    accept: &mut bool,
) {
    if kind == "empty_fixture:predicate" {
        CALLS.with_borrow_mut(|calls| calls.push(kind.to_owned()));
        *accept = stack.item.as_ref() == "minecraft:stone" && stack.damage == 7;
    }
    if kind.starts_with("placement_guard:") {
        CALLS.with_borrow_mut(|calls| calls.push(kind.to_owned()));
        *accept = false;
        if kind == "placement_guard:changes_context" {
            LIVE.set(false);
        }
    }
}
fn install() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        assert!(crate::native_stack_hooks::install(
            crate::native_stack_hooks::StackHooks {
                ingredient_test: matching,
                enchantments: |_, _, _| {},
                attributes: |_, _| {}
            }
        ));
    });
}
#[test]
fn a_changed_context_interrupts_a_compound_before_the_next_custom_leaf() {
    install();
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let child = |kind: &str| {
        let mut value = Ingredient::of(vec!["fixture:item".into()]);
        value.custom = Some((kind.into(), Default::default()));
        value
    };
    let mut ingredient = Ingredient::of(vec!["fixture:item".into()]);
    ingredient.children = Some((
        Shape::Compound,
        vec![
            child("placement_guard:changes_context"),
            child("placement_guard:must_not_run"),
        ],
    ));
    assert_eq!(
        ingredient.test_in_guarded(&context, &ItemStack::new("fixture:item", 1), &|| LIVE.get()),
        None
    );
    CALLS.with_borrow(|calls| assert_eq!(calls, &["placement_guard:changes_context"]));
}

fn empty_ingredient_recipes_retain_predicates_but_not_placement() {
    use crate::crafting::{Recipes, ResolvedSources};
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    install();
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let mut items = Items::for_tests(Default::default());
    items
        .item_tag_order
        .insert("empty_fixture:empty".into(), vec![]);
    let inputs = [
        ("direct", json!([]), false, false),
        ("missing", json!("#empty_fixture:missing"), false, false),
        ("tag", json!("#empty_fixture:empty"), true, false),
        (
            "nested_tag",
            json!({"type":"neoforge:compound","children":["#empty_fixture:empty"]}),
            true,
            false,
        ),
        (
            "custom",
            json!({"type":"empty_fixture:predicate"}),
            true,
            true,
        ),
        (
            "nested",
            json!({"type":"neoforge:compound",
            "children":[{"type":"empty_fixture:predicate"}]}),
            true,
            true,
        ),
    ];
    let mut sources: Vec<_> = inputs
        .iter()
        .flat_map(|(id, ingredient, _, _)| {
            [
                (
                    format!("empty_fixture:{id}"),
                    json!({
                        "type":"minecraft:crafting_shaped", "pattern":["A"],
                        "key":{"A":ingredient}, "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_cook"),
                    json!({
                        "type":"minecraft:smelting", "ingredient":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_cut"),
                    json!({
                        "type":"minecraft:stonecutting", "ingredient":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_smith"),
                    json!({
                        "type":"minecraft:smithing_transform",
                        "base":"minecraft:stone", "addition":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
            ]
        })
        .collect();
    sources.push((
        "empty_fixture:visible_neighbor".into(),
        json!({
            "type":"minecraft:crafting_shaped","pattern":["A"],
            "key":{"A":"minecraft:stone"},"result":{"id":"minecraft:stick"}
        }),
    ));
    let resolved = ResolvedSources::resolve(sources, &load).unwrap();
    let root = std::env::temp_dir();
    let recipes = Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &resolved).unwrap();
    let stations = crate::stations::StationRecipes::from_sources(&root, &items, &resolved);
    let mut accepted = ItemStack::new("minecraft:stone", 1);
    accepted.damage = 7;
    let book = crate::recipe_book::Book::build(&recipes, &items);
    let index = crate::recipe_book::RecipeIndex::from_recipes(&recipes);
    let mut learned = crate::recipe_book::Settings::default();
    assert_eq!(book.entries.len(), 1);
    assert_eq!(book.entries[0].id, "empty_fixture:visible_neighbor");
    assert_eq!(
        index.first_display("empty_fixture:visible_neighbor"),
        Some(0)
    );
    for (id, _, retained, matches) in inputs {
        let id = format!("empty_fixture:{id}");
        let entry = recipes.by_type["minecraft:crafting"]
            .iter()
            .find(|r| r.id == id);
        assert_eq!(entry.is_some(), retained, "{id}");
        assert!(!index.contains(&id));
        assert!(book.entry(&id).is_none());
        assert_eq!(
            learned.add_recipes(&index, std::slice::from_ref(&id)),
            (0, vec![])
        );
        assert!(!learned.knows(&id));
        assert_eq!(
            recipes.recipe_matches(&context, &id, &[Some(accepted.clone())], 1),
            matches
        );
        assert!(!recipes.recipe_matches(
            &context,
            &id,
            &[Some(ItemStack::new("minecraft:stone", 1))],
            1
        ));
        assert!(recipes.placement(&id).is_none_or(|p| p.impossible()));
        assert!(
            recipes
                .placement(&format!("{id}_cook"))
                .is_none_or(|p| p.impossible())
        );
        for (kind, suffix) in [
            ("minecraft:smelting", "_cook"),
            ("minecraft:stonecutting", "_cut"),
            ("minecraft:smithing", "_smith"),
        ] {
            assert_eq!(
                recipes.by_type[kind]
                    .iter()
                    .any(|r| r.id == format!("{id}{suffix}")),
                retained
            );
        }
        if let Some(entry) = entry {
            assert!(entry.ingredients[0].items.is_empty());
            if matches {
                assert!(
                    entry.ingredients[0].custom.is_some()
                        || entry.ingredients[0].children.is_some()
                );
            } else {
                assert!(entry.ingredients[0].custom.is_none());
            }
        }
    }
    assert_eq!(
        recipes
            .cook(&context, "minecraft:smelting", &accepted)
            .unwrap()
            .id,
        "empty_fixture:custom_cook"
    );
    assert!(stations.stonecutting.is_empty());
    assert!(stations.stonecutter(&context, &accepted).is_empty());
    assert!(!stations.smithing_accepts(&context, 1, &accepted));
    assert!(!recipes.cooking_accepts(&context, "minecraft:smelting", &accepted));
}

#[test]
fn empty_ingredient_load_diagnostics_distinguish_errors_and_warnings() {
    crate::test_log::install();
    use crate::crafting::Recipes;
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let mut items = Items::for_tests(Default::default());
    items.item_tag_order.insert("fixture:empty".into(), vec![]);
    for (ingredient, expected) in [
        (json!([]), log::Level::Error),
        (json!("#fixture:missing"), log::Level::Error),
        (json!("#fixture:empty"), log::Level::Warn),
        (json!({"type":"fixture:predicate"}), log::Level::Error),
    ] {
        crate::test_log::clear();
        let recipes = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            vec![(
                "fixture:recipe".into(),
                json!({"type":"minecraft:crafting_shaped",
                "pattern":["A"], "key":{"A":ingredient}, "result":{"id":"minecraft:stick"}}),
            )],
            &load,
        )
        .unwrap();
        let levels = crate::test_log::records();
        assert!(levels.iter().any(|(level, _)| *level == expected));
        if ingredient == json!("#fixture:missing") {
            assert!(
                levels
                    .iter()
                    .any(|(_, message)| message.contains("#fixture:missing"))
            );
        }
        let opposite = if expected == log::Level::Warn {
            log::Level::Error
        } else {
            log::Level::Warn
        };
        assert!(!levels.iter().any(|(level, _)| *level == opposite));
        assert_eq!(!recipes.crafting.is_empty(), expected == log::Level::Warn);
    }
}

struct ListenerIngredient(&'static [&'static str]);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ListenerIngredient {
    fn ingredient(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::IngredientEvent<'_>,
    ) {
        event.items.extend(self.0.iter().map(|i| i.to_string()));
    }
}

#[test]
fn unclaimed_custom_kind_fails_the_file_but_an_empty_answer_only_warns() {
    crate::test_log::install();
    use crate::crafting::Recipes;
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    let items = Items::for_tests(Default::default());
    let recipe = || {
        vec![(
            "fixture:needs_mod".to_string(),
            json!({"type":"minecraft:crafting_shapeless",
            "ingredients":[{"type":"absent:kind"}], "result":{"id":"minecraft:stick"}}),
        )]
    };
    let load = || {
        LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        )
    };
    let run = |mods: Vec<mod_api::ModEntry>| {
        crate::test_log::clear();
        let mut loaded = None;
        mod_platform::testing::with_mods(mods.into(), || {
            loaded = Some(
                Recipes::from_sources(
                    &std::env::temp_dir(),
                    &items,
                    &PackOverlay::NONE,
                    recipe(),
                    &load(),
                )
                .unwrap(),
            );
        });
        let logs = crate::test_log::records();
        (loaded.unwrap(), logs)
    };
    // No listener: the file fails at error level, naming file and kind.
    let (recipes, logs) = run(vec![]);
    assert!(recipes.crafting.is_empty());
    assert!(logs.iter().any(|(level, text)| *level == log::Level::Error
        && text.contains("fixture:needs_mod")
        && text.contains("absent:kind")));
    // A listener with an empty answer: kept, with a warning naming both.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "empty",
        Box::new(ListenerIngredient(&[])),
    )]);
    assert_eq!(recipes.crafting.len(), 1);
    assert!(logs.iter().any(|(level, text)| *level == log::Level::Warn
        && text.contains("fixture:needs_mod")
        && text.contains("absent:kind")));
    assert!(!logs.iter().any(|(level, _)| *level == log::Level::Error));
    // A listener with items: loads as before, silently.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "items",
        Box::new(ListenerIngredient(&["minecraft:stick"])),
    )]);
    assert_eq!(recipes.crafting.len(), 1);
    assert!(logs.is_empty());
}

fn cooking_indexes_empty_predicate_descendants_once_in_registration_order() {
    use crate::crafting::Recipes;
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    install();
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let items = Items::for_tests(Default::default());
    for (ingredient, damage, expected) in [
        (
            json!({"type":"neoforge:compound","children":[
            {"type":"empty_fixture:predicate"},"minecraft:dirt"]}),
            7,
            "fixture:a",
        ),
        (
            json!({"type":"neoforge:difference","base":"minecraft:stone",
            "subtracted":{"type":"empty_fixture:predicate"}}),
            7,
            "fixture:b",
        ),
    ] {
        let sources = vec![
            (
                "fixture:a".into(),
                json!({"type":"minecraft:smelting","ingredient":ingredient,
                "result":{"id":"minecraft:stick"}}),
            ),
            (
                "fixture:b".into(),
                json!({"type":"minecraft:smelting","ingredient":"minecraft:stone",
                "result":{"id":"minecraft:diamond"}}),
            ),
        ];
        let recipes = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            sources,
            &load,
        )
        .unwrap();
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack.damage = damage;
        CALLS.with_borrow_mut(Vec::clear);
        assert_eq!(
            recipes
                .cook(&context, "minecraft:smelting", &stack)
                .unwrap()
                .id,
            expected
        );
        CALLS.with_borrow(|calls| assert_eq!(calls.len(), 1));
    }
}

// A mod listens to the `ingredient` hook and answers with no items.
#[test]
fn empty_ingredient_recipes_retain_predicates_but_not_placement_with_a_listener() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "empty",
            Box::new(ListenerIngredient(&[])),
        )]
        .into(),
        empty_ingredient_recipes_retain_predicates_but_not_placement,
    );
}

// A mod listens to the `ingredient` hook and answers with no items.
#[test]
fn cooking_indexes_empty_predicate_descendants_once_in_registration_order_with_a_listener() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "empty",
            Box::new(ListenerIngredient(&[])),
        )]
        .into(),
        cooking_indexes_empty_predicate_descendants_once_in_registration_order,
    );
}
