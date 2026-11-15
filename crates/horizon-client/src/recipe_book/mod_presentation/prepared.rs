//! Resource-derived metadata survives stock changes. Ingredient previews are
//! resolved only for the current page, after search and inventory filtering.
use super::*;

pub struct PreparedBook {
    tab_ids: Vec<String>,
    icons: Vec<String>,
    recipes: Vec<PreparedRecipe>,
}
struct PreparedRecipe {
    category: usize,
    id: String,
    displays: Vec<PreparedDisplay>,
}
struct PreparedDisplay {
    source: u32,
    display: Display,
    results: Vec<ItemStack>,
    text: String,
    ingredients: Vec<crate::crafting::Ingredient>,
}
impl ModBookPanel<'_> {
    pub fn prepare(&self, resolver: &dyn DisplayResolver) -> PreparedBook {
        let mut book = PreparedBook {
            tab_ids: vec![self.search_group().to_owned()],
            icons: vec!["minecraft:compass".into()],
            recipes: Vec::new(),
        };
        for category in self.categories() {
            let index = book.tab_ids.len();
            book.tab_ids.push(category.id.clone());
            book.icons.push(category.icon.clone());
            for recipe in &category.displays {
                let displays = recipe
                    .displays
                    .iter()
                    .enumerate()
                    .filter_map(|(source, display)| {
                        let results = resolver.resolve(result(display));
                        if results.is_empty() {
                            return None;
                        }
                        let text = resolver.search_text(&results).to_lowercase();
                        Some(PreparedDisplay {
                            source: source as u32,
                            display: display.clone(),
                            results,
                            text,
                            ingredients: ingredients(display, resolver),
                        })
                    })
                    .collect();
                book.recipes.push(PreparedRecipe {
                    category: index,
                    id: recipe.recipe.clone(),
                    displays,
                });
            }
        }
        book
    }
}
impl PreparedBook {
    /// Re-evaluate inventory predicates without rebuilding resource metadata.
    /// Only the selected page resolves ingredients and constructs previews.
    pub fn presentation(
        &self,
        category: Option<&str>,
        query: &str,
        filtering: bool,
        page: usize,
        resolver: &dyn DisplayResolver,
    ) -> ModPresentation {
        let query = query.to_lowercase();
        let mut recipes = Vec::new();
        for recipe in &self.recipes {
            if category.is_some_and(|id| id != self.tab_ids[recipe.category]) {
                continue;
            }
            let mut alternatives: Vec<_> = recipe
                .displays
                .iter()
                .filter_map(
                    |PreparedDisplay {
                         source,
                         display,
                         results,
                         text,
                         ingredients,
                     }| {
                        if !text.contains(&query) {
                            return None;
                        }
                        let craftable = resolver.craftable(&recipe.id, display, ingredients);
                        (!filtering || craftable).then_some((*source, display, results, craftable))
                    },
                )
                .collect();
            if alternatives.is_empty() {
                continue;
            }
            alternatives.sort_by_key(|(_, _, _, craftable)| !craftable);
            let has_craftable = alternatives.iter().any(|(_, _, _, ready)| *ready);
            let sources = alternatives
                .iter()
                .map(|(source, _, _, _)| *source)
                .collect();
            let visible = recipes.len() / 20 == page;
            let (entries, overlay) = if visible {
                alternatives
                    .into_iter()
                    .map(|(_, display, results, craftable)| {
                        (
                            results.clone(),
                            OverlayRecipe {
                                craftable,
                                cells: cells(display, resolver),
                            },
                        )
                    })
                    .unzip()
            } else {
                (Vec::new(), Vec::new())
            };
            recipes.push(PresentedRecipe {
                recipe: recipe.id.clone(),
                sources,
                button: ButtonView {
                    has_craftable,
                    entries,
                    anim: 0.0,
                },
                overlay,
            });
        }
        ModPresentation {
            tab_ids: self.tab_ids.clone(),
            tabs: self
                .icons
                .iter()
                .enumerate()
                .map(|(index, icon)| TabView {
                    icons: (icon.clone(), None),
                    selected: if index == 0 {
                        category.is_none()
                    } else {
                        category == Some(self.tab_ids[index].as_str())
                    },
                    anim: 0.0,
                })
                .collect(),
            recipes,
        }
    }
}

fn ingredients(
    display: &Display,
    resolver: &dyn DisplayResolver,
) -> Vec<crate::crafting::Ingredient> {
    let slots: Vec<&Slot> = match display {
        Display::Shaped { ingredients, .. } | Display::Shapeless { ingredients, .. } => ingredients
            .iter()
            .filter(|slot| !matches!(slot, Slot::Empty))
            .collect(),
        Display::Furnace { ingredient, .. } => vec![ingredient],
        Display::Stonecutter { input, .. } => vec![input],
        Display::Smithing {
            template,
            base,
            addition,
            ..
        } => vec![template, base, addition],
    };
    slots
        .iter()
        .map(|slot| {
            crate::crafting::Ingredient::of(
                resolver
                    .resolve(slot)
                    .into_iter()
                    .map(|stack| stack.item.to_string())
                    .collect(),
            )
        })
        .collect()
}
