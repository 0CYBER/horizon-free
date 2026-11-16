//! Owned custom-book views for the shared recipe-book painter. These are display
//! identities only: they must never enter vanilla placement or ghost-slot paths.
use super::ModBookPanel;
use crate::{
    items::ItemStack,
    recipe_book_screen::{ButtonView, OverlayRecipe, TabView},
};
use mod_api::{RecipeBookDisplay as Display, RecipeSlotDisplay as Slot};
mod prepared;
pub use prepared::PreparedBook;

/// Resource and inventory boundary implemented by the screen owner. Slot
/// resolution must preserve components and alternatives for every API slot form.
pub trait DisplayResolver {
    fn resolve(&self, slot: &Slot) -> Vec<ItemStack>;
    /// Match the actual inventory against the recipe, not merely its icons.
    fn craftable(
        &self,
        recipe: &str,
        display: &Display,
        ingredients: &[crate::crafting::Ingredient],
    ) -> bool;
    /// Localized names and search terms of the resolved result stacks.
    fn search_text(&self, results: &[ItemStack]) -> String;
}

/// One button and the alternatives shown on right click. Recipe ids are never
/// vanilla display ids. The caller paginates these buttons in groups of twenty.
pub struct PresentedRecipe {
    pub recipe: String,
    /// Original indices inside the authoritative recipe entry, after sorting/filtering.
    pub sources: Vec<u32>,
    pub button: ButtonView,
    pub overlay: Vec<OverlayRecipe>,
}

pub struct ModPresentation {
    /// Search first, followed by category ids in stable panel order.
    pub tab_ids: Vec<String>,
    pub tabs: Vec<TabView>,
    pub recipes: Vec<PresentedRecipe>,
}

fn result(display: &Display) -> &Slot {
    match display {
        Display::Shaped { result, .. }
        | Display::Shapeless { result, .. }
        | Display::Furnace { result, .. }
        | Display::Stonecutter { result, .. }
        | Display::Smithing { result, .. } => result,
    }
}

fn cells(display: &Display, resolver: &dyn DisplayResolver) -> Vec<(i32, i32, Vec<ItemStack>)> {
    let mut out = Vec::new();
    let mut push = |x, y, slot| {
        let stacks = resolver.resolve(slot);
        if !stacks.is_empty() {
            out.push((x, y, stacks));
        }
    };
    match display {
        Display::Shaped {
            width,
            height,
            ingredients,
            ..
        } => {
            for (i, slot) in ingredients.iter().enumerate() {
                push(
                    (i % *width as usize) as i32 + i32::from(*width == 1),
                    (i / *width as usize) as i32 + i32::from(*height == 1),
                    slot,
                );
            }
        }
        Display::Shapeless { ingredients, .. } => {
            for (i, slot) in ingredients.iter().enumerate() {
                push((i % 3) as i32, (i / 3) as i32, slot);
            }
        }
        Display::Furnace { ingredient, .. }
        | Display::Stonecutter {
            input: ingredient, ..
        } => push(1, 1, ingredient),
        Display::Smithing {
            template,
            base,
            addition,
            ..
        } => {
            push(0, 1, template);
            push(1, 1, base);
            push(2, 1, addition);
        }
    }
    out
}

#[cfg(test)]
#[path = "mod_presentation_tests.rs"]
mod tests;
