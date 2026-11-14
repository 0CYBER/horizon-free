//! Literal registered-kind fixtures used by crafting menu tests.
use super::*;
use std::sync::Mutex;
type Calls = Arc<Mutex<Vec<Vec<u32>>>>;
struct Kind {
    input: mod_api::ItemKey,
    output: mod_api::ItemKey,
    rest: mod_api::ItemKey,
    calls: Calls,
}
impl mod_api::CraftingRecipeKind for Kind {
    type Value = u32;
    fn codec(&self) -> Option<mod_api::CraftingRecipeCodec<u32>> {
        Some(mod_api::CraftingRecipeCodec::wire())
    }
    fn decode(&self, json: &str) -> Result<u32, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        value["rest"]
            .as_u64()
            .filter(|n| *n <= 2)
            .map(|n| n as u32)
            .ok_or("rest must be 0, 1 or 2".into())
    }
    fn write(&self, value: &u32) -> Result<String, String> {
        Ok(format!("{{\"rest\":{value}}}"))
    }
    fn matches(&self, _: &u32, grid: CraftingGrid) -> bool {
        grid.cells.iter().enumerate().all(|(i, cell)| {
            if i == 0 {
                cell.as_ref().is_some_and(|s| s.item() == self.input)
            } else {
                cell.is_none()
            }
        })
    }
    fn assemble(&self, _: &u32, _: CraftingGrid) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(self.output, 3)
    }
    fn remaining_items(
        &self,
        value: &u32,
        grid: CraftingGrid,
    ) -> Option<Vec<Option<mod_api::ItemStack>>> {
        self.calls.lock().unwrap().push(
            grid.cells
                .iter()
                .map(|s| s.as_ref().map_or(0, |s| s.count()))
                .collect(),
        );
        let mut out = vec![None; grid.cells.len()];
        out[0] = match value {
            1 => mod_api::ItemStack::new(self.input, 1),
            2 => mod_api::ItemStack::new(self.rest, 1),
            _ => None,
        };
        Some(out)
    }
}
struct Mod {
    calls: Calls,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for Mod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let kind = Kind {
            input: r.item_key("test:input"),
            output: r.item_key("test:output"),
            rest: r.item_key("test:rest"),
            calls: self.calls.clone(),
        };
        r.register_crafting_recipe_serializer("literal", kind);
    }
}
pub(crate) fn with_recipe(rest: u32, f: impl FnOnce(Recipes, Calls)) {
    let calls: Calls = Default::default();
    let mods = vec![mod_api::ModEntry::new(
        "test",
        Box::new(Mod {
            calls: calls.clone(),
        }),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        let entries = [mod_api::ModEntry::new(
            "test",
            Box::new(Mod {
                calls: calls.clone(),
            }),
        )];
        mod_platform::__private::part_tests::install_parts_fixture(&entries).unwrap();
        mod_api::__private::resolve_items(["test:input", "test:output", "test:rest"]).unwrap();
        let mut recipes = Recipes::empty();
        recipes.mod_crafting = true;
        recipes.crafting.push((
            "test:literal".into(),
            load(
                "test:literal",
                &serde_json::json!({"type":"test:literal", "rest":rest}),
            )
            .unwrap(),
        ));
        f(recipes, calls);
    });
}
#[test]
fn serializer_refusal_skips_known_recipe_and_unknown_refuses_load() {
    with_recipe(0, |_, _| {
        let mut errors = Vec::new();
        assert!(
            load_reported(
                "test:bad.json",
                &serde_json::json!({"type":"test:literal", "rest":99}),
                &mut |e| errors.push(e)
            )
            .is_none()
        );
        assert_eq!(
            errors,
            ["recipe test:bad.json, crafting kind test:literal: rest must be 0, 1 or 2"]
        );
        let error = recipe_type("test:machine", "test:machine").unwrap_err();
        assert_eq!(
            error,
            "recipe data/test/recipe/machine.json, unregistered recipe serializer test:machine"
        );
    });
}

#[test]
fn registered_kind_written_value_syncs_only_to_horizon_clients() {
    with_recipe(2, |recipes, _| {
        let Recipe::Mod(recipe) = &recipes.crafting[0].1 else {
            panic!("registered kind");
        };
        assert_eq!(recipe.written().unwrap(), r#"{"rest":2}"#);
        let sources = vec![(
            "test:literal".into(),
            serde_json::json!({"type":"test:literal", "rest":2}),
        )];
        let mut horizon = crate::lan::recipes::PlayerRecipeSync::default();
        horizon.replace(true, vec!["minecraft:crafting".into()], &sources);
        let received: Vec<_> = horizon
            .payloads
            .iter()
            .flat_map(|part| {
                crate::lan::recipes::ContentPart::decode(part)
                    .unwrap()
                    .recipes
            })
            .collect();
        assert_eq!(received, sources);
        let mut vanilla = crate::lan::recipes::PlayerRecipeSync::default();
        vanilla.replace(false, vec!["minecraft:crafting".into()], &sources);
        assert!(vanilla.payloads.is_empty());
    });
}

struct EmptyKind(mod_api::ItemKey);
impl mod_api::CraftingRecipeKind for EmptyKind {
    type Value = ();
    fn codec(&self) -> Option<mod_api::CraftingRecipeCodec<()>> {
        Some(mod_api::CraftingRecipeCodec::wire())
    }
    fn decode(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn write(&self, _: &()) -> Result<String, String> {
        Ok("{}".into())
    }
    fn matches(&self, _: &(), grid: CraftingGrid) -> bool {
        grid.width == 2 && grid.height == 2 && grid.cells.iter().all(Option::is_none)
    }
    fn assemble(&self, _: &(), _: CraftingGrid) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(self.0, 3)
    }
}
struct EmptyMod;
#[mod_api::__subscriptions]
impl mod_api::Hooks for EmptyMod {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let key = r.item_key("test:output");
        r.register_crafting_recipe_serializer("empty", EmptyKind(key));
    }
}
#[test]
fn registered_kind_receives_the_full_empty_grid_at_every_match_entry() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("test", Box::new(EmptyMod))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
                "test",
                Box::new(EmptyMod),
            )])
            .unwrap();
            mod_api::__private::resolve_items(["test:output"]).unwrap();
            let mut recipes = Recipes::empty();
            recipes.mod_crafting = true;
            recipes.crafting.push((
                "test:empty".into(),
                load("test:empty", &serde_json::json!({"type":"test:empty"})).unwrap(),
            ));
            let grid = vec![None; 4];
            let context = crate::test_resources::server_loop_context();
            let (id, output) = recipes.find(context, &grid, 2).unwrap();
            assert_eq!(id, "test:empty");
            assert_eq!(output, ItemStack::new("test:output", 3));
            assert!(recipes.recipe_matches(context, "test:empty", &grid, 2));
            assert_eq!(
                recipes
                    .find_with_dimensions(context, &grid, 2, 2, 2, &|_| None)
                    .unwrap()
                    .1,
                ItemStack::new("test:output", 3)
            );
            assert_eq!(
                recipes.remainders("test:empty", &grid, 2, &|_| None),
                [None, None, None, None]
            );
        },
    );
}
