use super::*;

#[test]
fn crafting_remainder_consumption_and_quick_move() {
    use crate::crafting::{CraftingRemainder, Ingredient, Recipe, Template};
    struct Tool;
    impl CraftingRemainder for Tool {
        fn crafting_remainder(&self, s: &ItemStack) -> Option<ItemStack> {
            (s.damage < 4).then(|| ItemStack {
                count: 1,
                damage: s.damage + 1,
                ..s.clone()
            })
        }
    }
    bare_env(|base| {
        let mut recipes = Recipes::empty();
        recipes.crafting.push((
            "tool".into(),
            Recipe::Shapeless {
                ingredients: vec![Ingredient::of(vec!["test:tool".into()])],
                result: Template {
                    item: Some("test:result".into()),
                    count: 1,
                    damage: 0,
                    extra: None,
                    components: None,
                },
            },
        ));
        let env = Env {
            recipes: &recipes,
            crafting_remainder: Some(&Tool),
            ..*base
        };
        for (kind, cell) in [(Kind::Crafting, 4), (Kind::Inventory, 3)] {
            let mut menu = Menu::new(kind);
            menu.set_slot(
                &env,
                cell + 1,
                Some(ItemStack {
                    damage: 2,
                    ..ItemStack::new("test:tool", 1)
                }),
            );
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(cell + 1).map(|s| (s.damage, s.count)),
                Some((3, 1))
            );
            menu.carried = None;
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert_eq!(
                menu.slot(cell + 1).map(|s| (s.damage, s.count)),
                Some((4, 1))
            );
            menu.carried = None;
            menu.click(&env, 0, 0, Input::Pickup).unwrap();
            assert!(menu.slot(cell + 1).is_none());
            let mut menu = Menu::new(kind);
            menu.set_slot(
                &env,
                cell + 1,
                Some(ItemStack {
                    damage: 2,
                    ..ItemStack::new("test:tool", 1)
                }),
            );
            menu.click(&env, 0, 0, Input::QuickMove).unwrap();
            assert!(menu.slot(cell + 1).is_none());
            assert_eq!(
                menu.containers[PLAYER]
                    .iter()
                    .flatten()
                    .filter(|s| &*s.item == "test:result")
                    .map(|s| s.count)
                    .sum::<u32>(),
                3
            );
        }
    });
}

#[test]
fn registered_kind_shift_click_asks_once_per_pre_take_grid() {
    crate::crafting::mod_kinds::tests::with_recipe(0, |recipes, calls| {
        bare_env(|base| {
            let env = Env {
                recipes: &recipes,
                ..*base
            };
            for kind in [Kind::Inventory, Kind::Crafting] {
                calls.lock().unwrap().clear();
                let mut menu = Menu::new(kind);
                menu.set_slot(&env, 1, Some(ItemStack::new("test:input", 2)));
                menu.click(&env, 0, 0, Input::QuickMove).unwrap();
                assert!(menu.slot(1).is_none());
                assert_eq!(
                    menu.containers[PLAYER]
                        .iter()
                        .flatten()
                        .filter(|s| &*s.item == "test:output")
                        .map(|s| s.count)
                        .sum::<u32>(),
                    6
                );
                let counts: Vec<_> = calls.lock().unwrap().iter().map(|grid| grid[0]).collect();
                assert_eq!(counts, [2, 1]);
            }
        });
    });
}

#[test]
fn registered_kind_remainder_merges_or_enters_inventory_or_drops() {
    for (mode, full) in [(1, false), (2, false), (2, true)] {
        crate::crafting::mod_kinds::tests::with_recipe(mode, |recipes, calls| {
            bare_env(|base| {
                let env = Env {
                    recipes: &recipes,
                    ..*base
                };
                let mut menu = Menu::new(Kind::Crafting);
                if full {
                    for slot in &mut menu.containers[PLAYER] {
                        *slot = Some(ItemStack::new("test:blocked", 64));
                    }
                }
                menu.set_slot(&env, 1, Some(ItemStack::new("test:input", 2)));
                menu.click(&env, 0, 0, Input::Pickup).unwrap();
                assert_eq!(calls.lock().unwrap().len(), 1);
                assert_eq!(menu.slot(1).unwrap().count, if mode == 1 { 2 } else { 1 });
                if mode == 2 && full {
                    assert_eq!(menu.dropped, [ItemStack::new("test:rest", 1)]);
                } else if mode == 2 {
                    assert_eq!(
                        menu.containers[PLAYER]
                            .iter()
                            .flatten()
                            .filter(|s| &*s.item == "test:rest")
                            .map(|s| s.count)
                            .sum::<u32>(),
                        1
                    );
                    assert!(menu.dropped.is_empty());
                }
            });
        });
    }
}

#[test]
fn registered_kind_stale_result_takes_nothing_after_grid_change() {
    crate::crafting::mod_kinds::tests::with_recipe(0, |recipes, calls| {
        bare_env(|base| {
            let env = Env {
                recipes: &recipes,
                ..*base
            };
            for kind in [Kind::Crafting, mod_crafting_table()] {
                let mut menu = Menu::new(kind);
                menu.set_slot(&env, 1, Some(ItemStack::new("test:input", 2)));
                assert_eq!(menu.slot(0), Some(&ItemStack::new("test:output", 3)));
                menu.containers[GRID][0] = Some(ItemStack::new("test:rest", 2));
                menu.click(&env, 0, 0, Input::Pickup).unwrap();
                assert_eq!(
                    menu.containers[GRID][0],
                    Some(ItemStack::new("test:rest", 2))
                );
                assert!(menu.carried.is_none());
                assert!(calls.lock().unwrap().is_empty());
            }
        });
    });
}

#[test]
fn registered_kind_reload_removing_last_recipe_rejects_cached_result() {
    crate::crafting::mod_kinds::tests::with_recipe(0, |recipes, calls| {
        bare_env(|base| {
            let old = Env {
                recipes: &recipes,
                ..*base
            };
            for kind in [Kind::Crafting, mod_crafting_table()] {
                let mut menu = Menu::new(kind);
                menu.set_slot(&old, 1, Some(ItemStack::new("test:input", 2)));
                assert_eq!(menu.slot(0), Some(&ItemStack::new("test:output", 3)));
                let root = tempfile::tempdir().unwrap();
                std::fs::create_dir_all(root.path().join("data/minecraft/recipe")).unwrap();
                let reloaded = Recipes::from_sources(
                    root.path(),
                    base.items,
                    &PackOverlay::NONE,
                    Vec::new(),
                    &crate::test_resources::server_load(),
                )
                .unwrap();
                assert!(reloaded.crafting.is_empty());
                let current = Env {
                    recipes: &reloaded,
                    ..*base
                };
                menu.click(&current, 0, 0, Input::Pickup).unwrap();
                assert!(menu.carried.is_none());
                assert_eq!(menu.slot(1), Some(&ItemStack::new("test:input", 2)));
                assert!(calls.lock().unwrap().is_empty());
                assert_eq!(
                    mod_api::__private::parts_installation::crafting_recipe_kinds(),
                    ["test:literal"]
                );
            }
        });
    });
}
