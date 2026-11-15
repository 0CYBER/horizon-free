//! Indexed cooking recipe lookup.
use super::{CookIndex, Cooking, ItemStack, LoopContext, Recipes};

impl Recipes {
    pub fn cook(&self, context: &LoopContext, kind: &str, input: &ItemStack) -> Option<&Cooking> {
        self.cooking_lookup(context, kind, input, false)
    }
    pub(super) fn cook_placeable(
        &self,
        context: &LoopContext,
        kind: &str,
        input: &ItemStack,
    ) -> Option<&Cooking> {
        self.cooking_lookup(context, kind, input, true)
    }
    fn cooking_lookup(
        &self,
        context: &LoopContext,
        kind: &str,
        input: &ItemStack,
        placeable: bool,
    ) -> Option<&Cooking> {
        // Every recipe an item names, not just the first: a mod's match-time
        // hook may refuse the one the first recipe would take (`Ingredient::test`).
        let index = self.cook_index.get_or_init(|| {
            let mut index = CookIndex::new();
            for (i, c) in self.cooking.iter().enumerate() {
                let of_kind = index.entry(c.kind.clone()).or_default();
                for item in &c.ingredient.items {
                    of_kind.entry(item.clone()).or_default().push(i);
                }
                if !c.ingredient.is_simple() {
                    // Empty string cannot be an item id. This bucket retains
                    // all concrete predicates, including ones whose accepted stacks
                    // are absent from the default-stack display list.
                    of_kind.entry(String::new()).or_default().push(i);
                }
            }
            index
        });
        let index = index.get(kind)?;
        let mut named = index
            .get(&*input.item)
            .into_iter()
            .flatten()
            .copied()
            .peekable();
        let mut predicates = (!placeable)
            .then(|| index.get(""))
            .flatten()
            .into_iter()
            .flatten()
            .copied()
            .peekable();
        loop {
            // Both buckets are in recipe registration order. Merge without
            // allocation so a custom predicate does not change first-match order.
            let next = match (named.peek(), predicates.peek()) {
                (Some(a), Some(b)) if a == b => {
                    predicates.next();
                    named.next()
                }
                (Some(a), Some(b)) if a < b => named.next(),
                (Some(_), Some(_)) | (None, Some(_)) => predicates.next(),
                (Some(_), None) => named.next(),
                (None, None) => return None,
            }?;
            let recipe = &self.cooking[next];
            if recipe.ingredient.test_in(context, input) {
                return Some(recipe);
            }
        }
    }
    /// `cook` by scanning every recipe, as it was before the index.
    #[cfg(test)]
    pub(super) fn cook_scan(
        &self,
        context: &LoopContext,
        kind: &str,
        input: &ItemStack,
    ) -> Option<&Cooking> {
        self.cooking
            .iter()
            .find(|c| c.kind == kind && c.ingredient.test_in(context, input))
    }
}
