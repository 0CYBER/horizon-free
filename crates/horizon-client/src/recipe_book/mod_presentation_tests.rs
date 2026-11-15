use super::*;
use crate::recipe_book::{DisplayResources, ModBookIndex, RecipeBookCategory};
use mod_api::item::recipe_book::RecipeBookTypeKey;
struct Resources;
impl DisplayResources for Resources {
    fn book(&self, _: &RecipeBookTypeKey) -> bool {
        true
    }
    fn item(&self, _: &str) -> bool {
        true
    }
    fn stack(&self, _: &mod_api::ItemStack) -> bool {
        true
    }
    fn tag(&self, _: &str) -> bool {
        true
    }
    fn component(&self, _: &str) -> bool {
        true
    }
    fn trim_pattern(&self, _: &str) -> bool {
        true
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
struct Resolver;
impl DisplayResolver for Resolver {
    fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
        match slot {
            Slot::Item(id) => vec![ItemStack::new(id.as_str(), 1)],
            _ => vec![],
        }
    }
    fn craftable(&self, _: &str, display: &Display, _: &[crate::crafting::Ingredient]) -> bool {
        matches!(display, Display::Smithing { .. })
    }
    fn search_text(&self, stacks: &[ItemStack]) -> String {
        stacks
            .iter()
            .map(|s| s.item.as_ref())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
fn item(id: &str) -> Slot {
    Slot::Item(format!("demo:{id}"))
}
fn layouts() -> Vec<Display> {
    vec![
        Display::Shaped {
            width: 2,
            height: 2,
            ingredients: vec![item("a"), Slot::Empty, item("b"), item("c")],
            result: item("shaped"),
            station: item("station"),
        },
        Display::Shapeless {
            ingredients: vec![item("a"), item("b"), item("c"), item("d")],
            result: item("shapeless"),
            station: item("station"),
        },
        Display::Furnace {
            ingredient: item("a"),
            fuel: item("fuel"),
            result: item("furnace"),
            station: item("station"),
            duration: 200,
            experience: 0.0,
        },
        Display::Stonecutter {
            input: item("b"),
            result: item("stone"),
            station: item("station"),
        },
        Display::Smithing {
            template: item("a"),
            base: item("b"),
            addition: item("c"),
            result: item("smith"),
            station: item("station"),
        },
    ]
}
fn key(id: &str) -> RecipeBookTypeKey {
    let mut writer = mod_api::wire::Writer::new();
    writer.put(&id.to_owned());
    mod_api::wire::Reader::decode(writer.bytes()).unwrap()
}
fn category(book: &str, id: &str, recipe: &str) -> RecipeBookCategory {
    RecipeBookCategory {
        id: id.into(),
        book: key(book),
        target_menu: "demo:menu".into(),
        search_group: format!("{book}_search"),
        icon: "demo:icon".into(),
        recipe_type: "demo:type".into(),
        displays: vec![mod_api::RecipeBookDisplayEntry {
            recipe: recipe.into(),
            displays: layouts(),
            placement: None,
        }],
    }
}
#[test]
fn five_layouts_have_independent_expected_cells() {
    let expected = [
        vec![(0, 0, "demo:a"), (0, 1, "demo:b"), (1, 1, "demo:c")],
        vec![
            (0, 0, "demo:a"),
            (1, 0, "demo:b"),
            (2, 0, "demo:c"),
            (0, 1, "demo:d"),
        ],
        vec![(1, 1, "demo:a")],
        vec![(1, 1, "demo:b")],
        vec![(0, 1, "demo:a"), (1, 1, "demo:b"), (2, 1, "demo:c")],
    ];
    for (display, expected) in layouts().iter().zip(expected) {
        let actual = cells(display, &Resolver);
        let actual: Vec<_> = actual
            .iter()
            .map(|(x, y, s)| (*x, *y, s[0].item.as_ref()))
            .collect();
        assert_eq!(actual, expected);
    }
}
#[test]
fn panel_search_selection_and_inventory_filter_are_isolated() {
    let index = ModBookIndex::build(
        vec![
            category("demo:first", "demo:a", "demo:one"),
            category("demo:first", "demo:b", "demo:two"),
            category("demo:second", "demo:c", "demo:three"),
        ],
        &Resources,
    )
    .unwrap();
    let book = key("demo:first");
    let panel = index.panel("demo:menu", &book).unwrap();
    let prepared = panel.prepare(&Resolver);
    let all = prepared.presentation(None, "", false, 0, &Resolver);
    assert_eq!(all.tab_ids, ["demo:first_search", "demo:a", "demo:b"]);
    assert_eq!(all.tabs.iter().filter(|t| t.selected).count(), 1);
    assert_eq!(
        all.recipes
            .iter()
            .map(|r| r.recipe.as_str())
            .collect::<Vec<_>>(),
        ["demo:one", "demo:two"]
    );
    assert_eq!(all.recipes[0].button.entries.len(), 5);
    assert_eq!(all.recipes[0].sources, [4, 0, 1, 2, 3]);
    assert!(
        prepared
            .presentation(Some("demo:c"), "", false, 0, &Resolver)
            .recipes
            .is_empty()
    );
    assert_eq!(
        prepared
            .presentation(Some("demo:b"), "", false, 0, &Resolver)
            .recipes
            .len(),
        1
    );
    let filtered = prepared.presentation(None, "SMITH", true, 0, &Resolver);
    assert_eq!(filtered.recipes.len(), 2);
    assert_eq!(filtered.recipes[0].button.entries.len(), 1);
    assert_eq!(filtered.recipes[0].sources, [4]);
    assert!(filtered.recipes[0].button.has_craftable);
    assert!(
        prepared
            .presentation(None, "stone", true, 0, &Resolver)
            .recipes
            .is_empty()
    );
}

#[test]
fn stock_changes_only_materialize_visible_previews() {
    use std::cell::Cell;
    struct Counting {
        resolves: Cell<usize>,
        searches: Cell<usize>,
        stock: Cell<bool>,
    }
    impl DisplayResolver for Counting {
        fn resolve(&self, slot: &Slot) -> Vec<ItemStack> {
            self.resolves.set(self.resolves.get() + 1);
            Resolver.resolve(slot)
        }
        fn search_text(&self, stacks: &[ItemStack]) -> String {
            self.searches.set(self.searches.get() + 1);
            Resolver.search_text(stacks)
        }
        fn craftable(
            &self,
            _: &str,
            _: &Display,
            ingredients: &[crate::crafting::Ingredient],
        ) -> bool {
            assert_eq!(ingredients.len(), 1);
            self.stock.get()
        }
    }
    let mut cat = category("demo:book", "demo:category", "demo:first");
    cat.displays = (0..1000)
        .map(|i| mod_api::RecipeBookDisplayEntry {
            recipe: format!("demo:r{i}"),
            placement: None,
            displays: vec![Display::Stonecutter {
                input: item("input"),
                result: item("result"),
                station: item("station"),
            }],
        })
        .collect();
    let index = ModBookIndex::build(vec![cat], &Resources).unwrap();
    let key = key("demo:book");
    let panel = index.panel("demo:menu", &key).unwrap();
    let resolver = Counting {
        resolves: Cell::new(0),
        searches: Cell::new(0),
        stock: Cell::new(false),
    };
    let prepared = panel.prepare(&resolver);
    assert_eq!(resolver.resolves.get(), 2000);
    assert_eq!(resolver.searches.get(), 1000);
    resolver.resolves.set(0);
    for change in 0..10 {
        resolver.stock.set(change % 2 == 0);
        let view = prepared.presentation(None, "", false, 2, &resolver);
        assert_eq!(view.recipes.len(), 1000);
        assert_eq!(view.recipes[40].recipe, "demo:r40");
        assert_eq!(view.recipes[40].button.has_craftable, change % 2 == 0);
        assert_eq!(
            view.recipes[40].overlay[0].cells[0].2[0].item.as_ref(),
            "demo:input"
        );
        assert!(view.recipes[39].overlay.is_empty());
        assert!(view.recipes[60].overlay.is_empty());
    }
    assert_eq!(resolver.resolves.get(), 200);
    assert_eq!(resolver.searches.get(), 1000);
    resolver.stock.set(false);
    let filtered = prepared.presentation(None, "", true, 0, &resolver);
    assert!(filtered.recipes.is_empty());
    assert_eq!(resolver.resolves.get(), 200);
}
