//! Crafting remainders shared by menus and automated crafters.
use super::*;
impl Recipes {
    /// `Recipe.getRemainingItems` placed on the `size`x`size` grid: an item's
    /// crafting remainder (buckets, bottles), the patterned banner a duplicate
    /// keeps and the written book a clone keeps.
    pub fn remainders(
        &self,
        recipe: &str,
        grid: &[Option<ItemStack>],
        size: usize,
        remainder: &dyn Fn(&ItemStack) -> Option<ItemStack>,
    ) -> Vec<Option<ItemStack>> {
        let mut out = vec![None; size * size];
        let kind = self
            .crafting
            .iter()
            .find(|(n, _)| n == recipe)
            .map(|(_, r)| r);
        if let Some(Recipe::Mod(recipe)) = kind
            && let Some(result) = recipe.remainders(grid, size)
        {
            return result;
        }
        let Some(input) = Input::of(grid, size) else {
            return out;
        };
        for (i, cell) in input.cells.iter().enumerate() {
            let Some(s) = cell else { continue };
            let at = (input.top + i / input.width) * size + input.left + i % input.width;
            let rest = remainder(s);
            if rest.is_some() {
                out[at] = rest;
                continue;
            }
            let copy = ItemStack {
                count: 1,
                ..(*s).clone()
            };
            match kind {
                Some(Recipe::Special(Special::BannerDuplicate { .. }))
                    if !crate::banner::layers(s).is_empty() =>
                {
                    out[at] = Some(copy);
                }
                // BookCloningRecipe stops at the first copied book.
                Some(Recipe::Special(Special::BookCloning { .. }))
                    if s.data().is_some_and(|d| d.written.is_some()) =>
                {
                    out[at] = Some(copy);
                    break;
                }
                _ => {}
            }
        }
        out
    }
}
